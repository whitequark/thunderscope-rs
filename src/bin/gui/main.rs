use std::io::Read;
use std::num::NonZeroU32;
use std::thread;
use std::time::Duration;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};

use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::event::{StartCause, WindowEvent};
use winit::window::{Window, WindowId};
use winit::raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;

use glutin_winit::DisplayBuilder;

use glutin::config::ConfigTemplateBuilder;
use glutin::context::{Version, ContextApi, ContextAttributesBuilder};
use glutin::context::{NotCurrentGlContext, PossiblyCurrentContext};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, WindowSurface};
use glutin::display::{GetGlDisplay, GlDisplay};

use glow::{Context as GlowContext, HasContext};

use thunderscope::EdgeFilter;

const TRIGGER_EDGE: EdgeFilter = EdgeFilter::Rising;
const TRIGGER_LEVEL: i8 = 50;
const SAMPLE_COUNT: usize = 128_000;
const RENDER_LINES: bool = true;

#[derive(Debug)]
struct Waveform {
    buffer: thunderscope::RingBuffer,
    capture: Option<(thunderscope::RingCursor, usize)>
}

impl Waveform {
    pub fn new(size: usize) -> thunderscope::Result<Waveform> {
        let buffer = thunderscope::RingBuffer::new(size)?;
        Ok(Waveform { buffer, capture: None })
    }

    pub fn capture_data(&self) -> Option<&[i8]> {
        self.capture.map(|(cursor, length)| self.buffer.read(cursor, length))
    }
}

struct Sampler {
    // Sampler does not allocate the waveform buffers. It relies on a pair of channels acting like
    // a bucket brigade: any received `Waveform` objects are filled in with captures and sent for
    // further processing. Eventually the `Waveform` object comes back from the processing engine,
    // and the closed cycle continues.
    instrument: thunderscope::Device,
    waveform_recv: Receiver<Waveform>,
    waveform_send: Sender<Waveform>,
}

impl Sampler {
    pub fn new(
        instrument: thunderscope::Device,
        waveform_recv: Receiver<Waveform>,
        waveform_send: Sender<Waveform>
    ) -> Sampler {
        Sampler { instrument, waveform_recv, waveform_send }
    }

    pub fn run(mut self) -> std::thread::JoinHandle<thunderscope::Result<()>> {
        thread::spawn(move || {
            self.instrument.startup()?;
            self.instrument.configure(&thunderscope::DeviceParameters::derive(
                &thunderscope::DeviceCalibration::default(),
                &thunderscope::DeviceConfiguration {
                    channels: [Some(thunderscope::ChannelConfiguration {
                        ..Default::default()
                    }), None, None, None]
                }))?;
            self.trigger_and_capture()?;
            self.instrument.shutdown()?;
            Ok(())
        })
    }

    fn trigger_and_capture(&mut self) -> thunderscope::Result<()> {
        let mut reader = self.instrument.stream_data();
        let mut trigger = thunderscope::Trigger::new(TRIGGER_LEVEL, 2);
        // prime the queue
        let mut waveform = self.waveform_recv.recv().expect("failed to receive waveform");
        loop {
            waveform.capture = None;
            let buffer = &mut waveform.buffer;
            let mut cursor = buffer.cursor();
            let mut available = 0;
            // refill buffer
            let refill_by = buffer.len() - available;
            available += buffer.append(refill_by, |slice| reader.read(slice))?;
            log::debug!("sampler: refilled buffer for trigger by {} bytes ({} available)",
                refill_by, available);
            // find trigger
            let data = buffer.read(cursor, available);
            let (processed, edge) = trigger.find(data, TRIGGER_EDGE);
            cursor += processed;
            available -= processed;
            log::debug!("sampler: trigger consumed {} bytes ({} available)",
                processed, available);
            if let Some(edge) = edge {
                // check if we need to capture more
                if available < SAMPLE_COUNT {
                    let refill_by = SAMPLE_COUNT - available;
                    available += buffer.append(refill_by, |slice| reader.read(slice))?;
                    debug_assert!(available >= SAMPLE_COUNT);
                    log::debug!("sampler: refilled buffer for capture by {} bytes ({} available)",
                        refill_by, available);
                }
                // submit data for processing
                waveform.capture = Some((cursor, SAMPLE_COUNT));
                log::debug!("sampler: captured waveform for {:?} edge ({}+{})",
                    edge, cursor.into_inner(), SAMPLE_COUNT);
                match self.waveform_recv.try_recv() {
                    Ok(new_waveform) => {
                        self.waveform_send.send(waveform).expect("failed to send waveform");
                        log::debug!("sampler: submitted waveform");
                        waveform = new_waveform;
                    }
                    Err(TryRecvError::Empty) =>
                        log::debug!("sampler: discarded waveform"),
                    Err(TryRecvError::Disconnected) => {
                        log::debug!("sampler: done");
                        break
                    }
                }
                // reset trigger to resynchronize its state
                trigger.reset();
            }
        }
        Ok(())
    }
}

struct Renderer {
    program: <glow::Context as HasContext>::Program,
    vertex_array: <glow::Context as HasContext>::VertexArray,
    sample_array: <glow::Context as HasContext>::Buffer,
    waveform_recv: Receiver<Waveform>,
    waveform_send: Sender<Waveform>,
    waveform: Option<Waveform>
}

impl Renderer {
    pub fn new(
        gl: &glow::Context,
        waveform_recv: Receiver<Waveform>,
        waveform_send: Sender<Waveform>
    ) -> Self {
        let shaders = [
            (glow::VERTEX_SHADER,   include_str!("wave_vert.glsl")),
            (glow::FRAGMENT_SHADER, include_str!("wave_frag.glsl")),
        ];

        unsafe {
            let program = gl.create_program().expect("failed to create program");
            let mut native_shaders = Vec::new();
            for (kind, source) in shaders {
                let shader = gl.create_shader(kind).expect("failed to create shader");
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    panic!("could not compile shader: {}", gl.get_shader_info_log(shader));
                }
                gl.attach_shader(program, shader);
                native_shaders.push(shader);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                panic!("{}", gl.get_program_info_log(program));
            }
            for shader in native_shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }

            let vertex_array = gl.create_vertex_array().expect("failed to create vertex array");
            let data_array = gl.create_buffer().expect("failed to create buffer");
            Self {
                program,
                vertex_array,
                sample_array: data_array,
                waveform_recv,
                waveform_send,
                waveform: None
            }
        }
    }

    pub fn poll(&mut self) -> bool {
        match self.waveform_recv.try_recv() {
            err @ Err(TryRecvError::Disconnected) =>
                panic!("renderer: failed to receive waveform: {:?}", err),
            Err(TryRecvError::Empty) => false,
            Ok(new_waveform) => {
                log::debug!("renderer: acquired waveform");
                if let Some(old_waveform) = self.waveform.replace(new_waveform) {
                    self.waveform_send.send(old_waveform).expect("failed to return waveform");
                }
                true
            }
        }
    }

    pub fn resize(&mut self, gl: &glow::Context, width: u32, height: u32) {
        unsafe {
            gl.viewport(0, 0, width as i32, height as i32);
            gl.use_program(Some(self.program));
            gl.uniform_2_f32(gl.get_uniform_location(self.program, "resolution").as_ref(),
                width as f32, height as f32);
        }
    }

    pub fn render(&mut self, gl: &glow::Context) {
        let Some(samples) = self.waveform.as_ref()
            .and_then(|wfm| wfm.capture_data())
            .map(|data| bytemuck::cast_slice(data)) else { return };
        unsafe {
            let draw_lines_loc = gl.get_uniform_location(self.program, "draw_lines");
            let channel_color_loc = gl.get_uniform_location(self.program, "channel_color");
            let sample_count_loc = gl.get_uniform_location(self.program, "sample_count");
            let sample_value0_loc = gl.get_attrib_location(self.program, "sample_value0")
                .expect("could not retrieve attribute location");
            let sample_value1_loc = gl.get_attrib_location(self.program, "sample_value1")
                .expect("could not retrieve attribute location");

            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.enable(glow::BLEND);

            gl.use_program(Some(self.program));
            gl.uniform_1_u32(draw_lines_loc.as_ref(), RENDER_LINES as u32);
            gl.uniform_3_f32(channel_color_loc.as_ref(), 1.0, 1.0, 0.0);
            gl.uniform_1_i32(sample_count_loc.as_ref(), samples.len() as i32);
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.sample_array));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, samples, glow::STREAM_DRAW);
            gl.enable_vertex_attrib_array(sample_value0_loc);
            gl.vertex_attrib_pointer_f32(sample_value0_loc, 1, glow::BYTE, true, 1, 0);
            gl.vertex_attrib_divisor(sample_value0_loc, 1);
            gl.enable_vertex_attrib_array(sample_value1_loc);
            gl.vertex_attrib_pointer_f32(sample_value1_loc, 1, glow::BYTE, true, 1, 1);
            gl.vertex_attrib_divisor(sample_value1_loc, 1);
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, samples.len() as i32);
            gl.disable_vertex_attrib_array(sample_value0_loc);
            gl.disable_vertex_attrib_array(sample_value1_loc);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            gl.disable(glow::BLEND);
        }
    }

    pub fn destroy(&mut self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vertex_array);
        }
    }
}

struct Application {
    gl_context: PossiblyCurrentContext,
    gl_surface: Surface<WindowSurface>,
    gl_library: GlowContext,
    renderer: Renderer,
    window: Window,
}

impl ApplicationHandler for Application {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(&mut self, event_loop: &ActiveEventLoop,
            _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(size) if size.width != 0 && size.height != 0 => {
                self.renderer.resize(&self.gl_library, size.width, size.height);
                self.gl_surface.resize(&self.gl_context,
                    NonZeroU32::new(size.width).unwrap(),
                    NonZeroU32::new(size.height).unwrap(),
                );
            }
            WindowEvent::RedrawRequested => {
                self.window.pre_present_notify();
                let gl = &self.gl_library;
                unsafe {
                    gl.clear_color(0.1, 0.0, 0.1, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                }
                self.renderer.render(&self.gl_library);
                self.gl_surface.swap_buffers(&self.gl_context)
                    .expect("failed to swap buffers");
            }
            WindowEvent::CloseRequested => {
                self.renderer.destroy(&self.gl_library);
                event_loop.exit();
            }
            _ => ()
        }
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        match cause {
            StartCause::ResumeTimeReached { .. } => {
                if self.renderer.poll() {
                    self.window.request_redraw();
                }
                // The `winit` documentation recommends `Poll`, but if no waveforms are acquired,
                // this results in a busy loop waiting on `self.renderer.poll()`, pegging a core.
                // A 5 ms delay should be enough for even a 200 Hz display.
                event_loop.set_control_flow(ControlFlow::wait_duration(Duration::from_millis(5)));
            }
            _ => ()
        }
    }
}

fn main() {
    env_logger::Builder::from_default_env()
        .format_timestamp_micros()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();
    // create a window
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::wait_duration(Duration::ZERO));
    let attributes = Window::default_attributes()
        .with_title("ThunderScope");
    let config_template = ConfigTemplateBuilder::new()
        .prefer_hardware_accelerated(Some(true));
    let (window, gl_config) = DisplayBuilder::new()
        .with_window_attributes(Some(attributes))
        .build(&event_loop, config_template, |mut configs|
            configs.next().expect("no GL configurations available"))
        .expect("failed to create window");
    let window = window.unwrap();
    let window_handle = window.window_handle().expect("window has no handle");
    let (width, height) = window.inner_size().into();
    // create an OpenGL context
    let context_attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
        .build(Some(window_handle.into()));
    let gl_context = unsafe {
        gl_config.display().create_context(&gl_config, &context_attributes)
            .expect("failed to create GL context")
    };
    let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new()
        .build(window_handle.into(),
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap(),
        );
    let gl_surface = unsafe {
        gl_config.display().create_window_surface(&gl_config, &surface_attributes)
            .expect("failed to create GL surface")
    };
    let gl_context = gl_context.make_current(&gl_surface)
        .expect("failed to make GL context current");
    let gl_library = unsafe {
        GlowContext::from_loader_function_cstr(|func|
            gl_config.display().get_proc_address(func).cast())
    };
    // create communication channels and prime the bucket brigade
    let (sampler_to_renderer_send, sampler_to_renderer_recv) = channel();
    let (renderer_to_sampler_send, renderer_to_sampler_recv) = channel();
    for _ in 0..3 {
        let waveform = Waveform::new(SAMPLE_COUNT)
            .expect("failed to create a ring buffer for acquisition");
        renderer_to_sampler_send.send(waveform).unwrap();
    }
    // set up the acquisition and processing pipeline
    let instrument = thunderscope::Device::new().expect("failed to open instrument");
    let sampler = Sampler::new(instrument, renderer_to_sampler_recv, sampler_to_renderer_send);
    let renderer = Renderer::new(&gl_library, sampler_to_renderer_recv, renderer_to_sampler_send);
    // run the application
    let sampler_thread = sampler.run();
    event_loop.run_app(&mut Application {
        gl_context,
        gl_surface,
        gl_library,
        renderer,
        window
    }).expect("failed to run application");
    // clean up
    sampler_thread.join()
        .expect("acquisition thread panicked")
        .expect("acquisition failed");
}
