use std::io::Read;

use thunderscope::{Amplification, ChannelConfiguration, ChannelParameters, CoarseAttenuation};
use thunderscope::{Device, DeviceCalibration, DeviceConfiguration, DeviceParameters};
use thunderscope::{OffsetMagnitude, OffsetValue, FineAttenuation};

fn acquire_average_voltage(device: &Device, channel: usize, channel_params: ChannelParameters) 
        -> thunderscope::Result<f32> {
    const SAMPLE_COUNT: usize = 1 << 16;

    let mut config = DeviceConfiguration { channels: [None; 4] };
    config.channels[channel] = Some(ChannelConfiguration::default());
    let mut params = DeviceParameters::derive(&DeviceCalibration::default(), &config);
    params.channels[channel] = Some(channel_params.clone());

    let gain = params.gain(channel);
    let full_scale = params.full_scale(channel);
    let offset_magnitude = channel_params.offset_magnitude;
    let offset_value = channel_params.offset_value;
    println!("parameters:");
    println!("  channel gain:     {:.2} dB", gain);
    println!("  full scale:       {:-.6}..{:+.6} V", -full_scale / 2.0, full_scale / 2.0);
    println!("  offset magnitude: {:.2} ohm", offset_magnitude.ohms());
    println!("  offset value:     {:?}", offset_value);

    device.configure(&params)?;
    std::thread::sleep(std::time::Duration::from_millis(500));

    let mut samples = vec![0; SAMPLE_COUNT];
    device.stream_data().read_exact(samples.as_mut())?;

    let samples = samples.into_iter().map(|code| code as i8).collect::<Vec<_>>();
    let code_min = *samples.iter().min().unwrap();
    let code_max = *samples.iter().max().unwrap();
    let voltage_avg = samples.iter().map(|&code| params.code_to_volts(channel, code)).sum::<f32>() / SAMPLE_COUNT as f32;
    println!("measurement:");
    println!("  minimum: {:+.6} V ({:+} LSB)", params.code_to_volts(channel, code_min), code_min);
    println!("  maximum: {:+.6} V ({:+} LSB)", params.code_to_volts(channel, code_max), code_max);
    println!("  average: {:+.6} V", voltage_avg);

    Ok(voltage_avg)
}

fn calibrate_offset(device: &Device, channel: usize, channel_params: ChannelParameters)
        -> thunderscope::Result<(OffsetMagnitude, OffsetValue)> {

    println!("==> calibrate_offset {} {:?}", channel, channel_params);

    for offset_magnitude in (0x00..=0x7f).rev() {
        let channel_params = ChannelParameters {
            offset_magnitude: OffsetMagnitude::from_mcp4432t_503e_code(offset_magnitude),
            ..channel_params
        };
        // let average_voltage = acquire_average_voltage(device, channel, channel_params)?;
        // let offset_value_range = 
        //     if average_voltage > 0.0 {
        //         (0x0000..=0x3fff).rev()
        //     } else {
        //         (0x0000..=0x3fff)
        //     };
        for offset_value in (0x0000..=0x3fff).step_by(200).rev() {
            let channel_params = ChannelParameters {
                offset_value: OffsetValue::from_mcp4728_code(offset_value),
                ..channel_params
            };
            let average_voltage = acquire_average_voltage(device, channel, channel_params)?;
            if average_voltage < 0.0001 && average_voltage > -0.0001 {
                    return Ok((channel_params.offset_magnitude, channel_params.offset_value))
            }
        }
    }

    panic!("failed to calibrate channel {} with params {:?}", channel, channel_params)
}

fn main() -> thunderscope::Result<()> {
    env_logger::init();
    thunderscope::Device::with(|device| {
        #[allow(dead_code)]
        #[derive(Debug, Clone, Copy)]
        struct CalibrationRow {
            // inputs
            coarse_attenuation: CoarseAttenuation,
            amplification: Amplification,
            fine_attenuation: FineAttenuation,
            // outputs
            offset_magnitude: OffsetMagnitude,
            offset_value: OffsetValue,
        }

        //TODO FIRST: loop to find digipot and VDAC values 
        //Keep digipot at max value, start at highest VDAC value w/ correct polarity for the offset
        //If this value can't push the offset into the opposite polarity, then try lower digipot value and loop

        //TODO SECOND: Iterate across gain ranges

        //TODO THIRD: Iterate across channels

        let mut calibration_table: [Vec<CalibrationRow>; 4] = 
            [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for channel in 0..3 {
            for &coarse_attenuation in CoarseAttenuation::ALL.iter() {
                for &amplification in Amplification::ALL.iter() {
                    for &fine_attenuation in FineAttenuation::ALL.iter() {
                        let (offset_magnitude, offset_value) = 
                            calibrate_offset(device, channel, ChannelParameters {
                                probe_attenuation: 0.0,
                                coarse_attenuation,
                                amplification,
                                fine_attenuation,
                                ..Default::default()
                            })?;
                        calibration_table[channel].push(CalibrationRow { 
                            coarse_attenuation, 
                            amplification,
                            fine_attenuation, 
                            offset_magnitude,
                            offset_value 
                        })
                    }
                }
            }            
        }

        for channel in 0..3 {
            println!("calibration table for channel {}:", channel);
            for row in calibration_table[channel].iter() {
                println!("  {:?}", row)
            }
        }

        Ok(())
    })
}
