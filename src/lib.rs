mod sys;
mod regs;
mod params;
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

pub use params::{
    Termination,
    Coupling,
    CoarseAttenuation,
    Amplification,
    FineAttenuation,
    Filtering,
    OffsetMagnitude,
    OffsetValue,
    ChannelParameters,
    DeviceParameters,
};
pub use device::Device;
