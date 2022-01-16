use core::fmt::{Debug, Display};
use core::ptr;
use core::result::Result;

use embedded_svc::timer;

use esp_idf_sys::*;

struct EspTimerShared<'a> {
    callback: Option<Box<dyn Fn() + 'a>>,
}

impl<'a> EspTimerShared<'a> {
    extern "C" fn handle(arg: *mut c_types::c_void) {
        let shared = unsafe { (arg as *const EspTimerShared).as_ref().unwrap() };

        if let Some(callback) = shared.callback.as_ref() {
            callback();
        }
    }
}

pub struct EspTimer<'a> {
    shared: Box<EspTimerShared<'a>>,
    handle: esp_timer_handle_t,
}

impl<'a> Drop for EspTimer<'a> {
    fn drop(&mut self) {
        esp!(unsafe { esp_timer_delete(self.handle) }).unwrap();
    }
}

impl<'a> timer::Timer<'a> for EspTimer<'a> {
    type Error = EspError;

    fn callback<E>(
        &mut self,
        callback: Option<impl Fn() -> Result<(), E> + 'a>,
    ) -> Result<(), Self::Error>
    where
        E: Display + Debug,
    {
        self.shared.callback = callback.map(|callback| {
            let boxed: Box<dyn for<'b> Fn() + 'a> = Box::new(move || callback().unwrap());

            boxed
        });

        Ok(())
    }

    fn schedule(&mut self, after: std::time::Duration) -> Result<(), Self::Error> {
        self.cancel()?;

        esp!(unsafe { esp_timer_start_once(self.handle, after.as_micros() as _) })?;

        Ok(())
    }

    fn is_scheduled(&self) -> Result<bool, Self::Error> {
        Ok(unsafe { esp_timer_is_active(self.handle) })
    }

    fn cancel(&mut self) -> Result<bool, Self::Error> {
        let res = unsafe { esp_timer_stop(self.handle) };

        Ok(res != ESP_OK)
    }
}

struct PrivateData;

pub struct EspTimerService(PrivateData);

impl EspTimerService {
    pub fn new() -> Result<Self, EspError> {
        Ok(Self(PrivateData))
    }
}

impl timer::TimerService<'static> for EspTimerService {
    type Error = EspError;

    type Timer<'b> = EspTimer<'b>;

    fn timer(
        &self,
        _priority: timer::Priority,
        _name: impl AsRef<str>,
    ) -> Result<Self::Timer<'static>, Self::Error> {
        let mut handle: esp_timer_handle_t = ptr::null_mut();

        let shared = Box::new(EspTimerShared { callback: None });

        esp!(unsafe {
            esp_timer_create(
                &esp_timer_create_args_t {
                    callback: Some(EspTimerShared::handle),
                    name: b"rust\0" as *const _ as *const _, // TODO
                    arg: &shared as *const _ as *mut _,
                    dispatch_method: esp_timer_dispatch_t_ESP_TIMER_TASK,
                    skip_unhandled_events: false,
                },
                &mut handle as *mut _,
            )
        })?;

        Ok(EspTimer { shared, handle })
    }
}
