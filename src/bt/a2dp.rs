use core::{
    borrow::Borrow,
    fmt::{self, Debug},
    marker::PhantomData,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use esp_idf_sys::*;
use log::{debug, info};

use crate::bt::{BtClassicEnabled, BtDriver};

use super::{BdAddr, BtCallback};

pub trait A2dpMode {
    fn sink() -> bool;
    fn source() -> bool;
}

pub trait SinkEnabled: A2dpMode {}
pub trait SourceEnabled: A2dpMode {}

pub struct Sink;
impl SinkEnabled for Sink {}

impl A2dpMode for Sink {
    fn sink() -> bool {
        true
    }

    fn source() -> bool {
        false
    }
}

pub struct Source;
impl SourceEnabled for Source {}

impl A2dpMode for Source {
    fn sink() -> bool {
        false
    }

    fn source() -> bool {
        true
    }
}

pub struct Duplex;
impl SinkEnabled for Duplex {}
impl SourceEnabled for Duplex {}

impl A2dpMode for Duplex {
    fn sink() -> bool {
        true
    }

    fn source() -> bool {
        true
    }
}

#[derive(Clone)]
pub enum Codec {
    Sbc([u8; 4]),
    Mpeg1_2([u8; 4]),
    Mpeg2_4([u8; 6]),
    Atrac([u8; 7]),
    Unknown,
}

impl Codec {
    pub fn bitrate(&self) -> Option<u32> {
        if let Self::Sbc(data) = self {
            let oct0 = data[0];
            let sample_rate = if (oct0 & (0x01 << 6)) != 0 {
                32000
            } else if (oct0 & (0x01 << 5)) != 0 {
                44100
            } else if (oct0 & (0x01 << 4)) != 0 {
                48000
            } else {
                16000
            };

            Some(sample_rate)
        } else {
            None
        }
    }

    pub fn stereo(&self) -> Option<bool> {
        if let Self::Sbc(data) = self {
            Some((data[0] & (0x01 << 3)) == 0)
        } else {
            None
        }
    }
}

impl Debug for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sbc(data) => f.debug_tuple("Sbc").field(data).finish()?,
            Self::Mpeg1_2(data) => f.debug_tuple("Mpeg1_2").field(data).finish()?,
            Self::Mpeg2_4(data) => f.debug_tuple("Mpeg2_4").field(data).finish()?,
            Self::Atrac(data) => f.debug_tuple("Atrac").field(data).finish()?,
            Self::Unknown => write!(f, "Unknown")?,
        }

        write!(
            f,
            " / bitrate: {:?}, stereo: {:?}",
            self.bitrate(),
            self.stereo()
        )
    }
}

#[derive(Debug)]
pub enum A2dpEvent<'a> {
    ConfigureAudio { bd_addr: BdAddr, codec: Codec },
    SinkData(&'a [u8]),
    SourceData(&'a mut [u8]),
    Other,
}

#[allow(non_upper_case_globals)]
impl<'a> From<(esp_a2d_cb_event_t, &'a esp_a2d_cb_param_t)> for A2dpEvent<'a> {
    fn from(value: (esp_a2d_cb_event_t, &'a esp_a2d_cb_param_t)) -> Self {
        let (evt, param) = value;

        unsafe {
            match evt {
                esp_a2d_cb_event_t_ESP_A2D_AUDIO_CFG_EVT => A2dpEvent::ConfigureAudio {
                    bd_addr: param.audio_cfg.remote_bda.into(),
                    codec: match param.audio_cfg.mcc.type_ as u32 {
                        ESP_A2D_MCT_SBC => Codec::Sbc(param.audio_cfg.mcc.cie.sbc),
                        ESP_A2D_MCT_M12 => Codec::Mpeg1_2(param.audio_cfg.mcc.cie.m12),
                        ESP_A2D_MCT_M24 => Codec::Mpeg2_4(param.audio_cfg.mcc.cie.m24),
                        ESP_A2D_MCT_ATRAC => Codec::Atrac(param.audio_cfg.mcc.cie.atrac),
                        _ => Codec::Unknown,
                    },
                },
                _ => {
                    log::warn!("Unknown event {:?}", evt);
                    Self::Other
                    //panic!("Unknown event {:?}", evt)
                }
            }
        }
    }
}

pub struct EspA2dp<'d, M, T, S>
where
    M: BtClassicEnabled,
    T: Borrow<BtDriver<'d, M>>,
    S: A2dpMode,
{
    _driver: T,
    initialized: AtomicBool,
    _p: PhantomData<&'d ()>,
    _m: PhantomData<M>,
    _s: PhantomData<S>,
}

impl<'d, M, T> EspA2dp<'d, M, T, Sink>
where
    M: BtClassicEnabled,
    T: Borrow<BtDriver<'d, M>>,
{
    pub const fn new_sink(driver: T) -> Result<Self, EspError> {
        Ok(Self {
            _driver: driver,
            initialized: AtomicBool::new(false),
            _p: PhantomData,
            _m: PhantomData,
            _s: PhantomData,
        })
    }

    pub fn initialize<F>(&self, events_cb: F) -> Result<(), EspError>
    where
        F: Fn(A2dpEvent) + Send + 'd,
    {
        self.internal_initialize(move |event| {
            events_cb(event);
            0
        })
    }

    pub fn connect(&self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_a2d_sink_connect(bd_addr as *const _ as *mut _) })
    }

    pub fn disconnect(&self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_a2d_sink_disconnect(bd_addr as *const _ as *mut _) })
    }

    pub fn request_delay(&self) -> Result<(), EspError> {
        esp!(unsafe { esp_a2d_sink_get_delay_value() })
    }

    pub fn set_delay(&self, delay: Duration) -> Result<(), EspError> {
        esp!(unsafe { esp_a2d_sink_set_delay_value((delay.as_micros() / 100) as _) })
    }
}

impl<'d, M, T> EspA2dp<'d, M, T, Source>
where
    M: BtClassicEnabled,
    T: Borrow<BtDriver<'d, M>>,
{
    pub const fn new_source(driver: T) -> Result<Self, EspError> {
        Ok(Self {
            _driver: driver,
            initialized: AtomicBool::new(false),
            _p: PhantomData,
            _m: PhantomData,
            _s: PhantomData,
        })
    }

    pub fn initialize<F>(&self, events_cb: F) -> Result<(), EspError>
    where
        F: Fn(A2dpEvent) -> usize + Send + 'd,
    {
        self.internal_initialize(events_cb)
    }

    pub fn connect(&self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_a2d_source_connect(bd_addr as *const _ as *mut _) })
    }

    pub fn disconnect(&self, bd_addr: &BdAddr) -> Result<(), EspError> {
        esp!(unsafe { esp_a2d_source_disconnect(bd_addr as *const _ as *mut _) })
    }
}

impl<'d, M, T> EspA2dp<'d, M, T, Duplex>
where
    M: BtClassicEnabled,
    T: Borrow<BtDriver<'d, M>>,
{
    pub fn new_duplex(driver: T) -> Result<Self, EspError> {
        Ok(Self {
            _driver: driver,
            initialized: AtomicBool::new(false),
            _p: PhantomData,
            _m: PhantomData,
            _s: PhantomData,
        })
    }

    pub fn initialize<F>(&self, events_cb: F) -> Result<(), EspError>
    where
        F: Fn(A2dpEvent) -> usize + Send + 'd,
    {
        self.internal_initialize(events_cb)
    }
}

impl<'d, M, T, S> EspA2dp<'d, M, T, S>
where
    M: BtClassicEnabled,
    T: Borrow<BtDriver<'d, M>>,
    S: A2dpMode,
{
    fn internal_initialize<F>(&self, events_cb: F) -> Result<(), EspError>
    where
        F: Fn(A2dpEvent) -> usize + Send + 'd,
    {
        CALLBACK.set(events_cb)?;

        esp!(unsafe { esp_a2d_register_callback(Some(Self::event_handler)) })?;

        if S::sink() {
            esp!(unsafe { esp_a2d_sink_register_data_callback(Some(Self::sink_data_handler)) })?;
            esp!(unsafe { esp_a2d_sink_init() })?;
        }

        if S::source() {
            esp!(unsafe {
                esp_a2d_source_register_data_callback(Some(Self::source_data_handler))
            })?;
            esp!(unsafe { esp_a2d_source_init() })?;
        }

        self.initialized.store(true, Ordering::SeqCst);

        Ok(())
    }

    unsafe extern "C" fn event_handler(event: esp_a2d_cb_event_t, param: *mut esp_a2d_cb_param_t) {
        let param = unsafe { param.as_ref() }.unwrap();
        let event = A2dpEvent::from((event, param));

        info!("Got event {{ {:#?} }}", event);

        CALLBACK.call(event);
    }

    unsafe extern "C" fn sink_data_handler(buf: *const u8, len: u32) {
        let event = A2dpEvent::SinkData(core::slice::from_raw_parts(buf, len as _));
        debug!("Got event {{ {:#?} }}", event);

        CALLBACK.call(event);
    }

    unsafe extern "C" fn source_data_handler(buf: *mut u8, len: i32) -> i32 {
        let event = A2dpEvent::SourceData(core::slice::from_raw_parts_mut(buf, len as _));
        debug!("Got event {{ {:#?} }}", event);

        CALLBACK.call(event) as _
    }
}

impl<'d, M, T, S> Drop for EspA2dp<'d, M, T, S>
where
    M: BtClassicEnabled,
    T: Borrow<BtDriver<'d, M>>,
    S: A2dpMode,
{
    fn drop(&mut self) {
        if self.initialized.load(Ordering::SeqCst) {
            esp!(unsafe { esp_a2d_register_callback(None) }).unwrap();

            if S::sink() {
                esp!(unsafe { esp_a2d_sink_register_data_callback(None) }).unwrap();
                esp!(unsafe { esp_a2d_sink_deinit() }).unwrap();
            }

            if S::source() {
                esp!(unsafe { esp_a2d_source_register_data_callback(None) }).unwrap();
                esp!(unsafe { esp_a2d_source_deinit() }).unwrap();
            }

            CALLBACK.clear().unwrap();
        }
    }
}

static CALLBACK: BtCallback<A2dpEvent, usize> = BtCallback::new(0);
