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

#[cfg(all(not(esp32s2), feature = "critical-section", feature = "trouble"))]
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
    use embassy_futures::select::select;
    use embassy_time::Timer;

    use esp_idf_svc::log::EspLogger;
    use log::{info, warn};
    use trouble_host::prelude::*;

    /// Max number of connections
    const CONNECTIONS_MAX: usize = 2;

    /// Max number of L2CAP channels.
    const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att

    // GATT Server definition
    #[gatt_server]
    struct Server {
        battery_service: BatteryService,
    }

    /// Battery service
    #[gatt_service(uuid = service::BATTERY)]
    struct BatteryService {
        /// Battery Level
        #[descriptor(uuid = descriptors::VALID_RANGE, read, value = [0, 100])]
        #[descriptor(uuid = descriptors::MEASUREMENT_DESCRIPTION, name = "hello", read, value = "Battery Level")]
        #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 10)]
        level: u8,
        #[characteristic(uuid = "408813df-5dd4-1f87-ec11-cdb001100000", write, read, notify)]
        status: bool,
    }

    /// Run the BLE stack.
    pub async fn run<C, const L2CAP_MTU: usize>(controller: C)
    where
        C: Controller,
    {
        // Using a fixed "random" address can be useful for testing. In real scenarios, one would
        // use e.g. the MAC 6 byte array as the address (how to get that varies by the platform).
        let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);

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

        info!("Our address = {:?}", address);

        let mut resources: HostResources<CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, L2CAP_MTU> =
            HostResources::new();
        let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
        let Host {
            mut peripheral,
            runner,
            ..
        } = stack.build();

        info!("Starting advertising and GATT service");
        let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
            name: "TrouBLE",
            appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
        }))
        .unwrap();

        let _ = join(ble_task(runner), async {
            loop {
                match advertise("Trouble Example", &mut peripheral, &server).await {
                    Ok(conn) => {
                        // set up tasks when the connection is established to a central, so they don't run when no one is connected.
                        let a = gatt_events_task(&server, &conn);
                        let b = custom_task(&server, &conn, &stack);
                        // run until any task ends (usually because the connection has been closed),
                        // then return to advertising state.
                        select(a, b).await;
                    }
                    Err(e) => {
                        #[cfg(feature = "defmt")]
                        let e = defmt::Debug2Format(&e);
                        panic!("[adv] error: {:?}", e);
                    }
                }
            }
        })
        .await;
    }

    /// This is a background task that is required to run forever alongside any other BLE tasks.
    ///
    /// ## Alternative
    ///
    /// If you didn't require this to be generic for your application, you could statically spawn this with i.e.
    ///
    /// ```rust,ignore
    ///
    /// #[embassy_executor::task]
    /// async fn ble_task(mut runner: Runner<'static, SoftdeviceController<'static>>) {
    ///     runner.run().await;
    /// }
    ///
    /// spawner.must_spawn(ble_task(runner));
    /// ```
    async fn ble_task<C: Controller>(mut runner: Runner<'_, C>) {
        loop {
            if let Err(e) = runner.run().await {
                #[cfg(feature = "defmt")]
                let e = defmt::Debug2Format(&e);
                panic!("[ble_task] error: {:?}", e);
            }
        }
    }

    /// Stream Events until the connection closes.
    ///
    /// This function will handle the GATT events and process them.
    /// This is how we interact with read and write requests.
    async fn gatt_events_task(
        server: &Server<'_>,
        conn: &GattConnection<'_, '_>,
    ) -> Result<(), Error> {
        let level = server.battery_service.level;
        loop {
            match conn.next().await {
                GattConnectionEvent::Disconnected { reason } => {
                    info!("[gatt] disconnected: {:?}", reason);
                    break;
                }
                GattConnectionEvent::Gatt { event } => match event {
                    Ok(event) => {
                        match &event {
                            GattEvent::Read(event) => {
                                if event.handle() == level.handle {
                                    let value = server.get(&level);
                                    info!("[gatt] Read Event to Level Characteristic: {:?}", value);
                                }
                            }
                            GattEvent::Write(event) => {
                                if event.handle() == level.handle {
                                    info!(
                                        "[gatt] Write Event to Level Characteristic: {:?}",
                                        event.data()
                                    );
                                }
                            }
                        }

                        // This step is also performed at drop(), but writing it explicitly is necessary
                        // in order to ensure reply is sent.
                        match event.accept() {
                            Ok(reply) => {
                                reply.send().await;
                            }
                            Err(e) => warn!("[gatt] error sending response: {:?}", e),
                        }
                    }
                    Err(e) => warn!("[gatt] error processing event: {:?}", e),
                },
                _ => {}
            }
        }
        info!("[gatt] task finished");
        Ok(())
    }

    /// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
    async fn advertise<'a, 'b, C: Controller>(
        name: &'a str,
        peripheral: &mut Peripheral<'a, C>,
        server: &'b Server<'_>,
    ) -> Result<GattConnection<'a, 'b>, BleHostError<C::Error>> {
        let mut advertiser_data = [0; 31];
        let batt_level = server.battery_service.level;
        let val = batt_level.get(&server)?;
        let blub = [val];
        AdStructure::encode_slice(
            &[
                AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                //AdStructure::ServiceUuids16(&[[0x0f, 0x18]]),
                AdStructure::CompleteLocalName(name.as_bytes()),
                AdStructure::ServiceData16 {
                    uuid: [0x0f, 0x18],
                    data: &blub,
                },
            ],
            &mut advertiser_data[..],
        )?;
        let advertiser = peripheral
            .advertise(
                &Default::default(),
                Advertisement::ConnectableScannableUndirected {
                    adv_data: &advertiser_data[..],
                    scan_data: &[],
                },
            )
            .await?;
        info!("[adv] advertising");
        let conn = advertiser.accept().await?.with_attribute_server(server)?;
        info!("[adv] connection established");
        Ok(conn)
    }

    /// Example task to use the BLE notifier interface.
    /// This task will notify the connected central of a counter value every 2 seconds.
    /// It will also read the RSSI value every 2 seconds.
    /// and will stop when the connection is closed by the central or an error occurs.
    async fn custom_task<C: Controller>(
        server: &Server<'_>,
        conn: &GattConnection<'_, '_>,
        stack: &Stack<'_, C>,
    ) {
        let mut tick: u8 = 0;
        let level = server.battery_service.level;

        loop {
            tick = tick.wrapping_add(1);
            info!("[custom_task] notifying connection of tick {}", tick);

            if level.notify(conn, &tick).await.is_err() {
                info!("[custom_task] error notifying connection");
                break;
            };
            // read RSSI (Received Signal Strength Indicator) of the connection.
            if let Ok(rssi) = conn.raw().rssi(stack).await {
                info!("[custom_task] RSSI: {:?}", rssi);
            } else {
                info!("[custom_task] error getting RSSI");
                break;
            };
            Timer::after_secs(2).await;
        }
    }
}
