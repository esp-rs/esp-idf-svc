use core::fmt::{Debug, Display};
use core::mem;
use core::ptr;
use core::result::Result;

extern crate alloc;
use alloc::sync::Arc;

use ::log::*;

use embedded_svc::event_bus::{self, EventBus, Poster, Subscription, Timer};

use esp_idf_hal::mutex;

use esp_idf_sys::*;

use crate::private::cstr::RawCstrs;

#[derive(Debug)]
pub struct TaskConfiguration<'a> {
    pub name: &'a str,
    pub priority: u8,
    pub stack_size: usize,
    pub pin_to_core: Option<u8>,
}

impl<'a> Default for TaskConfiguration<'a> {
    fn default() -> Self {
        Self {
            name: "(unknown)",
            priority: 0,
            stack_size: 3072,
            pin_to_core: None,
        }
    }
}

#[derive(Debug)]
pub struct Configuration<'a> {
    pub queue_size: usize,
    pub task_configuration: Option<TaskConfiguration<'a>>,
}

impl<'a> From<&Configuration<'a>> for (esp_event_loop_args_t, RawCstrs) {
    fn from(conf: &Configuration<'a>) -> Self {
        let mut rcs = RawCstrs::new();

        let mut ela = esp_event_loop_args_t {
            queue_size: conf.queue_size as _,
            ..Default::default()
        };

        if let Some(ref task_conf) = conf.task_configuration {
            ela.task_name = rcs.as_ptr(task_conf.name);
            ela.task_priority = task_conf.priority as _;
            ela.task_stack_size = task_conf.stack_size as _;
            ela.task_core_id = task_conf.pin_to_core.unwrap_or(0) as _;
        }

        (ela, rcs)
    }
}

static TAKEN: mutex::Mutex<bool> = mutex::Mutex::new(false);

pub enum EventLoopType<'a> {
    System,
    Custom(Configuration<'a>),
}

impl<'a> Default for Configuration<'a> {
    fn default() -> Self {
        Self {
            queue_size: 1000,
            task_configuration: None,
        }
    }
}

struct EspSubscriptionShared<'a, P>
where
    P: Clone,
{
    source: event_bus::Source<P>,
    callback: Option<Box<dyn Fn(&P) + 'a>>,
}

impl<'a, P> EspSubscriptionShared<'a, P>
where
    P: Clone,
{
    fn new(source: event_bus::Source<P>) -> Self {
        Self {
            source,
            callback: None,
        }
    }

    extern "C" fn handle(
        event_handler_arg: *mut c_types::c_void,
        _event_base: esp_event_base_t,
        _event_id: i32,
        event_data: *mut c_types::c_void,
    ) {
        let shared = unsafe {
            (event_handler_arg as *const EspSubscriptionShared<P>)
                .as_ref()
                .unwrap()
        };

        if let Some(callback) = shared.callback.as_ref() {
            let payload: &P = unsafe { (event_data as *const P).as_ref().unwrap() };

            callback(payload);
        }
    }
}

pub struct EspSubscription<'a, P>
where
    P: Clone,
{
    event_loop_handle: Arc<EventLoopHandle>,
    handler_instance: esp_event_handler_instance_t,
    shared: Box<EspSubscriptionShared<'a, P>>,
}

impl<'a, P> Drop for EspSubscription<'a, P>
where
    P: Clone,
{
    fn drop(&mut self) {
        let raw_source = as_raw_source_id(&self.shared.source);

        let result = esp!(if self.event_loop_handle.is_global() {
            unsafe {
                esp_event_handler_instance_unregister(
                    raw_source,
                    ESP_EVENT_ANY_ID,
                    self.handler_instance,
                )
            }
        } else {
            unsafe {
                esp_event_handler_instance_unregister_with(
                    self.event_loop_handle.0,
                    raw_source,
                    ESP_EVENT_ANY_ID,
                    self.handler_instance,
                )
            }
        });

        result.unwrap();
    }
}

impl<'a, P> event_bus::Subscription<'a, P> for EspSubscription<'a, P>
where
    P: Clone,
{
    type Error = EspError;

    fn callback<E>(
        &mut self,
        callback: Option<impl for<'b> Fn(&'b P) -> Result<(), E> + 'a>,
    ) -> Result<(), Self::Error>
    where
        E: Display + Debug,
    {
        self.shared.callback = callback.map(|callback| {
            let boxed: Box<dyn for<'b> Fn(&'b P) + 'a> =
                Box::new(move |payload| callback(payload).unwrap());

            boxed
        });

        Ok(())
    }
}

struct EventLoopHandle(esp_event_loop_handle_t);

impl EventLoopHandle {
    fn new(event_loop_type: &EventLoopType) -> Result<Self, EspError> {
        match event_loop_type {
            EventLoopType::System => {
                let mut taken = TAKEN.lock();

                if *taken {
                    esp!(ESP_ERR_INVALID_STATE as i32)?;
                }

                esp!(unsafe { esp_event_loop_create_default() })?;

                *taken = true;

                Ok(Self(ptr::null_mut()))
            }
            &EventLoopType::Custom(ref conf) => {
                let mut handle: esp_event_loop_handle_t = ptr::null_mut();

                let (ela, _rcs) = conf.into();

                esp!(unsafe { esp_event_loop_create(&ela as *const _, &mut handle as _) })?;

                Ok(Self(handle))
            }
        }
    }

    fn is_global(&self) -> bool {
        self.0.is_null()
    }
}

impl Drop for EventLoopHandle {
    fn drop(&mut self) {
        if self.is_global() {
            let mut taken = TAKEN.lock();

            esp!(unsafe { esp_event_loop_delete_default() }).unwrap();
            *taken = false;
        } else {
            esp!(unsafe { esp_event_loop_delete(self.0) }).unwrap();
        }

        info!("Dropped");
    }
}

#[derive(Clone)]
pub struct EspEventLoop(Arc<EventLoopHandle>);

impl EspEventLoop {
    pub fn new(event_loop_type: &EventLoopType) -> Result<Self, EspError> {
        Ok(Self(Arc::new(EventLoopHandle::new(event_loop_type)?)))
    }
}

impl event_bus::Poster for EspEventLoop {
    type Error = EspError;

    fn post<P>(
        &self,
        _priority: event_bus::Priority,
        source: &event_bus::Source<P>,
        payload: &P,
    ) -> Result<(), Self::Error>
    where
        P: Copy,
    {
        let raw_source = source.id().as_ptr() as *const _; // TODO

        // TODO: Handle the case where data size is < 4 as an optimization

        esp!(unsafe {
            esp_event_post_to(
                self.0 .0,
                raw_source,
                0,
                payload as *const _ as *mut _,
                mem::size_of::<P>() as _,
                TickType_t::max_value(),
            )
        })
    }
}

impl event_bus::EventBus<'static> for EspEventLoop {
    type Error = EspError;

    type Subscription<'b, P>
    where
        P: Clone,
    = EspSubscription<'b, P>;

    fn subscribe<P>(
        &self,
        source: event_bus::Source<P>,
    ) -> Result<Self::Subscription<'static, P>, EspError>
    where
        P: Clone,
    {
        let raw_source = source.id().as_ptr() as *const _; // TODO
        let shared = Box::new(EspSubscriptionShared::new(source));

        let mut handler_instance: esp_event_handler_instance_t = ptr::null_mut();

        esp!(if self.0.is_global() {
            unsafe {
                esp_event_handler_instance_register(
                    raw_source,
                    ESP_EVENT_ANY_ID,
                    Some(EspSubscriptionShared::<P>::handle),
                    &shared as *const _ as *mut _,
                    &mut handler_instance as *mut _,
                )
            }
        } else {
            unsafe {
                esp_event_handler_instance_register_with(
                    self.0 .0,
                    raw_source,
                    ESP_EVENT_ANY_ID,
                    Some(EspSubscriptionShared::<P>::handle),
                    &shared as *const _ as *mut _,
                    &mut handler_instance as *mut _,
                )
            }
        })?;

        Ok(EspSubscription {
            event_loop_handle: self.0.clone(),
            handler_instance,
            shared,
        })
    }
}

pub struct EspTimer<'a> {
    timer: crate::timer::EspTimer<'a>,
    subscription: EspSubscription<'a, *const c_types::c_void>,
}

impl<'a> event_bus::Timer<'a> for EspTimer<'a> {
    type Error = EspError;

    fn callback<E>(
        &mut self,
        callback: Option<impl Fn() -> Result<(), E> + 'a>,
    ) -> Result<(), Self::Error>
    where
        E: Display + Debug,
    {
        todo!()

        // self.subscription
        //     .callback(callback.map(|callback| move |_| callback()))?;

        // Ok(())
    }

    fn schedule(&mut self, after: std::time::Duration) -> Result<(), Self::Error> {
        self.timer.schedule(after)
    }

    fn is_scheduled(&self) -> Result<bool, Self::Error> {
        self.timer.is_scheduled()
    }

    fn cancel(&mut self) -> Result<bool, Self::Error> {
        self.timer.cancel()
    }
}

impl event_bus::TimerService<'static> for EspEventLoop {
    type Error = EspError;

    type Timer<'b> = EspTimer<'b>;

    fn timer(
        &self,
        priority: event_bus::Priority,
        name: impl AsRef<str>,
    ) -> Result<Self::Timer<'static>, Self::Error> {
        todo!()

        // let mut timer = EspTimer {
        //     timer: crate::timer::EspTimerService::new()?.timer(priority, name)?,
        //     subscription: self.subscribe(source)?,
        // };

        // let event_bus = self.clone();

        // timer
        //     .timer
        //     .callback(Some(|| event_bus.post(priority, source, &())));
        // timer.subscription.callback(Some(|payload| {}))?;

        // Ok(timer)
    }
}

fn as_raw_source_id<P>(source: &event_bus::Source<P>) -> *const c_types::c_char {
    todo!()
}
