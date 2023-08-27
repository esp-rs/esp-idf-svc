use esp_idf_sys::{self as _}; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported

#[cfg(esp32)]
use esp_idf_svc::{
    eth::{BlockingEth, EspEth, EthDriver},
    eventloop::EspSystemEventLoop,
    hal::{gpio, prelude::Peripherals},
    log::EspLogger,
};
#[cfg(esp32)]
use log::info;

#[cfg(esp32)]
fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;
    let sys_loop = EspSystemEventLoop::take()?;

    // Make sure to configure ethernet in sdkconfig and adjust the parameters below for your hardware
    let eth_driver = EthDriver::new(
        peripherals.mac,
        pins.gpio25,
        pins.gpio26,
        pins.gpio27,
        pins.gpio23,
        pins.gpio22,
        pins.gpio21,
        pins.gpio19,
        pins.gpio18,
        esp_idf_svc::eth::RmiiClockConfig::<gpio::Gpio0, gpio::Gpio16, gpio::Gpio17>::OutputInvertedGpio17(
            pins.gpio17,
        ),
        Some(pins.gpio5),
        esp_idf_svc::eth::RmiiEthChipset::LAN87XX,
        Some(0),
        sys_loop.clone(),
    )?;
    let eth = EspEth::wrap(eth_driver)?;

    info!("Eth created");

    let mut eth = BlockingEth::wrap(eth, sys_loop.clone())?;

    info!("Starting eth...");

    eth.start()?;

    info!("Waiting for DHCP lease...");

    eth.wait_netif_up()?;

    let ip_info = eth.eth().netif().get_ip_info()?;

    info!("Eth DHCP info: {:?}", ip_info);

    Ok(())
}

#[cfg(not(esp32))]
fn main() {
    panic!("This example is configured for esp32, please adjust pins to your module");
}
