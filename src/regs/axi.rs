#![allow(dead_code)]

use bitflags::bitflags;

/// Thunderscope Control Register
pub const ADDR_CONTROL: usize = 0x0;

bitflags! {
    // See [doc/datamover_register.txt] for details.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Control: u32 {
        const DatamoverHaltN    = 1<<0;
        const FpgaAcqResetN     = 1<<1;

        const ChannelMux0       = 1<<4;
        const ChannelMux1       = 1<<5;

        const Ch1Termination    = 1<<12;
        const Ch2Termination    = 1<<13;
        const Ch3Termination    = 1<<14;
        const Ch4Termination    = 1<<15;

        const Ch1Attenuator     = 1<<16;
        const Ch2Attenuator     = 1<<17;
        const Ch3Attenuator     = 1<<18;
        const Ch4Attenuator     = 1<<19;

        const Ch1Coupling       = 1<<20;
        const Ch2Coupling       = 1<<21;
        const Ch3Coupling       = 1<<22;
        const Ch4Coupling       = 1<<23;

        const Rail3V3Enabled    = 1<<24;
        const ClockGenResetN    = 1<<25;
        const Rail5VEnabled     = 1<<26;
    }
}

impl Control {
    pub fn ch_termination(index: usize) -> Self {
        match index {
            0 => Control::Ch1Termination,
            1 => Control::Ch2Termination,
            2 => Control::Ch3Termination,
            3 => Control::Ch4Termination,
            _ => unreachable!()
        }
    }

    pub fn ch_coupling(index: usize) -> Self {
        match index {
            0 => Control::Ch1Coupling,
            1 => Control::Ch2Coupling,
            2 => Control::Ch3Coupling,
            3 => Control::Ch4Coupling,
            _ => unreachable!()
        }
    }

    pub fn ch_attenuator(index: usize) -> Self {
        match index {
            0 => Control::Ch1Attenuator,
            1 => Control::Ch2Attenuator,
            2 => Control::Ch3Attenuator,
            3 => Control::Ch4Attenuator,
            _ => unreachable!()
        }
    }
}

/// Thunderscope Status Register
pub const ADDR_STATUS: usize = 0x8;

bitflags! {
    // See [doc/transfer_counter_register.txt] for details.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Status: u32 {
        const FifoOverflow     = 1<<30;
        const DatamoverError   = 1<<31;
    }
}

impl Status {
    pub fn overflow_cycles(&self) -> u32 {
        (self.bits() >> 16) & 0x3FFF
    }

    pub fn pages_moved(self) -> usize {
        ((self.bits() >> 0) & 0xFFFF) as usize
    }
}

// For the FIFO registers, see the documentation for the Xilinx AXI4-Stream FIFO v4.1 core.
// /https://confluence.slac.stanford.edu/download/attachments/240276688/pg080-axi-fifo-mm-s.pdf

/// FIFO Interrupt Status Register
pub const ADDR_FIFO_ISR: usize = 0x00020000;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FifoIsr: u32 {
        /// Receive FIFO Programmable Empty: Generated when the difference between the read and
        /// write pointers of the receive FIFO reaches the programmable EMPTY threshold value when
        ///  the FIFO is being emptied
        const RFPE  = 1<<19;
        /// Receive FIFO Programmable Full: This interrupt is generated when the difference between
        /// the read and write pointers of the receive FIFO reaches the programmable FULL threshold
        /// value.
        const RFPF  = 1<<20;
        /// Transmit FIFO Programmable Empty: This interrupt is generated when the difference
        /// between the read and write pointers of the transmit FIFO reaches the programmable EMPTY
        /// threshold value when the FIFO is being emptied.
        const TFPE  = 1<<21;
        /// Transmit FIFO Programmable Full: This interrupt is generated when the difference between
        /// the read and write pointers of the transmit FIFO reaches the programmable FULL threshold
        /// value.
        const TFPF  = 1<<22;
        /// Receive Reset Complete: This interrupt indicates that a reset of the receive logic has
        /// completed.
        const RRC   = 1<<23;
        /// Transmit Reset Complete: This interrupt indicates that a reset of the transmit logic has
        /// completed.
        const TRC   = 1<<24;
        /// Transmit Size Error: This interrupt is generated if the number of words (including
        /// partial words in the count) written to the transmit data FIFO does not match the value
        /// written to the transmit length register (bytes) divided by 4/8 and rounded up to
        /// the higher integer value for trailing byte fractions. Interrupts occur only for mismatch
        /// of word count (including partial words). Interrupts do not occur due to mismatch of byte
        /// count.
        const TSE   = 1<<25;
        /// Receive Complete: Indicates that at least one successful receive has completed and that
        /// the receive packet data and packet data length is available. This signal is not set for
        /// unsuccessful receives. This interrupt can represent more than one packet received, so it
        /// is important to check the receive data FIFO occupancy value to determine if additional
        /// receive packets are ready to be processed.
        const RC    = 1<<26;
        /// Transmit Complete: Indicates that at least one transmit has completed.
        const TC    = 1<<27;
        /// Transmit Packet Overrun Error: This interrupt is generated if an attempt is made to
        /// write to the transmit data FIFO when it is full. A reset of the transmit logic is
        /// required to recover.
        const TPOE  = 1<<28;
        /// Receive Packet Underrun Error: This interrupt occurs when an attempt is made to read
        /// the receive FIFO when it is empty. The data read is not valid. A reset of the receive
        /// logic is required to recover.
        const RPUE  = 1<<29;
        /// Receive Packet Overrun Read Error: This interrupt occurs when more words are read from
        /// the receive data FIFO than are in the packet being processed. Even though the FIFO is
        /// not empty, the read has gone beyond the current packet and removed the data from
        /// the next packet. A reset of the receive logic is required to recover.
        const RPORE = 1<<30;
        /// Receive Packet Underrun Read Error: This interrupt occurs when an attempt is made to
        /// read the receive length register when it is empty. The data read is not valid. A reset
        /// of the receive logic is required to recover.
        const RPURE = 1<<31;
    }
}

/// FIFO Interrupt Enable Register
pub const ADDR_FIFO_IER: usize = 0x00020004;

/// FIFO Transmit Reset Register
pub const ADDR_FIFO_TDFR: usize = 0x00020008;

// FIFO Transmit Vacancy Register
pub const ADDR_FIFO_TDFV: usize = 0x0002000c;

/// FIFO Transmit Data Register
pub const ADDR_FIFO_TDFD: usize = 0x00020010;

/// FIFO Transmit Length Register
pub const ADDR_FIFO_TLR: usize = 0x00020014;

/// FIFO Transmit Destination Register
pub const ADDR_FIFO_TDR: usize = 0x0002002C;
