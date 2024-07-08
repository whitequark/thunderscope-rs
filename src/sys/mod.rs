use crate::Result;

#[cfg(all(feature = "hardware", any(target_os = "linux")))]
#[path = "linux.rs"]
mod imp;

#[cfg(not(all(feature = "hardware", any(target_os = "linux"))))]
#[path = "stub.rs"]
mod imp;

#[derive(Debug)]
pub struct Driver(imp::DriverData);

impl Driver {
    pub fn new(device_path: &str) -> Result<Self> {
        Ok(Self(imp::open(device_path)?))
    }

    pub fn read_user(&self, addr: usize, data: &mut [u8]) -> Result<()> {
        imp::read_user(&self.0, addr, data)
    }

    pub fn write_user(&self, addr: usize, data: &[u8]) -> Result<()> {
        imp::write_user(&self.0, addr, data)
    }

    pub fn read_dma(&self, addr: usize, data: &mut [u8]) -> Result<()> {
        imp::read_dma(&self.0, addr, data)
    }
}
