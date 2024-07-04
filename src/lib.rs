mod sys;
mod regs;
mod config;
mod params;
mod device;

#[derive(Debug)]
pub enum Error {
    NotFound,
    XdmaIo(std::io::Error),
    Overflow { required: usize, available: usize },
}

pub type Result<T> = core::result::Result<T, Error>;

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

pub type Device = device::Device<crate::sys::imp::ThunderscopeDriverImpl>;
