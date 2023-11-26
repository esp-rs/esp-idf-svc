//! Example of using blocking wifi with a static IP configuration
//!
//! Add your own ssid and password for the access point
//! Add your own gateway IP, netmask, and local device IP for interface configuration

use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration as WifiConfiguration};
use std::net::Ipv4Addr;

use esp_idf_svc::hal::prelude::Peripherals;
use esp_idf_svc::ipv4::{
    ClientConfiguration as IpClientConfiguration, ClientSettings as IpClientSettings,
    Configuration as IpConfiguration, Mask, Subnet,
};
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::netif::{EspNetif, NetifConfiguration, NetifStack};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};

use log::info;

const SSID: &str = env!("WIFI_SSID");
const PASSWORD: &str = env!("WIFI_PASS");

// Expects IPv4 address
const DEVICE_IP: &str = env!("ESP_DEVICE_IP");
// Expects IPv4 address
const GATEWAY_IP: &str = env!("GATEWAY_IP");
// Expects a number between 0 and 32, defaults to 24
const GATEWAY_NETMASK: Option<&str> = option_env!("GATEWAY_NETMASK");

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let mut wifi = EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?;
    configure_wifi(&mut wifi);

    let mut wifi = BlockingWifi::wrap(wifi, sys_loop)?;
    connect_wifi(&mut wifi)?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    info!("Wifi Interface info: {:?}", ip_info);

    info!("Shutting down in 5s...");

    std::thread::sleep(core::time::Duration::from_secs(5));

    Ok(())
}

fn configure_wifi(wifi: &mut EspWifi<'_>) -> anyhow::Result<()> {
    let wifi_configuration = WifiConfiguration::Client(ClientConfiguration {
        ssid: SSID.into(),
        bssid: None,
        auth_method: AuthMethod::WPA2Personal,
        password: PASSWORD.into(),
        channel: None,
    });
    wifi.set_configuration(&wifi_configuration)?;

    let netmask = GATEWAY_NETMASK.unwrap_or("24");
    let netmask = u8::from_str(netmask)?;
    let gateway_addr = Ipv4Addr::from_str(GATEWAY_IP)?;
    let static_ip = Ipv4Addr::from_str(DEVICE_IP)?;

    let sta_netif_cfg = EspNetif::new_with_conf(&NetifConfiguration {
        key: "sta0".into(),
        description: "sta0_cfg".into(),
        ip_configuration: IpConfiguration::Client(IpClientConfiguration::Fixed(IpClientSettings {
            ip: static_ip,
            subnet: Subnet {
                gateway: gateway_ip,
                mask: Mask(netmask),
            },
            // Can also be set to Ipv4Addrs if you need DNS
            dns: None,
            secondary_dns: None,
        })),
        stack: NetifStack::Sta,
        custom_mac: None,
    })?;
    // This is not actually used in this example, but it appears to be important that the
    // `key` value is not the same.
    let mut ap_netif_cfg = NetifConfiguration::wifi_default_router();
    ap_netif_cfg.key = "ap0".into();
    let ap_netif_cfg = EspNetif::new_with_conf(&ap_netif_cfg)?;

    let (_old_sta_cfg, _old_ap_cfg) = wifi.swap_netif(sta_netif_cfg, ap_netif_cfg)?;

    Ok(())
}

fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
    wifi.start()?;
    info!("Wifi started");

    wifi.connect()?;
    info!("Wifi connected");

    wifi.wait_netif_up()?;
    info!("Wifi netif up");

    Ok(())
}
