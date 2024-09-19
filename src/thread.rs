// TODO:
// - Prio A: Status report (we have driver::role() now; more, e.g. ipv6 notifications?)
// - Prio B: Option to switch between FTD (Full Thread Device) and MTD (Minimal Thread Device) (otDatasetCreateNewNetwork? probably needs CONFIG_OPENTHREAD_DEVICE_TYPE=CONFIG_OPENTHREAD_FTD/CONFIG_OPENTHREAD_MTD/CONFIG_OPENTHREAD_RADIO)
// - Prio B: API to enable the Joiner workflow (need to read on that, but not needed for Matter; CONFIG_OPENTHREAD_JOINER - also native OpenThread API https://github.com/espressif/esp-idf/issues/13475)
// - Prio B: API to to enable the Commissioner workflow (need to read on that, but not needed for Matter; CONFIG_OPENTHREAD_COMMISSIONER - also native OpenThread API https://github.com/espressif/esp-idf/issues/13475)
// - Prio B: Think of a minimal example
// - Prio C: How to support the OpenThread CLI (useful for debugging)
// - Prio C: Figure out what these do (bad/missing docu):
//   - CONFIG_OPENTHREAD_DNS_CLIENT (can this be enabled programmatically too - does not seem so, and why is this part of OpenThread and not the LwIP ipv6 stack?)
//   - CONFIG_OPENTHREAD_DIAG
//   - CONFIG_OPENTHREAD_CSL_ENABLE
//   - CONFIG_OPENTHREAD_DUA_ENABLE
//   - CONFIG_OPENTHREAD_SRP_CLIENT
//   - CONFIG_OPENTHREAD_DNS64_CLIENT? "Select this option to acquire NAT64 address from dns servers" why does this require explicit conf
//     or in fact why does this has anything to do with the OpenThread client?

use core::ffi::{self, c_void, CStr};
use core::fmt::Debug;
use core::marker::PhantomData;
use core::mem::MaybeUninit;

use log::debug;

use crate::eventloop::{EspEventDeserializer, EspEventSource, EspSystemEventLoop};
use crate::hal::delay;
use crate::hal::gpio::{InputPin, OutputPin};
use crate::hal::peripheral::Peripheral;
use crate::hal::task::CriticalSection;
use crate::hal::uart::Uart;
#[cfg(esp_idf_comp_esp_netif_enabled)]
use crate::handle::RawHandle;
use crate::io::vfs::MountedEventfs;
#[cfg(esp_idf_comp_esp_netif_enabled)]
use crate::netif::*;
use crate::nvs::EspDefaultNvsPartition;
use crate::sys::*;

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;

/// A trait shared between the `Host` and `RCP` modes providing the option for these
/// to do additional initialization.
pub trait Mode {
    fn init();
}

/// The driver will operate in Radio Co-Processor mode
///
/// The chip needs to be connected via UART or SPI to the host
#[cfg(esp_idf_soc_ieee802154_supported)]
#[derive(Debug)]
pub struct RCP(());

#[cfg(esp_idf_soc_ieee802154_supported)]
impl Mode for RCP {
    fn init() {
        extern "C" {
            fn otAppNcpInit(instance: *mut otInstance);
        }

        unsafe {
            otAppNcpInit(esp_openthread_get_instance());
        }
    }
}

/// The driver will operate as a host
///
/// This means that - unless the chip has a native Thread suppoort -
/// it needs to be connected via UART or SPI to another chip which does have
/// native Thread support and which is configured to operate in RCP mode
#[derive(Debug)]
pub struct Host(());

impl Mode for Host {
    fn init() {}
}

pub mod config {
    #[cfg(all(esp32c2, esp_idf_xtal_freq_26))]
    pub const UART_DEFAULT_BAUD_RATE: u32 = 74880;

    #[cfg(not(all(esp32c2, esp_idf_xtal_freq_26)))]
    pub const UART_DEFAULT_BAUD_RATE: u32 = 115200;
}

macro_rules! ot_esp {
    ($err:expr) => {{
        esp!({
            #[allow(non_upper_case_globals, non_snake_case)]
            let err = match $err as _ {
                otError_OT_ERROR_NONE => ESP_OK,
                otError_OT_ERROR_FAILED => ESP_FAIL,
                _ => ESP_FAIL, // For now
            };

            err
        })
    }};
}

/// Active scan result
pub struct ActiveScanResult<'a>(&'a otActiveScanResult);

impl<'a> ActiveScanResult<'a> {
    /// IEEE 802.15.4 Extended Address
    pub fn extended_address(&self) -> &'a [u8] {
        &self.0.mExtAddress.m8
    }

    /// Thread Network Name
    pub fn network_name_cstr(&self) -> &'a CStr {
        unsafe { ffi::CStr::from_ptr(&self.0.mNetworkName.m8 as *const _ as *const _) }
    }

    /// Thread Extended PAN ID
    pub fn extended_pan_id(&self) -> &[u8] {
        &self.0.mExtendedPanId.m8
    }

    /// Steering Data
    pub fn steering_data(&self) -> &[u8] {
        &self.0.mSteeringData.m8
    }

    /// IEEE 802.15.4 PAN ID
    pub fn pan_id(&self) -> u16 {
        self.0.mPanId
    }

    /// Joiner UDP Port
    pub fn joiner_udp_port(&self) -> u16 {
        self.0.mJoinerUdpPort
    }

    /// IEEE 802.15.4 Channel
    pub fn channel(&self) -> u8 {
        self.0.mChannel
    }

    /// The max RSSI (dBm)
    pub fn max_rssi(&self) -> i8 {
        self.0.mRssi
    }

    /// LQI
    pub fn lqi(&self) -> u8 {
        self.0.mLqi
    }

    /// Version
    pub fn version(&self) -> u8 {
        self.0.mVersion() as _
    }

    /// Native Commissioner
    pub fn native_commissioner(&self) -> bool {
        self.0.mIsNative()
    }

    /// Join permitted
    pub fn join_permitted(&self) -> bool {
        self.0.mIsJoinable()
    }
}

/// Energy scan result
pub struct EnergyScanResult<'a>(&'a otEnergyScanResult);

impl<'a> EnergyScanResult<'a> {
    /// IEEE 802.15.4 Channel
    pub fn channel(&self) -> u8 {
        self.0.mChannel
    }

    /// The max RSSI (dBm)
    pub fn max_rssi(&self) -> i8 {
        self.0.mMaxRssi
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Role {
    Disabled,
    Detached,
    Child,
    Router,
    Leader,
}

#[allow(non_upper_case_globals, non_snake_case)]
impl From<otDeviceRole> for Role {
    fn from(role: otDeviceRole) -> Self {
        match role {
            otDeviceRole_OT_DEVICE_ROLE_DISABLED => Role::Disabled,
            otDeviceRole_OT_DEVICE_ROLE_DETACHED => Role::Detached,
            otDeviceRole_OT_DEVICE_ROLE_CHILD => Role::Child,
            otDeviceRole_OT_DEVICE_ROLE_ROUTER => Role::Router,
            otDeviceRole_OT_DEVICE_ROLE_LEADER => Role::Leader,
            _ => Role::Disabled,
        }
    }
}

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
    cfg: esp_openthread_platform_config_t,
    cs: CriticalSection,
    //_subscription: EspSubscription<'static, System>,
    _nvs: EspDefaultNvsPartition,
    _mounted_event_fs: Arc<MountedEventfs>,
    _mode: T,
    _p: PhantomData<&'d mut ()>,
}

impl<'d> ThreadDriver<'d, Host> {
    /// Create a new Thread Host driver instance utilizing the
    /// native Thread radio on the MCU
    #[cfg(esp_idf_soc_ieee802154_supported)]
    pub fn new<M: crate::hal::modem::ThreadModemPeripheral>(
        modem: impl Peripheral<P = M> + 'd,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        Ok(Self {
            cfg: Self::native_host_cfg(modem),
            cs: CriticalSection::new(),
            _nvs: nvs,
            _mounted_event_fs: mounted_event_fs,
            _mode: Host(()),
            _p: PhantomData,
        })
    }

    /// Create a new Thread Host driver instance utilizing an SPI connection
    /// to another MCU running the Thread stack in RCP mode.
    #[cfg(not(esp_idf_version_major = "4"))]
    #[allow(clippy::too_many_arguments)]
    pub fn new_spi<S: crate::hal::spi::Spi>(
        spi: impl Peripheral<P = S> + 'd,
        mosi: impl Peripheral<P = impl InputPin> + 'd,
        miso: impl Peripheral<P = impl OutputPin> + 'd,
        sclk: impl Peripheral<P = impl InputPin + OutputPin> + 'd,
        cs: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        intr: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        config: &crate::hal::spi::config::Config,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        Ok(Self {
            cfg: Self::host_spi_cfg(spi, mosi, miso, sclk, cs, intr, config),
            cs: CriticalSection::new(),
            _nvs: nvs,
            _mounted_event_fs: mounted_event_fs,
            _mode: Host(()),
            _p: PhantomData,
        })
    }

    /// Create a new Thread Host driver instance utilizing a UART connection
    /// to another MCU running the Thread stack in RCP mode.
    pub fn new_uart<U: Uart>(
        uart: impl Peripheral<P = U> + 'd,
        tx: impl Peripheral<P = impl OutputPin> + 'd,
        rx: impl Peripheral<P = impl InputPin> + 'd,
        config: &crate::hal::uart::config::Config,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        Ok(Self {
            cfg: Self::host_uart_cfg(uart, tx, rx, config),
            cs: CriticalSection::new(),
            _nvs: nvs,
            _mounted_event_fs: mounted_event_fs,
            _mode: Host(()),
            _p: PhantomData,
        })
    }

    /// Retrieve the current role of the device in the Thread network
    pub fn role(&self) -> Result<Role, EspError> {
        let _lock = OtLock::acquire()?;

        Ok(unsafe { otThreadGetDeviceRole(esp_openthread_get_instance()) }.into())
    }

    /// Retrieve the active TOD (Thread Operational Dataset) in the user-supplied buffer
    ///
    /// Return the size of the TOD data written to the buffer
    ///
    /// The TOD is in Thread TLV format.
    pub fn tod(&self, buf: &mut [u8]) -> Result<usize, EspError> {
        self.internal_tod(true, buf)
    }

    /// Retrieve the pending TOD (Thread Operational Dataset) in the user-supplied buffer
    ///
    /// Return the size of the TOD data written to the buffer
    ///
    /// The TOD is in Thread TLV format.
    pub fn pending_tod(&self, buf: &mut [u8]) -> Result<usize, EspError> {
        self.internal_tod(false, buf)
    }

    /// Set the active TOD (Thread Operational Dataset) to the provided data
    ///
    /// The TOD data should be in Thread TLV format.
    pub fn set_tod(&self, tod: &[u8]) -> Result<(), EspError> {
        self.internal_set_tod(true, tod)
    }

    /// Set the pending TOD (Thread Operational Dataset) to the provided data
    ///
    /// The TOD data should be in Thread TLV format.
    pub fn set_pending_tod(&self, tod: &[u8]) -> Result<(), EspError> {
        self.internal_set_tod(false, tod)
    }

    /// Set the active TOD (Thread Operational Dataset) according to the
    /// `CONFIG_OPENTHREAD_` TOD-related parameters compiled into the app
    /// during build (via `sdkconfig*`)
    #[cfg(not(esp_idf_version_major = "4"))]
    pub fn set_tod_from_cfg(&self) -> Result<(), EspError> {
        ot_esp!(unsafe { esp_openthread_auto_start(core::ptr::null_mut()) })
    }

    /// Perform an active scan for Thread networks
    ///
    /// The callback will be called for each found network
    /// At the end of the scan, the callback will be called with `None`
    pub fn scan<F: FnMut(Option<ActiveScanResult>)>(&self, callback: F) -> Result<(), EspError> {
        let _lock = OtLock::acquire()?;

        #[allow(clippy::type_complexity)]
        let mut callback: Box<Box<dyn FnMut(Option<ActiveScanResult>)>> =
            Box::new(Box::new(callback));

        ot_esp!(unsafe {
            otLinkActiveScan(
                esp_openthread_get_instance(),
                0,
                0,
                Some(Self::on_active_scan_result),
                callback.as_mut() as *mut _ as *mut c_void,
            )
        })?;

        Ok(())
    }

    /// Check if an active scan is in progress
    pub fn is_scan_in_progress(&self) -> Result<bool, EspError> {
        let _lock = OtLock::acquire()?;

        Ok(unsafe { otLinkIsActiveScanInProgress(esp_openthread_get_instance()) })
    }

    /// Perform an energy scan for Thread networks
    ///
    /// The callback will be called for each found network
    /// At the end of the scan, the callback will be called with `None`
    pub fn energy_scan<F: FnMut(Option<EnergyScanResult>)>(
        &self,
        callback: F,
    ) -> Result<(), EspError> {
        let _lock = OtLock::acquire()?;

        #[allow(clippy::type_complexity)]
        let mut callback: Box<Box<dyn FnMut(Option<EnergyScanResult>)>> =
            Box::new(Box::new(callback));

        ot_esp!(unsafe {
            otLinkEnergyScan(
                esp_openthread_get_instance(),
                0,
                0,
                Some(Self::on_energy_scan_result),
                callback.as_mut() as *mut _ as *mut c_void,
            )
        })?;

        Ok(())
    }

    /// Check if an energy scan is in progress
    pub fn is_energy_scan_in_progress(&self) -> Result<bool, EspError> {
        let _lock = OtLock::acquire()?;

        Ok(unsafe { otLinkIsEnergyScanInProgress(esp_openthread_get_instance()) })
    }

    unsafe extern "C" fn on_active_scan_result(
        result: *mut otActiveScanResult,
        context: *mut c_void,
    ) {
        #[allow(clippy::type_complexity)]
        let callback =
            unsafe { (context as *mut Box<dyn FnMut(Option<ActiveScanResult>)>).as_mut() }.unwrap();

        if result.is_null() {
            callback(None);
        } else {
            callback(Some(ActiveScanResult(unsafe { result.as_ref() }.unwrap())));
        }
    }

    unsafe extern "C" fn on_energy_scan_result(
        result: *mut otEnergyScanResult,
        context: *mut c_void,
    ) {
        #[allow(clippy::type_complexity)]
        let callback =
            unsafe { (context as *mut Box<dyn FnMut(Option<EnergyScanResult>)>).as_mut() }.unwrap();

        if result.is_null() {
            callback(None);
        } else {
            callback(Some(EnergyScanResult(unsafe { result.as_ref() }.unwrap())));
        }
    }

    #[cfg(esp_idf_soc_ieee802154_supported)]
    fn host_native_cfg<M: crate::hal::modem::ThreadModemPeripheral>(
        _modem: impl Peripheral<P = M> + 'd,
    ) -> esp_openthread_platform_config_t {
        esp_openthread_platform_config_t {
            radio_config: esp_openthread_radio_config_t {
                radio_mode: esp_openthread_radio_mode_t_RADIO_MODE_NATIVE,
                ..Default::default()
            },
            host_config: esp_openthread_host_connection_config_t {
                host_connection_mode:
                    esp_openthread_host_connection_mode_t_HOST_CONNECTION_MODE_NONE,
                ..Default::default()
            },
            port_config: Self::PORT_CONFIG,
        }
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    #[allow(clippy::too_many_arguments)]
    fn host_spi_cfg<S: crate::hal::spi::Spi>(
        _spi: impl Peripheral<P = S> + 'd,
        mosi: impl Peripheral<P = impl InputPin> + 'd,
        miso: impl Peripheral<P = impl OutputPin> + 'd,
        sclk: impl Peripheral<P = impl InputPin + OutputPin> + 'd,
        cs: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        intr: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        config: &crate::hal::spi::config::Config,
    ) -> esp_openthread_platform_config_t {
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

        let mut icfg: spi_device_interface_config_t = config.into();
        icfg.spics_io_num = cs_pin as _;

        esp_openthread_platform_config_t {
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
                        spi_device: icfg,
                        intr_pin,
                    },
                },
            },
            host_config: esp_openthread_host_connection_config_t {
                host_connection_mode:
                    esp_openthread_host_connection_mode_t_HOST_CONNECTION_MODE_NONE,
                ..Default::default()
            },
            port_config: Self::PORT_CONFIG,
        }
    }

    fn host_uart_cfg<U: Uart>(
        _uart: impl Peripheral<P = U> + 'd,
        tx: impl Peripheral<P = impl OutputPin> + 'd,
        rx: impl Peripheral<P = impl InputPin> + 'd,
        config: &crate::hal::uart::config::Config,
    ) -> esp_openthread_platform_config_t {
        crate::hal::into_ref!(rx, tx);

        #[cfg(esp_idf_version_major = "4")]
        let cfg = esp_openthread_platform_config_t {
            radio_config: esp_openthread_radio_config_t {
                radio_mode: esp_openthread_radio_mode_t_RADIO_MODE_UART_RCP,
                radio_uart_config: esp_openthread_uart_config_t {
                    port: U::port() as _,
                    uart_config: config.into(),
                    rx_pin: rx.pin() as _,
                    tx_pin: tx.pin() as _,
                },
            },
            host_config: esp_openthread_host_connection_config_t {
                host_connection_mode:
                    esp_openthread_host_connection_mode_t_HOST_CONNECTION_MODE_NONE,
                ..Default::default()
            },
            port_config: Self::PORT_CONFIG,
        };

        #[cfg(not(esp_idf_version_major = "4"))]
        let cfg = esp_openthread_platform_config_t {
            radio_config: esp_openthread_radio_config_t {
                radio_mode: esp_openthread_radio_mode_t_RADIO_MODE_UART_RCP,
                __bindgen_anon_1: esp_openthread_radio_config_t__bindgen_ty_1 {
                    radio_uart_config: esp_openthread_uart_config_t {
                        port: U::port() as _,
                        uart_config: config.into(),
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
            port_config: Self::PORT_CONFIG,
        };

        cfg
    }

    fn internal_tod(&self, active: bool, buf: &mut [u8]) -> Result<usize, EspError> {
        let _lock = OtLock::acquire()?;

        let mut tlvs = MaybeUninit::<otOperationalDatasetTlvs>::uninit(); // TODO: Large buffer
        ot_esp!(unsafe {
            if active {
                otDatasetGetActiveTlvs(esp_openthread_get_instance(), tlvs.assume_init_mut())
            } else {
                otDatasetGetPendingTlvs(esp_openthread_get_instance(), tlvs.assume_init_mut())
            }
        })?;

        let tlvs = unsafe { tlvs.assume_init_mut() };

        let len = tlvs.mLength as usize;
        if buf.len() < len {
            Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>())?;
        }

        buf[..len].copy_from_slice(&tlvs.mTlvs[..len]);

        Ok(len)
    }

    fn internal_set_tod(&self, active: bool, data: &[u8]) -> Result<(), EspError> {
        let _lock = OtLock::acquire()?;

        let mut tlvs = MaybeUninit::<otOperationalDatasetTlvs>::uninit(); // TODO: Large buffer

        let tlvs = unsafe { tlvs.assume_init_mut() };

        if data.len() > core::mem::size_of_val(&tlvs.mTlvs) {
            Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>())?;
        }

        tlvs.mLength = data.len() as _;
        tlvs.mTlvs[..data.len()].copy_from_slice(data);

        ot_esp!(unsafe {
            if active {
                otDatasetSetActiveTlvs(esp_openthread_get_instance(), tlvs)
            } else {
                otDatasetSetPendingTlvs(esp_openthread_get_instance(), tlvs)
            }
        })?;

        Ok(())
    }
}

#[cfg(esp_idf_soc_ieee802154_supported)]
impl<'d> ThreadDriver<'d, RCP> {
    /// Create a new Thread RCP driver instance utilizing an SPI connection
    /// to another MCU running the Thread Host stack.
    #[cfg(not(esp_idf_version_major = "4"))]
    #[allow(clippy::too_many_arguments)]
    pub fn new_rcp_spi<M: crate::hal::modem::ThreadModemPeripheral, S: crate::hal::spi::Spi>(
        modem: impl Peripheral<P = M> + 'd,
        spi: impl Peripheral<P = S> + 'd,
        mosi: impl Peripheral<P = impl InputPin> + 'd,
        miso: impl Peripheral<P = impl OutputPin> + 'd,
        sclk: impl Peripheral<P = impl InputPin + OutputPin> + 'd,
        cs: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        intr: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        Ok(Self {
            cfg: Self::rcp_spi_cfg(modem, spi, mosi, miso, sclk, cs, intr),
            cs: CriticalSection::new(),
            _nvs: nvs,
            _mounted_event_fs: mounted_event_fs,
            _mode: RCP(()),
            _p: PhantomData,
        })
    }

    /// Create a new Thread RCP driver instance utilizing a UART connection
    /// to another MCU running the Thread Host stack.
    pub fn new_rcp_uart<M: crate::hal::modem::ThreadModemPeripheral, U: Uart>(
        modem: impl Peripheral<P = M> + 'd,
        uart: impl Peripheral<P = U> + 'd,
        tx: impl Peripheral<P = impl OutputPin> + 'd,
        rx: impl Peripheral<P = impl InputPin> + 'd,
        config: &crate::hal::uart::config::Config,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        Ok(Self {
            cfg: Self::rcp_uart_cfg(modem, uart, tx, rx, config),
            cs: CriticalSection::new(),
            _nvs: nvs,
            _mounted_event_fs: mounted_event_fs,
            _mode: RCP(()),
            _p: PhantomData,
        })
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    #[allow(clippy::too_many_arguments)]
    fn rcp_spi_cfg<M: crate::hal::modem::ThreadModemPeripheral, S: crate::hal::spi::Spi>(
        _modem: impl Peripheral<P = M> + 'd,
        _spi: impl Peripheral<P = S> + 'd,
        mosi: impl Peripheral<P = impl InputPin> + 'd,
        miso: impl Peripheral<P = impl OutputPin> + 'd,
        sclk: impl Peripheral<P = impl InputPin + OutputPin> + 'd,
        cs: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        intr: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
    ) -> esp_openthread_platform_config_t {
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

        esp_openthread_platform_config_t {
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
            port_config: Self::PORT_CONFIG,
        }
    }

    fn rcp_uart_cfg<M: crate::hal::modem::ThreadModemPeripheral, U: Uart>(
        _modem: impl Peripheral<P = M> + 'd,
        _uart: impl Peripheral<P = U> + 'd,
        tx: impl Peripheral<P = impl OutputPin> + 'd,
        rx: impl Peripheral<P = impl InputPin> + 'd,
        config: &crate::hal::uart::config::Config,
    ) -> esp_openthread_platform_config_t {
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
                    uart_config: config.into(),
                    rx_pin: rx.pin() as _,
                    tx_pin: tx.pin() as _,
                },
            },
            port_config: Self::PORT_CONFIG,
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
                        uart_config: config.into(),
                        rx_pin: rx.pin() as _,
                        tx_pin: tx.pin() as _,
                    },
                },
            },
            port_config: Self::PORT_CONFIG,
        };

        cfg
    }
}

impl<'d, T> ThreadDriver<'d, T>
where
    T: Mode,
{
    const PORT_CONFIG: esp_openthread_port_config_t = esp_openthread_port_config_t {
        storage_partition_name: b"nvs\0" as *const _ as *const _,
        netif_queue_size: 10,
        task_queue_size: 10,
    };

    /// Run the Thread stack
    ///
    /// The current thread would block while the stack is running
    /// Note that the stack will only exit if an error occurs
    pub fn run(&self) -> Result<(), EspError> {
        // TODO: Figure out how to stop running

        let _lock = self.cs.enter();

        esp!(unsafe { esp_openthread_init(&self.cfg) })?;

        debug!("Driver initialized");

        T::init();

        let result = esp!(unsafe { esp_openthread_launch_mainloop() });

        esp!(unsafe { esp_openthread_deinit() })?;

        debug!("Driver deinitialized");

        result
    }
}

struct OtLock(PhantomData<*const ()>);

impl OtLock {
    pub fn acquire() -> Result<Self, EspError> {
        if !unsafe { esp_openthread_lock_acquire(delay::BLOCK) } {
            Err(EspError::from_infallible::<ESP_ERR_TIMEOUT>())?;
        }

        Ok(Self(PhantomData))
    }
}

impl Drop for OtLock {
    fn drop(&mut self) {
        unsafe {
            esp_openthread_lock_release();
        }
    }
}

/// Trait shared between the modes of operation of the `EspThread` instance
pub trait NetifMode<'d> {
    fn driver(&self) -> &ThreadDriver<'d, Host>;
    fn driver_mut(&mut self) -> &mut ThreadDriver<'d, Host>;
}

/// The regular mode of operation for the `EspThread` instance
///
/// This is the only available mode if the Border Router functionality in ESP-IDF is not enabled
pub struct Node<'d>(ThreadDriver<'d, Host>);

impl<'d> NetifMode<'d> for Node<'d> {
    fn driver(&self) -> &ThreadDriver<'d, Host> {
        &self.0
    }

    fn driver_mut(&mut self) -> &mut ThreadDriver<'d, Host> {
        &mut self.0
    }
}

/// The Border Router mode of operation for the `EspThread` instance
#[cfg(all(
    esp_idf_comp_esp_netif_enabled,
    esp_idf_openthread_border_router,
    not(any(esp32h2, esp32h4))
))]
pub struct BorderRouter<'d, N>(ThreadDriver<'d, Host>, N)
where
    N: core::borrow::Borrow<EspNetif>;

#[cfg(all(
    esp_idf_comp_esp_netif_enabled,
    esp_idf_openthread_border_router,
    not(any(esp32h2, esp32h4))
))]
impl<'d, N> BorderRouter<'d, N>
where
    N: core::borrow::Borrow<EspNetif>,
{
    fn new(driver: ThreadDriver<'d, Host>, netif: N) -> Result<Self, EspError> {
        #[cfg(esp_idf_version_major = "4")]
        {
            esp!(unsafe { esp_openthread_border_router_init(netif.borrow().handle()) })?;
        }

        #[cfg(not(esp_idf_version_major = "4"))]
        {
            unsafe {
                esp_openthread_set_backbone_netif(netif.borrow().handle());
            }

            esp!(unsafe { esp_openthread_border_router_init() })?;
        }

        debug!("Border router initialized");

        Ok(Self(driver, netif))
    }
}

#[cfg(all(
    esp_idf_comp_esp_netif_enabled,
    esp_idf_openthread_border_router,
    not(any(esp32h2, esp32h4))
))]
impl<'d, N: core::borrow::Borrow<EspNetif>> Drop for BorderRouter<'d, N> {
    fn drop(&mut self) {
        esp!(unsafe { esp_openthread_border_router_deinit() }).unwrap();

        debug!("Border router dropped");
    }
}

#[cfg(all(
    esp_idf_comp_esp_netif_enabled,
    esp_idf_openthread_border_router,
    not(any(esp32h2, esp32h4))
))]
impl<'d, N: core::borrow::Borrow<EspNetif>> NetifMode<'d> for BorderRouter<'d, N> {
    fn driver(&self) -> &ThreadDriver<'d, Host> {
        &self.0
    }

    fn driver_mut(&mut self) -> &mut ThreadDriver<'d, Host> {
        &mut self.0
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
pub struct EspThread<T> {
    netif: EspNetif,
    driver: T,
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<'d> EspThread<Node<'d>> {
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
        config: &crate::hal::spi::config::Config,
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
            config,
            _sysloop,
            nvs,
            mounted_event_fs,
        )?)
    }

    /// Create a new `EspThread` instance utilizing a UART connection to another MCU
    /// which is expected to run the Thread RCP driver mode over UART
    pub fn new_uart<U: Uart>(
        _uart: impl Peripheral<P = U> + 'd,
        tx: impl Peripheral<P = impl OutputPin> + 'd,
        rx: impl Peripheral<P = impl InputPin> + 'd,
        config: &crate::hal::uart::config::Config,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
    ) -> Result<Self, EspError> {
        Self::wrap(ThreadDriver::new_uart(
            _uart,
            tx,
            rx,
            config,
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
        let mut this = Self {
            driver: Node(driver),
            netif,
        };

        this.attach_netif()?;
        Self::enable_network(true)?;

        Ok(this)
    }
}

#[cfg(all(
    esp_idf_comp_esp_netif_enabled,
    esp_idf_openthread_border_router,
    not(any(esp32h2, esp32h4))
))]
impl<'d, N> EspThread<BorderRouter<'d, N>>
where
    N: core::borrow::Borrow<EspNetif>,
{
    /// Create a new `EspThread` Border Router instance utilizing the native Thread radio on the MCU
    #[cfg(esp_idf_soc_ieee802154_supported)]
    pub fn new_br<M: crate::hal::modem::ThreadModemPeripheral>(
        modem: impl Peripheral<P = M> + 'd,
        sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
        backbone_netif: N,
    ) -> Result<Self, EspError> {
        Self::wrap_br(
            ThreadDriver::new(modem, sysloop, nvs, mounted_event_fs)?,
            backbone_netif,
        )
    }

    /// Create a new `EspThread` Border Router instance utilizing an SPI connection to another MCU
    /// which is expected to run the Thread RCP driver mode over SPI
    #[cfg(not(esp_idf_version_major = "4"))]
    #[allow(clippy::too_many_arguments)]
    pub fn new_br_spi<S: crate::hal::spi::Spi>(
        _spi: impl Peripheral<P = S> + 'd,
        mosi: impl Peripheral<P = impl InputPin> + 'd,
        miso: impl Peripheral<P = impl OutputPin> + 'd,
        sclk: impl Peripheral<P = impl InputPin + OutputPin> + 'd,
        cs: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        intr: Option<impl Peripheral<P = impl InputPin + OutputPin> + 'd>,
        config: &crate::hal::spi::config::Config,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
        backbone_netif: N,
    ) -> Result<Self, EspError> {
        Self::wrap_br(
            ThreadDriver::new_spi(
                _spi,
                mosi,
                miso,
                sclk,
                cs,
                intr,
                config,
                _sysloop,
                nvs,
                mounted_event_fs,
            )?,
            backbone_netif,
        )
    }

    /// Create a new `EspThread` Border Router instance utilizing a UART connection to another MCU
    /// which is expected to run the Thread RCP driver mode over UART
    #[allow(clippy::too_many_arguments)]
    pub fn new_br_uart<U: Uart>(
        _uart: impl Peripheral<P = U> + 'd,
        tx: impl Peripheral<P = impl OutputPin> + 'd,
        rx: impl Peripheral<P = impl InputPin> + 'd,
        config: &crate::hal::uart::config::Config,
        _sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        mounted_event_fs: Arc<MountedEventfs>,
        backbone_netif: N,
    ) -> Result<Self, EspError> {
        Self::wrap_br(
            ThreadDriver::new_uart(_uart, tx, rx, config, _sysloop, nvs, mounted_event_fs)?,
            backbone_netif,
        )
    }

    /// Wrap an already created Thread L2 driver instance and a backbone network interface
    /// to the outside world
    pub fn wrap_br(driver: ThreadDriver<'d, Host>, backbone_netif: N) -> Result<Self, EspError> {
        Self::wrap_br_all(driver, EspNetif::new(NetifStack::Thread)?, backbone_netif)
    }

    /// Wrap an already created Thread L2 driver instance, a network interface to be used for the
    /// Thread network, and a backbone network interface to the outside world
    pub fn wrap_br_all(
        driver: ThreadDriver<'d, Host>,
        netif: EspNetif,
        backbone_netif: N,
    ) -> Result<Self, EspError> {
        let mut this = Self {
            driver: BorderRouter::new(driver, backbone_netif)?,
            netif,
        };

        this.attach_netif()?;
        Self::enable_network(true)?;

        Ok(this)
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<'d, T> EspThread<T>
where
    T: NetifMode<'d>,
{
    /// Return the underlying [`ThreadDriver`]
    pub fn driver(&self) -> &ThreadDriver<'d, Host> {
        self.driver.driver()
    }

    /// Return the underlying [`ThreadDriver`], as mutable
    pub fn driver_mut(&mut self) -> &mut ThreadDriver<'d, Host> {
        self.driver.driver_mut()
    }

    /// Return the underlying [`EspNetif`]
    pub fn netif(&self) -> &EspNetif {
        &self.netif
    }

    /// Return the underlying [`EspNetif`] as mutable
    pub fn netif_mut(&mut self) -> &mut EspNetif {
        &mut self.netif
    }

    /// Retrieve the current role of the device in the Thread network
    pub fn role(&self) -> Result<Role, EspError> {
        self.driver().role()
    }

    /// Retrieve the active TOD (Thread Operational Dataset) in the user-supplied buffer
    ///
    /// Return the size of the TOD data written to the buffer
    ///
    /// The TOD is in Thread TLV format.
    pub fn tod(&self, buf: &mut [u8]) -> Result<usize, EspError> {
        self.driver().tod(buf)
    }

    /// Retrieve the pending TOD (Thread Operational Dataset) in the user-supplied buffer
    ///
    /// Return the size of the TOD data written to the buffer
    ///
    /// The TOD is in Thread TLV format.
    pub fn pending_tod(&self, buf: &mut [u8]) -> Result<usize, EspError> {
        self.driver().pending_tod(buf)
    }

    /// Set the active TOD (Thread Operational Dataset) to the provided data
    ///
    /// The TOD data should be in Thread TLV format.
    pub fn set_tod(&self, tod: &[u8]) -> Result<(), EspError> {
        self.driver().set_tod(tod)
    }

    /// Set the pending TOD (Thread Operational Dataset) to the provided data
    ///
    /// The TOD data should be in Thread TLV format.
    pub fn set_pending_tod(&self, tod: &[u8]) -> Result<(), EspError> {
        self.driver().set_pending_tod(tod)
    }

    /// Set the active TOD (Thread Operational Dataset) according to the
    /// `CONFIG_OPENTHREAD_` TOD-related parameters compiled into the app
    /// during build (via `sdkconfig*`)
    #[cfg(not(esp_idf_version_major = "4"))]
    pub fn set_tod_from_cfg(&self) -> Result<(), EspError> {
        self.driver().set_tod_from_cfg()
    }

    /// Perform an active scan for Thread networks
    /// The callback will be called for each found network
    ///
    /// At the end of the scan, the callback will be called with `None`
    pub fn scan<F: FnMut(Option<ActiveScanResult>)>(&self, callback: F) -> Result<(), EspError> {
        self.driver().scan(callback)
    }

    /// Check if an active scan is in progress
    pub fn is_scan_in_progress(&self) -> Result<bool, EspError> {
        self.driver().is_scan_in_progress()
    }

    /// Perform an energy scan for Thread networks
    /// The callback will be called for each found network
    ///
    /// At the end of the scan, the callback will be called with `None`
    pub fn energy_scan<F: FnMut(Option<EnergyScanResult>)>(
        &self,
        callback: F,
    ) -> Result<(), EspError> {
        self.driver().energy_scan(callback)
    }

    /// Check if an energy scan is in progress
    pub fn is_energy_scan_in_progress(&self) -> Result<bool, EspError> {
        self.driver().is_energy_scan_in_progress()
    }

    /// Run the Thread stack
    ///
    /// The current thread would block while the stack is running
    /// Note that the stack will only exit if an error occurs
    pub fn run(&self) -> Result<(), EspError> {
        self.driver.driver().run()
    }

    /// Replace the network interface with the provided one and return the
    /// existing network interface.
    pub fn swap_netif(&mut self, netif: EspNetif) -> Result<EspNetif, EspError> {
        Self::enable_network(false)?;
        self.detach_netif()?;

        let old = core::mem::replace(&mut self.netif, netif);

        self.attach_netif()?;
        Self::enable_network(true)?;

        Ok(old)
    }

    fn attach_netif(&mut self) -> Result<(), EspError> {
        esp!(unsafe {
            esp_netif_attach(
                self.netif.handle() as *mut _,
                esp_openthread_netif_glue_init(&self.driver.driver().cfg),
            )
        })?;

        Ok(())
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<T> EspThread<T> {
    fn enable_network(enabled: bool) -> Result<(), EspError> {
        ot_esp!(unsafe { otIp6SetEnabled(esp_openthread_get_instance(), enabled) })?;
        ot_esp!(unsafe { otThreadSetEnabled(esp_openthread_get_instance(), enabled) })?;

        Ok(())
    }

    fn detach_netif(&mut self) -> Result<(), EspError> {
        unsafe {
            esp_openthread_netif_glue_deinit();
        }

        Ok(())
    }
}

impl<T> Drop for EspThread<T> {
    fn drop(&mut self) {
        let _ = self.detach_netif();
    }
}

/// Events reported by the Thread stack on the system event loop
#[derive(Copy, Clone, Debug)]
pub enum ThreadEvent {
    /// Thread stack started
    Started,
    /// Thread stack stopped
    Stopped,
    /// Thread stack detached
    #[cfg(not(esp_idf_version_major = "4"))]
    Detached,
    /// Thread stack attached
    #[cfg(not(esp_idf_version_major = "4"))]
    Attached,
    /// Thread role changed
    #[cfg(not(esp_idf_version_major = "4"))]
    RoleChanged {
        current_role: Role,
        previous_role: Role,
    },
    /// Thread network interface up
    IfUp,
    /// Thread network interface down
    IfDown,
    /// Thread got IPv6 address
    GotIpv6,
    /// Thread lost IPv6 address
    LostIpv6,
    /// Thread multicast group joined
    MulticastJoined,
    /// Thread multicast group left
    MulticastLeft,
    /// Thread TREL IPv6 address added
    #[cfg(not(esp_idf_version_major = "4"))]
    TrelIpv6Added,
    /// Thread TREL IPv6 address removed
    #[cfg(not(esp_idf_version_major = "4"))]
    TrelIpv6Removed,
    /// Thread TREL multicast group joined
    #[cfg(not(esp_idf_version_major = "4"))]
    TrelMulticastJoined,
    /// Thread DNS server set
    // Since 5.1
    #[cfg(all(
        not(esp_idf_version_major = "4"),
        not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0"))
    ))]
    DnsServerSet,
    /// Thread Meshcop E Publish started
    // Since 5.2.2
    #[cfg(any(
        not(any(esp_idf_version_major = "4", esp_idf_version_major = "5")),
        all(
            esp_idf_version_major = "5",
            not(esp_idf_version_minor = "0"),
            not(esp_idf_version_minor = "1"),
            not(all(
                esp_idf_version_minor = "2",
                any(esp_idf_version_patch = "0", esp_idf_version_patch = "1")
            )),
        ),
    ))]
    MeshcopEPublishStarted,
    /// Thread Meshcop E Remove started
    // Since 5.2.2
    #[cfg(any(
        not(any(esp_idf_version_major = "4", esp_idf_version_major = "5")),
        all(
            esp_idf_version_major = "5",
            not(esp_idf_version_minor = "0"),
            not(esp_idf_version_minor = "1"),
            not(all(
                esp_idf_version_minor = "2",
                any(esp_idf_version_patch = "0", esp_idf_version_patch = "1")
            )),
        ),
    ))]
    MeshcopERemoveStarted,
}

unsafe impl EspEventSource for ThreadEvent {
    fn source() -> Option<&'static ffi::CStr> {
        Some(unsafe { ffi::CStr::from_ptr(OPENTHREAD_EVENT) })
    }
}

impl EspEventDeserializer for ThreadEvent {
    type Data<'d> = ThreadEvent;

    #[allow(non_upper_case_globals, non_snake_case)]
    fn deserialize(data: &crate::eventloop::EspEvent) -> ThreadEvent {
        let event_id = data.event_id as u32;

        match event_id {
            esp_openthread_event_t_OPENTHREAD_EVENT_START => ThreadEvent::Started,
            esp_openthread_event_t_OPENTHREAD_EVENT_STOP => ThreadEvent::Stopped,
            #[cfg(not(esp_idf_version_major = "4"))]
            esp_openthread_event_t_OPENTHREAD_EVENT_DETACHED => ThreadEvent::Detached,
            #[cfg(not(esp_idf_version_major = "4"))]
            esp_openthread_event_t_OPENTHREAD_EVENT_ATTACHED => ThreadEvent::Attached,
            #[cfg(not(esp_idf_version_major = "4"))]
            esp_openthread_event_t_OPENTHREAD_EVENT_ROLE_CHANGED => {
                let payload = unsafe {
                    (data.payload.unwrap() as *const _
                        as *const esp_openthread_role_changed_event_t)
                        .as_ref()
                }
                .unwrap();

                ThreadEvent::RoleChanged {
                    current_role: payload.current_role.into(),
                    previous_role: payload.previous_role.into(),
                }
            }
            esp_openthread_event_t_OPENTHREAD_EVENT_IF_UP => ThreadEvent::IfUp,
            esp_openthread_event_t_OPENTHREAD_EVENT_IF_DOWN => ThreadEvent::IfDown,
            esp_openthread_event_t_OPENTHREAD_EVENT_GOT_IP6 => ThreadEvent::GotIpv6,
            esp_openthread_event_t_OPENTHREAD_EVENT_LOST_IP6 => ThreadEvent::LostIpv6,
            esp_openthread_event_t_OPENTHREAD_EVENT_MULTICAST_GROUP_JOIN => {
                ThreadEvent::MulticastJoined
            }
            esp_openthread_event_t_OPENTHREAD_EVENT_MULTICAST_GROUP_LEAVE => {
                ThreadEvent::MulticastLeft
            }
            #[cfg(not(esp_idf_version_major = "4"))]
            esp_openthread_event_t_OPENTHREAD_EVENT_TREL_ADD_IP6 => ThreadEvent::TrelIpv6Added,
            #[cfg(not(esp_idf_version_major = "4"))]
            esp_openthread_event_t_OPENTHREAD_EVENT_TREL_REMOVE_IP6 => ThreadEvent::TrelIpv6Removed,
            #[cfg(not(esp_idf_version_major = "4"))]
            esp_openthread_event_t_OPENTHREAD_EVENT_TREL_MULTICAST_GROUP_JOIN => {
                ThreadEvent::TrelMulticastJoined
            }
            #[cfg(all(
                not(esp_idf_version_major = "4"),
                not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0"))
            ))]
            esp_openthread_event_t_OPENTHREAD_EVENT_SET_DNS_SERVER => ThreadEvent::DnsServerSet,
            // Since 5.2.2
            #[cfg(any(
                not(any(esp_idf_version_major = "4", esp_idf_version_major = "5")),
                all(
                    esp_idf_version_major = "5",
                    not(esp_idf_version_minor = "0"),
                    not(esp_idf_version_minor = "1"),
                    not(all(
                        esp_idf_version_minor = "2",
                        any(esp_idf_version_patch = "0", esp_idf_version_patch = "1")
                    )),
                ),
            ))]
            esp_openthread_event_t_OPENTHREAD_EVENT_PUBLISH_MESHCOP_E => {
                ThreadEvent::MeshcopEPublishStarted
            }
            // Since 5.2.2
            #[cfg(any(
                not(any(esp_idf_version_major = "4", esp_idf_version_major = "5")),
                all(
                    esp_idf_version_major = "5",
                    not(esp_idf_version_minor = "0"),
                    not(esp_idf_version_minor = "1"),
                    not(all(
                        esp_idf_version_minor = "2",
                        any(esp_idf_version_patch = "0", esp_idf_version_patch = "1")
                    )),
                ),
            ))]
            esp_openthread_event_t_OPENTHREAD_EVENT_REMOVE_MESHCOP_E => {
                ThreadEvent::MeshcopERemoveStarted
            }
            _ => panic!("unknown event ID: {}", event_id),
        }
    }
}
