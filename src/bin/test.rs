use std::io::Read;

use thunderscope::{ChannelConfiguration, DeviceCalibration, DeviceConfiguration, DeviceParameters};

const FILENAME: &str = "test.data";

fn main() -> thunderscope::Result<()> {
    env_logger::init();
    thunderscope::Device::with(|device| {
        let config = DeviceConfiguration {
            channels: [
                Some(ChannelConfiguration::default()),
                None,
                None,
                None
            ]
        };
        device.configure(&DeviceParameters::derive(&DeviceCalibration::default(), &config))?;
        let mut samples = vec![0; 200000];
        device.stream_data().read_exact(samples.as_mut())?;
        println!("got {} samples, first 32: {:02X?}", samples.len(), &samples[..256]);
        std::fs::write(FILENAME, &samples[..]).unwrap();
        println!("saved {} samples, run `python3 ./doc/plot_1ch.py {}`", samples.len(), FILENAME);
        Ok(())
    })
}
