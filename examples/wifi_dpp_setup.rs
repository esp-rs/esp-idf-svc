//! Example using Wi-Fi Easy Connect (DPP) to get a device onto a Wi-Fi network
//! without hardcoding credentials.

extern crate core;

use esp_idf_hal as _;

use std::time::Duration;
use embedded_svc::wifi::{ClientConfiguration, Configuration, Wifi};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{EspWifi, WifiWait};
use esp_idf_sys::EspError;
use log::{error, info, LevelFilter, warn};
use esp_idf_svc::wifi_dpp::EspDppBootstrapper;

fn main() {
    esp_idf_sys::link_patches();

    EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();

    let mut wifi = EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs)).unwrap();

    if !wifi_has_sta_config(&wifi).unwrap() {
        info!("No existing STA config, let's try DPP...");
        let config = dpp_listen_forever(&mut wifi).unwrap();
        info!("Got config: {config:?}");
        wifi.set_configuration(&Configuration::Client(config)).unwrap();
    }

    wifi.start().unwrap();

    let timeout = Duration::from_secs(60);
    loop {
        let ssid = match wifi.get_configuration().unwrap() {
            Configuration::None => None,
            Configuration::Client(ap) => Some(ap.ssid),
            Configuration::AccessPoint(_) => None,
            Configuration::Mixed(_, _) => None,
        }.unwrap();
        info!("Connecting to {ssid}...");
        wifi.connect().unwrap();
        let waiter = WifiWait::new(&sysloop).unwrap();
        let is_connected = waiter.wait_with_timeout(timeout, || wifi.is_connected().unwrap());
        if is_connected {
            info!("Connected!");
            waiter.wait(|| !wifi.is_connected().unwrap());
            warn!("Got disconnected, connecting again...");
        } else {
            error!("Failed to connect after {}s, trying again...", timeout.as_secs());
        }
    }
}

fn wifi_has_sta_config(wifi: &EspWifi) -> Result<bool, EspError> {
    match wifi.get_configuration()? {
        Configuration::Client(c) => Ok(!c.ssid.is_empty()),
        _ => Ok(false),
    }
}

fn dpp_listen_forever(wifi: &mut EspWifi) -> Result<ClientConfiguration, EspError> {
    let mut dpp = EspDppBootstrapper::new(wifi)?;
    let channels: Vec<_> = (1..=11).collect();
    let bootstrapped = dpp.gen_qrcode(&channels, None, None)?;
    println!("Got: {}", bootstrapped.data.0);
    println!("(use a QR code generator and scan the code in the Wi-Fi setup flow on your phone)");

    bootstrapped.listen_forever()
}
