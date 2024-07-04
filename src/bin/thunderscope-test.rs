use thunderscope::{ChannelConfiguration, DeviceCalibration, DeviceConfiguration, DeviceParameters};

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
        device.read_data(|buffer| {
            const FILENAME: &str = "test.data";
            const SAVE_SAMPLES: usize = 200000;
            let samples = buffer.read().unwrap();
            if samples.len() >= SAVE_SAMPLES {
                println!("got {} samples, first 32: {:02X?}", samples.len(), &samples[..32]);
                std::fs::write(FILENAME, &samples[..SAVE_SAMPLES]).unwrap();
                println!("saved {} samples, run `python3 ./doc/plot_1ch.py {}`",
                    SAVE_SAMPLES, FILENAME);
                Err(())
            } else {
                Ok(())
            }
        })?;
        Ok(())
    })
}
