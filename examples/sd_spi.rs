#[cfg(esp32)]
fn main() -> anyhow::Result<()> {
    use esp_idf_hal::{gpio, prelude::*};
    use esp_idf_svc::{
        fs::Fat,
        log::EspLogger,
        sd::{host::SdHost, spi::SpiDevice},
    };
    use std::{fs::File, io::Write};

    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;

    SpiDevice::initialize_host().expect("Failed to initialize SPI host");

    let spi_device = SpiDevice::new(
        peripherals.spi2,
        pins.gpio13,
        Option::<gpio::AnyInputPin>::None,
        Option::<gpio::AnyInputPin>::None,
        Option::<gpio::AnyInputPin>::None,
        #[cfg(not(any(
            esp_idf_version_major = "4",
            all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
            all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
        )))] // For ESP-IDF v5.2 and later
        Option::<bool>::None,
    )?;

    let host = SdHost::new_with_spi(spi_device);

    let _partition = Fat::builder()
        .build(host)
        .expect("Failed to mount fat partition");

    let mut file = File::create("/test.txt")?;

    file.write_all(b"Hello, world!")?;

    Ok(())
}

#[cfg(not(esp32))]
fn main() {
    use esp_idf_svc::{self as _};

    panic!("This example is configured for esp32, please adjust pins to your module");
}
