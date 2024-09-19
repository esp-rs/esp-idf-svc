//! Example of a Thread Border Router.
//!
//! This example only works on MCUs that have BOTH Thread and Wifi capabilities, like the ESP32-C6.
//!
//! For other MCUs, you need to use at least one Thread-capable MCU like the ESP32-H2 (which only supports Thread),
//! and then instead of Wifi, you need to use Ethernet via SPI.
//! ... or use a pair of Thread-capable MCU (as the Thread RCP) and Wifi-capable MCU (as the Thread Host) and connect
//! them over UART or SPI.
//!
//! NOTE NOTE NOTE:
//! To build, you need to put the following in your `sdkconfig.defaults`:
//! ```text
//! # Generic Thread functionality
//! CONFIG_OPENTHREAD_ENABLED=y
//!
//! # Thread Border Router
//! CONFIG_OPENTHREAD_BORDER_ROUTER=y
//! CONFIG_LWIP_IPV6_NUM_ADDRESSES=12
//! CONFIG_LWIP_NETIF_STATUS_CALLBACK=y
//!
//! # (These are also necessary for the Joiner feature)
//! CONFIG_MBEDTLS_CMAC_C=y
//! CONFIG_MBEDTLS_SSL_PROTO_DTLS=y
//! CONFIG_MBEDTLS_KEY_EXCHANGE_ECJPAKE=y
//! CONFIG_MBEDTLS_ECJPAKE_C=y
//! ```
//!
//! And also the following in your `Cargo.toml`:
//! ```toml
//! [[package.metadata.esp-idf-sys.extra_components]]
//! remote_component = { name = "espressif/mdns", version = "1.2" }
//! ```

#![allow(unexpected_cfgs)]

fn main() -> anyhow::Result<()> {
    #[cfg(i_have_done_all_of_the_above)] // Remove this `cfg` when you have done all of the above for the example to compile
    #[cfg(esp32c6)]
    router::main()?;

    // Remove this whole code block when you have done all of the above for the example to compile
    #[cfg(not(i_have_done_all_of_the_above))]
    {
        println!("Please follow the instructions in the source code.");
    }

    Ok(())
}

#[cfg(i_have_done_all_of_the_above)] // Remove this `cfg` when you have done all of the above for the example to compile
#[cfg(esp32c6)]
mod router {
    use core::convert::TryInto;

    use std::sync::Arc;

    use esp_idf_svc::eventloop::EspSystemSubscription;
    use log::info;

    use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};

    use esp_idf_svc::hal::prelude::Peripherals;
    use esp_idf_svc::io::vfs::MountedEventfs;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::thread::{EspThread, ThreadEvent};
    use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
    use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let (wifi_modem, thread_modem, _) = peripherals.modem.split();

        let mounted_event_fs = Arc::new(MountedEventfs::mount(4)?);

        let mut wifi = BlockingWifi::wrap(
            EspWifi::new(wifi_modem, sys_loop.clone(), Some(nvs.clone()))?,
            sys_loop.clone(),
        )?;

        connect_wifi(&mut wifi)?;

        let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

        info!("Wifi DHCP info: {:?}", ip_info);

        info!("Running Thread...");

        let _subscription = log_thread_sysloop(sys_loop.clone())?;

        let thread = EspThread::new_br(
            thread_modem,
            sys_loop,
            nvs,
            mounted_event_fs,
            wifi.wifi().sta_netif(),
        )?;

        thread.run()?;

        Ok(())
    }

    fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
        let wifi_configuration: Configuration = Configuration::Client(ClientConfiguration {
            ssid: SSID.try_into().unwrap(),
            bssid: None,
            auth_method: AuthMethod::WPA2Personal,
            password: PASSWORD.try_into().unwrap(),
            channel: None,
            ..Default::default()
        });

        wifi.set_configuration(&wifi_configuration)?;

        wifi.start()?;
        info!("Wifi started");

        wifi.connect()?;
        info!("Wifi connected");

        wifi.wait_netif_up()?;
        info!("Wifi netif up");

        Ok(())
    }

    fn log_thread_sysloop(
        sys_loop: EspSystemEventLoop,
    ) -> Result<EspSystemSubscription<'static>, anyhow::Error> {
        let subscription = sys_loop.subscribe::<ThreadEvent, _>(|event| {
            info!("Got: {:?}", event);
        })?;

        Ok(subscription)
    }
}
