//! Low-level parameters of the device that map 1:1 to values written into registers of
//! analog frontend components.

#![allow(dead_code)]

use std::fmt;

use crate::{config::{Bandwidth, Coupling, DeviceConfiguration, Termination}, ChannelConfiguration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoarseAttenuation {
    X1,
    #[default]
    X50,
}

impl CoarseAttenuation {
    pub const ALL: [Self; 2] = [
        Self::X1,
        Self::X50,
    ];

    /// Gain in this part of the signal path, in dB.
    fn gain(self) -> f32 {
        match self {
            CoarseAttenuation::X1  =>   0.0,
            CoarseAttenuation::X50 => -33.9794,
        }
    }
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Amplification {
    dB10,
    #[default]
    dB30,
}

impl Amplification {
    pub const ALL: [Self; 2] = [
        Self::dB10,
        Self::dB30
    ];

    pub(crate) fn lmh6518_code(self) -> u16 {
        (match self {
            Self::dB10 => 0b0, // "low gain"
            Self::dB30 => 0b1, // "high gain"
        }) << 4
    }

    /// Gain in this part of the signal path, in dB.
    fn gain(self) -> f32 {
        match self {
            Amplification::dB10 => 10.0,
            Amplification::dB30 => 30.0,
        }
    }
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FineAttenuation {
    #[default]
    dB0,
    dB2,
    dB4,
    dB6,
    dB8,
    dB10,
    dB12,
    dB14,
    dB16,
    dB18,
    dB20,
}

impl FineAttenuation {
    pub const ALL: [Self; 11] = [
        Self::dB0,
        Self::dB2,
        Self::dB4,
        Self::dB6,
        Self::dB8,
        Self::dB10,
        Self::dB12,
        Self::dB14,
        Self::dB16,
        Self::dB18,
        Self::dB20,
    ];

    pub(crate) fn lmh6518_code(self) -> u16 {
        (match self {
            Self::dB0  => 0b0000,
            Self::dB2  => 0b0001,
            Self::dB4  => 0b0010,
            Self::dB6  => 0b0011,
            Self::dB8  => 0b0100,
            Self::dB10 => 0b0101,
            Self::dB12 => 0b0110,
            Self::dB14 => 0b0111,
            Self::dB16 => 0b1000,
            Self::dB18 => 0b1001,
            Self::dB20 => 0b1010,
        }) << 0
    }

    /// Gain in this part of the signal path, in dB.
    fn gain(self) -> f32 {
        match self {
            FineAttenuation::dB0  =>  -0.0,
            FineAttenuation::dB2  =>  -2.0,
            FineAttenuation::dB4  =>  -4.0,
            FineAttenuation::dB6  =>  -6.0,
            FineAttenuation::dB8  =>  -8.0,
            FineAttenuation::dB10 => -10.0,
            FineAttenuation::dB12 => -12.0,
            FineAttenuation::dB14 => -14.0,
            FineAttenuation::dB16 => -16.0,
            FineAttenuation::dB18 => -18.0,
            FineAttenuation::dB20 => -20.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filtering {
    MHz20,
    #[default]
    MHz100,
    MHz200,
    MHz350,
    Off,
}

impl Filtering {
    pub(crate) fn lmh6518_code(self) -> u16 {
        (match self {
            Self::MHz20  => 0b001,
            Self::MHz100 => 0b010,
            Self::MHz200 => 0b011,
            Self::MHz350 => 0b100,
            Self::Off    => 0b000,
        }) << 6
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct OffsetMagnitude {
    code: u16,
}

impl fmt::Debug for OffsetMagnitude {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "OffsetMagnitude::from_ohms({})", self.ohms())
    }
}

impl Default for OffsetMagnitude {
    fn default() -> Self {
        OffsetMagnitude { code: 0x40 } // mid-scale
    }
}

impl OffsetMagnitude {
    pub/*(crate)*/ fn from_mcp4432t_503e_code(code: u16) -> Self {
        OffsetMagnitude { code }
    }

    pub/*(crate)*/ fn mcp4432t_503e_code(self) -> u16 {
        self.code
    }

    pub/*(crate)*/ fn from_ohms(ohms: u32) -> Self {
        assert!(ohms >= 75 && ohms <= 50000 + 75);
        const HALF_LSB: u32 = (50000 / 128) / 2;
        let code = (ohms - 75 + /* round to nearest */HALF_LSB) * 128 / 50000;
        OffsetMagnitude { code: code as u16 }
    }

    pub/*(crate)*/ fn ohms(self) -> u32 {
        self.code as u32 * 50000 / 128 + 75
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OffsetValue {
    code: u16, // 12 bit DAC
}

impl Default for OffsetValue {
    fn default() -> Self {
        OffsetValue { code: 0x3fff } // mid-scale
    }
}

impl OffsetValue {
    pub/*(crate)*/ fn from_mcp4728_code(code: u16) -> Self {
        OffsetValue { code }
    }

    pub/*(crate)*/ fn mcp4728_code(self) -> u16 {
        self.code
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ChannelParameters {
    pub probe_attenuation: f32, // in dB
    pub termination: Termination,
    pub coupling: Coupling,
    pub coarse_attenuation: CoarseAttenuation,
    pub amplification: Amplification,
    pub fine_attenuation: FineAttenuation,
    pub filtering: Filtering,
    pub offset_magnitude: OffsetMagnitude,
    pub offset_value: OffsetValue,
}

impl ChannelParameters {
    /// Returns total gain in the instrument signal path, in decibels.
    fn gain(&self, adc_coarse_gain: f32) -> f32 {
        -self.probe_attenuation
            + self.coarse_attenuation.gain() // 1X/50X attenuation switch
            + self.amplification.gain()      // LMH6518 pre-amplifier
            + self.fine_attenuation.gain()   // LMH6518 ladder attenuator
            + 8.8600                         // LMH6518 output amplifier
            + adc_coarse_gain                // HMCAD1520 coarse gain
            - 0.3546                         // HMCAD1520 full scale adjustment
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceParameters {
    pub channels: [Option<ChannelParameters>; 4],
}

impl Default for DeviceParameters {
    fn default() -> Self {
        DeviceParameters {
            channels: [Some(ChannelParameters::default()); 4]
        }
    }
}

impl DeviceParameters {
    /// Returns total gain in the instrument signal path for the given channel, in decibels.
    pub fn gain(&self, channel_index: usize) -> f32 {
        let channel_count = self.channels.iter().filter(|ch| ch.is_some()).count();
        assert!(channel_count > 0 && self.channels[channel_index].is_some());
        let adc_coarse_gain = match channel_count {
            4 |
            3 |
            2 =>  9.0,
            1 => 10.0,
            _ => unreachable!()
        };
        self.channels[channel_index].unwrap().gain(adc_coarse_gain)
    }

    /// Returns the voltage difference (as measured at the probe) between the most negative and
    /// most positive ADC code for the given channel, in volts.
    pub fn full_scale(&self, channel_index: usize) -> f32 {
        2.0 * 10.0f32.powf(-self.gain(channel_index) / 20.0)
    }

    /// Converts a voltage (as measured at the probe) to the ADC code, saturating to the most
    /// negative or most positive code for out of range values.
    pub fn volts_to_code(&self, channel_index: usize, volts: f32) -> i8 {
        let full_scale = self.full_scale(channel_index);
        // Since Rust 1.45 this performs a saturating cast. Nice!
        (256.0 * (volts / full_scale)) as i8
    }

    /// Converts an ADC code to voltage (as measured at the probe).
    pub fn code_to_volts(&self, channel_index: usize, code: i8) -> f32 {
        let full_scale = self.full_scale(channel_index);
        code as f32 / 256.0 * full_scale
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelCalibration {
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceCalibration {
    pub channels: [ChannelCalibration; 4],
}

impl DeviceParameters {
    pub fn derive(calibration: &DeviceCalibration, configuration: &DeviceConfiguration) -> Self {
        fn derive_channel(_calibration: &ChannelCalibration,
                configuration: &ChannelConfiguration) -> ChannelParameters {
            ChannelParameters {
                probe_attenuation: configuration.probe_attenuation,
                termination: configuration.termination,
                coupling: configuration.coupling,
                coarse_attenuation: CoarseAttenuation::X1, // FIXME
                amplification: Amplification::dB10, // FIXME
                fine_attenuation: FineAttenuation::dB20, // FIXME
                filtering: match configuration.bandwidth {
                    Bandwidth::MHz100 => Filtering::MHz100,
                    Bandwidth::MHz200 => Filtering::MHz200,
                    Bandwidth::MHz350 => Filtering::MHz350,
                },
                offset_magnitude: Default::default(), // FIXME
                offset_value: Default::default(), // FIXME
            }
        }

        DeviceParameters {
            channels: std::array::from_fn(|index|
                configuration.channels[index].map(|channel|
                    derive_channel(&calibration.channels[index], &channel)))
        }
    }
}
