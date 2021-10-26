use core::time::Duration;

use log::*;

use esp_idf_sys::c_types::*;
use esp_idf_sys::*;

use crate::private::cstr::CString;

#[allow(non_upper_case_globals)]
const pdPASS: c_int = 1;

pub struct TaskHandle(TaskHandle_t);

impl TaskHandle {
    pub fn stop(&self) {
        unsafe { vTaskDelete(self.0) };
    }
}

struct TaskInternal {
    name: String,
    f: Box<dyn FnOnce() -> anyhow::Result<()>>,
}

pub struct TaskConfig {
    stack_size: u32,
    priority: u32,
}

impl Default for TaskConfig {
    fn default() -> Self {
        TaskConfig {
            stack_size: DEFAULT_THREAD_STACKSIZE,
            priority: DEFAULT_THREAD_PRIO,
        }
    }
}

impl TaskConfig {
    pub fn new(stack_size: u32, priority: u32) -> Self {
        TaskConfig {
            stack_size,
            priority,
        }
    }

    pub fn stack_size(self, stack_size: u32) -> Self {
        TaskConfig { stack_size, ..self }
    }

    pub fn priority(self, priority: u32) -> Self {
        TaskConfig { priority, ..self }
    }

    pub fn spawn<F>(
        self,
        name: impl AsRef<str>,
        f: F,
    ) -> Result<TaskHandle, IdfError>
        where
           F: FnOnce() -> anyhow::Result<()>,
           F: Send + 'static {
        let parameters = TaskInternal {
            name: name.as_ref().to_string(),
            f: Box::new(f),
        };
        let parameters = Box::into_raw(Box::new(parameters)) as *mut _;

        info!("starting task {:?}", name.as_ref());

        let name = CString::new(name.as_ref()).unwrap();
        let mut handle: TaskHandle_t = core::ptr::null_mut();
        let res = unsafe {
            xTaskCreatePinnedToCore(
                Some(esp_idf_svc_task),
                name.as_ptr(),
                self.stack_size,
                parameters,
                self.priority,
                &mut handle,
                tskNO_AFFINITY as i32,
            )
        };
        if res != pdPASS {
            return Err(EspError::from(ESP_ERR_NO_MEM as i32).unwrap().into());
        }

        Ok(TaskHandle(handle))
    }
}

pub fn spawn<F>(
    name: impl AsRef<str>,
    f: F,
) -> Result<TaskHandle, IdfError>
where
    F: FnOnce() -> anyhow::Result<()>,
    F: Send + 'static,
{
    TaskConfig::default().spawn(name, f)
}

extern "C" fn esp_idf_svc_task(args: *mut c_void) {
    let internal = unsafe { Box::from_raw(args as *mut TaskInternal) };

    info!("started task {:?}", internal.name);

    match (internal.f)() {
        Err(e) => {
            panic!("unexpected error in task {:?}: {:?}", internal.name, e);
        }
        Ok(_) => {}
    }

    info!("destroying task {:?}", internal.name);

    unsafe { vTaskDelete(core::ptr::null_mut() as _) };
}

#[allow(non_upper_case_globals)]
pub const TICK_PERIOD_MS: u32 = 1000 / configTICK_RATE_HZ;

/// sleep tells FreeRTOS to put the current thread to sleep for at least the specified duration,
/// this is not an exact duration and can't be shorter than the rtos tick period.
pub fn sleep(duration: Duration) {
    unsafe {
        vTaskDelay(duration.as_millis() as u32 / TICK_PERIOD_MS);
    }
}
