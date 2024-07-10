use std::io::Read;

use thunderscope::{ChannelConfiguration, DeviceCalibration, DeviceConfiguration, DeviceParameters};

const FILENAME: &str = "test.data";

fn main() -> thunderscope::Result<()> {
    env_logger::init();
    thunderscope::Device::with(|device| {
        let config = DeviceConfiguration {
            channels: [Some(ChannelConfiguration::default()), None, None, None]
        };
        let params = DeviceParameters::derive(&DeviceCalibration::default(), &config);
        device.configure(&params)?;
        let mut samples = vec![0; 200000];
        device.stream_data().read_exact(samples.as_mut())?; // let the signal path stabilize
        device.stream_data().read_exact(samples.as_mut())?;
        println!("channel gain: {:.2} dB", params.gain(0));
        let full_scale = params.full_scale(0);
        println!("full scale: {:-.3} V to {:+.3} V", -full_scale/2.0, full_scale/2.0);
        let count = 64;
        println!("first {} codes:\n  {:02X?}", count, samples.iter()
            .take(count)
            .collect::<Vec<_>>());
        println!("first {} voltages:\n  {:.2?}", count, samples.iter()
            .take(count)
            .map(|&code| params.code_to_volts(0, code as i8))
            .collect::<Vec<_>>());
        std::fs::write(FILENAME, &samples[..]).unwrap();
        println!("saved {} samples, run `python3 ./doc/plot_1ch.py {}`", samples.len(), FILENAME);
        Ok(())
    })
}
