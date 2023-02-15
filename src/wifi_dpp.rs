//! Wi-Fi Easy Connect (DPP) support
//!
//! To use this feature, you must add CONFIG_WPA_DPP_SUPPORT=y to your sdkconfig.

use ::log::*;

use std::ffi::{c_char, CStr, CString};
use std::fmt::Write;
use std::ops::Deref;
use std::ptr;
use std::sync::mpsc::{Receiver, sync_channel, SyncSender};
use embedded_svc::wifi::{ClientConfiguration, Configuration, Wifi};
use esp_idf_sys::*;
use esp_idf_sys::EspError;
use crate::private::common::Newtype;
use crate::private::mutex;
use crate::wifi::EspWifi;

static EVENTS_TX: mutex::Mutex<Option<SyncSender<DppEvent>>> =
    mutex::Mutex::wrap(mutex::RawMutex::new(), None);

pub struct EspDppBootstrapper<'d, 'w> {
    wifi: &'d mut EspWifi<'w>,
    events_rx: Receiver<DppEvent>,
}

impl<'d, 'w> EspDppBootstrapper<'d, 'w> {
    pub fn new(wifi: &'d mut EspWifi<'w>) -> Result<Self, EspError> {
        if wifi.is_started()? {
            wifi.disconnect()?;
            wifi.stop()?;
        }

        Self::init(wifi)
    }

    fn init(wifi: &'d mut EspWifi<'w>) -> Result<Self, EspError> {
        let (events_tx, events_rx) = sync_channel(1);
        let mut dpp_event_relay = EVENTS_TX.lock();
        *dpp_event_relay = Some(events_tx);
        drop(dpp_event_relay);
        esp!(unsafe { esp_supp_dpp_init(Some(Self::dpp_event_cb_unsafe)) })?;
        Ok(Self {
            wifi,
            events_rx,
        })
    }

    /// Generate a QR code that can be scanned by a mobile phone or other configurator
    /// to securely provide us with the Wi-Fi credentials.  Must invoke a listen API on the returned
    /// bootstrapped instance (e.g. [EspDppBootstrapped::listen_once]) or scanning the
    /// QR code will not be able to deliver the credentials to us.
    ///
    /// Important implementation notes:
    ///
    /// 1. You must provide _all_ viable channels that the AP could be using
    /// in order to successfully acquire credentials!  For example, in the US, you can use
    /// `(1..=11).collect()`.
    ///
    /// 2. The WiFi driver will be forced started and with a default STA config unless the
    /// state is set-up ahead of time.  It's unclear if the AuthMethod that you select
    /// for this STA config affects the results.
    pub fn gen_qrcode<'b>(
        &'b mut self,
        channels: &[u8],
        key: Option<&[u8; 32]>,
        associated_data: Option<&[u8]>
    ) -> Result<EspDppBootstrapped<'b, QrCode>, EspError> {
        let mut channels_str = channels.into_iter()
            .fold(String::new(), |mut a, c| {
                write!(a, "{c},").unwrap();
                a
            });
        channels_str.pop();
        let channels_cstr = CString::new(channels_str).unwrap();
        let key_ascii_cstr = key.map(|k| {
            let result = k.iter()
                .fold(String::new(), |mut a, b| {
                    write!(a, "{b:02X}").unwrap();
                    a
                });
            CString::new(result).unwrap()
        });
        let associated_data_cstr = match associated_data {
            Some(associated_data) => {
                Some(CString::new(associated_data)
                    .map_err(|_| {
                        warn!("associated data contains an embedded NUL character!");
                        EspError::from_infallible::<ESP_ERR_INVALID_ARG>()
                    })?)
            }
            None => None,
        };
        debug!("dpp_bootstrap_gen...");
        esp!(unsafe {
      esp_supp_dpp_bootstrap_gen(
          channels_cstr.as_ptr(),
          dpp_bootstrap_type_DPP_BOOTSTRAP_QR_CODE,
          key_ascii_cstr.map_or_else(ptr::null, |x| x.as_ptr()),
          associated_data_cstr.map_or_else(ptr::null, |x| x.as_ptr()))
    })?;
        let event = self.events_rx.recv()
            .map_err(|_| {
                warn!("Internal error receiving event!");
                EspError::from_infallible::<ESP_ERR_INVALID_STATE>()
            })?;
        debug!("dpp_bootstrap_gen got: {event:?}");
        match event {
            DppEvent::UriReady(qrcode) => {
                // Bit of a hack to put the wifi driver in the correct mode.
                self.ensure_config_and_start()?;
                Ok(EspDppBootstrapped::<QrCode> {
                    events_rx: &self.events_rx,
                    data: QrCode(qrcode),
                })
            }
            _ => {
                warn!("Got unexpected event: {event:?}");
                Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())
            },
        }
    }

    fn ensure_config_and_start(&mut self) -> Result<ClientConfiguration, EspError> {
        let operating_config = match self.wifi.get_configuration()? {
            Configuration::Client(c) => c,
            _ => {
                let fallback_config = ClientConfiguration::default();
                self.wifi.set_configuration(&Configuration::Client(fallback_config.clone()))?;
                fallback_config
            },
        };
        if !self.wifi.is_started()? {
            self.wifi.start()?;
        }
        Ok(operating_config)
    }

    unsafe extern "C" fn dpp_event_cb_unsafe(
        evt: esp_supp_dpp_event_t,
        data: *mut ::core::ffi::c_void
    ) {
        debug!("dpp_event_cb_unsafe: evt={evt}");
        let event = match evt {
            esp_supp_dpp_event_t_ESP_SUPP_DPP_URI_READY => {
                DppEvent::UriReady(CStr::from_ptr(data as *mut c_char).to_str().unwrap().into())
            },
            esp_supp_dpp_event_t_ESP_SUPP_DPP_CFG_RECVD => {
                let config = data as *mut wifi_config_t;
                // TODO: We're losing pmf_cfg.required=true setting due to missing
                // information in ClientConfiguration.
                DppEvent::ConfigurationReceived(Newtype((*config).sta).into())
            },
            esp_supp_dpp_event_t_ESP_SUPP_DPP_FAIL => {
                DppEvent::Fail(EspError::from(data as esp_err_t).unwrap())
            }
            _ => panic!(),
        };
        dpp_event_cb(event)
    }
}

fn dpp_event_cb(event: DppEvent) {
    match EVENTS_TX.lock().deref() {
        Some(tx) => {
            debug!("Sending: {event:?}");
            if let Err(e) = tx.try_send(event) {
                error!("Cannot relay event: {e}");
            }
        }
        None => warn!("Got spurious {event:?} ???"),
    }
}


#[derive(Debug)]
enum DppEvent {
    UriReady(String),
    ConfigurationReceived(ClientConfiguration),
    Fail(EspError),
}

impl<'d, 'w> Drop for EspDppBootstrapper<'d, 'w> {
    fn drop(&mut self) {
        unsafe { esp_supp_dpp_deinit() };
    }
}

pub struct EspDppBootstrapped<'d, T> {
    events_rx: &'d Receiver<DppEvent>,
    pub data: T,
}

#[derive(Debug, Clone)]
pub struct QrCode(pub String);

impl<'d, T> EspDppBootstrapped<'d, T> {
    pub fn listen_once(&self) -> Result<ClientConfiguration, EspError> {
        esp!(unsafe { esp_supp_dpp_start_listen() })?;
        let event = self.events_rx.recv()
            .map_err(|e| {
                warn!("Internal receive error: {e}");
                EspError::from_infallible::<ESP_ERR_INVALID_STATE>()
            })?;
        match event {
            DppEvent::ConfigurationReceived(config) => Ok(config),
            DppEvent::Fail(e) => Err(e),
            _ => {
                warn!("Ignoring unexpected event {event:?}");
                Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())
            }
        }
    }

    pub fn listen_forever(&self) -> Result<ClientConfiguration, EspError> {
        loop {
            match self.listen_once() {
                Ok(config) => return Ok(config),
                Err(e) => warn!("DPP error: {e}, trying again..."),
            }
        }
    }
}
