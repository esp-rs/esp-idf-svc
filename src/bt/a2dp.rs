use core::{borrow::Borrow, marker::PhantomData};

use esp_idf_sys::*;
use log::debug;

use crate::bt::{BtClassicEnabled, BtDriver};

use super::BtCallback;

pub trait A2dpMode {
    fn sink() -> bool;
    fn source() -> bool;
}

pub trait SinkEnabled {}
pub trait SourceEnabled {}

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

#[derive(Debug)]
pub enum A2dpEvent<'a> {
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
                _ => {
                    log::warn!("Unhandled event {:?}", evt);
                    Self::Other
                    //panic!("Unhandled event {:?}", evt)
                }
            }
        }
    }
}
pub struct EspA2dp<'d, M, T, S>
where
    T: Borrow<BtDriver<'d, M>>,
    M: BtClassicEnabled,
    S: A2dpMode,
{
    _driver: T,
    _p: PhantomData<&'d ()>,
    _m: PhantomData<M>,
    _s: PhantomData<S>,
}

impl<'d, M, T> EspA2dp<'d, M, T, Sink>
where
    T: Borrow<BtDriver<'d, M>>,
    M: BtClassicEnabled,
{
    pub fn new_sink<F>(driver: T, events_cb: F) -> Result<Self, EspError>
    where
        F: Fn(A2dpEvent) + Send + 'static,
    {
        Self::internal_new(driver, move |event| {
            events_cb(event);
            0
        })
    }
}

impl<'d, M, T> EspA2dp<'d, M, T, Source>
where
    T: Borrow<BtDriver<'d, M>>,
    M: BtClassicEnabled,
{
    pub fn new_source<F>(driver: T, events_cb: F) -> Result<Self, EspError>
    where
        F: Fn(A2dpEvent) -> usize + Send + 'static,
    {
        Self::internal_new(driver, events_cb)
    }
}

impl<'d, M, T> EspA2dp<'d, M, T, Duplex>
where
    T: Borrow<BtDriver<'d, M>>,
    M: BtClassicEnabled,
{
    pub fn new_duplex<F>(driver: T, events_cb: F) -> Result<Self, EspError>
    where
        F: Fn(A2dpEvent) -> usize + Send + 'static,
    {
        Self::internal_new(driver, events_cb)
    }
}

impl<'d, M, T, S> EspA2dp<'d, M, T, S>
where
    T: Borrow<BtDriver<'d, M>>,
    M: BtClassicEnabled,
    S: A2dpMode,
{
    fn internal_new<F>(driver: T, events_cb: F) -> Result<Self, EspError>
    where
        F: Fn(A2dpEvent) -> usize + Send + 'static,
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

        Ok(Self {
            _driver: driver,
            _p: PhantomData,
            _m: PhantomData,
            _s: PhantomData,
        })
    }

    unsafe extern "C" fn event_handler(event: esp_a2d_cb_event_t, param: *mut esp_a2d_cb_param_t) {
        let param = unsafe { param.as_ref() }.unwrap();
        let event = A2dpEvent::from((event, param));

        debug!("Got event {{ {:#?} }}", event);

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
    T: Borrow<BtDriver<'d, M>>,
    M: BtClassicEnabled,
    S: A2dpMode,
{
    fn drop(&mut self) {
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

static CALLBACK: BtCallback<A2dpEvent, usize> = BtCallback::new(0);
