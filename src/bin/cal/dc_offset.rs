use std::io::Read;

use thunderscope::{ChannelConfiguration, DeviceCalibration, DeviceConfiguration, DeviceParameters};

fn main() -> thunderscope::Result<()> {
    env_logger::init();
    thunderscope::Device::with(|device| {

        //TODO THIRD: Iterate across channels

        let config = DeviceConfiguration {
            channels: [Some(ChannelConfiguration::default()), None, None, None]
        };
        

        let params = DeviceParameters::derive(&DeviceCalibration::default(), &config);
        
        //TODO SECOND: Iterate across gain ranges
        //Line to set gain goes here

        device.configure(&params)?;


        let mut samples = vec![0; 200000];
        device.stream_data().read_exact(samples.as_mut())?; // let the signal path stabilize
        device.stream_data().read_exact(samples.as_mut())?;

        let count: usize = 256;
        println!("channel gain: {:.2} dB", params.gain(0));
        let full_scale = params.full_scale(0);
        println!("full scale: {:-.3} V to {:+.3} V", -full_scale/2.0, full_scale/2.0);
        
        let vavg = (samples.iter().take(count).map(|&code| params.code_to_volts(0, code as i8)).sum::<f32>())/count as f32;
        println!("average voltage offset:\n  {:.2?}", vavg);

        //TODO FIRST: loop to find digipot and VDAC values 
        //Keep digipot at max value, start at highest VDAC value w/ correct polarity for the offset
        //If this value can't push the offset into the opposite polarity, then try lower digipot value and loop


        Ok(())
    })
}
