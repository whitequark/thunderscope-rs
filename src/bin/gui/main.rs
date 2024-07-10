use std::cell::Cell;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicI8, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};

use raw_window_handle::HasRawWindowHandle;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event_loop::{ControlFlow, EventLoop, EventLoopWindowTarget};
use winit::event::{Event, StartCause, WindowEvent};
use winit::window::{Window, WindowBuilder};

use glutin_winit::DisplayBuilder;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{Version, ContextApi, ContextAttributesBuilder};
use glutin::context::{NotCurrentGlContext, PossiblyCurrentContext};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, WindowSurface};
use glutin::display::{GetGlDisplay, GlDisplay};

use glow::{Context as GlowContext, HasContext};

use thunderscope::EdgeFilter;

const TRIGGER_EDGE: EdgeFilter = EdgeFilter::Rising;
static TRIGGER_LEVEL: AtomicI8 = AtomicI8::new(50);
const SAMPLE_COUNT: usize = 128_000;
const RENDER_LINES: bool = true;

struct SineGenerator {
    phase: f32,
    increment: f32,
}

impl SineGenerator {
    pub fn new(increment: f32) -> SineGenerator {
        SineGenerator { phase: 0.0, increment }
    }
}

impl std::io::Read for SineGenerator {
    fn read(&mut self, data: &mut [u8]) -> std::io::Result<usize> {
        for sample in data.iter_mut() {
            *sample = (self.phase.sin() * 100.0) as i8 as u8;
            self.phase = (self.phase + self.increment) % (2.0 * std::f32::consts::PI);
        }
        // simulate 1 GS/s capture rate
        std::thread::sleep(Duration::from_nanos(1) * (data.len() as u32));
        Ok(data.len())
    }
}

#[derive(Debug)]
struct Waveform {
    params: thunderscope::DeviceParameters,
    buffer: thunderscope::RingBuffer,
    capture: Option<(thunderscope::RingCursor, usize)>
}

impl Waveform {
    pub fn new(size: usize) -> thunderscope::Result<Waveform> {
        Ok(Waveform {
            params: thunderscope::DeviceParameters::default(),
            buffer: thunderscope::RingBuffer::new(size)?,
            capture: None
        })
    }

    pub fn capture_data(&self) -> Option<&[i8]> {
        self.capture.map(|(cursor, length)| self.buffer.read(cursor, length))
    }
}

struct WaveformSampler {
    // Sampler does not allocate the waveform buffers. It relies on a pair of channels acting like
    // a bucket brigade: any received `Waveform` objects are filled in with captures and sent for
    // further processing. Eventually the `Waveform` object comes back from the processing engine,
    // and the closed cycle continues.
    waveform_recv: Receiver<Waveform>,
    waveform_send: Sender<Waveform>,
}

impl WaveformSampler {
    pub fn new(
        waveform_recv: Receiver<Waveform>,
        waveform_send: Sender<Waveform>
    ) -> WaveformSampler {
        WaveformSampler { waveform_recv, waveform_send }
    }

    pub fn run(mut self, instrument: Option<thunderscope::Device>)
            -> std::thread::JoinHandle<thunderscope::Result<()>> {
        thread::spawn(move || {
            let params = thunderscope::DeviceParameters::derive(
                &thunderscope::DeviceCalibration::default(),
                &thunderscope::DeviceConfiguration {
                    channels: [Some(thunderscope::ChannelConfiguration {
                        ..Default::default()
                    }), None, None, None]
            });
            match instrument {
                None => {
                    let sine_generator = SineGenerator::new(12.0 / SAMPLE_COUNT as f32);
                    self.trigger_and_capture(&params, sine_generator)?
                }
                Some(mut instrument) => {
                    instrument.startup()?;
                    instrument.configure(&params)?;
                    self.trigger_and_capture(&params, instrument.stream_data())?;
                    instrument.shutdown()?;
                }
            }
            Ok(())
        })
    }

    fn trigger_and_capture(&mut self, params: &thunderscope::DeviceParameters,
            mut reader: impl std::io::Read) -> thunderscope::Result<()> {
        let mut wfm_active = self.waveform_recv.recv().expect("failed to receive waveform");
        let mut wfm_standby = None;
        let mut trigger = thunderscope::Trigger::new(TRIGGER_LEVEL.load(Ordering::SeqCst), 2);
        loop {
            // try to acquire a standby waveform buffer
            // at least one buffer must be available at all times to read samples into, so until
            // a standby buffer is available, the active buffer will not be submitted
            match self.waveform_recv.try_recv() {
                Ok(waveform) => wfm_standby = Some(waveform),
                Err(TryRecvError::Empty) => (),
                Err(TryRecvError::Disconnected) => {
                    log::debug!("sampler: done");
                    break
                }
            }
            // set up capturing in active buffer
            wfm_active.params = *params;
            wfm_active.capture = None;
            let mut cursor = wfm_active.buffer.cursor();
            let mut available = 0;
            // refill buffer
            let refill_by = wfm_active.buffer.len() - available;
            available += wfm_active.buffer.append(refill_by, |slice| reader.read(slice))?;
            log::debug!("sampler: refilled buffer for trigger by {} bytes ({} available)",
                refill_by, available);
            // find trigger
            let data = wfm_active.buffer.read(cursor, available);
            let (processed, edge) = trigger.find(data, TRIGGER_EDGE);
            cursor += processed;
            available -= processed;
            log::debug!("sampler: trigger consumed {} bytes ({} available)",
                processed, available);
            if let Some(edge) = edge {
                // check if we need to capture more
                if available < SAMPLE_COUNT {
                    let refill_by = SAMPLE_COUNT - available;
                    available += wfm_active.buffer.append(refill_by, |slice| reader.read(slice))?;
                    debug_assert!(available >= SAMPLE_COUNT);
                    log::debug!("sampler: refilled buffer for capture by {} bytes ({} available)",
                        refill_by, available);
                }
                // submit data for processing
                wfm_active.capture = Some((cursor - 2000, SAMPLE_COUNT));
                log::debug!("sampler: captured waveform for {:?} edge ({}+{})",
                    edge, cursor.into_inner(), SAMPLE_COUNT);
                if let Some(next_waveform) = wfm_standby.take() {
                    self.waveform_send.send(wfm_active).expect("failed to send waveform");
                    log::debug!("sampler: submitted waveform");
                    wfm_active = next_waveform;
                } else {
                    log::debug!("sampler: discarded waveform");
                }
                // reset trigger to resynchronize its state
                trigger.reset();
            }
            trigger = thunderscope::Trigger::new(TRIGGER_LEVEL.load(Ordering::SeqCst), 2);
        }
        Ok(())
    }
}

struct WaveformRenderer {
    program: <glow::Context as HasContext>::Program,
    vertex_array: <glow::Context as HasContext>::VertexArray,
    sample_array: <glow::Context as HasContext>::Buffer,
    waveform_recv: Receiver<Waveform>,
    waveform_send: Sender<Waveform>,
    current: Option<Waveform>,
}

impl WaveformRenderer {
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
                current: None
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
                if let Some(old_waveform) = self.current.replace(new_waveform) {
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
        unsafe {
            gl.clear_color(0.1, 0.0, 0.1, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            let Some(samples) = self.current.as_ref()
                .and_then(|waveform| waveform.capture_data())
                .map(|data| bytemuck::cast_slice(data)) else { return };

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

mod ui_defs {
    pub const FONT_DEFAULT_DATA: &[u8] = include_bytes!("DejaVuSansMono.ttf");
    pub const FONT_DEFAULT_SIZE: f32 = 18.0;

    pub const FONT_CONTROLS_DATA: &[u8] = include_bytes!("DejaVuSans-Bold.ttf");
    pub const FONT_CONTROLS_SIZE: f32 = 22.0;

    pub const FONT_LOGO_DATA: &[u8] = include_bytes!("DejaVuSerif.ttf");
    pub const FONT_LOGO_SIZE: f32  = 30.0;

    pub const LOGO_TEXT: &str = "ThunderScope";
    pub const LOGO_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

    pub const CONTROLS_V_MARGIN: f32 = 12.0;
    pub const CONTROLS_H_SPACING: f32 = 14.0;
    pub const CONTROLS_TRIGGER_WIDTH: f32 = 120.0;
    pub const CONTROLS_RUN_STOP_WIDTH: f32 = 72.0;

    pub const CHANNEL_V_PADDING: f32 = 10.0;

    pub const MARKER_FILL_COLOR: [f32; 4] = [1.0, 0.5, 0.0, 1.0];
    pub const MARKER_LINE_COLOR: [f32; 4] = [0.8, 0.4, 0.0, 1.0];
    pub const MARKER_TEXT_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

    pub const DEBUG_COLOR: [f32; 4] = [0.8, 0.0, 0.8, 1.0];
}

#[derive(Debug, Clone, Copy, Default)]
struct ChannelLayoutMetrics {
    inner_height: f32, // in logical px
    outer_height: f32, // in logical px; inner_height + padding
    zero_offset:  f32, // in volts
    full_scale:   f32, // in volts
}

impl ChannelLayoutMetrics {
    fn volts_to_pixels(&self, volts: f32) -> f32 {
        // the nominal full scale is 2 V: -1 V to 1 V
        let normalized_volts = (-volts - self.zero_offset) / self.full_scale * 2.0;
        (normalized_volts + 1.0) * self.outer_height / 2.0
    }

    fn pixels_to_volts(&self, pixels: f32) -> f32 {
        let normalized_volts = pixels * 2.0 / self.outer_height - 1.0;
        -(normalized_volts / 2.0 * self.full_scale + self.zero_offset)
    }
}

#[derive(Debug, Clone, Copy)]
struct InterfaceLayoutMetrics {
    overall_size:       [f32; 2], // in logical px
    logo_width:         f32,      // in logical px
    control_bar_height: f32,      // in logical px
    horz_scale_height:  f32,      // in logical px
    vert_scale_width:   f32,      // in logical px
    channels:           [ChannelLayoutMetrics; 4],
}

impl InterfaceLayoutMetrics {
    fn new(ui: &imgui::Ui, logo_font: imgui::FontId,
                channel_count: usize) -> InterfaceLayoutMetrics {
        let [overall_width, overall_height] = ui.window_size();
        let [logo_width, logo_height] = {
            let _t = ui.push_font(logo_font);
            ui.calc_text_size(ui_defs::LOGO_TEXT)
        };
        let control_bar_height = logo_height + ui_defs::CONTROLS_V_MARGIN * 2.0;
        let horz_scale_height = 32.0; // FIXME
        let vert_scale_width = 100.0;  // FIXME
        let channel_area_height = overall_height - control_bar_height - horz_scale_height -
            ui_defs::CHANNEL_V_PADDING;
        let channel_outer_height = channel_area_height / channel_count as f32;
        let mut channels = [ChannelLayoutMetrics::default(); 4];
        for index in 0..channel_count {
            channels[index].outer_height = channel_outer_height;
            channels[index].inner_height = channel_outer_height - ui_defs::CHANNEL_V_PADDING * 2.0;
            channels[index].zero_offset = 0.0; // FIXME
            channels[index].full_scale  = 10.0; // FIXME
        }
        InterfaceLayoutMetrics {
            overall_size: [overall_width, overall_height],
            logo_width,
            control_bar_height,
            horz_scale_height,
            vert_scale_width,
            channels,
        }
    }

    fn volts_to_pixels(&self, index: usize, volts: f32) -> f32 {
        let mut offset = self.control_bar_height + self.horz_scale_height;
        for index_above in 0..index {
            offset += self.channels[index_above].outer_height;
        }
        offset += self.channels[index].volts_to_pixels(volts);
        offset
    }

    fn pixels_to_volts(&self, index: usize, pixels: f32) -> f32 {
        let mut offset = pixels;
        offset -= self.control_bar_height + self.horz_scale_height;
        for index_above in 0..index {
            offset -= self.channels[index_above].outer_height;
        }
        dbg!(offset);
        self.channels[index].pixels_to_volts(offset)
    }

    fn channel_rect(&self, index: usize) -> ([f32; 2], [f32; 2]) {
        let mut vert_offset = self.control_bar_height + self.horz_scale_height;
        for index_above in 0..index {
            vert_offset += self.channels[index_above].outer_height;
        }
        let [overall_width, _] = self.overall_size;
        let channel_height = self.channels[index].outer_height;
        ([self.vert_scale_width, vert_offset],
         [overall_width - ui_defs::CONTROLS_H_SPACING, vert_offset + channel_height])
    }

    fn trace_origin(&self, index: usize) -> [f32; 2] {
        let mut vert_offset = 0.0;
        for index_above in 0..index {
            vert_offset += self.channels[index_above].outer_height;
        }
        vert_offset += self.channels[index].outer_height / 2.0;
        [self.vert_scale_width, vert_offset]
    }
}

#[derive(Debug, PartialEq, Eq, Default)]
struct InterfaceState {
    trigger_clicked: bool,
    run_stop_clicked: bool,
}

#[derive(Debug)]
struct InterfaceRenderer {
    controls_font: imgui::FontId,
    logo_font: imgui::FontId,

    dragging_h_marker: Cell<bool>,
    h_marker_pos: Cell<f32>,

    dragging_v_marker: Cell<bool>,
    v_marker_pos: Cell<f32>,
}

impl InterfaceRenderer {
    fn new(context: &mut imgui::Context, font_config: imgui::FontConfig) -> Self {
        use imgui::*;

        let ttf_font = |data, size_pixels| [
            FontSource::TtfData { data, size_pixels, config: Some(font_config.clone()) },
            FontSource::TtfData { data, size_pixels,
                config: Some(FontConfig {
                    glyph_ranges: FontGlyphRanges::from_slice(&[
                        '↑' as u32, '↑' as u32,
                        '↓' as u32, '↓' as u32,
                        '⇅' as u32, '⇅' as u32,
                        0
                    ]),
                    ..font_config.clone()
                }),
            },
        ];
        let _default_font = context.fonts().add_font(
            &ttf_font(ui_defs::FONT_DEFAULT_DATA, ui_defs::FONT_DEFAULT_SIZE));
        let controls_font = context.fonts().add_font(
            &ttf_font(ui_defs::FONT_CONTROLS_DATA, ui_defs::FONT_CONTROLS_SIZE));
        let logo_font = context.fonts().add_font(
            &ttf_font(ui_defs::FONT_LOGO_DATA, ui_defs::FONT_LOGO_SIZE));
        Self {
            controls_font,
            logo_font,
            dragging_h_marker: Cell::new(false),
            h_marker_pos: Cell::new(100.0),
            dragging_v_marker: Cell::new(false),
            v_marker_pos: Cell::new(3.3),
        }
    }

    fn render_logo(&self, ui: &imgui::Ui) -> [f32; 2] {
        let _t = ui.push_font(self.logo_font);
        let [w, _] = ui.cursor_pos();
        let [_, mut h] = ui.clone_style().window_padding;
        h -= ui.clone_style().frame_padding[1];
        ui.set_cursor_pos([w, h]);
        let logo_color = unsafe {
            let (mut r, mut g, mut b) = (0.0f32, 0.0f32, 0.0f32);
            let h = ui.frame_count() as f32 / 1000.0;
            imgui::sys::igColorConvertHSVtoRGB(h, 1.0, 1.0,
                &mut r as *mut f32,
                &mut g as *mut f32,
                &mut b as *mut f32);
            [r, g, b, 1.0]
        };
        ui.text_colored(logo_color, ui_defs::LOGO_TEXT);
        ui.calc_text_size(ui_defs::LOGO_TEXT)
    }

    fn render_minimap(&self, ui: &imgui::Ui, width: f32, height: f32) {
        let minimap = ui.child_window("##minimap")
            .size([width, height])
            .movable(false)
            .bring_to_front_on_focus(false);
        minimap.build(|| {
            let draw_list = ui.get_window_draw_list();
            let ([x, y], [w, h]) = (ui.window_pos(), ui.window_size());
            draw_list
                .add_rect([x, y], [x + w, y + h], ui_defs::DEBUG_COLOR)
                .build();
        });
    }

    fn with_controls_style<F: FnOnce() -> R, R>(&self, ui: &imgui::Ui, f: F) -> R {
        use imgui::*;

        let _t = ui.push_font(self.controls_font);
        let _t = ui.push_style_color(StyleColor::Button,        [0.00, 0.00, 0.00, 1.00]);
        let _t = ui.push_style_color(StyleColor::ButtonHovered, [0.20, 0.20, 0.20, 1.00]);
        let _t = ui.push_style_color(StyleColor::ButtonActive,  [0.40, 0.40, 0.40, 1.00]);
        let _t = ui.push_style_color(StyleColor::Border,        [0.25, 0.25, 0.25, 1.00]);
        let _t = ui.push_style_var(StyleVar::FrameRounding(5.0));
        let _t = ui.push_style_var(StyleVar::FrameBorderSize(2.0));
        f()
    }

    fn render_trigger_config(&self, ui: &imgui::Ui, width: f32, height: f32) -> bool {
        use imgui::*;

        self.with_controls_style(ui, || {
            let _t = ui.push_style_color(StyleColor::Text, [1.0, 1.0, 1.0, 1.0]);
            ui.button_with_size("T: CH1↑", [width, height])
        })
    }

    fn render_run_stop(&self, ui: &imgui::Ui, width: f32, height: f32) -> bool {
        use imgui::*;

        self.with_controls_style(ui, || {
            //let _t = ui.push_style_color(StyleColor::Text, [0.0, 1.0, 0.0, 1.0]);
            let _t = ui.push_style_color(StyleColor::Text, [1.0, 0.0, 0.0, 1.0]);
            ui.button_with_size("STOP", [width, height])
        })
    }

    /*
    fn render_trigger_offset_marker(&self, ui: &imgui::Ui) {
        let draw_list = ui.get_window_draw_list();

        let text = "-50ps";

        let [x, y] = [self.h_marker_pos.get(), 90.0];
        let [wt, ht] = ui.calc_text_size(text);
        let [wp, hp] = [wt+5.0, ht+5.0];
        let mut marker_outline = vec![
            [x, y],
            [x-wp/2.0, y-5.0],
            [x-wp/2.0, y-5.0-hp],
            [x+wp/2.0, y-5.0-hp],
            [x+wp/2.0, y-5.0],
            [x, y],
        ];
        let color = ui_style::MARKER_FILL_COLOR;
        if self.dragging_h_marker.get() {
            if ui.is_mouse_down(imgui::MouseButton::Left) {
                let [x, _] = ui.io().mouse_pos;
                self.h_marker_pos.set(x);
            } else {
                self.dragging_h_marker.set(false);
            }
        } else if ui.is_mouse_hovering_rect([x-wp/2.0,y-5.0-hp], [x+wp/2.0,y]) {
            if ui.is_mouse_down(imgui::MouseButton::Left) {
                self.dragging_h_marker.set(true)
            }
        }
        draw_list.add_polyline(marker_outline.clone(), color)
            .filled(true).build();
        marker_outline.push([x, y+400.0]);
        draw_list.add_polyline(marker_outline, ui_style::MARKER_LINE_COLOR)
            .thickness(1.0).build();
        draw_list.add_text([x-wt/2.0, y-2.5-ht-5.0], ui_style::MARKER_TEXT_COLOR, text);
    }
    */

    fn render_trigger_level_marker(&self, ui: &imgui::Ui, metrics: &InterfaceLayoutMetrics) {
        let draw_list = ui.get_window_draw_list();

        let channel_index = 1;

        let ([l, t], [r, b]) = metrics.channel_rect(channel_index);
        draw_list.add_rect([l, t], [r, b], ui_defs::DEBUG_COLOR).build();

        let volts = self.v_marker_pos.get();
        let text = format!("{:+.2}V", volts);

        let [x, y] = [metrics.vert_scale_width-8.0, metrics.volts_to_pixels(channel_index, volts)];
        let [wt, ht] = ui.calc_text_size(text.as_str());
        let [wp, hp] = [wt+5.0, ht+5.0];
        let mut marker_outline = vec![
            [x, y],
            [x-5.0, y-hp/2.0],
            [x-5.0-wp, y-hp/2.0],
            [x-5.0-wp, y+hp/2.0],
            [x-5.0, y+hp/2.0],
            [x, y],
        ];
        let color = ui_defs::MARKER_FILL_COLOR;
        if !self.dragging_h_marker.get() {
            if self.dragging_v_marker.get() {
                if ui.is_mouse_down(imgui::MouseButton::Left) {
                    let [_, y] = ui.io().mouse_pos;
                    self.v_marker_pos.set(metrics.pixels_to_volts(channel_index, y.max(t).min(b)));
                } else {
                    self.dragging_v_marker.set(false);
                }
            } else if ui.is_mouse_hovering_rect([x-5.0-wp, y-hp/2.0], [x, y+hp/2.0]) {
                if ui.is_mouse_down(imgui::MouseButton::Left) {
                    self.dragging_v_marker.set(true)
                }
            }
        }
        draw_list.add_polyline(marker_outline.clone(), color)
            .filled(true).build();
        marker_outline.push([r, y]);
        draw_list.add_polyline(marker_outline, ui_defs::MARKER_LINE_COLOR)
            .thickness(1.0).build();
        draw_list.add_text([x-wt-7.5, y-ht/2.0], ui_defs::MARKER_TEXT_COLOR, text.as_str());
    }

    fn render_controls(&self, ui: &imgui::Ui, state: &mut InterfaceState) {
        use imgui::*;

        let _t = ui.push_style_var(StyleVar::WindowPadding(
            [ui_defs::CONTROLS_H_SPACING, ui_defs::CONTROLS_V_MARGIN]));
        let _t = ui.window("##main")
            .size(ui.io().display_size, Condition::Always)
            .position([0.0, 0.0], Condition::Always)
            .no_decoration()
            .draw_background(false)
            .bring_to_front_on_focus(false)
            .begin();
        let metrics = InterfaceLayoutMetrics::new(ui, self.logo_font, 2);
        ui.group(|| {
            let _t = ui.push_style_var(StyleVar::ItemSpacing(
                [ui_defs::CONTROLS_H_SPACING, 0.0]));
            let control_height = metrics.control_bar_height - ui_defs::CONTROLS_V_MARGIN * 2.0;
            state.run_stop_clicked = self.render_run_stop(ui,
                ui_defs::CONTROLS_RUN_STOP_WIDTH, control_height);
            ui.same_line();
            state.trigger_clicked = self.render_trigger_config(ui,
                ui_defs::CONTROLS_TRIGGER_WIDTH, control_height);
            ui.same_line();
            let logo_width = metrics.logo_width + ui_defs::CONTROLS_H_SPACING;
            self.render_minimap(ui, -logo_width, control_height);
            ui.same_line();
            self.render_logo(ui);

            // self.render_trigger_offset_marker(ui);
            self.render_trigger_level_marker(ui, &metrics);
        });
    }

    fn render_trigger_config_popup(&self, ui: &imgui::Ui) {
        ui.popup("Trigger", || {
            use thunderscope::EdgeFilter;

            for (channel, label) in ["CH1", "CH2", "CH3", "CH4"].iter().enumerate() {
                if ui.menu_item_config(label).selected(channel == 0).build() {
                    // FIXME
                }
            }

            ui.separator();
            for (edge_filter, label) in [
                (EdgeFilter::Rising,  "↑ Rising"),
                (EdgeFilter::Falling, "↓ Falling"),
                (EdgeFilter::Both,    "⇅ Both"),
            ] {
                if ui.menu_item_config(label).selected(TRIGGER_EDGE == edge_filter).build() {
                    // FIXME
                }
            }

            ui.separator();
            ui.align_text_to_frame_padding();
            ui.text("Level");
            ui.same_line();
            ui.set_next_item_width(60.0);
            ui.input_float("V##Level", &mut (TRIGGER_LEVEL.load(Ordering::SeqCst) as f32 * 5.0 / 128.0))
                .build();
        });
    }

    fn render(&mut self, ui: &imgui::Ui) {
        use imgui::*;

        let mut state = InterfaceState::default();
        self.render_controls(ui, &mut state);

        if state != InterfaceState::default() {
            log::info!("{:?}", state)
        }
        if state.trigger_clicked {
            ui.open_popup("Trigger");
        }
        self.render_trigger_config_popup(ui);

        if ui.is_key_pressed(Key::Escape) {
            std::process::exit(0);
        }

        // ui.show_demo_window(&mut true);
    }
}

struct Application {
    gl_context: PossiblyCurrentContext,
    gl_surface: Surface<WindowSurface>,
    gl_library: GlowContext,
    wfm_renderer: WaveformRenderer,
    imgui_context: imgui::Context,
    imgui_platform: imgui_winit_support::WinitPlatform,
    imgui_texture_map: imgui_glow_renderer::SimpleTextureMap,
    imgui_renderer: imgui_glow_renderer::Renderer,
    ui_state: InterfaceRenderer,
    window: Window,
}

impl Application {
    fn process_event<T>(&mut self, event: Event<T>, window_target: &EventLoopWindowTarget<T>) {
        match event {
            Event::NewEvents(StartCause::ResumeTimeReached { requested_resume, .. }) => {
                // handle waveform updates
                if self.wfm_renderer.poll() {
                    self.window.request_redraw();
                }
                // handle UI updates
                self.imgui_context.io_mut().update_delta_time(
                    Instant::now().duration_since(requested_resume));
                self.imgui_platform.prepare_frame(self.imgui_context.io_mut(), &self.window)
                    .expect("failed to prepare UI frame");
                // The `winit` documentation recommends `Poll`, but if no waveforms are acquired,
                // this results in a busy loop waiting on `self.renderer.poll()`, pegging a core.
                // A 5 ms delay should be enough for even a 200 Hz display.
                window_target.set_control_flow(
                    ControlFlow::wait_duration(Duration::from_millis(5)));
            }
            Event::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                self.window.pre_present_notify();
                // draw waveforms
                self.wfm_renderer.render(&self.gl_library);
                // draw UI widgets
                let ui = self.imgui_context.frame();
                self.ui_state.render(&ui);
                self.imgui_platform.prepare_render(ui, &self.window);
                self.imgui_renderer.render(
                        &self.gl_library, &self.imgui_texture_map, self.imgui_context.render())
                    .expect("failed to render UI");
                // handle OpenGL
                self.gl_surface.swap_buffers(&self.gl_context)
                    .expect("failed to swap buffers");
            }
            Event::WindowEvent { event: WindowEvent::Resized(size), .. }
                    if size.width != 0 && size.height != 0 => {
                self.wfm_renderer.resize(&self.gl_library, size.width, size.height);
                self.imgui_platform.handle_event(self.imgui_context.io_mut(), &self.window, &event);
                self.gl_surface.resize(&self.gl_context,
                    NonZeroU32::new(size.width).unwrap(),
                    NonZeroU32::new(size.height).unwrap(),
                );
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                window_target.exit();
            }
            Event::LoopExiting => {
                self.wfm_renderer.destroy(&self.gl_library);
                self.imgui_renderer.destroy(&self.gl_library);
            }
            event => {
                self.imgui_platform.handle_event(self.imgui_context.io_mut(), &self.window, &event);
                self.window.request_redraw();
            }
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
    let window_builder = WindowBuilder::new()
        .with_title("ThunderScope");
    let config_template_builder = ConfigTemplateBuilder::new()
        .prefer_hardware_accelerated(Some(true));
    let (window, gl_config) = DisplayBuilder::new()
        .with_window_builder(Some(window_builder))
        .build(&event_loop, config_template_builder, |mut configs|
            configs.next().expect("no GL configurations available"))
        .expect("failed to create window");
    let window = window.unwrap();
    let (width, height) = window.inner_size().into();
    // create an OpenGL context
    let context_attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
        .build(Some(window.raw_window_handle()));
    let gl_context = unsafe {
        gl_config.display().create_context(&gl_config, &context_attributes)
            .expect("failed to create GL context")
    };
    let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new()
        .build(window.raw_window_handle(),
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
    // determine UI scale
    let scale_factor = window.scale_factor();
    log::info!("scaling UI by a factor of {:.2}×", scale_factor);
    let window_size = LogicalSize::new(1280.0, 720.0);
    let _ = window.request_inner_size(
        PhysicalSize::<f64>::from_logical(window_size, scale_factor));
    // create ImGui context
    let mut imgui_context = imgui::Context::create();
    imgui_context.style_mut().use_light_colors();
    imgui_context.set_ini_filename(None); // disable ini autosaving
    // create UI state
    let font_config = imgui::FontConfig {
        rasterizer_density: scale_factor as f32,
        oversample_h: 1,
        ..Default::default()
    };
    let ui_state = InterfaceRenderer::new(&mut imgui_context, font_config);
    // create ImGui renderer
    let mut imgui_platform = imgui_winit_support::WinitPlatform::init(&mut imgui_context);
    imgui_platform.attach_window(imgui_context.io_mut(), &window,
        imgui_winit_support::HiDpiMode::Locked(scale_factor));
    let mut imgui_texture_map = imgui_glow_renderer::SimpleTextureMap::default();
    let imgui_renderer = imgui_glow_renderer::Renderer::initialize(&gl_library,
            &mut imgui_context, &mut imgui_texture_map, /*output_srgb=*/true)
        .expect("failed to create UI renderer");
    // create communication channels and prime the bucket brigade
    let (sampler_to_renderer_send, sampler_to_renderer_recv) = channel();
    let (renderer_to_sampler_send, renderer_to_sampler_recv) = channel();
    for _ in 0..4 {
        let waveform = Waveform::new(SAMPLE_COUNT)
            .expect("failed to create a ring buffer for acquisition");
        renderer_to_sampler_send.send(waveform).unwrap();
    }
    // set up the acquisition and processing pipeline
    let sampler = WaveformSampler::new(
        renderer_to_sampler_recv, sampler_to_renderer_send);
    let wfm_renderer = WaveformRenderer::new(&gl_library,
        sampler_to_renderer_recv, renderer_to_sampler_send);
    // set up acquisition
    let instrument = thunderscope::Device::new().ok();
    let sampler_thread = sampler.run(instrument);
    // run the application
    {
        let mut application = Application {
            gl_context,
            gl_surface,
            gl_library,
            wfm_renderer,
            imgui_context,
            imgui_platform,
            imgui_texture_map,
            imgui_renderer,
            ui_state,
            window
        };
        event_loop.run(|event, window_target|
            application.process_event(event, window_target))
    }.expect("failed to run application");
    // clean up acquisition
    sampler_thread.join()
        .expect("acquisition thread panicked")
        .expect("acquisition failed");
}
