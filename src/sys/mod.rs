use crate::Error;

pub trait Driver {
    fn read_user(&mut self, addr: usize, data: &mut [u8]) -> Result<(), Error>;
    fn write_user(&mut self, addr: usize, data: &[u8]) -> Result<(), Error>;

    fn read_d2h(&mut self, addr: usize, data: &mut [u8]) -> Result<(), Error>;
}

#[cfg(any(target_os = "linux"))]
#[path = "linux.rs"]
pub mod imp;
