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
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::modem::sim::sim7600::SIM7600;
use esp_idf_svc::modem::sim::SimModem;
use esp_idf_svc::modem::EspModem;
use esp_idf_svc::{hal::prelude::Peripherals, http::client::EspHttpConnection};

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

    let mut sim_device = SIM7600::new();
    let mut buff = [0u8; 64];
    match sim_device.negotiate(&mut serial, buff) {
        Err(x) => log::error!("Error = {}", x),
        Ok(()) => log::info!("Device in PPP mode"),
    }

    let mut modem = EspModem::new(&mut serial, sys_loop)?;

    let _scope = std::thread::scope::<_, anyhow::Result<()>>(|s| {
        let my_thread: ScopedJoinHandle<anyhow::Result<()>> = s.spawn(|| {
            match modem.run(&mut buff) {
                Err(x) => log::error!("Error: {:?}", x),
                Ok(_x) => (),
            };
            Ok(())
        });
        std::thread::sleep(Duration::from_secs(10));

        // while !modem.netif().is_up()? {}
        // while !(modem.is_connected()?) {}

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
