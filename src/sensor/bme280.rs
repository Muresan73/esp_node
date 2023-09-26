use bme280_rs::{Bme280, Configuration, Oversampling, SensorMode};
use embassy_time::Delay;
use hal::i2c::I2C;
use hal::i2c::{self, Error};

use hal::peripherals::I2C0;
use log::{error, info};

#[derive(Debug)]
pub enum Bme280Error {
    // I2cDriverError(EspError),
    SensorInitError(i2c::Error),
}

pub trait Bme280Extention {
    fn configure(&mut self) -> Result<(), i2c::Error>;
    fn read_temperature_status(&mut self) -> Option<TempStatus>;
    fn read_pressure_status(&mut self) -> Option<PressureStatus>;
    fn read_humidity_status(&mut self) -> Option<HumidityStatus>;
}

enum TempStatus {
    Freezing,
    Cold,
    Optimal,
    Hot,
}

enum HumidityStatus {
    Dry,
    Optimal,
    Moist,
    Wet,
}

enum PressureStatus {
    Low,
    Optimal,
    High,
}

impl Bme280Extention for Bme280<I2C<'_, I2C0>, Delay> {
    fn configure(&mut self) -> Result<(), Error> {
        self.init()?;

        // 4. Read and print the sensor's device ID.
        match self.chip_id() {
            Ok(id) => {
                info!("Device ID BME280: {:#02x}", id);
            }
            Err(e) => {
                error!("{:?}", e);
            }
        };

        let configuration = Configuration::default()
            .with_temperature_oversampling(Oversampling::Oversample1)
            .with_pressure_oversampling(Oversampling::Oversample1)
            .with_humidity_oversampling(Oversampling::Oversample1)
            .with_sensor_mode(SensorMode::Normal);
        self.set_sampling_configuration(configuration)?;
        Ok(())
    }
    fn read_temperature_status(&mut self) -> Option<TempStatus> {
        let temp = self.read_temperature().ok()??;
        match temp {
            t if t < 0.0 => Some(TempStatus::Freezing),
            t if t < 18.0 => Some(TempStatus::Cold),
            t if t < 25.0 => Some(TempStatus::Optimal),
            _ => Some(TempStatus::Hot),
        }
    }
    fn read_humidity_status(&mut self) -> Option<HumidityStatus> {
        let humidity = self.read_humidity().ok()??;
        match humidity {
            h if h < 30.0 => Some(HumidityStatus::Dry),
            h if h < 50.0 => Some(HumidityStatus::Optimal),
            h if h < 70.0 => Some(HumidityStatus::Moist),
            _ => Some(HumidityStatus::Wet),
        }
    }
    fn read_pressure_status(&mut self) -> Option<PressureStatus> {
        let pressure = self.read_pressure().ok()??;
        match pressure {
            p if p < 1000.0 => Some(PressureStatus::Low),
            p if p < 1013.0 => Some(PressureStatus::Optimal),
            _ => Some(PressureStatus::High),
        }
    }
}
