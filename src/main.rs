#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use dotenvy_macro::dotenv;
use embassy_executor::_export::StaticCell;
use embassy_net::{Config, Stack, StackResources};
use embassy_time::{Duration, Timer};
use embedded_svc::wifi::{ClientConfiguration, Configuration, Wifi};
use esp_backtrace as _;
use esp_println::println;
use esp_wifi::wifi::{WifiController, WifiDevice, WifiEvent, WifiMode, WifiState};
use esp_wifi::{initialize, EspWifiInitFor};
use hal::adc::{AdcConfig, Attenuation, ADC, ADC1};
use hal::clock::ClockControl;
use hal::embassy::executor::Executor;
use hal::i2c::I2C;
use hal::peripherals::{Interrupt, I2C0};
use hal::{embassy, peripherals::Peripherals, prelude::*, timer::TimerGroup, IO};
use hal::{interrupt, Rng, Rtc};

mod sensor;
use log::info;
use sensor::bme280::Bme280Extention;
use sensor::soil::SoilMoisture;

const SSID: &str = dotenv!("SSID");
const PASSWORD: &str = dotenv!("PASSWORD");

macro_rules! singleton {
    ($val:expr) => {{
        type T = impl Sized;
        static STATIC_CELL: StaticCell<T> = StaticCell::new();
        let (x,) = STATIC_CELL.init(($val,));
        x
    }};
}

static EXECUTOR: StaticCell<Executor> = StaticCell::new();

#[entry]
fn main() -> ! {
    // #[cfg(feature = "log")]
    esp_println::logger::init_logger(log::LevelFilter::Info);

    let peripherals = Peripherals::take();

    let mut system = peripherals.DPORT.split();
    let clocks = ClockControl::max(system.clock_control).freeze();

    let timer = hal::timer::TimerGroup::new(
        peripherals.TIMG1,
        &clocks,
        &mut system.peripheral_clock_control,
    )
    .timer0;

    let mut rtc = Rtc::new(peripherals.RTC_CNTL);
    rtc.rwdt.disable();

    let mut delay = hal::Delay::new(&clocks);

    let mut rng = Rng::new(peripherals.RNG);
    let seed = rng.random() as u64;

    let init = initialize(
        EspWifiInitFor::Wifi,
        timer,
        rng,
        system.radio_clock_control,
        &clocks,
    )
    .unwrap();

    let (wifi, ..) = peripherals.RADIO.split();
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiMode::Sta).unwrap();

    let timer_group0 = TimerGroup::new(
        peripherals.TIMG0,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    embassy::init(&clocks, timer_group0.timer0);

    let config = Config::dhcpv4(Default::default());

    // Init network stack
    let stack = &*singleton!(Stack::new(
        wifi_interface,
        config,
        singleton!(StackResources::<3>::new()),
        seed
    ));

    // ================  init sensors  ================

    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);
    let i2c0 = hal::i2c::I2C::new(
        peripherals.I2C0,
        io.pins.gpio21,
        io.pins.gpio22,
        400u32.kHz(),
        &mut system.peripheral_clock_control,
        &clocks,
    );

    // Create ADC instances
    let analog = peripherals.SENS.split();
    let mut adc1_config = AdcConfig::new();
    let adc_pin =
        adc1_config.enable_pin(io.pins.gpio36.into_analog(), Attenuation::Attenuation11dB);
    let adc1 = ADC::<ADC1>::adc(analog.adc1, adc1_config).unwrap();
    let soil_sensor = SoilMoisture::new(adc1, adc_pin).unwrap();

    // ================

    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(connection(controller)).ok();
        spawner.spawn(net_task(stack)).ok();
        spawner.spawn(task(stack)).ok();
        spawner.spawn(measurements(i2c0, soil_sensor)).ok();
        spawner.spawn(sleepy_joe(rtc, delay)).ok();
    })
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");
    println!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        if let WifiState::StaConnected = esp_wifi::wifi::get_wifi_state() {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: SSID.into(),
                password: PASSWORD.into(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi");
            controller.start().await.unwrap();
            println!("Wifi started!");
        }
        println!("About to connect...");

        match controller.connect().await {
            Ok(_) => println!("Wifi connected!"),
            Err(e) => {
                println!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static>>) {
    stack.run().await
}

#[embassy_executor::task]
async fn sleepy_joe(mut rtc: Rtc<'static>, mut delay: hal::Delay) {
    loop {
        Timer::after(Duration::from_secs(60)).await;
        let sleep_timer =
            hal::rtc_cntl::sleep::TimerWakeupSource::new(core::time::Duration::from_secs(600));
        println!("up and runnning!");
        let reason = hal::rtc_cntl::get_reset_reason(hal::Cpu::ProCpu)
            .unwrap_or(hal::rtc_cntl::SocResetReason::ChipPowerOn);
        println!("reset reason: {:?}", reason);
        let wake_reason = hal::rtc_cntl::get_wakeup_cause();
        println!("wake reason: {:?}", wake_reason);

        rtc.sleep_deep(&[&sleep_timer], &mut delay);
    }
}

#[embassy_executor::task]
async fn measurements(i2c: I2C<'static, I2C0>, mut soil_sensor: SoilMoisture<'static>) {
    println!("start measurements task");
    interrupt::enable(Interrupt::I2C_EXT0, interrupt::Priority::Priority1).unwrap();

    let mut bme280 = bme280_rs::Bme280::new(i2c, embassy_time::Delay);
    if bme280.init().and_then(|_| bme280.configure()).is_err() {
        return;
    }

    println!("wait init to complet");
    Timer::after(Duration::from_secs(5)).await;

    loop {
        if let Ok(measurement) = bme280.read_humidity() {
            match measurement {
                Some(value) => println!("Humidity: {:.2}%", value),
                None => println!("Error reading humidity"),
            }
        }
        match soil_sensor.get_moisture_precentage() {
            Ok(value) => info!("Soil moisture: {:.2}%", value),
            _ => println!("Soil sensor not connected"),
        };
        info!(
            "Soil moisture: {:.2}%",
            soil_sensor.get_raw_moisture().unwrap()
        );
        Timer::after(Duration::from_secs(10)).await;
    }
}

#[embassy_executor::task]
async fn task(stack: &'static Stack<WifiDevice<'static>>) {
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    println!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            println!("Got IP: {}", config.address);
            println!("DNS servers:");
            for server in config.dns_servers {
                println!("Dns IP: {}", server);
            }
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
    info!("Ended")
}
