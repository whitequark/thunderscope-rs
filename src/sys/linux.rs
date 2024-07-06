use std::ffi::{CStr, CString};
use std::{fs, io};
use libc::{c_int, c_void};
use crate::Result;

#[derive(Debug)]
struct Fd(c_int);

impl Fd {
    fn open(path: &CStr) -> io::Result<Fd> {
        unsafe {
            let fd = libc::open(path.as_ptr(), libc::O_RDWR);
            if fd == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(Fd(fd))
            }
        }
    }

    fn read_at(&self, offset: usize, data: &mut [u8]) -> io::Result<()> {
        unsafe {
            let bytes_read = libc::pread(
                self.0, data.as_mut_ptr() as *mut c_void, data.len(), offset as i64) as usize;
            if bytes_read != data.len() {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }

    fn write_at(&self, offset: usize, data: &[u8]) -> io::Result<()> {
        unsafe {
            let bytes_written = libc::pwrite(
                self.0, data.as_ptr() as *const c_void, data.len(), offset as i64) as usize;
            if bytes_written != data.len() {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }
}

impl Drop for Fd {
    fn drop(&mut self) {
        unsafe {
            if libc::close(self.0) == -1 {
                panic!("error closing fd: {}", io::Error::last_os_error())
            }
        }
    }
}

#[derive(Debug)]
pub struct DriverData {
    user_fd: Fd,
    c2h_fd: Fd,
}

pub fn open(device_path: &str) -> Result<DriverData> {
    let control_path = device_path.to_owned() + "_control";
    if fs::metadata(control_path).is_ok() {
        let user_path = CString::new(device_path.to_owned() + "_user").unwrap();
        let d2h_path = CString::new(device_path.to_owned() + "_c2h_0").unwrap();
        Ok(DriverData {
            user_fd: Fd::open(user_path.as_ref())?,
            c2h_fd: Fd::open(d2h_path.as_ref())?,
        })
    } else {
        Err(crate::Error::NotFound)
    }
}

pub fn read_user(driver_data: &DriverData, addr: usize, data: &mut [u8]) -> Result<()> {
    Ok(driver_data.user_fd.read_at(addr, data)?)
}

pub fn write_user(driver_data: &DriverData, addr: usize, data: &[u8]) -> Result<()> {
    Ok(driver_data.user_fd.write_at(addr, data)?)
}

pub fn read_dma(driver_data: &DriverData, addr: usize, data: &mut [u8]) -> Result<()> {
    Ok(driver_data.c2h_fd.read_at(addr, data)?)
}
