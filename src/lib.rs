#![feature(array_chunks)]

mod sys;
mod regs;
mod config;
mod params;
mod device;
mod trigger;

#[derive(Debug)]
pub enum Error {
    NotFound,
    XdmaIo(std::io::Error),
    Other(Box<dyn std::error::Error + Sync + Send + 'static>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::NotFound =>
                write!(f, "device not connected"),
            Self::XdmaIo(io_error) =>
                write!(f, "XDMA I/O error: {}", io_error),
            Self::Other(error) =>
                write!(f, "{}", error),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            &Self::XdmaIo(ref io_error) => Some(io_error),
            _ => None
        }
    }
}

impl From<Error> for std::io::Error {
    fn from(error: Error) -> Self {
        match error {
            Error::NotFound => // converted from std::io::Error in first place
                Self::new(std::io::ErrorKind::NotFound, error),
            Error::XdmaIo(io_error) =>
                io_error,
            Error::Other(error) => {
                match error.downcast::<std::io::Error>() {
                    Ok(error) => *error,
                    Err(error) => std::io::Error::new(io::ErrorKind::Other, error)
                }
            }
        }
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

pub type Result<T> =
    core::result::Result<T, Error>;

use std::io;

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

pub type Device =
    device::Device<crate::sys::imp::ThunderscopeDriverImpl>;

pub use trigger::{
    EdgeFilter,
    Edge,
    Trigger,
};
