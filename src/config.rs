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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelConfiguration {
    /// Probe attenuation in dB. For a 1X probe, `0.0`; for a 10X probe, `20.0`.
    pub probe_attenuation: f32,
    pub termination: Termination,
    pub coupling: Coupling,
    pub bandwidth: Bandwidth,
}

impl Default for ChannelConfiguration {
    fn default() -> Self {
        Self {
            probe_attenuation: 20.0, // 10X probe
            termination: Default::default(),
            coupling: Default::default(),
            bandwidth: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
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
