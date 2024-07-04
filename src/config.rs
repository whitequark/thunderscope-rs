//! High-level configuration of the device in terms of physical qualities.

#![allow(dead_code)]


#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Termination {
    #[default]
    Ohm1M,
    Ohm50,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Coupling {
    #[default]
    DC,
    AC
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Bandwidth {
    #[default]
    MHz100,
    MHz200,
    MHz350,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ChannelConfiguration {
    pub termination: Termination,
    pub coupling: Coupling,
    pub bandwidth: Bandwidth,
}

#[derive(Debug, Clone, Copy)]
pub struct DeviceConfiguration {
    pub channels: [Option<ChannelConfiguration>; 4]
}

impl Default for DeviceConfiguration {
    fn default() -> Self {
        DeviceConfiguration {
            channels: [Some(ChannelConfiguration::default()); 4]
        }
    }
}
