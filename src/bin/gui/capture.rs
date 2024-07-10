use std::f32::consts::PI;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::io::Read;

use thunderscope::{Result, DeviceCalibration, DeviceConfiguration, DeviceParameters};
use thunderscope::{RingBuffer, RingCursor};
use thunderscope::{EdgeFilter, Trigger};

const TRIGGER_HYSTERESIS: u8 = 2;

const SAMPLE_COUNT: usize = 1000;

#[derive(Debug, Clone, Copy)]
pub struct TriggerParameters {
    channel: usize,
    level: f32, // in volts
    edge: EdgeFilter,
}

#[derive(Debug, Clone, Copy)]
pub enum OperationMode {
    Idle,
    FreeRunning,
    SingleTrigger(TriggerParameters),
    RepeatTrigger(TriggerParameters),
}

#[derive(Debug, Clone, Copy)]
pub struct Parameters {
    device: DeviceParameters,
    mode: OperationMode,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            device: DeviceParameters::derive(
                &DeviceCalibration::default(),
                &DeviceConfiguration::default()
            ),
            mode: OperationMode::Idle
        }
    }
}

impl Parameters {
    pub fn demo() -> Self {
        Self {
            device: DeviceParameters::derive(
                &DeviceCalibration::default(),
                &DeviceConfiguration { channels: [Some(Default::default()), None, None, None] }
            ),
            mode: OperationMode::RepeatTrigger(TriggerParameters {
                channel: 0,
                level: 1.0,
                edge: EdgeFilter::Rising,
            })
        }
    }
}

#[derive(Debug)]
pub struct Waveform {
    params: Parameters,
    buffer: RingBuffer,
    capture: Option<(RingCursor, usize)>
}

impl Waveform {
    pub fn new(size: usize) -> Result<Waveform> {
        Ok(Waveform {
            params: Parameters::default(),
            buffer: RingBuffer::new(size)?,
            capture: None
        })
    }

    pub fn capture_data(&self) -> Option<&[i8]> {
        self.capture.map(|(cursor, length)| self.buffer.read(cursor, length))
    }
}

struct SineGenerator {
    phase: f32,
    step: f32,
}

impl SineGenerator {
    fn new(frequency: f32) -> SineGenerator {
        SineGenerator {
            phase: 0.0,
            step: 1e9 * 2.0 * PI / frequency,
        }
    }
}

impl std::io::Read for SineGenerator {
    fn read(&mut self, data: &mut [u8]) -> std::io::Result<usize> {
        for sample in data.iter_mut() {
            *sample = (self.phase.sin() * 100.0) as i8 as u8;
            self.phase = (self.phase + self.step) % (2.0 * PI);
        }
        // simulate 1 GS/s capture rate
        std::thread::sleep(std::time::Duration::from_nanos(1) * (data.len() as u32));
        Ok(data.len())
    }
}

#[derive(Debug)]
pub enum DataSource {
    Hardware(thunderscope::Device),
    SineGenerator { frequency: f32 }, // in Hz
}

pub struct Sampler {
    params_recv: Receiver<Parameters>,
    // Sampler does not allocate the waveform buffers. It relies on a pair of channels acting like
    // a bucket brigade: any received `Waveform` objects are filled in with captures and sent for
    // further processing. Eventually the `Waveform` object comes back from the processing engine,
    // and the closed cycle continues.
    waveform_recv: Receiver<Waveform>,
    waveform_send: Sender<Waveform>,
}

impl Sampler {
    pub fn new(
        params_recv: Receiver<Parameters>,
        waveform_recv: Receiver<Waveform>,
        waveform_send: Sender<Waveform>
    ) -> Sampler {
        Sampler { params_recv, waveform_recv, waveform_send }
    }

    pub fn run(mut self, source: DataSource) -> std::thread::JoinHandle<Result<()>> {
        std::thread::spawn(move || {
            match source {
                DataSource::SineGenerator { frequency } => {
                    let sine_generator = SineGenerator::new(frequency);
                    self.trigger_and_capture(sine_generator,
                        |_params| Ok(()))?
                }
                DataSource::Hardware(instrument) => {
                    instrument.startup()?;
                    self.trigger_and_capture(instrument.stream_data(),
                        |params| instrument.configure(params))?;
                    instrument.shutdown()?;
                }
            }
            Ok(())
        })
    }

    fn trigger_and_capture<F>(&mut self, mut reader: impl Read, mut reconfigure: F) -> Result<()>
            where F: FnMut(&DeviceParameters) -> Result<()> {
        let mut wfm_active = self.waveform_recv.recv().expect("failed to receive waveform");
        let mut wfm_standby = None;
        let mut params = Parameters::default();
        let mut trigger = None;
        loop {
            // switch capture parameters, if requested
            match self.params_recv.try_recv() {
                Ok(new_params) => {
                    log::info!("sampler: switching parameters to {:#?}", new_params);
                    params = new_params;
                    trigger = match new_params.mode {
                        OperationMode::Idle |
                        OperationMode::FreeRunning => None,
                        OperationMode::SingleTrigger(trigger) |
                        OperationMode::RepeatTrigger(trigger) =>
                            Some((Trigger::new(
                                new_params.device.volts_to_code(trigger.channel, trigger.level),
                                TRIGGER_HYSTERESIS
                            ), trigger.edge)),
                    };
                    reconfigure(&new_params.device)?;
                }
                Err(_) => {}
            }
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
            wfm_active.params = params;
            wfm_active.capture = None;
            let mut cursor = wfm_active.buffer.cursor();
            let mut available = 0;
            // refill buffer
            let refill_by = wfm_active.buffer.len() - available;
            available += wfm_active.buffer.append(refill_by, |slice| reader.read(slice))?;
            log::debug!("sampler: refilled buffer by {} bytes ({} available)",
                refill_by, available);
            if let OperationMode::FreeRunning = params.mode {
                // accept capture as-is
                wfm_active.capture = Some((cursor, SAMPLE_COUNT));
                log::debug!("sampler: captured waveform free running ({}+{})",
                    cursor.into_inner(), SAMPLE_COUNT);
            } else if let Some((mut trigger, edge_filter)) = trigger {
                // find trigger point
                let data = wfm_active.buffer.read(cursor, available);
                let (processed, edge) = trigger.find(data, edge_filter);
                cursor += processed;
                available -= processed;
                log::debug!("sampler: trigger consumed {} bytes ({} available)",
                    processed, available);
                if let Some(edge) = edge {
                    // check if we need to capture more
                    if available < SAMPLE_COUNT {
                        let refill_by = SAMPLE_COUNT - available;
                        available += wfm_active.buffer.append(refill_by,
                            |slice| reader.read(slice))?;
                        debug_assert!(available >= SAMPLE_COUNT);
                        log::debug!("sampler: refilled buffer by {} bytes ({} available)",
                            refill_by, available);
                    }
                    // accept capture at trigger point
                    wfm_active.capture = Some((cursor, SAMPLE_COUNT));
                    log::debug!("sampler: captured waveform for {:?} edge ({}+{})",
                        edge, cursor.into_inner(), SAMPLE_COUNT);
                    // reset trigger to resynchronize its state
                    trigger.reset();
                }
            }
            // if there is a capture, try to submit it for processing
            if wfm_active.capture.is_some() {
                if let Some(next_waveform) = wfm_standby.take() {
                    if let OperationMode::SingleTrigger(_) = params.mode {
                        // if only a single capture was requested, stop capturing
                        params.mode = OperationMode::Idle;
                        trigger = None;
                    }
                    self.waveform_send.send(wfm_active).expect("failed to send waveform");
                    log::debug!("sampler: submitted waveform");
                    wfm_active = next_waveform;
                } else {
                    wfm_active.capture = None;
                    log::debug!("sampler: discarded waveform");
                }
            }
        }
        Ok(())
    }
}
