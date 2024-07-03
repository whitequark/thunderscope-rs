#![allow(dead_code)]

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Termination {
    Ohm50,
    #[default]
    Ohm1M,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Coupling {
    #[default]
    DC,
    AC
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoarseAttenuation {
    X1,
    #[default]
    X50,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Amplification {
    dB10,
    #[default]
    dB30,
}

impl Amplification {
    pub(crate) fn lmh6518_code(self) -> u16 {
        (match self {
            Self::dB10 => 0b0, // "low gain"
            Self::dB30 => 0b1, // "high gain"
        }) << 4
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
        OffsetMagnitude { code: 0x3f } // mid-scale
    }
}

impl OffsetMagnitude {
    pub(crate) fn mcp4432t_503e_code(self) -> u16 {
        self.code
    }

    pub(crate) fn from_ohms(ohms: u32) -> Self {
        assert!(ohms >= 75 && ohms <= 50000 + 75);
        const HALF_LSB: u32 = (50000 / 128) / 2;
        let code = (ohms - 75 + /* round to nearest */HALF_LSB) * 128 / 50000;
        OffsetMagnitude { code: code as u16 }
    }

    pub(crate) fn ohms(self) -> u32 {
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
    pub(crate) fn mcp4728_code(self) -> u16 {
        self.code
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelParameters {
    pub termination: Termination,
    pub coupling: Coupling,
    pub coarse_attenuation: CoarseAttenuation,
    pub amplification: Amplification,
    pub fine_attenuation: FineAttenuation,
    pub filtering: Filtering,
    pub offset_magnitude: OffsetMagnitude,
    pub offset_value: OffsetValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
