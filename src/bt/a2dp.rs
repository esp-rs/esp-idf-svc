use core::{borrow::Borrow, marker::PhantomData};

use esp_idf_sys::*;

use crate::bt::{BtClassicEnabled, BtDriver};

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

pub struct A2dpEvent {}

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
        F: Fn(&A2dpEvent) + Send + 'static,
    {
        Self::internal_new(driver, events_cb)
    }
}

impl<'d, M, T> EspA2dp<'d, M, T, Source>
where
    T: Borrow<BtDriver<'d, M>>,
    M: BtClassicEnabled,
{
    pub fn new_source<F>(driver: T, events_cb: F) -> Result<Self, EspError>
    where
        F: Fn(&A2dpEvent) + Send + 'static,
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
        F: Fn(&A2dpEvent) + Send + 'static,
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
        F: Fn(&A2dpEvent) + Send + 'static,
    {
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
        //let event = A2dpEvent::from((event, param));

        //debug!("Got GAP event {{ {:#?} }}", &event);
    }

    unsafe extern "C" fn sink_data_handler(buf: *const u8, len: u32) {
        //debug!("Got GAP event {{ {:#?} }}", &event);
    }

    unsafe extern "C" fn source_data_handler(buf: *mut u8, len: i32) -> i32 {
        //debug!("Got GAP event {{ {:#?} }}", &event);
        panic!()
    }
}

impl<'d, M, T, S> Drop for EspA2dp<'d, M, T, S>
where
    T: Borrow<BtDriver<'d, M>>,
    M: BtClassicEnabled,
    S: A2dpMode,
{
    fn drop(&mut self) {
        if S::sink() {
            esp!(unsafe { esp_a2d_sink_register_data_callback(None) }).unwrap();
            esp!(unsafe { esp_a2d_sink_init() }).unwrap();
        }

        if S::source() {
            esp!(unsafe { esp_a2d_source_register_data_callback(None) }).unwrap();
            esp!(unsafe { esp_a2d_source_init() }).unwrap();
        }

        esp!(unsafe { esp_a2d_register_callback(None) }).unwrap();
    }
}
