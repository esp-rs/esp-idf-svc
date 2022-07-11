use ::log::{Level, LevelFilter, Metadata, Record};

use esp_idf_sys::*;

use crate::private::common::*;
use crate::private::cstr::*;

#[allow(non_upper_case_globals)]
impl From<Newtype<esp_log_level_t>> for LevelFilter {
    fn from(level: Newtype<esp_log_level_t>) -> Self {
        match level.0 {
            esp_log_level_t_ESP_LOG_NONE => LevelFilter::Off,
            esp_log_level_t_ESP_LOG_ERROR => LevelFilter::Error,
            esp_log_level_t_ESP_LOG_WARN => LevelFilter::Warn,
            esp_log_level_t_ESP_LOG_INFO => LevelFilter::Info,
            esp_log_level_t_ESP_LOG_DEBUG => LevelFilter::Debug,
            esp_log_level_t_ESP_LOG_VERBOSE => LevelFilter::Trace,
            _ => LevelFilter::Trace,
        }
    }
}

impl From<LevelFilter> for Newtype<esp_log_level_t> {
    fn from(level: LevelFilter) -> Self {
        Newtype(match level {
            LevelFilter::Off => esp_log_level_t_ESP_LOG_NONE,
            LevelFilter::Error => esp_log_level_t_ESP_LOG_ERROR,
            LevelFilter::Warn => esp_log_level_t_ESP_LOG_WARN,
            LevelFilter::Info => esp_log_level_t_ESP_LOG_INFO,
            LevelFilter::Debug => esp_log_level_t_ESP_LOG_DEBUG,
            LevelFilter::Trace => esp_log_level_t_ESP_LOG_VERBOSE,
        })
    }
}

#[allow(non_upper_case_globals)]
impl From<Newtype<esp_log_level_t>> for Level {
    fn from(level: Newtype<esp_log_level_t>) -> Self {
        match level.0 {
            esp_log_level_t_ESP_LOG_ERROR => Level::Error,
            esp_log_level_t_ESP_LOG_WARN => Level::Warn,
            esp_log_level_t_ESP_LOG_INFO => Level::Info,
            esp_log_level_t_ESP_LOG_DEBUG => Level::Debug,
            esp_log_level_t_ESP_LOG_VERBOSE => Level::Trace,
            _ => Level::Trace,
        }
    }
}

impl From<Level> for Newtype<esp_log_level_t> {
    fn from(level: Level) -> Self {
        Newtype(match level {
            Level::Error => esp_log_level_t_ESP_LOG_ERROR,
            Level::Warn => esp_log_level_t_ESP_LOG_WARN,
            Level::Info => esp_log_level_t_ESP_LOG_INFO,
            Level::Debug => esp_log_level_t_ESP_LOG_DEBUG,
            Level::Trace => esp_log_level_t_ESP_LOG_VERBOSE,
        })
    }
}

static LOGGER: EspLogger = EspLogger;

pub struct EspLogger;

unsafe impl Send for EspLogger {}
unsafe impl Sync for EspLogger {}

impl EspLogger {
    pub fn initialize_default() {
        ::log::set_logger(&LOGGER)
            .map(|()| LOGGER.initialize())
            .unwrap();
    }

    pub fn initialize(&self) {
        ::log::set_max_level(self.get_max_level());
    }

    pub fn get_max_level(&self) -> LevelFilter {
        LevelFilter::from(Newtype(CONFIG_LOG_DEFAULT_LEVEL))
    }

    pub fn set_target_level(&self, target: impl AsRef<str>, level_filter: LevelFilter) {
        let ctarget = CString::new(target.as_ref()).unwrap();

        unsafe {
            esp_log_level_set(
                ctarget.as_c_str().as_ptr(),
                Newtype::<esp_log_level_t>::from(level_filter).0,
            )
        };
    }

    fn get_marker(level: Level) -> &'static str {
        match level {
            Level::Error => "E",
            Level::Warn => "W",
            Level::Info => "I",
            Level::Debug => "D",
            Level::Trace => "V",
        }
    }

    fn get_color(level: Level) -> Option<u8> {
        if CONFIG_LOG_COLORS == 0 {
            None
        } else {
            match level {
                Level::Error => Some(31), // LOG_COLOR_RED
                Level::Warn => Some(33),  // LOG_COLOR_BROWN
                Level::Info => Some(32),  // LOG_COLOR_GREEN,
                _ => None,
            }
        }
    }

    fn should_log(record: &Record) -> bool {
        let level = Newtype::<esp_log_level_t>::from(record.level()).0;
        let max_level = unsafe {
            esp_log_level_get(b"rust-logging\0" as *const u8 as *const _) // TODO: use record target?
        };
        level <= max_level
    }
}

impl ::log::Log for EspLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= LevelFilter::from(Newtype(CONFIG_LOG_DEFAULT_LEVEL))
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) && Self::should_log(record) {
            if let Some(color) = Self::get_color(record.level()) {
                println!(
                    "\x1b[0;{}m{} ({}) {}: {}\x1b[0m",
                    color,
                    Self::get_marker(record.metadata().level()),
                    unsafe { esp_log_timestamp() },
                    record.metadata().target(),
                    record.args(),
                );
            } else {
                println!(
                    "{} ({}) {}: {}",
                    Self::get_marker(record.metadata().level()),
                    unsafe { esp_log_timestamp() },
                    record.metadata().target(),
                    record.args(),
                );
            }
        }
    }

    fn flush(&self) {}
}
