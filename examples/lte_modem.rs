//! Connect a SIM7600 to lwIP using PPP over UART.
//!
//! Set `CELLULAR_APN` at compile time. If it is absent, `internet` is used.

use std::time::Duration;

use embedded_svc::{
    http::{client::Client as HttpClient, Method},
    utils::io,
};

use esp_idf_hal::uart::UartDriver;
use esp_idf_hal::units::Hertz;
use esp_idf_hal::{
    delay,
    gpio::{self, PinDriver},
};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::modem::{EspModem, ModemConfig};
use esp_idf_svc::{hal::prelude::Peripherals, http::client::EspHttpConnection};

use log::{error, info};

const APN: &str = match option_env!("CELLULAR_APN") {
    Some(apn) => apn,
    None => "internet",
};

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;

    let serial = peripherals.uart2;
    let tx = peripherals.pins.gpio17;
    let rx = peripherals.pins.gpio18;

    let mut lte_reset = PinDriver::output(peripherals.pins.gpio42).unwrap();
    lte_reset.set_low().unwrap();

    let mut lte_power = PinDriver::output(peripherals.pins.gpio41).unwrap();
    let mut lte_on = PinDriver::output(peripherals.pins.gpio40).unwrap();
    // turn lte device on
    log::info!("Reset GSM Device");
    lte_power.set_high().unwrap();
    let delay = delay::Delay::new_default();
    delay.delay_ms(100);
    lte_on.set_high().unwrap();
    delay.delay_ms(100);
    lte_on.set_low().unwrap();
    delay.delay_ms(10000);
    log::info!("Reset Complete");

    let serial = UartDriver::new(
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

    let (tx, rx) = serial.into_split();
    let (runner, handle) = EspModem::new(ModemConfig::new(APN), tx, rx, sys_loop)?;
    let worker = std::thread::spawn(move || runner.run());

    let application_result = (|| -> anyhow::Result<()> {
        let status = handle.wait_connected(Duration::from_secs(240))?;
        info!("PPP connected: {:?}", status.ip_info);

        let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);
        get_request(&mut client)
    })();

    handle.request_stop();
    let runner_result = worker
        .join()
        .map_err(|_| anyhow::anyhow!("modem task panicked"))?;
    runner_result?;
    application_result
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
