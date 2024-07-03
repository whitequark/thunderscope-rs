use std::time::Duration;
use std::thread::sleep;

use crate::Result;
use crate::sys::Driver;
use crate::regs::axi::{self, Control, FifoIsr, Status};
use crate::regs::hmc;

const SPI_BUS_ADC: u8 = 0;
const SPI_BUS_PGA: [u8; 4] = [2, 3, 4, 5];

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
        // enable the 3V3 rail and wait for it to stabilize
        self.write_control(Control::ClockGenResetN | Control::Rail3V3Enabled)?;
        sleep(Duration::from_millis(10));

        // The RSTN pin must be asserted once after power-up.
        // Reset should be asserted for at least 1μs.
        self.modify_control(|val| val.remove(Control::ClockGenResetN))?;
        sleep(Duration::from_micros(100));

        // System software must wait at least 100μs after RSTN is deasserted
        // and wait for GLOBISR.BCDONE=1 before configuring the device.
        self.modify_control(|val| val.insert(Control::ClockGenResetN))?;
        sleep(Duration::from_millis(1));

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
        sleep(Duration::from_millis(10));

        // align the PLL output phases
        self.init_pll_registers(&[
            0x010002, 0x010042
        ])?;
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
        self.modify_control(|val| val.insert(Control::Rail5VEnabled))?;
        sleep(Duration::from_millis(5));

        // turn off aux output of PGAs as soon as rail is up to keep current consumption in check
        self.set_pga(0)?;
        self.set_pga(1)?;
        self.set_pga(2)?;
        self.set_pga(3)?;

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

    pub fn set_pga(&mut self, channel: usize) -> Result<()> {
        // Hardcoded gain and anti-aliasing filter settings for now
        // Will need some sort of structure to hold this info per channel
        let aux_off: bool = true; // high disables aux output
        let filter_val: u8 = 0x0; // 4 bits, 0 is full BW
        let preamp_high_gain: bool = false;
        let ladder_atten_val: u8 = 0x9; // 4 bits, 9 is POR value

        self.write_pga_command(SPI_BUS_PGA[channel],
            (aux_off as u16) << 10 |
            (filter_val as u16) << 5 |
            (preamp_high_gain as u16) << 4 |
            (ladder_atten_val as u16) & 0x0f
        )
    }

    pub fn read_data(&mut self) -> Result<()> {
        let mut data = Vec::new();
        data.resize(1 << 23, 0);
        self.driver.read_d2h(0, &mut data[..])?;
        let status = self.read_status()?;
        log::debug!("pages_moved = {:?}", status.pages_moved());
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

    pub fn modify_control<F: FnOnce(&mut Control)>(&mut self, f: F) -> Result<()> {
        let mut value = self.read_control()?;
        f(&mut value);
        self.write_control(value)
    }

    pub fn read_status(&mut self) -> Result<Status> {
        let value = Status::from_bits_retain(self.read_user_u32(axi::ADDR_STATUS)?);
        log::debug!("read_status() = {:?}", value);
        Ok(value)
    }

    pub fn disable_datamover(&mut self) -> Result<()> {
        // halt the data mover
        self.modify_control(|val| val.remove(Control::DatamoverHaltN))?;
        // wait for data mover to halt
        sleep(Duration::from_millis(5));
        // reset the acquisition subsystem
        self.modify_control(|val| val.remove(Control::FpgaAcqResetN))?;
        Ok(())
    }

    pub fn enable_datamover(&mut self) -> Result<()> {
        // take the acquisition system out of reset
        self.modify_control(|val| val.insert(Control::DatamoverHaltN | Control::FpgaAcqResetN))?;
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
        packet.push(0xff);        // select I2C
        packet.push(i2c_addr);
        packet.extend_from_slice(data);
        self.write_fifo(packet.as_ref())?;
        // the I2C engine doesn't use TLAST to detect packet boundaries and runs at 400 kHz;
        // make sure the engine is  done before releasing it. the delay has a 100% safety factor.
        sleep(Duration::from_micros((50 * data.len()) as u64));
        Ok(())
    }

    // bus 0 (0xfd): ADC
    // bus 2..5 (0xfb..0xf7): PGAn
    pub fn write_spi(&mut self, spi_bus: u8, data: &[u8]) -> Result<()> {
        log::debug!("write_spi({:?}, {:02x?})", spi_bus, data);
        let mut packet = Vec::<u8>::new();
        packet.push(0xfd - spi_bus);
        packet.extend_from_slice(data);
        self.write_fifo(packet.as_ref())?;
        // the SPI engine doesn't use TLAST either, but it runs at 16 MHz. the delay is enough
        // for 160 bytes.
        sleep(Duration::from_micros(10));
        Ok(())
    }

    pub fn write_pll_register(&mut self, reg_addr: u16, value: u8) -> Result<()> {
        log::debug!("write_pll_register({:#06x}, {:#04x})", reg_addr, value);
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
        log::debug!("write_adc_register({:#04x}, {:#06x})", reg_addr, value);
        self.write_spi(SPI_BUS_ADC, &[
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

    pub fn write_pga_command(&mut self, pga_bus: u8, command: u16) -> Result<()> {
        log::debug!("write_pga_command({:?}, {:#06x})", pga_bus, command);
        self.write_spi(pga_bus, &[
            0x00, // write command word
            (command >> 8) as u8,
            (command >> 0) as u8,
        ])
    }
}
