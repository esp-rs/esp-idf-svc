use core::fmt::Debug;
use core::marker::PhantomData;

use esp_idf_hal::task::CriticalSection;
use log::debug;

use crate::eventloop::EspSystemEventLoop;
use crate::hal::gpio::{InputPin, OutputPin};
use crate::hal::peripheral::Peripheral;
use crate::hal::uart::Uart;
#[cfg(esp_idf_comp_esp_netif_enabled)]
use crate::handle::RawHandle;
use crate::io::vfs::MountedEventfs;
#[cfg(esp_idf_comp_esp_netif_enabled)]
use crate::netif::*;
use crate::nvs::EspDefaultNvsPartition;
use crate::sys::*;

extern crate alloc;

use alloc::sync::Arc;

/// The driver will operate in Radio Co-Processor mode
///
/// The chip needs to be connected via UART or SPI to the host
#[cfg(esp_idf_soc_ieee802154_supported)]
#[derive(Debug)]
pub struct RCP;

/// The driver will operate as a host
///
/// This means that - unless the chip has a native Thread suppoort -
/// it needs to be connected via UART or SPI to another chip which does have
/// native Thread support and which is configured to operate in RCP mode
pub struct Host(esp_openthread_platform_config_t);

impl Debug for Host {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Host").finish()
    }
}

#[cfg(all(esp32c2, esp_idf_xtal_freq_26))]
const BAUD_RATE: u32 = 74880;

#[cfg(not(all(esp32c2, esp_idf_xtal_freq_26)))]
const BAUD_RATE: u32 = 115200;

static CS: CriticalSection = CriticalSection::new();

/// This struct provides a safe wrapper over the ESP IDF Thread C driver.
///
/// The driver works on Layer 2 (Data Link) in the OSI model, in that it provides
/// facilities for sending and receiving ethernet packets over the Thread radio.
///
/// For most use cases, utilizing `EspThread` - which provides a networking (IP)
/// layer as well - should be preferred. Using `ThreadDriver` directly is beneficial
/// only when one would like to utilize a custom, non-STD network stack like `smoltcp`.
///
/// The driver can work in two modes:
/// - RCP (Radio Co-Processor) mode: The driver operates as a co-processor to the host,
///   which is expected to be another chip connected to ours via SPI or UART. This is
///   of course only supported with MCUs that do have a Thread radio, like esp32c2 and esp32c6
/// - Host mode: The driver operates as a host, and if the chip does not have a Thread radio
///   it has to be connected via SPI or USB to a chip which runs the Thread stack in RCP mode
pub struct ThreadDriver<'d, T> {
    mode: T,
    _mounted_event_fs: Arc<MountedEventfs>,
    //_subscription: EspSubscription<'static, System>,
    _nvs: EspDefaultNvsPartition,
    _p: PhantomData<&'d mut ()>,
}

impl<'d> ThreadDriver<'d, Host> {
    /// Create a new Thread Host driver instance utilizing the
    /// native Thread radio on the MCU
    #[cfg(esp_idf_soc_ieee802154_supported)]
    pub fn new<M: crate::hal::modem::ThreadModemPeripheral>(
        _modem: impl Peripheral<P = M> + 'd,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        let cfg = esp_openthread_platform_config_t {
            radio_config: esp_openthread_radio_config_t {
                radio_mode: esp_openthread_radio_mode_t_RADIO_MODE_NATIVE,
                ..Default::default()
            },
            host_config: esp_openthread_host_connection_config_t {
                host_connection_mode:
                    esp_openthread_host_connection_mode_t_HOST_CONNECTION_MODE_NONE,
                ..Default::default()
            },
            port_config: esp_openthread_port_config_t {
                storage_partition_name: b"TODO\0" as *const _ as *const _,
                netif_queue_size: 10,
                task_queue_size: 10,
            },
        };
        esp!(unsafe { esp_openthread_init(&cfg) })?;

        debug!("Driver initialized");

        Ok(Self {
            mode: Host(cfg),
            _mounted_event_fs: mounted_event_fs,
            _nvs: nvs,
            _p: PhantomData,
        })
    }

    /// Create a new Thread Host driver instance utilizing an SPI connection
    /// to another MCU running the Thread stack in RCP mode.
    #[cfg(not(esp_idf_version_major = "4"))]
    #[allow(clippy::too_many_arguments)]
    pub fn new_spi<S: crate::hal::spi::Spi>(
        _spi: impl Peripheral<P = S> + 'd,
        mosi: impl Peripheral<P = impl InputPin> + 'd,
        miso: impl Peripheral<P = impl OutputPin> + 'd,
        sclk: impl Peripheral<P = impl InputPin + OutputPin> + 'd,
        cs: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        intr: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        crate::hal::into_ref!(mosi, miso, sclk);

        let cs_pin = if let Some(cs) = cs {
            crate::hal::into_ref!(cs);

            cs.pin() as _
        } else {
            -1
        };

        let intr_pin = if let Some(intr) = intr {
            crate::hal::into_ref!(intr);

            intr.pin() as _
        } else {
            -1
        };

        let cfg = esp_openthread_platform_config_t {
            radio_config: esp_openthread_radio_config_t {
                radio_mode: esp_openthread_radio_mode_t_RADIO_MODE_SPI_RCP,
                __bindgen_anon_1: esp_openthread_radio_config_t__bindgen_ty_1 {
                    radio_spi_config: esp_openthread_spi_host_config_t {
                        host_device: S::device() as _,
                        dma_channel: spi_common_dma_t_SPI_DMA_CH_AUTO,
                        spi_interface: spi_bus_config_t {
                            __bindgen_anon_1: spi_bus_config_t__bindgen_ty_1 {
                                mosi_io_num: mosi.pin() as _,
                            },
                            __bindgen_anon_2: spi_bus_config_t__bindgen_ty_2 {
                                miso_io_num: miso.pin() as _,
                            },
                            sclk_io_num: sclk.pin() as _,
                            ..Default::default()
                        },
                        spi_device: spi_device_interface_config_t {
                            spics_io_num: cs_pin as _,
                            cs_ena_pretrans: 2,
                            input_delay_ns: 100,
                            mode: 0,
                            clock_speed_hz: 2500 * 1000,
                            queue_size: 5,
                            ..Default::default()
                        },
                        intr_pin,
                    },
                },
            },
            host_config: esp_openthread_host_connection_config_t {
                host_connection_mode:
                    esp_openthread_host_connection_mode_t_HOST_CONNECTION_MODE_NONE,
                ..Default::default()
            },
            port_config: esp_openthread_port_config_t {
                storage_partition_name: b"TODO\0" as *const _ as *const _,
                netif_queue_size: 10,
                task_queue_size: 10,
            },
        };
        esp!(unsafe { esp_openthread_init(&cfg) })?;

        debug!("Driver initialized");

        Ok(Self {
            mode: Host(cfg),
            _mounted_event_fs: mounted_event_fs,
            _nvs: nvs,
            _p: PhantomData,
        })
    }

    /// Create a new Thread Host driver instance utilizing a UART connection
    /// to another MCU running the Thread stack in RCP mode.
    pub fn new_uart<U: Uart>(
        _uart: impl Peripheral<P = U> + 'd,
        rx: impl Peripheral<P = impl InputPin> + 'd,
        tx: impl Peripheral<P = impl OutputPin> + 'd,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        crate::hal::into_ref!(rx, tx);

        #[cfg(esp_idf_version_major = "4")]
        let cfg = esp_openthread_platform_config_t {
            radio_config: esp_openthread_radio_config_t {
                radio_mode: esp_openthread_radio_mode_t_RADIO_MODE_UART_RCP,
                radio_uart_config: esp_openthread_uart_config_t {
                    port: U::port() as _,
                    uart_config: uart_config_t {
                        baud_rate: BAUD_RATE as _,
                        data_bits: 8,
                        parity: 0,
                        stop_bits: 1,
                        flow_ctrl: 0,
                        rx_flow_ctrl_thresh: 0,
                        ..Default::default()
                    },
                    rx_pin: rx.pin() as _,
                    tx_pin: tx.pin() as _,
                },
            },
            host_config: esp_openthread_host_connection_config_t {
                host_connection_mode:
                    esp_openthread_host_connection_mode_t_HOST_CONNECTION_MODE_NONE,
                ..Default::default()
            },
            port_config: esp_openthread_port_config_t {
                storage_partition_name: b"TODO\0" as *const _ as *const _,
                netif_queue_size: 10,
                task_queue_size: 10,
            },
        };

        #[cfg(not(esp_idf_version_major = "4"))]
        let cfg = esp_openthread_platform_config_t {
            radio_config: esp_openthread_radio_config_t {
                radio_mode: esp_openthread_radio_mode_t_RADIO_MODE_UART_RCP,
                __bindgen_anon_1: esp_openthread_radio_config_t__bindgen_ty_1 {
                    radio_uart_config: esp_openthread_uart_config_t {
                        port: U::port() as _,
                        uart_config: uart_config_t {
                            baud_rate: BAUD_RATE as _,
                            data_bits: 8,
                            parity: 0,
                            stop_bits: 1,
                            flow_ctrl: 0,
                            rx_flow_ctrl_thresh: 0,
                            ..Default::default()
                        },
                        rx_pin: rx.pin() as _,
                        tx_pin: tx.pin() as _,
                    },
                },
            },
            host_config: esp_openthread_host_connection_config_t {
                host_connection_mode:
                    esp_openthread_host_connection_mode_t_HOST_CONNECTION_MODE_NONE,
                ..Default::default()
            },
            port_config: esp_openthread_port_config_t {
                storage_partition_name: b"TODO\0" as *const _ as *const _,
                netif_queue_size: 10,
                task_queue_size: 10,
            },
        };

        esp!(unsafe { esp_openthread_init(&cfg) })?;

        debug!("Driver initialized");

        Ok(Self {
            mode: Host(cfg),
            _mounted_event_fs: mounted_event_fs,
            _nvs: nvs,
            _p: PhantomData,
        })
    }
}

#[cfg(esp_idf_soc_ieee802154_supported)]
impl<'d> ThreadDriver<'d, RCP> {
    /// Create a new Thread RCP driver instance utilizing an SPI connection
    /// to another MCU running the Thread Host stack.
    #[cfg(not(esp_idf_version_major = "4"))]
    #[allow(clippy::too_many_arguments)]
    pub fn new_rcp_spi<M: crate::hal::modem::ThreadModemPeripheral, S: crate::hal::spi::Spi>(
        _modem: impl Peripheral<P = M> + 'd,
        _spi: impl Peripheral<P = S> + 'd,
        mosi: impl Peripheral<P = impl InputPin> + 'd,
        miso: impl Peripheral<P = impl OutputPin> + 'd,
        sclk: impl Peripheral<P = impl InputPin + OutputPin> + 'd,
        cs: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        intr: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        crate::hal::into_ref!(mosi, miso, sclk);

        let cs_pin = if let Some(cs) = cs {
            crate::hal::into_ref!(cs);

            cs.pin() as _
        } else {
            -1
        };

        let intr_pin = if let Some(intr) = intr {
            crate::hal::into_ref!(intr);

            intr.pin() as _
        } else {
            -1
        };

        let cfg = esp_openthread_platform_config_t {
            radio_config: esp_openthread_radio_config_t {
                radio_mode: esp_openthread_radio_mode_t_RADIO_MODE_NATIVE,
                ..Default::default()
            },
            host_config: esp_openthread_host_connection_config_t {
                host_connection_mode:
                    esp_openthread_host_connection_mode_t_HOST_CONNECTION_MODE_RCP_UART,
                __bindgen_anon_1: esp_openthread_host_connection_config_t__bindgen_ty_1 {
                    spi_slave_config: esp_openthread_spi_slave_config_t {
                        host_device: S::device() as _,
                        bus_config: spi_bus_config_t {
                            __bindgen_anon_1: spi_bus_config_t__bindgen_ty_1 {
                                mosi_io_num: mosi.pin() as _,
                            },
                            __bindgen_anon_2: spi_bus_config_t__bindgen_ty_2 {
                                miso_io_num: miso.pin() as _,
                            },
                            sclk_io_num: sclk.pin() as _,
                            ..Default::default()
                        },
                        slave_config: spi_slave_interface_config_t {
                            spics_io_num: cs_pin as _,
                            ..Default::default()
                        },
                        intr_pin,
                    },
                },
            },
            port_config: esp_openthread_port_config_t {
                storage_partition_name: b"TODO\0" as *const _ as *const _,
                netif_queue_size: 10,
                task_queue_size: 10,
            },
        };
        esp!(unsafe { esp_openthread_init(&cfg) })?;

        debug!("Driver initialized");

        Ok(Self {
            mode: RCP,
            _mounted_event_fs: mounted_event_fs,
            _nvs: nvs,
            _p: PhantomData,
        })
    }

    /// Create a new Thread RCP driver instance utilizing a UART connection
    /// to another MCU running the Thread Host stack.
    pub fn new_rcp_uart<M: crate::hal::modem::ThreadModemPeripheral, U: Uart>(
        _modem: impl Peripheral<P = M> + 'd,
        _uart: impl Peripheral<P = U> + 'd,
        rx: impl Peripheral<P = impl InputPin> + 'd,
        tx: impl Peripheral<P = impl OutputPin> + 'd,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        crate::hal::into_ref!(rx, tx);

        #[cfg(esp_idf_version_major = "4")]
        let cfg = esp_openthread_platform_config_t {
            radio_config: esp_openthread_radio_config_t {
                radio_mode: esp_openthread_radio_mode_t_RADIO_MODE_NATIVE,
                ..Default::default()
            },
            host_config: esp_openthread_host_connection_config_t {
                host_connection_mode:
                    esp_openthread_host_connection_mode_t_HOST_CONNECTION_MODE_RCP_UART,
                host_uart_config: esp_openthread_uart_config_t {
                    port: U::port() as _,
                    uart_config: uart_config_t {
                        baud_rate: BAUD_RATE as _,
                        data_bits: 8,
                        parity: 0,
                        stop_bits: 1,
                        flow_ctrl: 0,
                        rx_flow_ctrl_thresh: 0,
                        ..Default::default()
                    },
                    rx_pin: rx.pin() as _,
                    tx_pin: tx.pin() as _,
                },
            },
            port_config: esp_openthread_port_config_t {
                storage_partition_name: b"TODO\0" as *const _ as *const _,
                netif_queue_size: 10,
                task_queue_size: 10,
            },
        };

        #[cfg(not(esp_idf_version_major = "4"))]
        let cfg = esp_openthread_platform_config_t {
            radio_config: esp_openthread_radio_config_t {
                radio_mode: esp_openthread_radio_mode_t_RADIO_MODE_NATIVE,
                ..Default::default()
            },
            host_config: esp_openthread_host_connection_config_t {
                host_connection_mode:
                    esp_openthread_host_connection_mode_t_HOST_CONNECTION_MODE_RCP_UART,
                __bindgen_anon_1: esp_openthread_host_connection_config_t__bindgen_ty_1 {
                    host_uart_config: esp_openthread_uart_config_t {
                        port: U::port() as _,
                        uart_config: uart_config_t {
                            baud_rate: BAUD_RATE as _,
                            data_bits: 8,
                            parity: 0,
                            stop_bits: 1,
                            flow_ctrl: 0,
                            rx_flow_ctrl_thresh: 0,
                            ..Default::default()
                        },
                        rx_pin: rx.pin() as _,
                        tx_pin: tx.pin() as _,
                    },
                },
            },
            port_config: esp_openthread_port_config_t {
                storage_partition_name: b"TODO\0" as *const _ as *const _,
                netif_queue_size: 10,
                task_queue_size: 10,
            },
        };

        esp!(unsafe { esp_openthread_init(&cfg) })?;

        debug!("Driver initialized");

        Ok(Self {
            mode: RCP,
            _mounted_event_fs: mounted_event_fs,
            _nvs: nvs,
            _p: PhantomData,
        })
    }
}

impl<'d, T> ThreadDriver<'d, T> {
    /// Run the Thread stack
    ///
    /// The current thread would block while the stack is running
    /// Note that the stack will only exit if an error occurs
    pub fn run(&self) -> Result<(), EspError> {
        // TODO: Figure out how to stop running

        let _cs = CS.enter();

        esp!(unsafe { esp_openthread_launch_mainloop() })
    }
}

impl<'d, T> Drop for ThreadDriver<'d, T> {
    fn drop(&mut self) {
        esp!(unsafe { esp_openthread_deinit() }).unwrap();
    }
}

/// `EspThread` wraps a `ThreadDriver` Data Link layer instance, and binds the OSI
/// Layer 3 (network) facilities of ESP IDF to it.
///
/// In other words, it connects the ESP IDF Netif interface to the Thread driver.
/// This allows users to utilize the Rust STD APIs for working with TCP and UDP sockets.
///
/// This struct should be the default option for a Thread driver in all use cases
/// but the niche one where bypassing the ESP IDF Netif and lwIP stacks is
/// desirable. E.g., using `smoltcp` or other custom IP stacks on top of the
/// ESP IDF Thread radio.
#[cfg(esp_idf_comp_esp_netif_enabled)]
pub struct EspThread<'d> {
    netif: EspNetif,
    driver: ThreadDriver<'d, Host>,
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<'d> EspThread<'d> {
    /// Create a new `EspThread` instance utilizing the native Thread radio on the MCU
    #[cfg(esp_idf_soc_ieee802154_supported)]
    pub fn new<M: crate::hal::modem::ThreadModemPeripheral>(
        modem: impl Peripheral<P = M> + 'd,
        sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        Self::wrap(ThreadDriver::new(modem, sysloop, nvs, mounted_event_fs)?)
    }

    /// Create a new `EspThread` instance utilizing an SPI connection to another MCU
    /// which is expected to run the Thread RCP driver mode over SPI
    #[cfg(not(esp_idf_version_major = "4"))]
    #[allow(clippy::too_many_arguments)]
    pub fn new_spi<S: crate::hal::spi::Spi>(
        _spi: impl Peripheral<P = S> + 'd,
        mosi: impl Peripheral<P = impl InputPin> + 'd,
        miso: impl Peripheral<P = impl OutputPin> + 'd,
        sclk: impl Peripheral<P = impl InputPin + OutputPin> + 'd,
        cs: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        intr: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        Self::wrap(ThreadDriver::new_spi(
            _spi,
            mosi,
            miso,
            sclk,
            cs,
            intr,
            _sysloop,
            nvs,
            mounted_event_fs,
        )?)
    }

    /// Create a new `EspThread` instance utilizing a UART connection to another MCU
    /// which is expected to run the Thread RCP driver mode over UART
    pub fn new_uart<U: Uart>(
        _uart: impl Peripheral<P = U> + 'd,
        rx: impl Peripheral<P = impl InputPin> + 'd,
        tx: impl Peripheral<P = impl OutputPin> + 'd,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        Self::wrap(ThreadDriver::new_uart(
            _uart,
            rx,
            tx,
            _sysloop,
            nvs,
            mounted_event_fs,
        )?)
    }

    /// Wrap an already created Thread L2 driver instance
    pub fn wrap(driver: ThreadDriver<'d, Host>) -> Result<Self, EspError> {
        Self::wrap_all(driver, EspNetif::new(NetifStack::Thread)?)
    }

    /// Wrap an already created Thread L2 driver instance and a network interface
    pub fn wrap_all(driver: ThreadDriver<'d, Host>, netif: EspNetif) -> Result<Self, EspError> {
        let mut this = Self { driver, netif };

        this.attach_netif()?;

        Ok(this)
    }

    /// Replace the network interface with the provided one and return the
    /// existing network interface.
    pub fn swap_netif(&mut self, netif: EspNetif) -> Result<EspNetif, EspError> {
        self.detach_netif()?;

        let old = core::mem::replace(&mut self.netif, netif);

        self.attach_netif()?;

        Ok(old)
    }

    /// Return the underlying [`ThreadDriver`]
    pub fn driver(&self) -> &ThreadDriver<'d, Host> {
        &self.driver
    }

    /// Return the underlying [`ThreadDriver`], as mutable
    pub fn driver_mut(&mut self) -> &mut ThreadDriver<'d, Host> {
        &mut self.driver
    }

    /// Return the underlying [`EspNetif`]
    pub fn netif(&self) -> &EspNetif {
        &self.netif
    }

    /// Return the underlying [`EspNetif`] as mutable
    pub fn netif_mut(&mut self) -> &mut EspNetif {
        &mut self.netif
    }

    /// Run the Thread stack
    ///
    /// The current thread would block while the stack is running
    /// Note that the stack will only exit if an error occurs
    pub fn run(&self) -> Result<(), EspError> {
        self.driver.run()
    }

    fn attach_netif(&mut self) -> Result<(), EspError> {
        esp!(unsafe {
            esp_netif_attach(
                self.netif.handle() as *mut _,
                esp_openthread_netif_glue_init(&self.driver.mode.0),
            )
        })?;

        Ok(())
    }

    fn detach_netif(&mut self) -> Result<(), EspError> {
        unsafe {
            esp_openthread_netif_glue_deinit();
        }

        Ok(())
    }
}

impl Drop for EspThread<'_> {
    fn drop(&mut self) {
        let _ = self.detach_netif();
    }
}
