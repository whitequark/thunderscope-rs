mod sys;
pub mod regs;
mod device;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub type Result<T> = core::result::Result<T, Error>;

pub use device::Device;
