//! Example of using async wifi.
//!
//! Add your own ssid and password
//!
//! Note: Requires `nightly` and `experimental` cargo feature to be enabled

extern crate esp_idf_svc;
use embedded_svc::wifi::asynch::Wifi;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_hal::prelude::Peripherals;
use esp_idf_svc::wifi::AsyncWifiDriver;
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};
use futures::executor::block_on;

const SSID: &'static str = "<Your SSID here>";
const PASSWORD: &'static str = "<Your password here>";

fn main() -> anyhow::Result<()> {
    let peripherals = Peripherals::take().unwrap();
    let sys_loop = EspSystemEventLoop::take().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();
    let mut base_driver = WifiDriver::new(peripherals.modem, sys_loop, Some(nvs))?;
    let mut wifi = AsyncWifiDriver::new(base_driver);

    block_on(do_wifi(&mut wifi))?;

    Ok(())
}

async fn do_wifi<'a>(wifi: &mut AsyncWifiDriver<'a>) -> anyhow::Result<()> {
    let wifi_configuration: Configuration = Configuration::Client(ClientConfiguration {
        ssid: SSID.into(),
        bssid: None,
        auth_method: AuthMethod::WPA2Personal,
        password: PASSWORD.into(),
        channel: None,
    });

    wifi.set_configuration(&wifi_configuration).await?;
    wifi.connect().await.map_err(|e| e.into())
}
