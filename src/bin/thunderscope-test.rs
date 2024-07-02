fn main() -> thunderscope::Result<()> {
    env_logger::init();

    let mut device = thunderscope::Device::new()?;
    device.startup()?;
    device.read_data()?;
    device.teardown()?;

    Ok(())
}
