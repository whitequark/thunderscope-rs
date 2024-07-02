use std::time::Duration;
use std::thread::sleep;

use crate::Result;
use crate::sys::Driver;
use crate::regs::axi::{self, Control, FifoIsr, Status};
use crate::regs::hmc;

#[derive(Debug)]
pub struct Device<D: Driver> {
    driver: D,
}

impl Device<crate::sys::imp::ThunderscopeDriverImpl> {
    pub fn new() -> Result<Device<crate::sys::imp::ThunderscopeDriverImpl>> {
        // FIXME: do this better
        #[cfg(any(target_os = "linux"))]
        let driver = crate::sys::imp::ThunderscopeDriverImpl::new("/dev/xdma0")?;
        Ok(Device { driver })
    }
}

impl<D: Driver> Device<D> {
    pub fn startup(&mut self) -> Result<()> {
        let mut control = Control::Rail3V3Enabled | Control::ClockGenResetN;
        self.write_control(control)?;
        sleep(Duration::from_millis(10)); // wait for the rail to stabilize

        // reset the PLL
        control.remove(Control::ClockGenResetN);
        self.write_control(control)?;
        // wait for the PLL to reset
        sleep(Duration::from_millis(10));
        // start the PLL
        control.insert(Control::ClockGenResetN);
        self.write_control(control)?;
        // wait for the PLL to start
        sleep(Duration::from_millis(10));
        // configure the PLL using the Rev4 blob
        self.init_pll_registers(&[
            0x042308, 0x000301, 0x000402, 0x000521,
            0x000701, 0x010042, 0x010100, 0x010201,
            0x010600, 0x010700, 0x010800, 0x010900,
            0x010A20, 0x010B03, 0x012160, 0x012790,
            0x014100, 0x014200, 0x014300, 0x014400,
            0x0145A0, 0x015300, 0x015450, 0x0155CE,
            0x018000, 0x020080, 0x020105, 0x025080,
            0x025102, 0x04300C, 0x043000
        ])?;
        // wait for the PLL to configure
        sleep(Duration::from_millis(10));
        // align the PLL output phases
        self.init_pll_registers(&[
            0x010002, 0x010042
        ])?;
        // wait for the PLL output phases to align
        sleep(Duration::from_millis(10));

        // configure the ADC, but leave it powered down or it'll be very unhappy about its clock
        self.init_adc_registers(&[
            // reset ADC
            (hmc::ADDR_HMCAD1520_RESET, 0x0001),
            // power down ADC
            (hmc::ADDR_HMCAD1520_POWER, 0x0200),
            // invert channels
            (hmc::ADDR_HMCAD1520_INVERT, 0x007F),
            // adjust full scale value
            (hmc::ADDR_HMCAD1520_FS_CNTRL, 0x0010),
            // course gain on
            (hmc::ADDR_HMCAD1520_GAIN_CFG, 0x0000),
            // course gain 4-CH set
            (hmc::ADDR_HMCAD1520_QUAD_GAIN, 0x9999),
            // course gain 1-CH & 2-CH set
            (hmc::ADDR_HMCAD1520_DUAL_GAIN, 0x0A99),
            // select 8-bit mode for HMCAD1520s
            (hmc::ADDR_HMCAD1520_RES_SEL, 0x0000),
            // set LVDS phase to 0 deg & drive strength to RSDS
            (hmc::ADDR_HMCAD1520_LVDS_PHASE, 0x0060),
            (hmc::ADDR_HMCAD1520_LVDS_DRIVE, 0x0222),
            // set ADC to ramp test mode
            (hmc::ADDR_HMCAD1520_LVDS_PATTERN, 1<<6),
        ])?;
        // enable all ADC channels; this also enables the data mover
        self.set_adc_channels([true, true, true, true])?;

        // enable the frontend
        control.insert(Control::Rail5VEnabled);
        self.write_control(control)?;

        // done!
        Ok(())
    }

    pub fn set_adc_channels(&mut self, enabled: [bool; 4]) -> Result<()> {
        // compute number of enabled ADC channels and ADC clock divisor
        let clkdiv;
        let chnum;
        match enabled.iter().map(|&en| en as u8).sum() {
            1 => { clkdiv = 0; chnum = 1; }
            2 => { clkdiv = 1; chnum = 2; }
            3 => { clkdiv = 2; chnum = 4; } // same as 4
            4 => { clkdiv = 2; chnum = 4; }
            _ => panic!("unsupported channel configuration"),
        };
        // compute input select permutation
        let insel = match chnum {
            1 => {
                let ch1_index = enabled.iter().position(|&en| en).unwrap();
                [ch1_index, ch1_index, ch1_index, ch1_index]
            }
            2 => {
                let ch1_index = enabled.iter().position(|&en| en).unwrap();
                let ch2_index = enabled.iter().skip(ch1_index + 1).position(|&en| en).unwrap();
                [ch1_index, ch1_index, ch2_index, ch2_index]
            }
            4 => [3, 2, 1, 0],
            _ => unreachable!()
        };
        // put data mover into reset (it cannot run without ADC clock)
        self.disable_datamover()?;
        // reconfigure ADC
        self.init_adc_registers(&[
            // power down ADC
            (hmc::ADDR_HMCAD1520_POWER, 0x0200),
            // configure clock divisor and channel count
            (hmc::ADDR_HMCAD1520_CHNUM_CLKDIV, (clkdiv << 8) | chnum),
            // power up ADC
            (hmc::ADDR_HMCAD1520_POWER, 0x0000),
            // configure channel mapping
            (hmc::ADDR_HMCAD1520_INSEL12, 0x0200 << insel[1] | 0x0002 << insel[0]),
            (hmc::ADDR_HMCAD1520_INSEL34, 0x0200 << insel[3] | 0x0002 << insel[2]),
        ])?;
        // take data mover out of reset now that ADC clock is available
        self.enable_datamover()?;
        Ok(())
    }

    pub fn read_data(&mut self) -> Result<()> {
        let mut data = Vec::new();
        data.resize(1 << 23, 0);
        self.driver.read_d2h(0, &mut data[..])?;
        std::fs::write("test.data", &data[..])?;
        Ok(())
    }

    pub fn teardown(&mut self) -> Result<()> {
        self.disable_datamover()?;
        // power down the frontend and board 3V3
        self.write_control(Control::empty())?;
        Ok(())
    }

    fn read_user_u32(&mut self, addr: usize) -> Result<u32> {
        let mut bytes = [0u8; 4];
        self.driver.read_user(addr, &mut bytes[..])?;
        let data = u32::from_le_bytes(bytes);
        log::trace!("read_user_u32({:#x}) = {:#x}", addr, data);
        Ok(data)
    }

    fn write_user_u32(&mut self, addr: usize, data: u32) -> Result<()> {
        log::trace!("write_user_u32({:#x}, {:#x})", addr, data);
        let bytes = u32::to_le_bytes(data);
        self.driver.write_user(addr, &bytes[..])?;
        Ok(())
    }

    pub fn read_control(&mut self) -> Result<Control> {
        let value = Control::from_bits_retain(self.read_user_u32(axi::ADDR_CONTROL)?);
        log::debug!("read_control() = {:?}", value);
        Ok(value)
    }

    pub fn write_control(&mut self, value: Control) -> Result<()> {
        log::debug!("write_control({:?})", value);
        Ok(self.write_user_u32(axi::ADDR_CONTROL, value.bits())?)
    }

    pub fn read_status(&mut self) -> Result<Status> {
        let value = Status::from_bits_retain(self.read_user_u32(axi::ADDR_STATUS)?);
        log::debug!("read_status() = {:?}", value);
        Ok(value)
    }

    pub fn disable_datamover(&mut self) -> Result<()> {
        let mut control = self.read_control()?;
        // halt the data mover
        control.remove(Control::DatamoverHaltN);
        self.write_control(control)?;
        // wait for data mover to halt
        sleep(Duration::from_millis(5));
        // reset the acquisition subsystem
        control.remove(Control::FpgaAcqResetN);
        self.write_control(control)?;
        Ok(())
    }

    pub fn enable_datamover(&mut self) -> Result<()> {
        let mut control = self.read_control()?;
        // take the acquisition system out of reset
        control.insert(Control::DatamoverHaltN | Control::FpgaAcqResetN);
        self.write_control(control)?;
        Ok(())
    }

    pub fn write_fifo(&mut self, data: &[u8]) -> Result<()> {
        log::trace!("write_fifo({:02x?})", data);
        // enqueue data into the FIFO
        for &byte in data {
            self.write_user_u32(axi::ADDR_FIFO_TDFD, byte as u32)?;
        }
        // start transmission; the FIFO is configured for 32-bit datapath length, but the top
        // three bytes are ignored by the SPI/I2C gateware connected to it
        self.write_user_u32(axi::ADDR_FIFO_TLR, data.len() as u32 * 4)?;
        // clear transmit complete flag
        self.write_user_u32(axi::ADDR_FIFO_ISR, FifoIsr::TC.bits())?;
        // wait for the packet to be transmitted
        loop {
            let isr = FifoIsr::from_bits_retain(self.read_user_u32(axi::ADDR_FIFO_ISR)?);
            assert!(!isr.contains(FifoIsr::TPOE), "Transmit FIFO overflow! ISR = {:?}", isr);
            if isr.contains(FifoIsr::TC) { break } // done!
        }
        Ok(())
    }

    pub fn write_i2c(&mut self, i2c_addr: u8, data: &[u8]) -> Result<()> {
        log::debug!("write_i2c({:#08b}, {:02x?})", i2c_addr, data);
        let mut packet = Vec::<u8>::new();
        packet.push(0xff);     // select I2C
        packet.push(i2c_addr);
        packet.extend_from_slice(data);
        self.write_fifo(&packet[..])
    }

    pub fn write_pll_register(&mut self, reg_addr: u16, value: u8) -> Result<()> {
        log::debug!("set_pll_register({:#06x}, {:#04x})", reg_addr, value);
        self.write_i2c(0b11101000, &[
            0x02,                  // register write
            (reg_addr >> 8) as u8, // register address high
            (reg_addr >> 0) as u8, // register address low
            value
        ])
    }

    pub fn init_pll_registers(&mut self, init_words: &[u32]) -> Result<()> {
        for &init_word in init_words {
            self.write_pll_register((init_word >> 8) as u16, init_word as u8)?;
        }
        Ok(())
    }

    pub fn write_adc_register(&mut self, reg_addr: u8, value: u16) -> Result<()> {
        log::debug!("set_adc_register({:#04x}, {:#06x})", reg_addr, value);
        self.write_fifo(&[
            0xfd,
            reg_addr,
            (value >> 8) as u8,
            (value >> 0) as u8,
        ])
    }

    pub fn init_adc_registers(&mut self, init_pairs: &[(u8, u16)]) -> Result<()> {
        for &(reg_addr, value) in init_pairs {
            self.write_adc_register(reg_addr, value)?;
        }
        Ok(())
    }
}
