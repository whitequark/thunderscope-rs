use crate::Result;

#[derive(Debug)]
pub struct DriverData;

pub fn open(_device_path: &str) -> Result<DriverData> {
    unimplemented!()
}

pub fn read_user(_driver_data: &DriverData, _addr: usize, _data: &mut [u8]) -> Result<()> {
    unimplemented!()
}

pub fn write_user(_driver_data: &DriverData, _addr: usize, _data: &[u8]) -> Result<()> {
    unimplemented!()
}

pub fn read_dma(_driver_data: &DriverData, _addr: usize, _data: &mut [u8]) -> Result<()> {
    unimplemented!()
}
