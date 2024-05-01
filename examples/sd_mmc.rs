#[cfg(all(esp32, sdmmc_host_enabled))]
use esp_idf_svc::{
    fs::Fat,
    log::EspLogger,
    sd::{host::SdHost, mmc::SlotConfiguration, spi::SpiDevice},
};
#[cfg(all(esp32, sdmmc_host_enabled))]
use std::{fs::File, io::Write};

#[cfg(all(esp32, sdmmc_host_enabled))]
fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    SpiDevice::initialize_host().expect("Failed to initialize SPI host");

    let slot_configuration = SlotConfiguration::default();

    let host = SdHost::new_with_mmc();

    let _partition = Fat::builder()
        .set_host(host)
        .set_slot_configuration(slot_configuration)
        .build()
        .expect("Failed to build FAT");

    let mut file = File::create("/test.txt")?;

    file.write_all(b"Hello, world!")?;

    Ok(())
}

#[cfg(any(not(esp32), not(esp_idf_soc_sdmmc_host_supported)))]
fn main() {
    use esp_idf_svc::{self as _};

    panic!("This example is configured for esp32, please adjust pins to your module");
}
