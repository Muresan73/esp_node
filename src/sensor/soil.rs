use core::fmt;
use hal::{
    adc::{AdcPin, ADC, ADC1},
    gpio::{Analog, GpioPin},
    prelude::*,
};
const MAX_WET: u16 = 2800;
const MAX_DRY: u16 = 1300;

const MOISTURE_RANGE: u16 = MAX_WET - MAX_DRY;
const FULL_PRECENTAGE: f32 = 100.0;
const NO_PRECENTAGE: f32 = 0.0;

#[derive(Clone, PartialEq)]
pub enum SoilStatus {
    Dry,
    Optimal,
    Damp,
    Wet,
}

impl fmt::Display for SoilStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SoilStatus::Dry => write!(f, "Dry🔥"),
            SoilStatus::Optimal => write!(f, "Optimal 💚"),
            SoilStatus::Damp => write!(f, "Damp ⚠️"),
            SoilStatus::Wet => write!(f, "Wet 💦"),
        }
    }
}
#[derive(Debug)]
pub enum MoistureError {
    SensorNotConnected(),
    EspError(),
}
type MoistureResult<T> = Result<T, MoistureError>;

pub struct SoilMoisture<'a> {
    adc_driver: ADC<'a, ADC1>,
    adc_pin: AdcPin<GpioPin<Analog, 36>, ADC1>,
}

impl<'a> SoilMoisture<'a> {
    pub fn new(adc: ADC<'a, ADC1>, pin: AdcPin<GpioPin<Analog, 36>, ADC1>) -> MoistureResult<Self> {
        Ok(SoilMoisture {
            adc_driver: adc,
            adc_pin: pin,
        })
    }

    /// Get the raw read of the moisture result, analog read
    pub fn get_raw_moisture(&mut self) -> MoistureResult<u16> {
        let value: u16 = nb::block!(self.adc_driver.read(&mut self.adc_pin)).unwrap();
        Ok(value)
    }

    /// Get precentage read of the moisture.
    pub fn get_moisture_precentage(&mut self) -> MoistureResult<f32> {
        let measurement = match self.get_raw_moisture()? {
            msmnt if msmnt < 1000 => Err(MoistureError::SensorNotConnected()),
            msmnt => Ok(msmnt),
        }?;

        if measurement < MAX_DRY {
            return Ok(NO_PRECENTAGE);
        } else if measurement > MAX_WET {
            return Ok(FULL_PRECENTAGE);
        }

        let value_diff = measurement - MAX_DRY;
        Ok((value_diff as f32 / MOISTURE_RANGE as f32) * FULL_PRECENTAGE)
    }

    /// Get the status of the soil
    /// Dry -> 0-20%
    /// Optimal -> 20-40%
    /// Da -> 40-55%
    /// Wet -> 55-100%
    pub fn get_soil_status(&mut self) -> Option<SoilStatus> {
        let percentage = self.get_moisture_precentage().ok()?;

        match percentage {
            p if p < 20.0 => Some(SoilStatus::Dry),
            p if p < 40.0 => Some(SoilStatus::Optimal),
            p if p < 55.0 => Some(SoilStatus::Damp),
            _ => Some(SoilStatus::Wet),
        }
    }
}
