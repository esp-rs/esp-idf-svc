#[cfg(all(esp32, esp_idf_soc_sdmmc_host_supported))]
fn main() -> anyhow::Result<()> {
    use esp_idf_svc::{
        fs::{Fat, FatConfiguration},
        hal::{
            gpio::{self, AnyIOPin},
            prelude::*,
        },
        log::EspLogger,
        sd::{host::SdHost, mmc::SlotConfiguration, SdConfiguration},
    };

    use std::{
        fs::{read_dir, File},
        io::{Read, Seek, Write},
    };

    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;

    let slot = SlotConfiguration::new_slot_0(
        pins.gpio11,
        pins.gpio6,
        pins.gpio7,
        Some(pins.gpio8),
        Some(pins.gpio9),
        Some(pins.gpio10),
        Option::<gpio::Gpio16>::None,
        Option::<gpio::Gpio17>::None,
        Option::<gpio::Gpio15>::None,
        Option::<gpio::Gpio18>::None,
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
    );

    let host_config = SdConfiguration::new();

    let host = SdHost::new_with_mmc(&host_config, slot);

    let fat_config = FatConfiguration::new();

    let _fat = Fat::mount(fat_config, host, "/sdmmc");

    let content = b"Hello, world!";

    {
        let mut file = File::create("/sdmmc/test.txt")?;

        file.write_all(content).expect("Write failed");

        file.seek(std::io::SeekFrom::Start(0)).expect("Seek failed");
    }

    {
        let mut file = File::open("/sdmmc/test.txt")?;

        let mut file_content = String::new();

        file.read_to_string(&mut file_content).expect("Read failed");

        assert_eq!(file_content.as_bytes(), content);
    }

    {
        let directory = read_dir("/sdmmc")?;

        for entry in directory {
            log::info!("Entry: {:?}", entry?.file_name());
        }
    }

    Ok(())
}

#[cfg(not(all(esp32, esp_idf_soc_sdmmc_host_supported)))]
fn main() {
    use esp_idf_svc::{self as _};

    panic!("This example is configured for esp32, please adjust pins to your module");
}
