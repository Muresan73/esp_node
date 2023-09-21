#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

extern crate alloc;
use dotenvy_macro::dotenv;
use embassy_executor::_export::StaticCell;
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Config, Ipv4Address, Stack, StackResources};
use embassy_time::{Duration, Timer};
use embedded_svc::wifi::{ClientConfiguration, Configuration, Wifi};
use esp_backtrace as _;
use esp_println::println;
use esp_wifi::wifi::{WifiController, WifiDevice, WifiEvent, WifiMode, WifiState};
use esp_wifi::{initialize, EspWifiInitFor};
use hal::clock::ClockControl;
use hal::embassy::executor::Executor;
use hal::rng::Rng;
use hal::{embassy, peripherals::Peripherals, prelude::*, timer::TimerGroup};

const SSID: &str = dotenv!("SSID");
const PASSWORD: &str = dotenv!("PASSWORD");
const DISCORD_HOST: &str = dotenv!("DISCORD_HOST");
const DISCORD_WEBHOOK: &str = dotenv!("DISCORD_WEBHOOK");

macro_rules! singleton {
    ($val:expr) => {{
        type T = impl Sized;
        static STATIC_CELL: StaticCell<T> = StaticCell::new();
        let (x,) = STATIC_CELL.init(($val,));
        x
    }};
}

static EXECUTOR: StaticCell<Executor> = StaticCell::new();

#[global_allocator]
static ALLOCATOR: esp_alloc::EspHeap = esp_alloc::EspHeap::empty();

fn init_heap() {
    const HEAP_SIZE: usize = 1024;
    static mut HEAP: core::mem::MaybeUninit<[u8; HEAP_SIZE]> = core::mem::MaybeUninit::uninit();

    unsafe {
        ALLOCATOR.init(HEAP.as_mut_ptr() as *mut u8, HEAP_SIZE);
    }
}

#[entry]
fn main() -> ! {
    // #[cfg(feature = "log")]
    esp_println::logger::init_logger(log::LevelFilter::Info);

    init_heap();

    let peripherals = Peripherals::take();

    let mut system = peripherals.DPORT.split();
    let clocks = ClockControl::max(system.clock_control).freeze();

    let timer = hal::timer::TimerGroup::new(
        peripherals.TIMG1,
        &clocks,
        &mut system.peripheral_clock_control,
    )
    .timer0;

    let init = initialize(
        EspWifiInitFor::Wifi,
        timer,
        Rng::new(peripherals.RNG),
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

    let seed = 1234; // TODO add time

    // Init network stack
    let stack = &*singleton!(Stack::new(
        wifi_interface,
        config,
        singleton!(StackResources::<3>::new()),
        seed
    ));

    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(connection(controller)).ok();
        spawner.spawn(net_task(stack)).ok();
        spawner.spawn(message_discord(stack)).ok();
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
async fn message_discord(stack: &'static Stack<WifiDevice<'static>>) {
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    // Add await once available from embasst_net crate
    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    println!("Waiting to get IP address...");
    loop {
        if stack.is_config_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    let config = stack.config_v4().unwrap();
    println!("Got IP: {}", config.address);
    println!("DNS servers:");
    for server in config.dns_servers {
        println!("Dns IP: {}", server);
    }

    loop {
        Timer::after(Duration::from_millis(1_000)).await;

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);

        socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

        let address = match stack
            .dns_query(DISCORD_HOST, DnsQueryType::A)
            .await
            .map(|a| a[0])
        {
            Ok(address) => address,
            Err(e) => {
                println!("DNS lookup error: {e:?}");
                continue;
            }
        };

        println!("{}", address);
        let remote_endpoint = (address, 443);
        println!("connecting...");
        let r = socket.connect(remote_endpoint).await;
        if let Err(e) = r {
            println!("connect error: {:?}", e);
            continue;
        }
        println!("connected!");
        let mut buf = [0; 1024];
        let url = alloc::format!(DISCORD_WEBHOOK, "443");
        let mut client = reqwless::client::HttpClient::new_with_tls(
            &socket,
            &DNS,
            TlsConfig::new(seed as u64, &mut tls_rx, &mut tls_tx, TlsVerify::None),
        ); // Types implementing embedded-nal-async
        let mut rx_buf = [0; 4096];
        let response = client
            .request(Method::POST, &url)
            .await
            .unwrap()
            .body(b"PING")
            .content_type(ContentType::TextPlain)
            .send(&mut rx_buf)
            .await
            .unwrap();

        let message = "{\"msg\":\"hello\"}";
        let request = alloc::format!( "POST /api/webhooks/1151636331458461856/joImzPMcPrh6Qj-2yT1Yk3dmZlZguJxpToEisW5yItWD4PcPt7JHnv0T5dM5IFOd_7D1 HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{{\"content\":\"hello\"}}",
            DISCORD_HOST,
            message.len()
        );

        return;
        // let message = "{\"msg\":\"hello\"}";
        // let request = alloc::format!( "POST /api/webhooks/1151636331458461856/joImzPMcPrh6Qj-2yT1Yk3dmZlZguJxpToEisW5yItWD4PcPt7JHnv0T5dM5IFOd_7D1 HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{{\"content\":\"hello\"}}",
        //     DISCORD_HOST,
        //     message.len()
        // );

        // println!("{request}");

        // use embedded_svc::io::asynch::Write;
        // let r = socket.write_all(request.as_bytes()).await;
        // if let Err(e) = r {
        //     println!("write error: {:?}", e);
        //     break;
        // }
        // let n = match socket.read(&mut buf).await {
        //     Ok(0) => {
        //         println!("read EOF");
        //         break;
        //     }
        //     Ok(n) => n,
        //     Err(e) => {
        //         println!("read error: {:?}", e);
        //         break;
        //     }
        // };
        // println!("{}", core::str::from_utf8(&buf[..n]).unwrap());
        // return;
    }
}

//https://github.com/drogue-iot/embedded-tls/blob/main/examples/embassy/src/main.rs
// https://docs.rs/embedded-tls/latest/embedded_tls/
//innen folyt kov
