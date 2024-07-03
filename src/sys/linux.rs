use std::ffi::{CStr, CString};
use std::io;
use libc::{c_int, c_void};
use crate::Result;

#[derive(Debug)]
struct Fd(c_int);

impl Fd {
    fn open(path: &CStr) -> io::Result<Fd> {
        unsafe {
            let user_fd = libc::open(path.as_ptr(), libc::O_RDWR);
            if user_fd == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(Fd(user_fd))
            }
        }
    }

    fn read_at(&self, offset: usize, data: &mut [u8]) -> io::Result<()> {
        unsafe {
            let bytes_read = libc::pread(self.0, data.as_mut_ptr() as *mut c_void, data.len(), offset as i64) as usize;
            if bytes_read != data.len() {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }

    fn write_at(&self, offset: usize, data: &[u8]) -> io::Result<()> {
        unsafe {
            let bytes_written = libc::pwrite(self.0, data.as_ptr() as *const c_void, data.len(), offset as i64) as usize;
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
pub struct ThunderscopeDriverImpl {
    user_fd: Fd,
    d2h_fd: Fd,
}

impl ThunderscopeDriverImpl {
    pub fn new(device_path: &str) -> Result<ThunderscopeDriverImpl> {
        let user_path = CString::new(device_path.to_owned() + "_user").unwrap();
        let d2h_path = CString::new(device_path.to_owned() + "_c2h_0").unwrap();
        Ok(ThunderscopeDriverImpl {
            user_fd: Fd::open(user_path.as_ref())?,
            d2h_fd: Fd::open(d2h_path.as_ref())?,
        })
    }
}

impl super::Driver for ThunderscopeDriverImpl {
    fn read_user(&self, addr: usize, data: &mut [u8]) -> Result<()> {
        Ok(self.user_fd.read_at(addr, data)?)
    }

    fn write_user(&self, addr: usize, data: &[u8]) -> Result<()> {
        Ok(self.user_fd.write_at(addr, data)?)
    }

    fn read_d2h(&self, addr: usize, data: &mut [u8]) -> Result<()> {
        Ok(self.d2h_fd.read_at(addr, data)?)
    }
}
