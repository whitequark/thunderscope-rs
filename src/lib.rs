#![feature(array_chunks)]

mod sys;
mod regs;
mod config;
mod params;
mod device;
mod trigger;
mod capture;

#[derive(Debug)]
pub enum Error {
    NotFound,
    Xdma(std::io::Error),
    Vmap(vmap::Error),
    Other(Box<dyn std::error::Error + Sync + Send + 'static>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::NotFound =>
                write!(f, "device not connected"),
            Self::Xdma(error) =>
                write!(f, "XDMA error: {}", error),
            Self::Vmap(error) =>
                write!(f, "virtual memory mapping error: {}", error),
            Self::Other(error) =>
                write!(f, "{}", error),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            &Self::Xdma(ref error) => Some(error),
            &Self::Vmap(ref error) => Some(error),
            _ => None
        }
    }
}

impl From<vmap::Error> for Error {
    fn from(error: vmap::Error) -> Self {
        Error::Vmap(error)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        match error.downcast::<Self>() {
            Ok(error) => error,
            Err(error) => Error::Other(error.into()),
        }
    }
}

impl From<Error> for std::io::Error {
    fn from(error: Error) -> Self {
        match error {
            Error::NotFound => // converted from std::io::Error in first place
                Self::new(std::io::ErrorKind::NotFound, error),
            Error::Xdma(error) => error,
            Error::Vmap(error) => error.into(),
            Error::Other(error) => {
                match error.downcast::<std::io::Error>() {
                    Ok(error)  => *error,
                    Err(error) => std::io::Error::new(std::io::ErrorKind::Other, error)
                }
            }
        }
    }
}

pub type Result<T> =
    core::result::Result<T, Error>;

pub use config::{
    Termination,
    Coupling,
    Bandwidth,
    ChannelConfiguration,
    DeviceConfiguration,
};

pub use params::{
    CoarseAttenuation,
    Amplification,
    FineAttenuation,
    Filtering,
    OffsetMagnitude,
    OffsetValue,
    ChannelParameters,
    DeviceParameters,
    ChannelCalibration,
    DeviceCalibration,
};

pub use device::Device;

pub use trigger::{
    EdgeFilter,
    Edge,
    Trigger,
};

pub use capture::{
    RingCursor,
    RingBuffer,
};
