use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::event::{StartCause, WindowEvent};
use winit::window::{Window, WindowId};
use winit::raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;

use glutin_winit::DisplayBuilder;

use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext, Version};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, WindowSurface};
use glutin::display::{GetGlDisplay, GlDisplay};

use glow::{Context as GlowContext, HasContext};

pub struct Renderer {
    pub program: <glow::Context as HasContext>::Program,
    pub vertex_array: <glow::Context as HasContext>::VertexArray,
    pub data_array: <glow::Context as HasContext>::Buffer,
}

impl Renderer {
    pub fn new(gl: &glow::Context) -> Self {
        let shaders = [
            (glow::VERTEX_SHADER,   include_str!("dot_vert.glsl")),
            (glow::FRAGMENT_SHADER, include_str!("dot_frag.glsl")),
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

            let renderer = Self {
                program,
                vertex_array:
                    gl.create_vertex_array().expect("failed to create vertex array"),
                data_array:
                    gl.create_buffer().expect("failed to create buffer"),
            };
            renderer.update(gl, &[0u8; 200000]);
            renderer
        }
    }

    pub fn update(&self, gl: &glow::Context, data: &[u8]) {
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.data_array));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, data, glow::STREAM_DRAW);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
    }

    pub fn render(&self, gl: &glow::Context) {
        unsafe {
            let channel_color_loc = gl.get_uniform_location(self.program, "channel_color");
            let adc_data_loc = gl.get_attrib_location(self.program, "adc_data")
                .expect("failed to retrieve location for `adc_data`");

            gl.clear_color(0.1, 0.0, 0.1, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.enable(glow::BLEND);

            gl.use_program(Some(self.program));
            gl.uniform_3_f32(channel_color_loc.as_ref(), 1.0, 1.0, 0.0);
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.data_array));
            gl.enable_vertex_attrib_array(adc_data_loc);
            gl.vertex_attrib_pointer_f32(adc_data_loc, 1, glow::BYTE, true, 1, 0);
            gl.vertex_attrib_divisor(adc_data_loc, 1);
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, 200000);
            gl.disable_vertex_attrib_array(adc_data_loc);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
    }

    pub fn resize(&self, gl: &glow::Context, width: u32, height: u32) {
        unsafe {
            gl.viewport(0, 0, width as i32, height as i32);
            gl.use_program(Some(self.program));
            gl.uniform_2_f32(gl.get_uniform_location(self.program, "resolution").as_ref(),
                width as f32, height as f32);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vertex_array);
        }
    }
}

struct Application {
    capture_data: Arc<Mutex<Vec<u8>>>,
    gl_context: PossiblyCurrentContext,
    gl_surface: Surface<WindowSurface>,
    glow_context: GlowContext,
    renderer: Renderer,
    window: Window,
}

impl ApplicationHandler for Application {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(&mut self, event_loop: &ActiveEventLoop,
            _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => {
                self.renderer.render(&self.glow_context);
                self.gl_surface.swap_buffers(&self.gl_context)
                    .expect("failed to swap buffers");
            }
            WindowEvent::Resized(size) if size.width != 0 && size.height != 0 => {
                self.renderer.resize(&self.glow_context, size.width, size.height);
                self.gl_surface.resize(&self.gl_context,
                    NonZeroU32::new(size.width).unwrap(),
                    NonZeroU32::new(size.height).unwrap(),
                );
            }
            WindowEvent::CloseRequested => {
                self.renderer.destroy(&self.glow_context);
                event_loop.exit();
            }
            _ => ()
        }
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        match cause {
            StartCause::ResumeTimeReached { requested_resume, .. } => {
                self.renderer.update(&self.glow_context, self.capture_data.lock().unwrap().as_ref());
                self.window.request_redraw();
                event_loop.set_control_flow(ControlFlow::WaitUntil(requested_resume +
                    Duration::from_millis(15)));
            }
            _ => ()
        }
    }
}

fn main() {
    env_logger::init();
    // create a window
    let event_loop = EventLoop::new().expect("failed to create event loop");
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
    let glow_context = unsafe {
        GlowContext::from_loader_function_cstr(|func|
            gl_config.display().get_proc_address(func).cast())
    };
    // create the shared data buffer
    let reader_capture_data = Arc::new(Mutex::new(Vec::new()));
    let writer_capture_data = Arc::clone(&reader_capture_data);
    // open and configure the instrument
    let _acquisition_thread = thread::spawn(move || {
        writer_capture_data.lock().unwrap().resize(200000, 0);
        thunderscope::Device::with(|device| {
            device.startup()?;
            device.configure(&thunderscope::DeviceParameters::derive(
                &thunderscope::DeviceCalibration::default(),
                &thunderscope::DeviceConfiguration {
                    channels: [Some(thunderscope::ChannelConfiguration::default()), None, None, None]
                }))?;
            device.read_data(|buffer| {
                let processed = {
                    let samples = buffer.read().unwrap_or(&mut []);
                    match writer_capture_data.try_lock() {
                        Ok(mut buffer) if buffer.len() <= samples.len() => {
                            let samples2 = &samples[..buffer.len()];
                            buffer.copy_from_slice(samples2);
                            samples2.len() // all of them
                        }
                        _ => 0
                    };
                    samples.len()
                };
                buffer.decommit(processed);
                Ok(())
            })?;
            Ok(())
        }).expect("failed to open instrument");
    });
    //
    // create the application
    let renderer = Renderer::new(&glow_context);
    let mut application = Application {
        capture_data: reader_capture_data,
        gl_context,
        gl_surface,
        glow_context,
        renderer,
        window
    };
    // run the application
    event_loop.set_control_flow(ControlFlow::wait_duration(Duration::ZERO));
    event_loop.run_app(&mut application)
        .expect("failed to run application");
}
