#![allow(unexpected_cfgs)]
//! Example using the embassy trouble BLE Host stack over VHCI
//! on top of the esp ble controller.
//!
//! Build with `--features trouble,critical-section` (for now).
//!
//! This examples aims to show how in general the trouble BLE stack
//! can run ontop of esp-idf-svc. For more examples visit the
//! trouble git repository https://github.com/embassy-rs/trouble/tree/main/examples
//!
//! Note that initializing the stack consumes a lot of memory, so `sdkconfig.defaults` should be carefully configured
//! to avoid running out of memory.
//!
//! Here's a working configuration, but you might need to adjust further to your concrete use-case:
//!
//! CONFIG_ESP_MAIN_TASK_STACK_SIZE=50000
//! CONFIG_BT_ENABLED=y
//! CONFIG_BT_BLUEDROID_ENABLED=n
//! CONFIG_BT_CONTROLLER_ONLY=y
//! CONFIG_BT_CTRL_HCI_MODE_VHCI=y
//! CONFIG_BT_CONTROLLER_ENABLED=y
//! CONFIG_BT_CTRL_BLE_SCAN=y
//! 
//! 
#[cfg(all(not(esp32s2), feature = "critical-section", feature = "trouble"))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(any(esp32s2, not(feature = "critical-section", feature = "trouble")))]
fn main() -> anyhow::Result<()> {
    #[cfg(esp32s2)]
    panic!("ESP32-S2 does not have a BLE radio");

    #[cfg(not(feature = "critical-section", feature = "trouble"))]
    panic!("Use `--features trouble,critical-section` when building this example");
}

#[cfg(all(not(esp32s2), feature = "experimental", feature = "trouble"))]
mod example {
    use bt_hci::controller::ExternalController;
    use esp_idf_hal::peripherals::Peripherals;
    use esp_idf_hal::task::block_on;
    use esp_idf_svc::bt::{Ble, BtDriver};
    use esp_idf_svc::nvs::EspDefaultNvsPartition;

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let per = Peripherals::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let bt: BtDriver<'_, Ble> = BtDriver::new(per.modem, Some(nvs))?;

        let controller: ExternalController<BtDriver<'_, Ble>, 20> = ExternalController::new(bt);

        block_on(async { run::<_, 255>(controller).await });

        Ok(())
    }

    use embassy_futures::join::join;
    use embassy_time::{Duration, Timer};
    use esp_idf_svc::log::EspLogger;
    use log::info;
    use trouble_host::prelude::*;

    /// Max number of connections
    const CONNECTIONS_MAX: usize = 2;

    /// Max number of L2CAP channels.
    const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att

    pub async fn run<C, const L2CAP_MTU: usize>(controller: C)
    where
        C: Controller,
    {
        // Using a fixed "random" address can be useful for testing. In real scenarios, one would
        // use e.g. the MAC 6 byte array as the address (how to get that varies by the platform).
        let address: Address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
        info!("Our address = {:?}", address);

        // On esp-idf-svc the device unique BL radio MAC can be retrieved via the following code

        // use esp_idf_sys::{esp, esp_mac_type_t_ESP_MAC_BT};
        // let mut raw_mac: [u8; 6] = [0; 6];
        // let _ = esp!(unsafe {
        //     esp_idf_svc::sys::esp_read_mac(raw_mac.as_mut_ptr(), esp_mac_type_t_ESP_MAC_BT)
        // });
        // let address: Address = Address {
        //     kind: AddrKind::PUBLIC,
        //     addr: BdAddr::new(raw_mac),
        // };

        let mut resources: HostResources<CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, L2CAP_MTU> =
            HostResources::new();
        let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
        let Host {
            mut central,
            mut runner,
            ..
        } = stack.build();

        // NOTE: Modify this to match the address of the peripheral you want to connect to.
        // Currently it matches the address used by the peripheral examples
        let target: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);

        let config = ConnectConfig {
            connect_params: Default::default(),
            scan_config: ScanConfig {
                filter_accept_list: &[(target.kind, &target.addr)],
                ..Default::default()
            },
        };

        info!("Scanning for peripheral...");
        let _ = join(runner.run(), async {
            info!("Connecting");

            let conn = central.connect(&config).await.unwrap();
            info!("Connected, creating gatt client");

            let client = GattClient::<C, 10, 24>::new(&stack, &conn).await.unwrap();

            let _ = join(client.task(), async {
                info!("Looking for battery service");
                let services = client
                    .services_by_uuid(&Uuid::new_short(0x180f))
                    .await
                    .unwrap();

                let service = services.first().unwrap().clone();

                info!("Looking for value handle");
                let c: Characteristic<u8> = client
                    .characteristic_by_uuid(&service, &Uuid::new_short(0x2a19))
                    .await
                    .unwrap();

                info!("Subscribing notifications");
                let mut listener = client.subscribe(&c, false).await.unwrap();

                let _ = join(
                    async {
                        loop {
                            let mut data = [0; 1];
                            client.read_characteristic(&c, &mut data[..]).await.unwrap();
                            info!("Read value: {}", data[0]);
                            Timer::after(Duration::from_secs(10)).await;
                        }
                    },
                    async {
                        loop {
                            let data = listener.next().await;
                            info!(
                                "Got notification: {:?} (val: {})",
                                data.as_ref(),
                                data.as_ref()[0]
                            );
                        }
                    },
                )
                .await;
            })
            .await;
        })
        .await;
    }
}
