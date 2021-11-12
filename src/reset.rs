use alloc::sync::Arc;
use core::time::Duration;

use log::*;
use mutex_trait::Mutex;

use esp_idf_sys::*;

use crate::nvs::EspDefaultNvs;
use crate::task;
use crate::time::micros_since_boot;

const FLAG_SET: u16 = 0x1234;
const FLAG_CLEAR: u16 = 0x4321;
const FLAG_KEY: &str = "drd_flag";

static mut DOUBLE_RESET: EspMutex<Option<bool>> = EspMutex::new(None);

pub fn detect_double_reset(nvs: Arc<EspDefaultNvs>, timeout: Duration) -> Result<bool, IdfError> {
    unsafe {
        DOUBLE_RESET.lock(move |v| {
            if let Some(double_reset) = v {
                Ok(*double_reset)
            } else {
                let mut drd = DoubleResetDetector::new(nvs.clone(), timeout);

                let double_reset = drd.detect()?;
                if double_reset {
                    info!("detected double reset");
                    drd.stop()?;
                } else {
                    task::spawn("drd", move || {
                        task::sleep(timeout - Duration::from_micros(micros_since_boot()));
                        drd.stop()?;
                        Ok(())
                    })?;
                }

                *v = Some(double_reset);
                Ok(double_reset)
            }
        })
    }
}

pub struct DoubleResetDetector {
    // task_handle: Option<TaskHandle_t>,
    nvs: Arc<EspDefaultNvs>,
    timeout: Duration,
    waiting_for_double_reset: bool,
}

impl DoubleResetDetector {
    pub fn new(nvs: Arc<EspDefaultNvs>, timeout: Duration) -> Self {
        DoubleResetDetector {
            // task_handle: None,
            nvs,
            timeout,
            waiting_for_double_reset: false,
        }
    }

    fn start(&mut self) -> Result<(), EspError> {
        self.waiting_for_double_reset = true;
        self.set_flag()?;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), EspError> {
        info!("stopping double reset detector");
        self.waiting_for_double_reset = false;
        self.clear_flag()?;
        Ok(())
    }

    fn detect(&mut self) -> Result<bool, EspError> {
        let flag = self.has_flag()?;
        self.start()?;
        Ok(flag)
    }

    fn clear_flag(&mut self) -> Result<(), EspError> {
        let mut handle = self.nvs.open("esp_idf_svc", true)?;
        handle.set_u16(FLAG_KEY, FLAG_CLEAR)
    }

    fn set_flag(&mut self) -> Result<(), EspError> {
        let mut handle = self.nvs.open("esp_idf_svc", true)?;
        handle.set_u16(FLAG_KEY, FLAG_SET)
    }

    fn has_flag(&self) -> Result<bool, EspError> {
        let handle = self.nvs.open("esp_idf_svc", true)?;
        let res = handle.get_u16(FLAG_KEY)?.map_or(false, |f| f == FLAG_SET);
        Ok(res)
    }

    pub fn run(&mut self) -> Result<(), EspError> {
        if self.waiting_for_double_reset && micros_since_boot() > self.timeout.as_micros() as u64 {
            self.stop()?;
        }
        Ok(())
    }
}
