use thunderscope::{
    Amplification, CoarseAttenuation, FineAttenuation,
    ChannelParameters, DeviceParameters
};

fn main() -> thunderscope::Result<()> {
    env_logger::init();

    let mut device = thunderscope::Device::new()?;
    device.startup()?;
    let ch_params = Some(ChannelParameters {
        coarse_attenuation: CoarseAttenuation::X1,
        amplification: Amplification::dB10,
        fine_attenuation: FineAttenuation::dB20,
        ..Default::default()
    });
    device.configure(&DeviceParameters { channels: [ch_params, None, None, None] })?;
    device.read_data(|buffer| {
        let samples = buffer.read().unwrap();
        const SAVE_SAMPLES: usize = 512;
        if samples.len() >= SAVE_SAMPLES {
            println!("got {} samples, first 32: {:02X?}", samples.len(), &samples[..32]);
            std::fs::write("test.data", &samples[..SAVE_SAMPLES]).unwrap();
            println!("run `python3 ./doc/plot_1ch.py test.data`");
            Err(())
        } else {
            Ok(())
        }
    })?;
    device.teardown()?;
    Ok(())
}
