use std::time::Duration;
use std::thread::sleep;

fn main() -> thunderscope::Result<()> {
    env_logger::init();

    let mut device = thunderscope::Device::new()?;
    device.startup()?;
    sleep(Duration::from_secs(1));
    device.read_data()?;
    device.teardown()?;

    Ok(())
}
