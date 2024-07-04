use std::ffi::{CStr, CString};
use std::io;
use libc::{c_int, c_void};
use crate::{Result, Error};

#[derive(Debug)]
struct FileDescriptor {
    inner: c_int
}

impl FileDescriptor {
    fn open(path: &CStr) -> io::Result<FileDescriptor> {
        unsafe {
            let fd = libc::open(path.as_ptr(), libc::O_RDWR);
            if fd == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(FileDescriptor { inner: fd })
            }
        }
    }

    fn read_at(&self, offset: usize, data: &mut [u8]) -> io::Result<()> {
        unsafe {
            let bytes_read = libc::pread(
                self.inner, data.as_mut_ptr() as *mut c_void, data.len(), offset as i64) as usize;
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
                self.inner, data.as_ptr() as *const c_void, data.len(), offset as i64) as usize;
            if bytes_written != data.len() {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }
}

impl Drop for FileDescriptor {
    fn drop(&mut self) {
        unsafe {
            if libc::close(self.inner) == -1 {
                panic!("error closing fd: {}", io::Error::last_os_error())
            }
        }
    }
}

#[derive(Debug)]
pub struct ThunderscopeDriverImpl {
    user_fd: FileDescriptor,
    c2h_fd: FileDescriptor,
}

impl ThunderscopeDriverImpl {
    pub fn new(device_path: &str) -> Result<ThunderscopeDriverImpl> {
        let control_path = device_path.to_owned() + "_control";
        match std::fs::exists(control_path) {
            Ok(true) => {
                let user_path = CString::new(device_path.to_owned() + "_user").unwrap();
                let d2h_path = CString::new(device_path.to_owned() + "_c2h_0").unwrap();
                Ok(ThunderscopeDriverImpl {
                    user_fd: FileDescriptor::open(user_path.as_ref()).map_err(Error::XdmaIo)?,
                    c2h_fd: FileDescriptor::open(d2h_path.as_ref()).map_err(Error::XdmaIo)?,
                })
            }
            _ => Err(Error::NotFound)
        }
    }
}

impl super::Driver for ThunderscopeDriverImpl {
    fn read_user(&self, addr: usize, data: &mut [u8]) -> Result<()> {
        self.user_fd.read_at(addr, data).map_err(Error::XdmaIo)
    }

    fn write_user(&self, addr: usize, data: &[u8]) -> Result<()> {
        self.user_fd.write_at(addr, data).map_err(Error::XdmaIo)
    }

    fn read_dma(&self, addr: usize, data: &mut [u8]) -> Result<()> {
        self.c2h_fd.read_at(addr, data).map_err(Error::XdmaIo)
    }
}
