use std::time::Duration;
use std::thread;

use crate::Result;
use crate::sys::Driver;
use crate::regs::axi::{self, Control, FifoIsr, Status};
use crate::regs::adc;
use crate::config::{Coupling, Termination};
use crate::params::{ChannelParameters, CoarseAttenuation, DeviceParameters};

const SPI_BUS_ADC: u8 = 0;
const SPI_BUS_PGA: [u8; 4] = [2, 3, 4, 5];

#[derive(Debug)]
pub struct Device {
    driver: Driver,
}

impl Device {
    pub fn new() -> Result<Device> {
        // FIXME: do this better
        #[cfg(any(target_os = "linux"))]
        let driver = Driver::new("/dev/xdma0")?;
        Ok(Device { driver })
    }

    pub fn with<F, R>(f: F) -> Result<R>
            where F: FnOnce(&mut Self) -> Result<R> {
        let mut device = Self::new()?;
        device.startup()?;
        let result = f(&mut device);
        device.shutdown()?;
        result
    }
}

impl Device {
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

    fn read_control(&mut self) -> Result<Control> {
        let value = Control::from_bits_retain(self.read_user_u32(axi::ADDR_CONTROL)?);
        log::debug!("read_control() = {:?}", value);
        Ok(value)
    }

    fn write_control(&mut self, value: Control) -> Result<()> {
        log::debug!("write_control({:?})", value);
        Ok(self.write_user_u32(axi::ADDR_CONTROL, value.bits())?)
    }

    fn modify_control<F: FnOnce(&mut Control)>(&mut self, f: F) -> Result<()> {
        let mut value = self.read_control()?;
        f(&mut value);
        self.write_control(value)
    }

    fn read_status(&mut self) -> Result<Status> {
        let value = Status::from_bits_retain(self.read_user_u32(axi::ADDR_STATUS)?);
        log::trace!("read_status() = {:?}", value);
        Ok(value)
    }

    fn write_fifo(&mut self, data: &[u8]) -> Result<()> {
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

    fn write_i2c(&mut self, i2c_addr: u8, data: &[u8]) -> Result<()> {
        log::debug!("write_i2c({:#08b}, {:02x?})", i2c_addr, data);
        let mut packet = Vec::<u8>::new();
        packet.push(0xff);        // select I2C
        packet.push(i2c_addr);
        packet.extend_from_slice(data);
        self.write_fifo(packet.as_ref())?;
        // the I2C engine doesn't use TLAST to detect packet boundaries and runs at 400 kHz;
        // make sure the engine is  done before releasing it. the delay has a 100% safety factor.
        thread::sleep(Duration::from_micros((50 * data.len()) as u64));
        Ok(())
    }

    // bus 0 (0xfd): ADC
    // bus 2..5 (0xfb..0xf7): PGAn
    fn write_spi(&mut self, spi_bus: u8, data: &[u8]) -> Result<()> {
        log::debug!("write_spi({:?}, {:02x?})", spi_bus, data);
        let mut packet = Vec::<u8>::new();
        packet.push(0xfd - spi_bus);
        packet.extend_from_slice(data);
        self.write_fifo(packet.as_ref())?;
        // the SPI engine doesn't use TLAST either, but it runs at 16 MHz. the delay is enough
        // for 160 bytes.
        thread::sleep(Duration::from_micros(10));
        Ok(())
    }

    fn write_pll_register(&mut self, reg_addr: u16, value: u8) -> Result<()> {
        log::debug!("write_pll_register({:#06x}, {:#04x})", reg_addr, value);
        self.write_i2c(0b11101000, &[
            0x02,                  // register write
            (reg_addr >> 8) as u8, // register address high
            (reg_addr >> 0) as u8, // register address low
            value
        ])
    }

    fn init_pll_registers(&mut self, init_words: &[u32]) -> Result<()> {
        for &init_word in init_words {
            self.write_pll_register((init_word >> 8) as u16, init_word as u8)?;
        }
        Ok(())
    }

    fn write_adc_register(&mut self, reg_addr: u8, value: u16) -> Result<()> {
        log::debug!("write_adc_register({:#04x}, {:#06x})", reg_addr, value);
        self.write_spi(SPI_BUS_ADC, &[
            reg_addr,
            (value >> 8) as u8,
            (value >> 0) as u8,
        ])
    }

    fn init_adc_registers(&mut self, init_pairs: &[(u8, u16)]) -> Result<()> {
        for &(reg_addr, value) in init_pairs {
            self.write_adc_register(reg_addr, value)?;
        }
        Ok(())
    }

    fn enable_adc_channels(&mut self, enabled: [bool; 4]) -> Result<()> {
        log::debug!("enable_adc_channels({:?})", enabled);
        // compute number of enabled ADC channels and ADC clock divisor
        // channels CH1..CH4 on the faceplate are mapped to IN4..IN1 on the ADC, so this function
        // has to perform a really annoying permutation
        let clkdiv; // in ADC
        let chnum;  // in ADC
        let chmux;  // in FPGA
        match enabled.iter().map(|&en| en as u8).sum() {
            1 => { clkdiv = 0; chnum = 1; chmux = Control::empty(); }
            2 => { clkdiv = 1; chnum = 2; chmux = Control::ChannelMux0; }
            3 => { clkdiv = 2; chnum = 4; chmux = Control::ChannelMux1; } // same as 4
            4 => { clkdiv = 2; chnum = 4; chmux = Control::ChannelMux1; }
            _ => panic!("unsupported channel configuration"),
        };
        // compute ADC input select permutation
        let insel = match chnum {
            1 => {
                let ch1_index = enabled.iter().rev().position(|&en| en).unwrap();
                [ch1_index, ch1_index, ch1_index, ch1_index]
            }
            2 => {
                let ch1_index = enabled.iter().rev().position(|&en| en).unwrap();
                let ch2_index = ch1_index + 1 +
                    enabled.iter().rev().skip(ch1_index + 1).position(|&en| en).unwrap();
                // this is permuted later again
                // the (faceplate) channel order in the data is ch1,ch2,ch1,ch2
                [ch2_index, ch2_index, ch1_index, ch1_index]
            }
            4 => {
                // the (faceplate) channel order in the data is ch1,ch2,ch3,ch4
                [3, 2, 1, 0]
            }
            _ => unreachable!()
        };
        // reconfigure ADC
        self.init_adc_registers(&[
            // power down ADC
            (adc::ADDR_HMCAD1520_POWER, 0x0200),
            // configure clock divisor and channel count
            (adc::ADDR_HMCAD1520_CHNUM_CLKDIV, (clkdiv << 8) | chnum),
            // power up ADC
            (adc::ADDR_HMCAD1520_POWER, 0x0000),
            // configure channel mapping
            (adc::ADDR_HMCAD1520_INSEL12, 0x0200 << insel[1] | 0x0002 << insel[0]),
            (adc::ADDR_HMCAD1520_INSEL34, 0x0200 << insel[3] | 0x0002 << insel[2]),
        ])?;
        // reconfigure channel mux in the FPGA
        self.modify_control(|val| {
            val.remove(Control::ChannelMux0 | Control::ChannelMux1);
            val.insert(chmux);
        })?;
        Ok(())
    }

    fn write_pga_command(&mut self, pga_bus: u8, command: u16) -> Result<()> {
        log::debug!("write_pga_command({:?}, {:#06x})", pga_bus, command);
        self.write_spi(pga_bus, &[
            0x00, // write command word
            (command >> 8) as u8,
            (command >> 0) as u8,
        ])
   }

   fn configure_pga(&mut self, index: usize, params: &ChannelParameters) -> Result<()> {
        self.write_pga_command(SPI_BUS_PGA[index],
            (1 << 10) | // always turn off auxiliary output to save power
            params.filtering.lmh6518_code() |
            params.amplification.lmh6518_code() |
            params.fine_attenuation.lmh6518_code()
        )
    }

   fn write_digipot_input(&mut self, addr: u8, input: u16) -> Result<()> {
        let command_data =
            ((addr as u16) << 12) | // device address
            (0b00 << 10) | // write
            ((input & 0x3ff) << 0);
        self.write_i2c(0b0101100, &[
            (command_data >> 8) as u8,
            (command_data >> 0) as u8,
        ])
   }

   fn write_trimdac_input(&mut self, channel: u8, input: u16) -> Result<()> {
        log::debug!("write_trimdac_input({:?}, {:#06x})", channel, input);
        self.write_i2c(0b1100000, &[
            0b01011_00_0 | ((channel & 0b11) << 1),
            (input >> 8) as u8,
            (input >> 0) as u8,
        ])
   }

   fn configure_digipot_trimdac(&mut self, index: usize, params: &ChannelParameters) -> Result<()> {
        const WIPER_ADDRESS: [u8; 4] = [0x6, 0x0, 0x1, 0x7];
        self.write_digipot_input(WIPER_ADDRESS[index],
            params.offset_magnitude.mcp4432t_503e_code())?;
        self.write_trimdac_input(index as u8,
            (1 << 15) | // always use Vref as reference
            params.offset_value.mcp4728_code()
        )?;
        Ok(())
    }

    fn enable_datamover(&mut self) -> Result<()> {
        // take the acquisition system out of reset
        self.modify_control(|val| val.insert(Control::DatamoverHaltN | Control::FpgaAcqResetN))?;
        Ok(())
    }

    fn disable_datamover(&mut self) -> Result<()> {
        // halt the data mover
        self.modify_control(|val| val.remove(Control::DatamoverHaltN))?;
        // wait for data mover to halt
        thread::sleep(Duration::from_millis(5));
        // reset the acquisition subsystem
        self.modify_control(|val| val.remove(Control::FpgaAcqResetN))?;
        Ok(())
    }

    pub fn configure(&mut self, params: &DeviceParameters) -> Result<()> {
        if *params == Default::default() {
            log::info!("configure(DeviceParameters::default())");
        } else {
            log::info!("configure({:#?})", params);
        }
        // configure the PGAs first; this keeps current consumption in check for the initial
        // `configure()` call from `startup()` by turning off the PGA aux outputs that (for all
        // PGAs together) consume almost 2W
        for (index, ch_params) in params.channels.iter().enumerate() {
            let ch_params = ch_params.unwrap_or_default();
            self.configure_pga(index, &ch_params)?;
        }
        // configure termination, coupling, and attenuator
        for (index, ch_params) in params.channels.iter().enumerate() {
            let ch_params = ch_params.unwrap_or_default();
            self.modify_control(|val| {
                match ch_params.termination {
                    Termination::Ohm1M => val.remove(Control::ch_termination(index)),
                    Termination::Ohm50 => val.insert(Control::ch_termination(index)),
                }
                match ch_params.coupling {
                    Coupling::AC => val.remove(Control::ch_coupling(index)),
                    Coupling::DC => val.insert(Control::ch_coupling(index)),
                }
                match ch_params.coarse_attenuation {
                    CoarseAttenuation::X50 => val.remove(Control::ch_attenuator(index)),
                    CoarseAttenuation::X1  => val.insert(Control::ch_attenuator(index)),
                }
            })?;
        }
        // configure voltage offset
        for (index, ch_params) in params.channels.iter().enumerate() {
            let ch_params = ch_params.unwrap_or_default();
            self.configure_digipot_trimdac(index, &ch_params)?;
        }
        // put data mover into reset (it cannot run without ADC clock or tolerate glitches on it)
        self.disable_datamover()?;
        // configure the ADC input selector, clock divisor, channel mapping, and FPGA data mux
        // this disables data mover first and (re-)enables it after
        self.enable_adc_channels([
            params.channels[0].is_some(),
            params.channels[1].is_some(),
            params.channels[2].is_some(),
            params.channels[3].is_some(),
        ])?;
        // take data mover out of reset now that ADC clock is available (again)
        self.enable_datamover()?;
        Ok(())
    }

    pub fn startup(&mut self) -> Result<()> {
        log::info!("startup()");
        // disable the data mover first and let it stop, in case it was running before
        // this prevents device crashes after unclean shutdowns (think ^C)
        self.disable_datamover()?;
        // enable the 3V3 rail and wait for it to stabilize
        self.modify_control(|val| val.insert(Control::ClockGenResetN | Control::Rail3V3Enabled))?;
        thread::sleep(Duration::from_millis(10));
        // The RSTN pin must be asserted once after power-up.
        // Reset should be asserted for at least 1μs.
        self.modify_control(|val| val.remove(Control::ClockGenResetN))?;
        thread::sleep(Duration::from_micros(100));
        // System software must wait at least 100μs after RSTN is deasserted
        // and wait for GLOBISR.BCDONE=1 before configuring the device.
        self.modify_control(|val| val.insert(Control::ClockGenResetN))?;
        thread::sleep(Duration::from_millis(1));
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
        thread::sleep(Duration::from_millis(10));
        // align the PLL output phases
        self.init_pll_registers(&[
            0x010002, 0x010042
        ])?;
        thread::sleep(Duration::from_millis(10));
        // configure the ADC, but leave it powered down or it'll be very unhappy about its clock
        self.init_adc_registers(&[
            // reset ADC
            (adc::ADDR_HMCAD1520_RESET, 0x0001),
            // power down ADC
            (adc::ADDR_HMCAD1520_POWER, 0x0200),
            // invert channels
            (adc::ADDR_HMCAD1520_INVERT, 0x007F),
            // adjust full scale value
            (adc::ADDR_HMCAD1520_FS_CNTRL, 0x0020),
            // enable coarse gain
            (adc::ADDR_HMCAD1520_GAIN_CFG, 0x0000),
            // set coarse gain for 4 channel mode
            (adc::ADDR_HMCAD1520_QUAD_GAIN, 0x9999),
            // set coarse gain for 1 and 2 channel modes
            (adc::ADDR_HMCAD1520_DUAL_GAIN, 0x0A99),
            // select 8-bit output (for HMCAD1520s)
            (adc::ADDR_HMCAD1520_RES_SEL, 0x0000),
            // set LVDS phase to 0 deg and drive strength to RSDS
            (adc::ADDR_HMCAD1520_LVDS_PHASE, 0x0060),
            (adc::ADDR_HMCAD1520_LVDS_DRIVE, 0x0222),
            // configure output in ramp test mode
            // (hmc::ADDR_HMCAD1520_LVDS_PATTERN, 0x0040),
        ])?;
        // enable the frontend
        // this causes a current spike due to PGA aux output being enabled by default, and *must*
        // be quickly followed by a call to `configure()` (with any parameters) to disable that
        // output as soon as possible, or risk an overcurrent condition
        self.modify_control(|val| val.insert(Control::Rail5VEnabled))?;
        thread::sleep(Duration::from_millis(5));
        // configure to a known (default) state
        // this also enables the data mover
        self.configure(&DeviceParameters::default())?;
        // done!
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<()> {
        log::info!("shutdown()");
        // disable the data mover first and let it stop, since it runs on ADC clock
        self.disable_datamover()?;
        // power down the frontend 5V0 and board 3V3
        self.write_control(Control::empty())?;
        Ok(())
    }

    pub fn stream_data<'a>(&'a mut self) -> Streamer<'a> {
        Streamer { device: self, cursor: None }
    }
}

#[derive(Debug)]
pub struct Streamer<'a> {
    device: &'a mut Device,
    cursor: Option<usize>,
}

impl<'a> std::io::Read for Streamer<'a> {
    fn read(&mut self, mut buffer: &mut [u8]) -> std::io::Result<usize> {
        const PAGE_BITS: usize = 12; // 4 Ki
        const MEMORY_SIZE: usize = 1 << 16 << PAGE_BITS; // 64 Ki x (1 << PAGE_BITS) = 256 Mi

        let mut written = 0;
        while buffer.len() > 0 {
            // check if there is an error condition set
            // these should never appear so long as the FPGA is functioning correctly
            let status = self.device.read_status()?;
            if status.intersects(Status::FifoOverflow | Status::DatamoverError) {
                log::error!("data mover failure, power cycle the device");
                panic!("data mover failure: {:?} (overflow by {} cycles)",
                    status, status.overflow_cycles());
            }
            // read any newly available data
            let next_cursor = status.pages_moved() << PAGE_BITS;
            let (prev_cursor, length) = match self.cursor {
                None => { // first ever read
                    self.cursor = Some(next_cursor);
                    continue
                }
                Some(prev_cursor) if next_cursor < prev_cursor => // wraparound
                    (prev_cursor, buffer.len().min(MEMORY_SIZE - prev_cursor)),
                Some(prev_cursor) => // no wraparound
                    (prev_cursor, buffer.len().min(next_cursor - prev_cursor)),
            };
            if length > 0 {
                log::debug!("streaming at {:08X}: reading {:08X}", prev_cursor, length);
                let (chunk, rest) = buffer.split_at_mut(length);
                self.device.driver.read_dma(prev_cursor, chunk)?;
                self.cursor = Some((prev_cursor + length) % MEMORY_SIZE);
                written += length;
                buffer = rest;
            } else {
                break
            }
        }
        Ok(written)
    }

}
