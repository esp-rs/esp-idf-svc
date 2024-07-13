#[cfg(all(esp32, esp_idf_soc_sdmmc_host_supported))]
fn main() -> anyhow::Result<()> {
    use esp_idf_hal::sd::SdMmcConfiguration;
    use esp_idf_svc::fs::fat::FatFs;
    use esp_idf_svc::hal::gpio;
    use esp_idf_svc::hal::prelude::*;
    use esp_idf_svc::hal::sd::{mmc::SdMmcHostDriver, SdCardDriver};
    use esp_idf_svc::io::vfs::MountedFatFs;
    use esp_idf_svc::log::EspLogger;

    use std::fs::{read_dir, File};
    use std::io::{Read, Seek, Write};

    esp_idf_svc::sys::link_patches();

    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;

    let sd_card_driver = SdCardDriver::new_mmc(
        &SdMmcConfiguration::new(),
        SdMmcHostDriver::new_slot_1(
            peripherals.sdmmc1,
            pins.gpio15,
            pins.gpio14,
            pins.gpio2,
            pins.gpio4,
            pins.gpio12,
            pins.gpio13,
            None::<gpio::Gpio34>,
            None::<gpio::Gpio35>,
        )?,
    )?;

    // Keep it around or else it will be dropped and unmounted
    let _mounted_fat_fs = MountedFatFs::mount(FatFs::new_sdcard(0, sd_card_driver)?, "/sdcard", 4)?;

    let content = b"Hello, world!";

    {
        let mut file = File::create("/sdcard/test.txt")?;

        file.write_all(content).expect("Write failed");

        file.seek(std::io::SeekFrom::Start(0)).expect("Seek failed");
    }

    {
        let mut file = File::open("/sdcard/test.txt")?;

        let mut file_content = String::new();

        file.read_to_string(&mut file_content).expect("Read failed");

        assert_eq!(file_content.as_bytes(), content);
    }

    {
        let directory = read_dir("/sdcard")?;

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
