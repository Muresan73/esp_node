#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use hal::{
    clock::ClockControl,
    embassy::{self, executor::Executor},
    peripherals::Peripherals,
    prelude::*,
    timer::TimerGroup,
};
use static_cell::make_static;

#[embassy_executor::task]
async fn run1() {
    loop {
        esp_println::println!("Hello world from embassy using esp-hal-async!");
        Timer::after(Duration::from_millis(1_000)).await;
    }
}

#[embassy_executor::task]
async fn run2() {
    loop {
        esp_println::println!("Bing!");
        Timer::after(Duration::from_millis(5_000)).await;
    }
}

#[entry]
fn main() -> ! {
    esp_println::println!("Init!");
    let peripherals = Peripherals::take();
    let mut system = peripherals.DPORT.split();
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();

    let timer_group0 = TimerGroup::new(
        peripherals.TIMG0,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    embassy::init(&clocks, timer_group0.timer0);

    let executor = make_static!(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(run1()).ok();
        spawner.spawn(run2()).ok();
    });
    // not needed just random compiler error :/
    loop {}
}