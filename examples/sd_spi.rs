#[cfg(esp32)]
fn main() -> anyhow::Result<()> {
    use esp_idf_svc::{
        fs::{Fat, FatConfiguration},
        hal::{
            gpio,
            prelude::*,
            spi::{config::DriverConfig, Dma, SpiDriver, SPI3},
        },
        log::EspLogger,
        sd::{host::SdHost, spi::SpiDevice, SdConfiguration},
    };
    use std::{
        fs::{read_dir, File},
        io::{Read, Seek, Write},
    };

    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;

    let spi_driver = SpiDriver::new::<SPI3>(
        peripherals.spi3,
        pins.gpio18,
        pins.gpio23,
        Some(pins.gpio19),
        &DriverConfig::default().dma(Dma::Auto(4092)),
    )?;

    let spi_device = SpiDevice::new(
        spi_driver,
        pins.gpio5,
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

    let host_config = SdConfiguration::new();

    let host = SdHost::new_with_spi(&host_config, spi_device);

    let fat_configuration = FatConfiguration::new();

    let _fat = Fat::mount(fat_configuration, &host, "/sdspi")?;

    let content = b"Hello, world!";

    {
        let mut file = File::create("/sdspi/test.txt")?;

        file.write_all(content).expect("Write failed");

        file.seek(std::io::SeekFrom::Start(0)).expect("Seek failed");
    }

    {
        let mut file = File::open("/sdspi/test.txt")?;

        let mut file_content = String::new();

        file.read_to_string(&mut file_content).expect("Read failed");

        assert_eq!(file_content.as_bytes(), content);
    }

    {
        let directory = read_dir("/sdspi")?;

        for entry in directory {
            log::info!("Entry: {:?}", entry?.file_name());
        }
    }

    Ok(())
}

#[cfg(not(esp32))]
fn main() {
    use esp_idf_svc::{self as _};

    panic!("This example is configured for esp32, please adjust pins to your module");
}
