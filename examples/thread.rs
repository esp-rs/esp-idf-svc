//! Example of using Thread.
//! The example just starts Thread and logs the events, without doing anything else useful.
//!
//! NOTE: This example only works on MCUs that has Thread capabilities, like the ESP32-C6 or ESP32-H2.

fn main() -> anyhow::Result<()> {
    #[cfg(any(esp32h2, esp32c6))]
    router::main()?;

    #[cfg(not(any(esp32h2, esp32c6)))]
    println!("This example only works on MCUs that have Thread capabilities, like the ESP32-C6 or ESP32-H2.");

    Ok(())
}

#[cfg(any(esp32h2, esp32c6))]
mod router {
    use std::sync::Arc;

    use esp_idf_svc::eventloop::EspSystemSubscription;
    use log::info;

    use esp_idf_svc::hal::prelude::Peripherals;
    use esp_idf_svc::io::vfs::MountedEventfs;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::thread::{EspThread, ThreadEvent};
    use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let mounted_event_fs = Arc::new(MountedEventfs::mount(4)?);

        info!("Running Thread...");

        let _subscription = log_thread_sysloop(sys_loop.clone())?;

        let thread = EspThread::new(peripherals.modem, sys_loop.clone(), nvs, mounted_event_fs)?;

        thread.run()?;

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
