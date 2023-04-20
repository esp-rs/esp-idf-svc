//! WiFi support
use core::marker::PhantomData;
use core::time::Duration;
use core::{cmp, ffi};

extern crate alloc;
use alloc::boxed::Box;
use alloc::sync::Arc;

use ::log::*;

use enumset::*;

use embedded_svc::wifi::*;

use esp_idf_hal::modem::WifiModemPeripheral;
use esp_idf_hal::peripheral::Peripheral;

use esp_idf_sys::*;

use crate::eventloop::EspEventLoop;
use crate::eventloop::{
    EspSubscription, EspSystemEventLoop, EspTypedEventDeserializer, EspTypedEventSource, System,
};
use crate::handle::RawHandle;
#[cfg(esp_idf_comp_esp_netif_enabled)]
use crate::netif::*;
use crate::nvs::EspDefaultNvsPartition;
use crate::private::common::*;
use crate::private::cstr::*;
use crate::private::mutex;
use crate::private::waitable::*;

pub mod config {
    use core::time::Duration;

    use esp_idf_sys::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ScanType {
        Active { min: Duration, max: Duration },
        Passive(Duration),
    }

    impl ScanType {
        pub const fn new() -> Self {
            Self::Active {
                min: Duration::from_secs(0),
                max: Duration::from_secs(0),
            }
        }
    }

    impl Default for ScanType {
        fn default() -> Self {
            Self::new()
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ScanConfig {
        pub bssid: Option<[u8; 6]>,
        pub ssid: Option<heapless::String<32>>,
        pub channel: Option<u8>,
        pub scan_type: ScanType,
        pub show_hidden: bool,
    }

    impl ScanConfig {
        pub const fn new() -> Self {
            Self {
                bssid: None,
                ssid: None,
                channel: None,
                scan_type: ScanType::new(),
                show_hidden: false,
            }
        }
    }

    impl Default for ScanConfig {
        fn default() -> Self {
            Self::new()
        }
    }

    impl From<&ScanConfig> for wifi_scan_config_t {
        fn from(s: &ScanConfig) -> Self {
            Self {
                bssid: s.bssid.map_or(core::ptr::null(), |v| v.as_ptr()) as *mut u8,
                ssid: s.ssid.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()) as *mut u8,
                scan_time: wifi_scan_time_t {
                    active: wifi_active_scan_time_t {
                        min: match s.scan_type {
                            ScanType::Active { min, .. } => min.as_millis() as _,
                            _ => 0,
                        },
                        max: match s.scan_type {
                            ScanType::Active { max, .. } => max.as_millis() as _,
                            _ => 0,
                        },
                    },
                    passive: match s.scan_type {
                        ScanType::Passive(time) => time.as_millis() as _,
                        _ => 0,
                    },
                },
                channel: s.channel.unwrap_or_default(),
                scan_type: matches!(s.scan_type, ScanType::Active { .. }).into(),
                show_hidden: s.show_hidden,
            }
        }
    }
}

impl From<AuthMethod> for Newtype<wifi_auth_mode_t> {
    fn from(method: AuthMethod) -> Self {
        Newtype(match method {
            AuthMethod::None => wifi_auth_mode_t_WIFI_AUTH_OPEN,
            AuthMethod::WEP => wifi_auth_mode_t_WIFI_AUTH_WEP,
            AuthMethod::WPA => wifi_auth_mode_t_WIFI_AUTH_WPA_PSK,
            AuthMethod::WPA2Personal => wifi_auth_mode_t_WIFI_AUTH_WPA2_PSK,
            AuthMethod::WPAWPA2Personal => wifi_auth_mode_t_WIFI_AUTH_WPA_WPA2_PSK,
            AuthMethod::WPA2Enterprise => wifi_auth_mode_t_WIFI_AUTH_WPA2_ENTERPRISE,
            AuthMethod::WPA3Personal => wifi_auth_mode_t_WIFI_AUTH_WPA3_PSK,
            AuthMethod::WPA2WPA3Personal => wifi_auth_mode_t_WIFI_AUTH_WPA2_WPA3_PSK,
            AuthMethod::WAPIPersonal => wifi_auth_mode_t_WIFI_AUTH_WAPI_PSK,
        })
    }
}

impl From<Newtype<wifi_auth_mode_t>> for AuthMethod {
    #[allow(non_upper_case_globals)]
    fn from(mode: Newtype<wifi_auth_mode_t>) -> Self {
        match mode.0 {
            wifi_auth_mode_t_WIFI_AUTH_OPEN => AuthMethod::None,
            wifi_auth_mode_t_WIFI_AUTH_WEP => AuthMethod::WEP,
            wifi_auth_mode_t_WIFI_AUTH_WPA_PSK => AuthMethod::WPA,
            wifi_auth_mode_t_WIFI_AUTH_WPA2_PSK => AuthMethod::WPA2Personal,
            wifi_auth_mode_t_WIFI_AUTH_WPA_WPA2_PSK => AuthMethod::WPAWPA2Personal,
            wifi_auth_mode_t_WIFI_AUTH_WPA2_ENTERPRISE => AuthMethod::WPA2Enterprise,
            wifi_auth_mode_t_WIFI_AUTH_WPA3_PSK => AuthMethod::WPA3Personal,
            wifi_auth_mode_t_WIFI_AUTH_WPA2_WPA3_PSK => AuthMethod::WPA2WPA3Personal,
            wifi_auth_mode_t_WIFI_AUTH_WAPI_PSK => AuthMethod::WAPIPersonal,
            _ => panic!(),
        }
    }
}

impl From<&ClientConfiguration> for Newtype<wifi_sta_config_t> {
    fn from(conf: &ClientConfiguration) -> Self {
        let bssid: [u8; 6] = match &conf.bssid {
            Some(bssid_ref) => *bssid_ref,
            None => [0; 6],
        };

        let mut result = wifi_sta_config_t {
            ssid: [0; 32],
            password: [0; 64],
            scan_method: wifi_scan_method_t_WIFI_ALL_CHANNEL_SCAN,
            bssid_set: conf.bssid.is_some(),
            bssid,
            channel: conf.channel.unwrap_or(0u8),
            listen_interval: 0,
            sort_method: wifi_sort_method_t_WIFI_CONNECT_AP_BY_SIGNAL,
            threshold: wifi_scan_threshold_t {
                rssi: -127,
                authmode: Newtype::<wifi_auth_mode_t>::from(conf.auth_method).0,
            },
            pmf_cfg: wifi_pmf_config_t {
                capable: false,
                required: false,
            },
            ..Default::default()
        };

        set_str(&mut result.ssid, conf.ssid.as_ref());
        set_str(&mut result.password, conf.password.as_ref());

        Newtype(result)
    }
}

impl From<Newtype<wifi_sta_config_t>> for ClientConfiguration {
    fn from(conf: Newtype<wifi_sta_config_t>) -> Self {
        Self {
            ssid: from_cstr(&conf.0.ssid).into(),
            bssid: if conf.0.bssid_set {
                Some(conf.0.bssid)
            } else {
                None
            },
            auth_method: Newtype(conf.0.threshold.authmode).into(),
            password: from_cstr(&conf.0.password).into(),
            channel: if conf.0.channel != 0 {
                Some(conf.0.channel)
            } else {
                None
            },
        }
    }
}

impl From<&AccessPointConfiguration> for Newtype<wifi_ap_config_t> {
    fn from(conf: &AccessPointConfiguration) -> Self {
        let mut result = wifi_ap_config_t {
            ssid: [0; 32],
            password: [0; 64],
            ssid_len: conf.ssid.len() as u8,
            channel: conf.channel,
            authmode: Newtype::<wifi_auth_mode_t>::from(conf.auth_method).0,
            ssid_hidden: u8::from(conf.ssid_hidden),
            max_connection: cmp::max(conf.max_connections, 16) as u8,
            beacon_interval: 100,
            ..Default::default()
        };

        set_str(&mut result.ssid, conf.ssid.as_ref());
        set_str(&mut result.password, conf.password.as_ref());

        Newtype(result)
    }
}

impl From<Newtype<wifi_ap_config_t>> for AccessPointConfiguration {
    fn from(conf: Newtype<wifi_ap_config_t>) -> Self {
        Self {
            ssid: if conf.0.ssid_len == 0 {
                from_cstr(&conf.0.ssid).into()
            } else {
                unsafe {
                    core::str::from_utf8_unchecked(&conf.0.ssid[0..conf.0.ssid_len as usize]).into()
                }
            },
            ssid_hidden: conf.0.ssid_hidden != 0,
            channel: conf.0.channel,
            secondary_channel: None,
            auth_method: AuthMethod::from(Newtype(conf.0.authmode)),
            protocols: EnumSet::<Protocol>::empty(), // TODO
            password: from_cstr(&conf.0.password).into(),
            max_connections: conf.0.max_connection as u16,
        }
    }
}

impl From<Newtype<&wifi_ap_record_t>> for AccessPointInfo {
    #[allow(non_upper_case_globals)]
    fn from(ap_info: Newtype<&wifi_ap_record_t>) -> Self {
        let a = ap_info.0;

        Self {
            ssid: from_cstr(&a.ssid).into(),
            bssid: a.bssid,
            channel: a.primary,
            secondary_channel: match a.second {
                wifi_second_chan_t_WIFI_SECOND_CHAN_NONE => SecondaryChannel::None,
                wifi_second_chan_t_WIFI_SECOND_CHAN_ABOVE => SecondaryChannel::Above,
                wifi_second_chan_t_WIFI_SECOND_CHAN_BELOW => SecondaryChannel::Below,
                _ => panic!(),
            },
            signal_strength: a.rssi,
            protocols: EnumSet::<Protocol>::empty(), // TODO
            auth_method: AuthMethod::from(Newtype::<wifi_auth_mode_t>(a.authmode)),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WifiDeviceId {
    Ap,
    Sta,
}

impl From<WifiDeviceId> for wifi_interface_t {
    fn from(id: WifiDeviceId) -> Self {
        match id {
            WifiDeviceId::Ap => wifi_interface_t_WIFI_IF_AP,
            WifiDeviceId::Sta => wifi_interface_t_WIFI_IF_STA,
        }
    }
}

#[allow(non_upper_case_globals)]
impl From<wifi_interface_t> for WifiDeviceId {
    fn from(id: wifi_interface_t) -> Self {
        match id {
            wifi_interface_t_WIFI_IF_AP => WifiDeviceId::Ap,
            wifi_interface_t_WIFI_IF_STA => WifiDeviceId::Sta,
            _ => unreachable!(),
        }
    }
}

extern "C" {
    fn esp_wifi_internal_reg_rxcb(
        ifx: wifi_interface_t,
        rxcb: Option<
            unsafe extern "C" fn(
                buffer: *mut ffi::c_void,
                len: u16,
                eb: *mut ffi::c_void,
            ) -> esp_err_t,
        >,
    ) -> esp_err_t;

    fn esp_wifi_internal_free_rx_buffer(buffer: *mut ffi::c_void);

    fn esp_wifi_internal_tx(
        wifi_if: wifi_interface_t,
        buffer: *mut ffi::c_void,
        len: u16,
    ) -> esp_err_t;
}

#[allow(clippy::type_complexity)]
static mut RX_CALLBACK: Option<
    Box<dyn FnMut(WifiDeviceId, &[u8]) -> Result<(), EspError> + 'static>,
> = None;
#[allow(clippy::type_complexity)]
static mut TX_CALLBACK: Option<Box<dyn FnMut(WifiDeviceId, &[u8], bool) + 'static>> = None;

pub struct WifiDriver<'d> {
    status: Arc<mutex::Mutex<(WifiEvent, WifiEvent)>>,
    _subscription: EspSubscription<System>,
    #[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
    _nvs: Option<EspDefaultNvsPartition>,
    _p: PhantomData<&'d mut ()>,
}

impl<'d> WifiDriver<'d> {
    #[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
    pub fn new<M: WifiModemPeripheral>(
        _modem: impl Peripheral<P = M> + 'd,
        sysloop: EspSystemEventLoop,
        nvs: Option<EspDefaultNvsPartition>,
    ) -> Result<Self, EspError> {
        Self::init(nvs.is_some())?;

        let (status, subscription) = Self::subscribe(&sysloop)?;

        Ok(Self {
            status,
            _subscription: subscription,
            _nvs: nvs,
            _p: PhantomData,
        })
    }

    #[cfg(not(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled)))]
    pub fn new<M: WifiModemPeripheral>(
        _modem: impl Peripheral<P = M> + 'd,
        sysloop: EspSystemEventLoop,
    ) -> Result<Self, EspError> {
        Self::init(false)?;

        let (status, subscription) = Self::subscribe(&sysloop)?;

        Ok(Self {
            status,
            _subscription: subscription,
            _p: PhantomData,
        })
    }

    #[allow(clippy::type_complexity)]
    fn subscribe(
        sysloop: &EspEventLoop<System>,
    ) -> Result<
        (
            Arc<mutex::Mutex<(WifiEvent, WifiEvent)>>,
            EspSubscription<System>,
        ),
        EspError,
    > {
        let status = Arc::new(mutex::Mutex::wrap(
            mutex::RawMutex::new(),
            (WifiEvent::StaStopped, WifiEvent::ApStopped),
        ));
        let s_status = status.clone();

        let subscription = sysloop.subscribe(move |event: &WifiEvent| {
            let mut guard = s_status.lock();

            match event {
                WifiEvent::ApStarted => guard.1 = WifiEvent::ApStarted,
                WifiEvent::ApStopped => guard.1 = WifiEvent::ApStopped,
                WifiEvent::StaStarted => guard.0 = WifiEvent::StaStarted,
                WifiEvent::StaStopped => guard.0 = WifiEvent::StaStopped,
                WifiEvent::StaConnected => guard.0 = WifiEvent::StaConnected,
                WifiEvent::StaDisconnected => guard.0 = WifiEvent::StaDisconnected,
                _ => (),
            };
        })?;

        Ok((status, subscription))
    }

    fn init(nvs_enabled: bool) -> Result<(), EspError> {
        #[allow(clippy::needless_update)]
        let cfg = wifi_init_config_t {
            #[cfg(esp_idf_version_major = "4")]
            event_handler: Some(esp_event_send_internal),
            osi_funcs: unsafe { &mut g_wifi_osi_funcs },
            wpa_crypto_funcs: unsafe { g_wifi_default_wpa_crypto_funcs },
            static_rx_buf_num: CONFIG_ESP32_WIFI_STATIC_RX_BUFFER_NUM as _,
            dynamic_rx_buf_num: CONFIG_ESP32_WIFI_DYNAMIC_RX_BUFFER_NUM as _,
            tx_buf_type: CONFIG_ESP32_WIFI_TX_BUFFER_TYPE as _,
            static_tx_buf_num: WIFI_STATIC_TX_BUFFER_NUM as _,
            dynamic_tx_buf_num: WIFI_DYNAMIC_TX_BUFFER_NUM as _,
            cache_tx_buf_num: WIFI_CACHE_TX_BUFFER_NUM as _,
            csi_enable: WIFI_CSI_ENABLED as _,
            ampdu_rx_enable: WIFI_AMPDU_RX_ENABLED as _,
            ampdu_tx_enable: WIFI_AMPDU_TX_ENABLED as _,
            amsdu_tx_enable: WIFI_AMSDU_TX_ENABLED as _,
            nvs_enable: i32::from(nvs_enabled),
            nano_enable: WIFI_NANO_FORMAT_ENABLED as _,
            //tx_ba_win: WIFI_DEFAULT_TX_BA_WIN as _,
            rx_ba_win: WIFI_DEFAULT_RX_BA_WIN as _,
            wifi_task_core_id: WIFI_TASK_CORE_ID as _,
            beacon_max_len: WIFI_SOFTAP_BEACON_MAX_LEN as _,
            mgmt_sbuf_num: WIFI_MGMT_SBUF_NUM as _,
            feature_caps: unsafe { g_wifi_feature_caps },
            sta_disconnected_pm: WIFI_STA_DISCONNECTED_PM_ENABLED != 0,
            espnow_max_encrypt_num: CONFIG_ESP_WIFI_ESPNOW_MAX_ENCRYPT_NUM as i32,
            magic: WIFI_INIT_CONFIG_MAGIC as _,
            ..Default::default()
        };
        esp!(unsafe { esp_wifi_init(&cfg) })?;

        info!("Driver initialized");

        Ok(())
    }

    pub fn get_capabilities(&self) -> Result<EnumSet<Capability>, EspError> {
        let caps = Capability::Client | Capability::AccessPoint | Capability::Mixed;

        info!("Providing capabilities: {:?}", caps);

        Ok(caps)
    }

    pub fn start(&mut self) -> Result<(), EspError> {
        info!("Start requested");

        esp!(unsafe { esp_wifi_start() })?;

        info!("Starting");

        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), EspError> {
        info!("Stop requested");

        esp!(unsafe { esp_wifi_stop() })?;

        info!("Stopping");

        Ok(())
    }

    pub fn connect(&mut self) -> Result<(), EspError> {
        info!("Connect requested");

        esp!(unsafe { esp_wifi_connect() })?;

        info!("Connecting");

        Ok(())
    }

    pub fn disconnect(&mut self) -> Result<(), EspError> {
        info!("Disconnect requested");

        esp!(unsafe { esp_wifi_disconnect() })?;

        info!("Disconnecting");

        Ok(())
    }

    pub fn is_ap_enabled(&self) -> Result<bool, EspError> {
        let mut mode: wifi_mode_t = 0;
        esp!(unsafe { esp_wifi_get_mode(&mut mode) })?;

        Ok(mode == wifi_mode_t_WIFI_MODE_AP || mode == wifi_mode_t_WIFI_MODE_APSTA)
    }

    pub fn is_sta_enabled(&self) -> Result<bool, EspError> {
        let mut mode: wifi_mode_t = 0;
        esp!(unsafe { esp_wifi_get_mode(&mut mode) })?;

        Ok(mode == wifi_mode_t_WIFI_MODE_STA || mode == wifi_mode_t_WIFI_MODE_APSTA)
    }

    pub fn is_ap_started(&self) -> Result<bool, EspError> {
        Ok(self.status.lock().1 == WifiEvent::ApStarted)
    }

    pub fn is_sta_started(&self) -> Result<bool, EspError> {
        let guard = self.status.lock();

        Ok(guard.0 == WifiEvent::StaStarted
            || guard.0 == WifiEvent::StaConnected
            || guard.0 == WifiEvent::StaDisconnected)
    }

    pub fn is_sta_connected(&self) -> Result<bool, EspError> {
        Ok(self.status.lock().0 == WifiEvent::StaConnected)
    }

    pub fn is_started(&self) -> Result<bool, EspError> {
        let ap_enabled = self.is_ap_enabled()?;
        let sta_enabled = self.is_sta_enabled()?;

        if !ap_enabled && !sta_enabled {
            Ok(false)
        } else {
            Ok(
                (!ap_enabled || self.is_ap_started()?)
                    && (!sta_enabled || self.is_sta_started()?),
            )
        }
    }

    pub fn is_connected(&self) -> Result<bool, EspError> {
        let ap_enabled = self.is_ap_enabled()?;
        let sta_enabled = self.is_sta_enabled()?;

        if !ap_enabled && !sta_enabled {
            Ok(false)
        } else {
            let guard = self.status.lock();

            Ok((!ap_enabled || guard.1 == WifiEvent::ApStarted)
                && (!sta_enabled || guard.0 == WifiEvent::StaConnected))
        }
    }

    #[allow(non_upper_case_globals)]
    pub fn get_configuration(&self) -> Result<Configuration, EspError> {
        info!("Getting configuration");

        let mut mode: wifi_mode_t = 0;
        esp!(unsafe { esp_wifi_get_mode(&mut mode) })?;

        let conf = match mode {
            wifi_mode_t_WIFI_MODE_NULL => Configuration::None,
            wifi_mode_t_WIFI_MODE_AP => Configuration::AccessPoint(self.get_ap_conf()?),
            wifi_mode_t_WIFI_MODE_STA => Configuration::Client(self.get_sta_conf()?),
            wifi_mode_t_WIFI_MODE_APSTA => {
                Configuration::Mixed(self.get_sta_conf()?, self.get_ap_conf()?)
            }
            _ => panic!(),
        };

        info!("Configuration gotten: {:?}", &conf);

        Ok(conf)
    }

    pub fn set_configuration(&mut self, conf: &Configuration) -> Result<(), EspError> {
        info!("Setting configuration: {:?}", conf);

        match conf {
            Configuration::None => {
                unsafe {
                    esp!(esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_NULL))?;
                }
                info!("Wifi mode NULL set");
            }
            Configuration::AccessPoint(ap_conf) => {
                unsafe {
                    esp!(esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_AP))?;
                }
                info!("Wifi mode AP set");

                self.set_ap_conf(ap_conf)?;
            }
            Configuration::Client(client_conf) => {
                unsafe {
                    esp!(esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_STA))?;
                }
                info!("Wifi mode STA set");

                self.set_sta_conf(client_conf)?;
            }
            Configuration::Mixed(client_conf, ap_conf) => {
                unsafe {
                    esp!(esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_APSTA))?;
                }
                info!("Wifi mode APSTA set");

                self.set_sta_conf(client_conf)?;
                self.set_ap_conf(ap_conf)?;
            }
        }

        info!("Configuration set");

        Ok(())
    }

    /// Scan for nearby, visible access points.
    ///
    /// It scans for all available access points nearby, but returns only the first `N` access points found.
    /// In addition, it returns the actual amount it found. The function blocks until the scan is done.
    ///
    /// Before calling this function the Wifi driver must be configured and started in either Client or Mixed mode.
    ///
    /// # Example
    /// ```ignore
    /// let mut wifi_driver = WifiDriver::new(peripherals.modem, sysloop.clone());
    /// wifi_driver.set_configuration(
    ///     &Configuration::Client(ClientConfiguration::default())
    /// )
    /// .unwrap();
    /// wifi_driver.start().unwrap();
    ///
    /// let (scan_result, found_aps) = wifi_driver.scan_n::<10>().unwrap();
    /// ```
    // For backwards compatibility
    pub fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        self.start_scan(&Default::default(), true)?;
        self.get_scan_result_n()
    }

    /// Scan for nearby, visible access points.
    ///
    /// Unlike [`WifiDriver::scan_n()`], it returns all found access points by allocating memory
    /// dynamically.
    ///
    /// For more details see [`WifiDriver::scan_n()`].
    // For backwards compatibility
    #[cfg(feature = "alloc")]
    pub fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        self.start_scan(&Default::default(), true)?;
        self.get_scan_result()
    }

    /// Start scanning for nearby, visible access points.
    ///
    /// Unlike [`WifiDriver::scan_n()`] or [`WifiDriver::scan()`] it can be called as either blocking or not blocking.
    /// A [`ScanConfig`] can be provided as well. To get the scan result call either [`WifiDriver::get_scan_result_n()`] or
    /// [`WifiDriver::get_scan_result()`].
    ///
    /// This function can be used in `async` context, when the current thread shouldn't be blocked.
    ///
    /// # Example
    ///
    /// This example shows how to use it in a `async` context.
    ///
    /// ```ignore
    /// let mut wifi_driver = WifiDriver::new(peripherals.modem, sysloop.clone());
    /// wifi_driver.set_configuration(
    ///     &Configuration::Client(ClientConfiguration::default())
    /// )
    /// .unwrap();
    /// wifi_driver.start().unwrap();
    ///
    /// let scan_finish_signal = Arc::new(channel_bridge::notification::Notification::new());
    /// let _sub = {
    ///     let scan_finish_signal = scan_finish_signal.clone();
    ///     sysloop.subscribe::<WifiEvent>(move |event| {
    ///         if *event == WifiEvent::ScanDone {
    ///             scan_finish_signal.notify();
    ///         }
    ///     }).unwrap()
    /// };
    ///
    /// wifi_driver.start_scan(&ScanConfig::default(), false).unwrap();
    ///
    /// scan_finish_signal.wait().await;
    ///
    /// let res = wifi_driver.get_scan_result().unwrap();
    /// ```
    pub fn start_scan(
        &mut self,
        scan_config: &config::ScanConfig,
        blocking: bool,
    ) -> Result<(), EspError> {
        info!("About to scan for access points");

        let scan_config: wifi_scan_config_t = scan_config.into();
        esp!(unsafe { esp_wifi_scan_start(&scan_config as *const wifi_scan_config_t, blocking) })
    }

    /// Stops a previous started access point scan.
    pub fn stop_scan(&mut self) -> Result<(), EspError> {
        info!("About to stop scan for access points");

        esp!(unsafe { esp_wifi_scan_stop() })
    }

    /// Get the results of an access point scan.
    ///
    /// This call returns a list of the first `N` found access points. A scan can be started with [`WifiDriver::start_scan()`].
    /// As [`WifiDriver::scan_n()`] it returns the actual amount of found access points as well.
    pub fn get_scan_result_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        let scanned_count = self.get_scan_count()?;

        let mut ap_infos_raw: heapless::Vec<wifi_ap_record_t, N> = heapless::Vec::new();
        unsafe {
            ap_infos_raw.set_len(scanned_count.min(N));
        }

        let fetched_count = self.fetch_scan_result(&mut ap_infos_raw)?;

        let result = ap_infos_raw[..fetched_count]
            .iter()
            .map::<AccessPointInfo, _>(|ap_info_raw| Newtype(ap_info_raw).into())
            .inspect(|ap_info| info!("Found access point {:?}", ap_info))
            .collect();

        Ok((result, scanned_count))
    }

    /// Get the results of an access point scan.
    ///
    /// Unlike [`WifiDriver::get_scan_result_n()`], it returns all found access points by allocating memory
    /// dynamically.
    ///
    /// For more details see [`WifiDriver::get_scan_result_n()`].
    #[cfg(feature = "alloc")]
    pub fn get_scan_result(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        let scanned_count = self.get_scan_count()?;

        let mut ap_infos_raw: alloc::vec::Vec<wifi_ap_record_t> =
            alloc::vec::Vec::with_capacity(scanned_count);
        #[allow(clippy::uninit_vec)]
        // ... because we are filling it in on the next line and only reading the initialized members
        unsafe {
            ap_infos_raw.set_len(scanned_count)
        };

        let fetched_count = self.fetch_scan_result(&mut ap_infos_raw)?;

        let result = ap_infos_raw[..fetched_count]
            .iter()
            .map::<AccessPointInfo, _>(|ap_info_raw| Newtype(ap_info_raw).into())
            .inspect(|ap_info| info!("Found access point {:?}", ap_info))
            .collect();

        Ok(result)
    }

    pub fn set_callbacks<R, T>(
        &mut self,
        mut rx_callback: R,
        mut tx_callback: T,
    ) -> Result<(), EspError>
    where
        R: FnMut(WifiDeviceId, &[u8]) -> Result<(), EspError> + Send + 'static,
        T: FnMut(WifiDeviceId, &[u8], bool) + Send + 'static,
    {
        let _ = self.disconnect();
        let _ = self.stop();

        #[allow(clippy::type_complexity)]
        let rx_callback: Box<
            Box<dyn FnMut(WifiDeviceId, &[u8]) -> Result<(), EspError> + 'static>,
        > = Box::new(Box::new(move |device_id, data| {
            rx_callback(device_id, data)
        }));

        #[allow(clippy::type_complexity)]
        let tx_callback: Box<Box<dyn FnMut(WifiDeviceId, &[u8], bool) + 'static>> =
            Box::new(Box::new(move |device_id, data, status| {
                tx_callback(device_id, data, status)
            }));

        unsafe {
            RX_CALLBACK = Some(rx_callback);
            TX_CALLBACK = Some(tx_callback);

            esp!(esp_wifi_internal_reg_rxcb(
                WifiDeviceId::Ap.into(),
                Some(Self::handle_rx_ap),
            ))?;

            esp!(esp_wifi_internal_reg_rxcb(
                WifiDeviceId::Sta.into(),
                Some(Self::handle_rx_sta),
            ))?;

            esp!(esp_wifi_set_tx_done_cb(Some(Self::handle_tx)))?;
        }

        Ok(())
    }

    pub fn send(&mut self, device_id: WifiDeviceId, frame: &[u8]) -> Result<(), EspError> {
        esp!(unsafe {
            esp_wifi_internal_tx(device_id.into(), frame.as_ptr() as *mut _, frame.len() as _)
        })
    }

    fn get_sta_conf(&self) -> Result<ClientConfiguration, EspError> {
        let mut wifi_config: wifi_config_t = Default::default();
        esp!(unsafe { esp_wifi_get_config(wifi_interface_t_WIFI_IF_STA, &mut wifi_config) })?;

        let result: ClientConfiguration = unsafe { Newtype(wifi_config.sta).into() };

        info!("Providing STA configuration: {:?}", &result);

        Ok(result)
    }

    fn set_sta_conf(&mut self, conf: &ClientConfiguration) -> Result<(), EspError> {
        info!("Checking current STA configuration");
        let current_config = self.get_sta_conf()?;

        if current_config != *conf {
            info!("Setting STA configuration: {:?}", conf);

            let mut wifi_config = wifi_config_t {
                sta: Newtype::<wifi_sta_config_t>::from(conf).0,
            };

            esp!(unsafe { esp_wifi_set_config(wifi_interface_t_WIFI_IF_STA, &mut wifi_config) })?;
        } else {
            info!("Same STA configuration already present");
        }

        info!("STA configuration done");

        Ok(())
    }

    fn get_ap_conf(&self) -> Result<AccessPointConfiguration, EspError> {
        let mut wifi_config: wifi_config_t = Default::default();
        esp!(unsafe { esp_wifi_get_config(wifi_interface_t_WIFI_IF_AP, &mut wifi_config) })?;

        let result: AccessPointConfiguration = unsafe { Newtype(wifi_config.ap).into() };

        info!("Providing AP configuration: {:?}", &result);

        Ok(result)
    }

    fn set_ap_conf(&mut self, conf: &AccessPointConfiguration) -> Result<(), EspError> {
        info!("Checking current AP configuration");
        let current_config = self.get_ap_conf()?;

        if current_config != *conf {
            info!("Setting AP configuration: {:?}", conf);

            let mut wifi_config = wifi_config_t {
                ap: Newtype::<wifi_ap_config_t>::from(conf).0,
            };

            esp!(unsafe { esp_wifi_set_config(wifi_interface_t_WIFI_IF_AP, &mut wifi_config) })?;
        } else {
            info!("Same AP configuration already present");
        }

        info!("AP configuration done");

        Ok(())
    }

    fn clear_all(&mut self) -> Result<(), EspError> {
        let _ = self.disconnect();
        let _ = self.stop();

        unsafe {
            esp!(esp_wifi_deinit())?;
        }

        unsafe {
            // Callbacks are already deregistered by `esp_wifi_deinit`, just null-ify our own refs
            RX_CALLBACK = None;
            TX_CALLBACK = None;
        }

        info!("Driver deinitialized");

        Ok(())
    }

    fn get_scan_count(&mut self) -> Result<usize, EspError> {
        let mut found_ap: u16 = 0;
        esp!(unsafe { esp_wifi_scan_get_ap_num(&mut found_ap as *mut _) })?;

        info!("Found {} access points", found_ap);

        Ok(found_ap as usize)
    }

    fn fetch_scan_result(
        &mut self,
        ap_infos_raw: &mut [wifi_ap_record_t],
    ) -> Result<usize, EspError> {
        info!("About to get info for found access points");

        let mut ap_count: u16 = ap_infos_raw.len() as u16;

        esp!(unsafe { esp_wifi_scan_get_ap_records(&mut ap_count, ap_infos_raw.as_mut_ptr(),) })?;

        info!("Got info for {} access points", ap_count);

        Ok(ap_count as usize)
    }

    unsafe extern "C" fn handle_rx_ap(
        buf: *mut ffi::c_void,
        len: u16,
        eb: *mut ffi::c_void,
    ) -> esp_err_t {
        Self::handle_rx(WifiDeviceId::Ap, buf, len, eb)
    }

    unsafe extern "C" fn handle_rx_sta(
        buf: *mut ffi::c_void,
        len: u16,
        eb: *mut ffi::c_void,
    ) -> esp_err_t {
        Self::handle_rx(WifiDeviceId::Sta, buf, len, eb)
    }

    unsafe fn handle_rx(
        device_id: WifiDeviceId,
        buf: *mut ffi::c_void,
        len: u16,
        eb: *mut ffi::c_void,
    ) -> esp_err_t {
        let res = RX_CALLBACK.as_mut().unwrap()(
            device_id,
            core::slice::from_raw_parts(buf as *mut _, len as usize),
        );

        esp_wifi_internal_free_rx_buffer(eb);

        match res {
            Ok(_) => ESP_OK,
            Err(e) => e.code(),
        }
    }

    unsafe extern "C" fn handle_tx(ifidx: u8, data: *mut u8, len: *mut u16, tx_status: bool) {
        TX_CALLBACK.as_mut().unwrap()(
            (ifidx as wifi_interface_t).into(),
            core::slice::from_raw_parts(data as *const _, len as usize),
            tx_status,
        );
    }
}

unsafe impl<'d> Send for WifiDriver<'d> {}

impl<'d> Drop for WifiDriver<'d> {
    fn drop(&mut self) {
        self.clear_all().unwrap();

        info!("Dropped");
    }
}

impl<'d> Wifi for WifiDriver<'d> {
    type Error = EspError;

    fn get_capabilities(&self) -> Result<EnumSet<Capability>, Self::Error> {
        WifiDriver::get_capabilities(self)
    }

    fn is_started(&self) -> Result<bool, Self::Error> {
        WifiDriver::is_started(self)
    }

    fn is_connected(&self) -> Result<bool, Self::Error> {
        WifiDriver::is_connected(self)
    }

    fn get_configuration(&self) -> Result<Configuration, Self::Error> {
        WifiDriver::get_configuration(self)
    }

    fn set_configuration(&mut self, conf: &Configuration) -> Result<(), Self::Error> {
        WifiDriver::set_configuration(self, conf)
    }

    fn start(&mut self) -> Result<(), Self::Error> {
        WifiDriver::start(self)
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        WifiDriver::stop(self)
    }

    fn connect(&mut self) -> Result<(), Self::Error> {
        WifiDriver::connect(self)
    }

    fn disconnect(&mut self) -> Result<(), Self::Error> {
        WifiDriver::disconnect(self)
    }

    fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), Self::Error> {
        WifiDriver::scan_n(self)
    }

    #[cfg(feature = "alloc")]
    fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, Self::Error> {
        WifiDriver::scan(self)
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
pub struct EspWifi<'d> {
    ap_netif: EspNetif,
    sta_netif: EspNetif,
    driver: WifiDriver<'d>,
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<'d> EspWifi<'d> {
    #[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
    pub fn new<M: WifiModemPeripheral>(
        modem: impl Peripheral<P = M> + 'd,
        sysloop: EspSystemEventLoop,
        nvs: Option<EspDefaultNvsPartition>,
    ) -> Result<Self, EspError> {
        Self::wrap(WifiDriver::new(modem, sysloop, nvs)?)
    }

    #[cfg(not(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled)))]
    pub fn new<M: WifiModemPeripheral>(
        modem: impl Peripheral<P = M> + 'd,
        sysloop: EspSystemEventLoop,
    ) -> Result<Self, EspError> {
        Self::wrap(WifiDriver::new(modem, sysloop)?)
    }

    pub fn wrap(driver: WifiDriver<'d>) -> Result<Self, EspError> {
        Self::wrap_all(
            driver,
            EspNetif::new(NetifStack::Sta)?,
            EspNetif::new(NetifStack::Ap)?,
        )
    }

    pub fn wrap_all(
        driver: WifiDriver<'d>,
        sta_netif: EspNetif,
        ap_netif: EspNetif,
    ) -> Result<Self, EspError> {
        let mut this = Self {
            driver,
            sta_netif,
            ap_netif,
        };

        this.attach_netif()?;

        Ok(this)
    }

    pub fn swap_netif(
        &mut self,
        sta_netif: EspNetif,
        ap_netif: EspNetif,
    ) -> Result<(EspNetif, EspNetif), EspError> {
        self.detach_netif()?;

        let old_sta = core::mem::replace(&mut self.sta_netif, sta_netif);
        let old_ap = core::mem::replace(&mut self.ap_netif, ap_netif);

        self.attach_netif()?;

        Ok((old_sta, old_ap))
    }

    pub fn driver(&self) -> &WifiDriver<'d> {
        &self.driver
    }

    pub fn driver_mut(&mut self) -> &mut WifiDriver<'d> {
        &mut self.driver
    }

    pub fn sta_netif(&self) -> &EspNetif {
        &self.sta_netif
    }

    pub fn sta_netif_mut(&mut self) -> &mut EspNetif {
        &mut self.sta_netif
    }

    pub fn ap_netif(&self) -> &EspNetif {
        &self.ap_netif
    }

    pub fn ap_netif_mut(&mut self) -> &mut EspNetif {
        &mut self.ap_netif
    }

    pub fn get_capabilities(&self) -> Result<EnumSet<Capability>, EspError> {
        self.driver().get_capabilities()
    }

    pub fn is_started(&self) -> Result<bool, EspError> {
        self.driver().is_started()
    }

    pub fn is_up(&self) -> Result<bool, EspError> {
        if !self.driver().is_connected()? {
            Ok(false)
        } else {
            let ap_enabled = self.driver().is_ap_enabled()?;
            let sta_enabled = self.driver().is_sta_enabled()?;

            Ok((!ap_enabled || self.ap_netif().is_up()?)
                && (!sta_enabled || self.sta_netif().is_up()?))
        }
    }

    pub fn get_configuration(&self) -> Result<Configuration, EspError> {
        self.driver().get_configuration()
    }

    pub fn set_configuration(&mut self, conf: &Configuration) -> Result<(), EspError> {
        self.driver_mut().set_configuration(conf)
    }

    pub fn start(&mut self) -> Result<(), EspError> {
        self.driver_mut().start()
    }

    pub fn stop(&mut self) -> Result<(), EspError> {
        self.driver_mut().stop()
    }

    pub fn connect(&mut self) -> Result<(), EspError> {
        self.driver_mut().connect()
    }

    pub fn disconnect(&mut self) -> Result<(), EspError> {
        self.driver_mut().disconnect()
    }

    /// Scan for nearby, visible access points.
    ///
    /// For more details see [`WifiDriver::scan_n()`].
    pub fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        self.driver_mut().scan_n()
    }

    /// Scan for nearby, visible access points.
    ///
    /// For more details see [`WifiDriver::scan()`].
    #[cfg(feature = "alloc")]
    pub fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        self.driver_mut().scan()
    }

    /// Start scanning for nearby, visible access points.
    ///
    /// For more details see [`WifiDriver::start_scan()`].
    pub fn start_scan(
        &mut self,
        scan_config: &config::ScanConfig,
        blocking: bool,
    ) -> Result<(), EspError> {
        self.driver_mut().start_scan(scan_config, blocking)
    }

    /// Stops a previous started access point scan.
    pub fn stop_scan(&mut self) -> Result<(), EspError> {
        self.driver_mut().stop_scan()
    }

    /// Get the results of an access point scan.
    ///
    /// For more details see [`WifiDriver::get_scan_result_n()`].
    pub fn get_scan_result_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        self.driver_mut().get_scan_result_n()
    }

    /// Get the results of an access point scan.
    ///
    /// For more details see [`WifiDriver::get_scan_result()`].
    #[cfg(feature = "alloc")]
    pub fn get_scan_result(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        self.driver_mut().get_scan_result()
    }

    fn attach_netif(&mut self) -> Result<(), EspError> {
        let _ = self.driver.stop();

        esp!(unsafe { esp_netif_attach_wifi_ap(self.ap_netif.handle()) })?;
        esp!(unsafe { esp_wifi_set_default_wifi_ap_handlers() })?;

        esp!(unsafe { esp_netif_attach_wifi_station(self.sta_netif.handle()) })?;
        esp!(unsafe { esp_wifi_set_default_wifi_sta_handlers() })?;

        Ok(())
    }

    fn detach_netif(&mut self) -> Result<(), EspError> {
        let _ = self.driver.stop();

        esp!(unsafe {
            esp_wifi_clear_default_wifi_driver_and_handlers(
                self.ap_netif.handle() as *mut ffi::c_void
            )
        })?;

        esp!(unsafe {
            esp_wifi_clear_default_wifi_driver_and_handlers(
                self.sta_netif.handle() as *mut ffi::c_void
            )
        })?;

        Ok(())
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<'d> Drop for EspWifi<'d> {
    fn drop(&mut self) {
        self.detach_netif().unwrap();
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
unsafe impl<'d> Send for EspWifi<'d> {}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<'d> Wifi for EspWifi<'d> {
    type Error = EspError;

    fn get_capabilities(&self) -> Result<EnumSet<Capability>, Self::Error> {
        EspWifi::get_capabilities(self)
    }

    fn is_started(&self) -> Result<bool, Self::Error> {
        EspWifi::is_started(self)
    }

    fn is_connected(&self) -> Result<bool, Self::Error> {
        EspWifi::is_up(self)
    }

    fn get_configuration(&self) -> Result<Configuration, Self::Error> {
        EspWifi::get_configuration(self)
    }

    fn set_configuration(&mut self, conf: &Configuration) -> Result<(), Self::Error> {
        EspWifi::set_configuration(self, conf)
    }

    fn start(&mut self) -> Result<(), Self::Error> {
        EspWifi::start(self)
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        EspWifi::stop(self)
    }

    fn connect(&mut self) -> Result<(), Self::Error> {
        EspWifi::connect(self)
    }

    fn disconnect(&mut self) -> Result<(), Self::Error> {
        EspWifi::disconnect(self)
    }

    fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), Self::Error> {
        EspWifi::scan_n(self)
    }

    fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, Self::Error> {
        EspWifi::scan(self)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WifiEvent {
    Ready,

    ScanStarted,
    ScanDone,

    StaStarted,
    StaStopped,
    StaConnected,
    StaDisconnected,
    StaAuthmodeChanged,
    StaBssRssiLow,
    StaBeaconTimeout,
    StaWpsSuccess,
    StaWpsFailed,
    StaWpsTimeout,
    StaWpsPin,
    StaWpsPbcOverlap,

    ApStarted,
    ApStopped,
    ApStaConnected,
    ApStaDisconnected,
    ApProbeRequestReceived,

    FtmReport,
    ActionTxStatus,
    RocDone,
}

impl EspTypedEventSource for WifiEvent {
    fn source() -> *const ffi::c_char {
        unsafe { WIFI_EVENT }
    }
}

impl EspTypedEventDeserializer<WifiEvent> for WifiEvent {
    #[allow(non_upper_case_globals, non_snake_case)]
    fn deserialize<R>(
        data: &crate::eventloop::EspEventFetchData,
        f: &mut impl for<'a> FnMut(&'a WifiEvent) -> R,
    ) -> R {
        let event_id = data.event_id as u32;

        let event = if event_id == wifi_event_t_WIFI_EVENT_WIFI_READY {
            WifiEvent::Ready
        } else if event_id == wifi_event_t_WIFI_EVENT_SCAN_DONE {
            WifiEvent::ScanDone
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_START {
            WifiEvent::StaStarted
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_STOP {
            WifiEvent::StaStopped
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_CONNECTED {
            WifiEvent::StaConnected
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_DISCONNECTED {
            WifiEvent::StaDisconnected
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_AUTHMODE_CHANGE {
            WifiEvent::StaAuthmodeChanged
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_WPS_ER_SUCCESS {
            WifiEvent::StaWpsSuccess
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_WPS_ER_FAILED {
            WifiEvent::StaWpsFailed
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_WPS_ER_TIMEOUT {
            WifiEvent::StaWpsTimeout
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_WPS_ER_PIN {
            WifiEvent::StaWpsPin
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_WPS_ER_PBC_OVERLAP {
            WifiEvent::StaWpsPbcOverlap
        } else if event_id == wifi_event_t_WIFI_EVENT_AP_START {
            WifiEvent::ApStarted
        } else if event_id == wifi_event_t_WIFI_EVENT_AP_STOP {
            WifiEvent::ApStopped
        } else if event_id == wifi_event_t_WIFI_EVENT_AP_STACONNECTED {
            WifiEvent::ApStaConnected
        } else if event_id == wifi_event_t_WIFI_EVENT_AP_STADISCONNECTED {
            WifiEvent::ApStaDisconnected
        } else if event_id == wifi_event_t_WIFI_EVENT_AP_PROBEREQRECVED {
            WifiEvent::ApProbeRequestReceived
        } else if event_id == wifi_event_t_WIFI_EVENT_FTM_REPORT {
            WifiEvent::FtmReport
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_BSS_RSSI_LOW {
            WifiEvent::StaBssRssiLow
        } else if event_id == wifi_event_t_WIFI_EVENT_ACTION_TX_STATUS {
            WifiEvent::ActionTxStatus
        } else if event_id == wifi_event_t_WIFI_EVENT_STA_BEACON_TIMEOUT {
            WifiEvent::StaBeaconTimeout
        } else if event_id == wifi_event_t_WIFI_EVENT_ROC_DONE {
            WifiEvent::RocDone
        } else {
            panic!("Unknown event ID: {}", event_id);
        };

        f(&event)
    }
}

pub struct WifiWait {
    _subscription: EspSubscription<System>,
    waitable: Arc<Waitable<()>>,
}

impl WifiWait {
    pub fn new(sysloop: &EspEventLoop<System>) -> Result<Self, EspError> {
        let waitable: Arc<Waitable<()>> = Arc::new(Waitable::new(()));

        let s_waitable = waitable.clone();
        let subscription =
            sysloop.subscribe(move |event: &WifiEvent| Self::on_wifi_event(&s_waitable, event))?;

        Ok(Self {
            waitable,
            _subscription: subscription,
        })
    }

    pub fn wait(&self, matcher: impl Fn() -> bool) {
        info!("About to wait");

        self.waitable.wait_while(|_| !matcher());

        info!("Waiting done - success");
    }

    pub fn wait_with_timeout(&self, dur: Duration, matcher: impl Fn() -> bool) -> bool {
        info!("About to wait for duration {:?}", dur);

        let (timeout, _) = self
            .waitable
            .wait_timeout_while_and_get(dur, |_| !matcher(), |_| ());

        if !timeout {
            info!("Waiting done - success");
            true
        } else {
            info!("Timeout while waiting");
            false
        }
    }

    fn on_wifi_event(waitable: &Waitable<()>, event: &WifiEvent) {
        info!("Got wifi event: {:?}", event);

        if matches!(
            event,
            WifiEvent::ApStarted
                | WifiEvent::ApStopped
                | WifiEvent::StaStarted
                | WifiEvent::StaStopped
                | WifiEvent::StaConnected
                | WifiEvent::StaDisconnected
        ) {
            waitable.cvar.notify_all();
        }
    }
}
