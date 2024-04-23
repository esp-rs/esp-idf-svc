#[cfg(esp32)]
use esp_idf_svc::{
    fs::Fat,
    log::EspLogger,
    sd::{host::SdHost, spi::SpiDevice},
};

use std::{fs::File, io::Write};

#[cfg(esp32)]
fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    SpiDevice::initialize_host().expect("Failed to initialize SPI host");

    let device_builder = SpiDevice::builder();

    let spi_device = device_builder.build().expect("Failed to build SPI device");

    let host = SdHost::new_with_spi(&spi_device);

    let _partition = Fat::builder()
        .set_host(host)
        .set_spi_device(spi_device)
        .build()
        .expect("Failed to build FAT");

    let mut file = File::create("/test.txt")?;

    file.write_all(b"Hello, world!")?;

    Ok(())
}

#[cfg(not(esp32))]
fn main() {
    use esp_idf_svc::{self as _};

    panic!("This example is configured for esp32, please adjust pins to your module");
}
