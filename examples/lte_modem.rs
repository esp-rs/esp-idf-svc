//! Example of using blocking wifi.
//!
//! Add your own ssid and password

use core::convert::TryInto;

use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};

use esp_idf_hal::gpio;
use esp_idf_hal::uart::UartDriver;
use esp_idf_hal::units::Hertz;
use esp_idf_svc::hal::prelude::Peripherals;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::modem::EspModem;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};

use log::info;

// const SSID: &str = env!("WIFI_SSID");
// const PASSWORD: &str = env!("WIFI_PASS");

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
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
    log::error!("Hello");
    let mut modem = EspModem::new(&mut serial);

    match modem.setup_data_mode() {
        Err(x) => log::error!("Error: {:?}", x),
        Ok(x) => (),
    }

    std::thread::sleep(core::time::Duration::from_secs(5));

    Ok(())
}
