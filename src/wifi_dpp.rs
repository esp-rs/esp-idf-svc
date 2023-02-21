//! Wi-Fi Easy Connect (DPP) support
//!
//! # Example
//!
//! Note that to use this feature, you must add CONFIG_WPA_DPP_SUPPORT=y to your sdkconfig
//!
//! ```no_run
//! use esp_idf_hal::peripherals::Peripherals;
//! use esp_idf_svc::eventloop::EspSystemEventLoop;
//! use esp_idf_svc::nvs::EspDefaultNvsPartition;
//! use esp_idf_svc::wifi::EspWifi;
//! use esp_idf_svc::wifi_dpp::EspWifiDpp;
//!
//! let peripherals = Peripherals::take().unwrap();
//! let sysloop = EspSystemEventLoop::take()?;
//! let nvs = EspDefaultNvsPartition::take()?;
//! let mut wifi = EspWifi::new(peripherals.modem, sysloop, Some(nvs))?;
//!
//! let channels = [6];
//! // Test key, please use secure keys for your project (or None to generate one on the fly)!
//! let privkey = Some([
//!     0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
//!     0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
//! ]);
//! let associated_data = None;
//! loop {
//!     let dpp = EspWifiDpp::generate_qrcode(
//!         &mut wifi,
//!         &channels,
//!         privkey.as_ref(),
//!         associated_data)?;
//!     log::info!("Got QR code text: {}", dpp.get_bootstrapped_data().0);
//!
//!     match dpp.start_listen()?.wait_for_credentials() {
//!         Ok(c) => break,
//!         Err(e) => {
//!             // Will generate the same QR code again since the inputs have not changed...
//!             log::error!("DPP error: {e}, bootstrapping again...");
//!         }
//!     }
//! }
//! ```

use core::fmt::Write;
use core::marker::PhantomData;
use core::mem;
use core::ops::{Deref, DerefMut};
use core::ptr;
use core::time::Duration;
use alloc::sync::Arc;
use alloc::sync::Weak;

use ::log::*;
use embedded_svc::wifi::{ClientConfiguration, Configuration};
use esp_idf_sys::*;
use esp_idf_sys::EspError;

use crate::private::common::Newtype;
use crate::private::cstr::*;
use crate::private::mutex::{Mutex, RawMutex};
use crate::private::waitable::Waitable;
use crate::wifi::EspWifi;

/// Global singleton that proves we can't have imbalanced access to esp_supp_dpp_init/deinit.
/// This is statically enforced through requiring a mutable borrow of EspWifi in the API.
struct DppInitialized {
    /// Holds the most recently received callback event that is yet to be processed/handled.
    pending: Waitable<Option<DppState>>,
}

/// Global weak reference so that we can respond to the stateless C callbacks provided in
/// esp_supp_dpp_init.
static DPP_INITIALIZED: Mutex<Option<Weak<DppInitialized>>> = Mutex::wrap(RawMutex::new(), None);

impl DppInitialized {
    fn new(wifi: &mut EspWifi) -> Result<Self, EspError> {
        let _ = wifi.disconnect();
        let _ = wifi.stop();

        info!("Initializing DPP...");
        esp!(unsafe { esp_supp_dpp_init(Some(Self::dpp_event_cb_unsafe)) })?;

        Ok(Self {
            pending: Waitable::new(None),
        })
    }

    fn store_weak_global(self: &Arc<Self>) -> Result<(), EspError> {
        let weak_self = Arc::downgrade(self);
        match mem::replace(DPP_INITIALIZED.lock().deref_mut(), Some(weak_self)) {
            Some(existing) if existing.upgrade().is_some() => {
                warn!("DPP already initialized, please file a bug!");
                Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())
            }
            _ => Ok(()),
        }
    }

    fn clear_weak_global() {
        mem::take(DPP_INITIALIZED.lock().deref_mut());
    }

    fn upgrade_weak_global() -> Option<Arc<DppInitialized>> {
        match DPP_INITIALIZED.lock().deref() {
            None => None,
            Some(dpp_weak) => dpp_weak.upgrade(),
        }
    }

    #[allow(non_upper_case_globals)]
    unsafe extern "C" fn dpp_event_cb_unsafe(
        evt: esp_supp_dpp_event_t,
        data: *mut ::core::ffi::c_void,
    ) {
        let event = match evt {
            esp_supp_dpp_event_t_ESP_SUPP_DPP_URI_READY => {
                match ptr::NonNull::new(data as *mut c_char) {
                    None => {
                        warn!("Unknown input error from esp_dpp: null uri provided!");
                        Some(DppState::Fail(EspError::from_infallible::<ESP_ERR_INVALID_ARG>()))
                    },
                    Some(ptr) => {
                        let rust_str = from_cstr_ptr(ptr.as_ptr()).into();
                        Some(DppState::BootstrappedUriReady(rust_str))
                    }
                }
            },
            esp_supp_dpp_event_t_ESP_SUPP_DPP_CFG_RECVD => {
                let config = data as *mut wifi_config_t;
                // TODO: We're losing pmf_cfg.required=true setting due to missing
                // information in ClientConfiguration.
                Some(DppState::ConfigurationReceived(Newtype((*config).sta).into()))
            }
            esp_supp_dpp_event_t_ESP_SUPP_DPP_FAIL => {
                Some(DppState::Fail(EspError::from(data as esp_err_t).unwrap()))
            }
            _ => {
                warn!("Unsupported DPP event: {evt}, ignoring...");
                None
            }
        };
        if let Some(event) = event {
            if let Err(event) = Self::maybe_set_pending_state(event) {
                warn!("Spurious DPP event after deinit: {event:?}");
            }
        }
    }

    fn maybe_set_pending_state(state: DppState) -> Result<(), DppState> {
        if let Some(dpp) = Self::upgrade_weak_global() {
            dpp.set_pending_state(state);
            Ok(())
        } else {
            Err(state)
        }
    }

    fn wait_for_next_state(&self) -> DppState {
        self.pending
            .wait_while_and_get_mut(
                |state| state.is_none(),
                |state| state.take().unwrap())
    }

    fn wait_for_next_state_with_timeout(&self, timeout: Duration) -> Option<DppState> {
        let (timeout, state_opt) = self.pending.wait_timeout_while_and_get_mut(
            timeout,
            |state| state.is_none(),
            |state| state.take(),
        );
        if timeout {
            None
        } else {
            Some(state_opt.unwrap())
        }
    }

    fn set_pending_state(&self, state: DppState) {
        self.pending.get_mut(|s| {
            *s = Some(state);
        });
        self.pending.cvar.notify_all();
    }
}

impl Drop for DppInitialized {
    fn drop(&mut self) {
        info!("Deinitializing DPP...");
        unsafe { esp_supp_dpp_deinit() };

        DppInitialized::clear_weak_global();
    }
}

pub struct EspWifiDpp<'d, 'w, T> {
    /// Store the only strong reference to the initialized state in a struct that is guaranteed
    /// to borrow EspWifi for its lifetime.  This provides the compile-time guarantee that
    /// we cannot initialize DPP twice.
    dpp: Arc<DppInitialized>,
    _phantom: PhantomData<&'d PhantomData<&'w ()>>,

    bootstrapped_data: T,
}

impl<'d, 'w> EspWifiDpp<'d, 'w, QrCode> {
    /// Generate a QR code that can be scanned by a mobile phone or other configurator
    /// to securely provide Wi-Fi credentials.  On success, the caller must invoke
    /// [::start_listen] to actually start listening.  To wait for the credentials to
    /// become available, see [DppWait].
    ///
    /// Note that [EspWifi] is mutably borrowed for the lifecycle of this object to ensure
    /// that conflicting usage of the WiFi driver does not occur concurrently with DPP.  It is
    /// not known the effect this would have and in general it is assumed to be unsafe.
    ///
    /// * `wifi` - Mutable borrow for the lifetime of DPP to prevent concurrent usage of the Wi-Fi
    /// driver.
    /// * `channels` - List of channels to listen for DPP auth messages.
    /// * `key` - (Optional) NIST P-256 private key to use when generating the QR code.  This can
    /// be useful for example so that the QR code can be printed and distributed with the device.
    /// If omitted, a unique private key is generated on each invocation.  Do not include PEM
    /// or DER formatting data as it will be added automatically depending on which version of
    /// ESP-IDF is being used.
    /// * `associated_data` - (Optional) Arbitrary extra information to include with the QR
    /// code that may be relevant to the configurator.
    pub fn generate_qrcode(
        wifi: &'d mut EspWifi<'w>,
        channels: &[u8],
        key: Option<&[u8; 32]>,
        associated_data: Option<&[u8]>,
    ) -> Result<Self, EspError> {
        let dpp = Arc::new(DppInitialized::new(wifi)?);
        dpp.store_weak_global()?;

        Self::do_bootstrap_gen(channels, key, associated_data)?;
        match dpp.wait_for_next_state() {
            DppState::BootstrappedUriReady(qrcode) => {
                wifi.set_configuration(&Configuration::Client(Default::default()))?;
                wifi.start()?;

                Ok(Self {
                    dpp,
                    bootstrapped_data: QrCode(qrcode),
                    _phantom: PhantomData,
                })
            },
            DppState::Fail(e) => Err(e),
            other => Err(unexpected_state(other)),
        }
    }

    fn do_bootstrap_gen(
        channels: &[u8],
        key: Option<&[u8; 32]>,
        associated_data: Option<&[u8]>,
    ) -> Result<(), EspError> {
        let mut channels_str = channels.into_iter().fold(String::new(), |mut a, c| {
            write!(a, "{c},").unwrap();
            a
        });
        channels_str.pop();
        let channels_cstr = CString::new(channels_str).unwrap();

        let key_ascii_cstr = key.map(|k| {
            let result = frame_key(k).iter().fold(String::new(), |mut a, b| {
                write!(a, "{b:02x}").unwrap();
                a
            });
            CString::new(result).unwrap()
        });

        let associated_data_cstr = match associated_data {
            Some(associated_data) => Some(CString::new(associated_data).map_err(|_| {
                warn!("associated data contains an embedded NUL character!");
                EspError::from_infallible::<ESP_ERR_INVALID_ARG>()
            })?),
            None => None,
        };

        info!("Bootstrapping DPP with: channels={channels_cstr:?}, key={key_ascii_cstr:?}");
        esp!(unsafe {
            esp_supp_dpp_bootstrap_gen(
                channels_cstr.as_ptr(),
                dpp_bootstrap_type_DPP_BOOTSTRAP_QR_CODE,
                key_ascii_cstr.as_ref().map_or_else(ptr::null, |x| x.as_ptr()),
                associated_data_cstr.as_ref().map_or_else(ptr::null, |x| x.as_ptr()),
            )
        })?;

        // Guarantees we get a compiler error if we mess up the lifetime...
        drop(channels_cstr);
        drop(key_ascii_cstr);
        drop(associated_data_cstr);

        Ok(())
    }
}

impl<'d, 'w, T> EspWifiDpp<'d, 'w, T> {
    pub fn get_bootstrapped_data(&self) -> &T {
        &self.bootstrapped_data
    }

    pub fn start_listen(self) -> Result<EspWifiDppListener<'d, 'w, T>, EspError> {
        EspWifiDppListener::start_listen(self)
    }
}

pub struct EspWifiDppListener<'d, 'w, T> {
    bootstrapped: EspWifiDpp<'d, 'w, T>,
}

impl<'d, 'w, T> EspWifiDppListener<'d, 'w, T> {
    fn start_listen(bootstrapped: EspWifiDpp<'d, 'w, T>) -> Result<Self, EspError> {
        info!("Starting DPP listener...");
        esp!(unsafe { esp_supp_dpp_start_listen() })?;
        Ok(Self {
            bootstrapped,
        })
    }

    /// Blocking wait for credentials or a possibly retryable error.  Note that user error
    /// such as scanning the wrong QR code can trigger this error case.  Retries are highly
    /// recommended, and especially via [Self::attempt_retry].
    pub fn wait_for_credentials(&self) -> Result<ClientConfiguration, EspError> {
        let next_state = self.bootstrapped.dpp.wait_for_next_state();
        self.handle_next_state(next_state)
    }

    /// Blocking wait for credentials, a timeout, or a terminal error.  If the timeout is
    /// reached, `Err(None)` is returned.
    pub fn wait_for_credentials_with_timeout(
        &self,
        timeout: Duration,
    ) -> Result<ClientConfiguration, Option<EspError>> {
        match self.bootstrapped.dpp.wait_for_next_state_with_timeout(timeout) {
            None => {
                self.stop_listen();
                Err(None)
            },
            Some(state) => Ok(self.handle_next_state(state)?),
        }
    }

    fn handle_next_state(&self, state: DppState) -> Result<ClientConfiguration, EspError> {
        match state {
            DppState::ConfigurationReceived(c) => Ok(c),
            DppState::Fail(e) => Err(e),
            DppState::Stopped => {
                info!("Caller requested DPP stop listening!");
                Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())
            }
            other => {
                self.stop_listen();
                Err(unexpected_state(other))
            },
        }
    }

    /// Stop listening for credentials.  If any callers are actively blocked waiting for credentials
    /// they will be notified with EspError(ESP_ERR_INVALID_STATE).  This method is not
    /// necessary to call after [Self::wait_for_credentials] or
    /// [Self::wait_for_credentials_with_timeout] returns as the esp_dpp API automatically
    /// stops listening on success or failure.
    pub fn stop_listen(&self) {
        info!("Stopping DPP listener...");

        // SAFETY: This function should be safe to call without any locks as it mostly just
        // sets a flag to halt at next chance.
        unsafe { esp_supp_dpp_stop_listen() };

        self.bootstrapped.dpp.set_pending_state(DppState::Stopped);
    }

    /// Attempt to retry after a transient failure in [Self::wait_for_credentials] (for example
    /// if a user scanned a bogus QR code or there was a recoverable transmit error on the
    /// channel).  This API will fail at runtime if ESP-IDF is not patched with
    /// https://github.com/espressif/esp-idf/pull/10865.
    ///
    /// On failure to retry, the caller is expected to bootstrap again in order to logically retry
    /// however this will potentially lose state and generate a new QR code if any parameters
    /// change (such as a new key being generated).
    pub fn attempt_retry(self) -> Result<EspWifiDpp<'d, 'w, T>, ()> {
        if Self::is_start_listen_patched() {
            Ok(self.bootstrapped)
        } else {
            Err(())
        }
    }

    // TODO: Sure would be nice to be able to write esp_idf_version >= 5.1...
    #[cfg(
        any(
            esp_idf_version_major = "4",
            all(esp_idf_version_major = "5", esp_idf_version_minor = "0")
        )
    )]
    fn is_start_listen_patched() -> bool { false }

    #[cfg(
        not(
            any(
                esp_idf_version_major = "4",
                all(esp_idf_version_major = "5", esp_idf_version_minor = "0")
            )
        )
    )]
    fn is_start_listen_patched() -> bool { true }
}

fn unexpected_state(state: DppState) -> EspError {
    warn!("Unexpected DPP state: {state:?}");
    EspError::from_infallible::<ESP_ERR_INVALID_STATE>()
}

#[derive(Debug)]
enum DppState {
    BootstrappedUriReady(String),
    ConfigurationReceived(ClientConfiguration),
    Fail(EspError),
    Stopped,
}

pub struct QrCode(pub String);

#[cfg(esp_idf_version_major = "4")]
/// ESP-IDF 4.x internally framed the key inside esp_dpp APIs.
fn frame_key(unframed: &[u8; 32]) -> &[u8] {
    unframed
}

#[cfg(not(esp_idf_version_major = "4"))]
/// ESP-IDF 5.x requires the caller put the key into the PEM format
fn frame_key(unframed: &[u8; 32]) -> Vec<u8> {
    let prefix = [0x30, 0x31, 0x02, 0x01, 0x01, 0x04, 0x20];
    let postfix = [0xa0, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];

    let mut ret = Vec::with_capacity(prefix.len() + unframed.len() + postfix.len());
    ret.extend(prefix);
    ret.extend(unframed);
    ret.extend(postfix);

    ret
}
