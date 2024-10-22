//! Example of using blocking wifi.
//!
//! Add your own ssid and password

use std::{thread::ScopedJoinHandle, time::Duration};

use embedded_svc::{
    http::{client::Client as HttpClient, Method},
    utils::io,
};

use esp_idf_hal::gpio;
use esp_idf_hal::uart::UartDriver;
use esp_idf_hal::units::Hertz;
use esp_idf_svc::modem::sim::sim7600::SIM7600;
use esp_idf_svc::modem::sim::SimModem;
use esp_idf_svc::modem::EspModem;
use esp_idf_svc::{eventloop::EspSystemEventLoop, modem::BufferedRead};
use esp_idf_svc::{hal::prelude::Peripherals, http::client::EspHttpConnection};
use esp_idf_svc::{log::EspLogger, modem::ModemPhaseStatus};

use log::{error, info};

// const SSID: &str = env!("WIFI_SSID");
// const PASSWORD: &str = env!("WIFI_PASS");

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;

    let serial = peripherals.uart2;
    let tx = peripherals.pins.gpio17;
    let rx = peripherals.pins.gpio18;

    let mut serial = UartDriver::new(
        serial,
        tx,
        rx,
        Option::<gpio::Gpio0>::None,
        Option::<gpio::Gpio0>::None,
        &esp_idf_hal::uart::UartConfig {
            baudrate: Hertz(115200),
            ..Default::default()
        },
    )?;

    let mut buff = [0u8; 64];

    let (mut tx, rx) = serial.split();

    let buf_reader = BufferedRead::new(rx, &mut buff);

    let mut sim_device = SIM7600::new();

    match sim_device.negotiate(&mut tx, &mut buf_reader) {
        Err(x) => log::error!("Error = {}", x),
        Ok(()) => log::info!("Device in PPP mode"),
    }

    let mut modem = EspModem::new(&mut tx, &mut buf_reader, sys_loop)?;

    let _scope = std::thread::scope::<_, anyhow::Result<()>>(|s| {
        let my_thread: ScopedJoinHandle<anyhow::Result<()>> = s.spawn(|| {
            match modem.run(&mut buff) {
                Err(x) => log::error!("Error: {:?}", x),
                Ok(_x) => (),
            };
            Ok(())
        });
        std::thread::sleep(Duration::from_secs(10));

       

        let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);

        // GET
        loop {
            std::thread::sleep(Duration::from_secs(10));
            match get_request(&mut client) {
                Err(x) => log::error!("Failed, reason = {}", x),
                Ok(_) => break,
            }
        }
        my_thread.join().unwrap()?;
        Ok(())
    });

    std::thread::sleep(core::time::Duration::from_secs(5));

    Ok(())
}

/// Send an HTTP GET request.
fn get_request(client: &mut HttpClient<EspHttpConnection>) -> anyhow::Result<()> {
    // Prepare headers and URL
    let headers = [("accept", "text/plain")];
    let url = "http://ifconfig.net/";

    // Send request
    //
    // Note: If you don't want to pass in any headers, you can also use `client.get(url, headers)`.
    let request = client.request(Method::Get, url, &headers)?;
    info!("-> GET {}", url);
    let mut response = request.submit()?;

    // Process response
    let status = response.status();
    info!("<- {}", status);
    let mut buf = [0u8; 1024];
    let bytes_read = io::try_read_full(&mut response, &mut buf).map_err(|e| e.0)?;
    info!("Read {} bytes", bytes_read);
    match std::str::from_utf8(&buf[0..bytes_read]) {
        Ok(body_string) => info!(
            "Response body (truncated to {} bytes): {:?}",
            buf.len(),
            body_string
        ),
        Err(e) => error!("Error decoding response body: {}", e),
    };

    // Drain the remaining response bytes
    while response.read(&mut buf)? > 0 {}

    Ok(())
}
