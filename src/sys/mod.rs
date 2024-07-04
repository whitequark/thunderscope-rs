use crate::Error;

pub trait Driver {
    fn read_user(&self, addr: usize, data: &mut [u8]) -> Result<(), Error>;
    fn write_user(&self, addr: usize, data: &[u8]) -> Result<(), Error>;

    fn read_dma(&self, addr: usize, data: &mut [u8]) -> Result<(), Error>;
}

#[cfg(any(target_os = "linux"))]
#[path = "linux.rs"]
pub mod imp;
