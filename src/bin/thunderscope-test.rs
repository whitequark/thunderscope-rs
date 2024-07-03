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
    device.read_data()?;
    device.teardown()?;
    Ok(())
}
