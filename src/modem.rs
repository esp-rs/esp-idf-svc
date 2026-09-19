//! SIM7600 PPP-over-serial integration for ESP-NETIF/lwIP.

use core::time::Duration;
use std::{
    ffi::CString,
    string::String,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Instant,
    vec::Vec,
};

use embedded_svc::io::{ErrorType, Write};
use enumset::EnumSet;
use esp_idf_hal::{
    delay::{FreeRtos, TickType},
    io::EspIOError,
    uart::UartRxDriver,
};

use crate::{
    eventloop::{EspSubscription, EspSystemEventLoop, System},
    handle::RawHandle,
    ipv4,
    netif::{
        EspNetif, EspNetifDriver, IpEvent, NetifStack, PppAuthentication, PppConfiguration,
        PppEvent,
    },
    sys::{EspError, ESP_ERR_TIMEOUT},
};

const READ_POLL: Duration = Duration::from_millis(100);
const ESCAPE_GUARD_MS: u32 = 1100;

/// A UART-like reader which returns `Ok(0)` when its timeout expires.
pub trait TimedRead: ErrorType {
    fn read_timeout(&mut self, buffer: &mut [u8], timeout: Duration) -> Result<usize, Self::Error>;
}

impl TimedRead for UartRxDriver<'_> {
    fn read_timeout(&mut self, buffer: &mut [u8], timeout: Duration) -> Result<usize, Self::Error> {
        match UartRxDriver::read(self, buffer, TickType::from(timeout).ticks()) {
            Ok(len) => Ok(len),
            Err(error) if error.code() == ESP_ERR_TIMEOUT => Ok(0),
            Err(error) => Err(EspIOError(error)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModemConfig {
    pub apn: String,
    pub pin: Option<String>,
    pub authentication: Option<PppAuthConfig>,
    pub command_timeout: Duration,
    pub registration_timeout: Duration,
    pub dial_timeout: Duration,
    pub ppp_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub retry_delays: Vec<Duration>,
}

impl ModemConfig {
    pub fn new(apn: impl Into<String>) -> Self {
        Self {
            apn: apn.into(),
            pin: None,
            authentication: None,
            command_timeout: Duration::from_secs(5),
            registration_timeout: Duration::from_secs(180),
            dial_timeout: Duration::from_secs(60),
            ppp_timeout: Duration::from_secs(60),
            shutdown_timeout: Duration::from_secs(30),
            retry_delays: vec![
                Duration::from_secs(2),
                Duration::from_secs(5),
                Duration::from_secs(10),
            ],
        }
    }

    fn validate(&self) -> Result<(), ModemError> {
        validate_at_argument("APN", &self.apn)?;
        if self.apn.is_empty() {
            return Err(ModemError::Configuration("APN cannot be empty".into()));
        }
        if let Some(pin) = &self.pin {
            validate_at_argument("PIN", pin)?;
        }
        if let Some(auth) = &self.authentication {
            if auth.username.as_bytes().contains(&0) || auth.password.as_bytes().contains(&0) {
                return Err(ModemError::Configuration(
                    "PPP credentials cannot contain NUL".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct PppAuthConfig {
    pub protocol: PppAuthProtocol,
    pub username: String,
    pub password: String,
}

#[derive(Clone, Copy, Debug)]
pub enum PppAuthProtocol {
    Pap,
    Chap,
    PapOrChap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModemState {
    Initializing,
    Registering,
    Dialing,
    NegotiatingPpp,
    Connected,
    Recovering,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModemPhaseStatus {
    Dead,
    Master,
    Holdoff,
    Initialize,
    SerialConnection,
    Dormant,
    Establish,
    Authenticate,
    Callback,
    Network,
    Running,
    Terminate,
    Disconnect,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModemPPPError {
    Parameter,
    Open,
    Device,
    Alloc,
    User,
    Disconnect,
    AuthFail,
    Protocol,
    PeerDead,
    IdleTimeout,
    MaxConnectTimeout,
    Loopback,
}

#[derive(Clone, Debug)]
pub enum ModemError {
    Configuration(String),
    Io,
    At(String),
    Timeout(&'static str),
    BufferOverflow,
    SimPinRequired,
    SimPinRejected,
    RegistrationDenied,
    Ppp(ModemPPPError),
    ConnectionLost,
    RecoveryRequired,
    ShutdownTimeout,
    Esp(EspError),
}

impl ModemError {
    fn retryable(&self) -> bool {
        !matches!(
            self,
            Self::Configuration(_)
                | Self::SimPinRequired
                | Self::SimPinRejected
                | Self::RegistrationDenied
                | Self::RecoveryRequired
                | Self::ShutdownTimeout
                | Self::Ppp(ModemPPPError::AuthFail)
        )
    }
}

impl core::fmt::Display for ModemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Configuration(message) => write!(f, "invalid modem configuration: {message}"),
            Self::Io => f.write_str("modem transport I/O failed"),
            Self::At(message) => write!(f, "modem rejected command: {message}"),
            Self::Timeout(operation) => write!(f, "timed out while {operation}"),
            Self::BufferOverflow => f.write_str("AT response exceeded the receive buffer"),
            Self::SimPinRequired => f.write_str("SIM requires a PIN"),
            Self::SimPinRejected => f.write_str("SIM PIN was rejected"),
            Self::RegistrationDenied => f.write_str("cellular registration was denied"),
            Self::Ppp(error) => write!(f, "PPP failed: {error:?}"),
            Self::ConnectionLost => f.write_str("PPP connection was lost"),
            Self::RecoveryRequired => f.write_str("modem requires a hardware reset"),
            Self::ShutdownTimeout => f.write_str("PPP did not stop before its deadline"),
            Self::Esp(error) => write!(f, "ESP-IDF error: {error}"),
        }
    }
}

impl std::error::Error for ModemError {}

impl From<EspError> for ModemError {
    fn from(value: EspError) -> Self {
        Self::Esp(value)
    }
}

#[derive(Clone, Debug)]
pub struct ModemStatus {
    pub state: ModemState,
    pub phase: ModemPhaseStatus,
    pub ip_info: Option<ipv4::IpInfo>,
    pub last_error: Option<ModemError>,
}

struct SharedStatus {
    status: Mutex<ModemStatus>,
    changed: Condvar,
    stop: AtomicBool,
    ppp_error: Mutex<Option<ModemPPPError>>,
}

impl SharedStatus {
    fn new() -> Self {
        Self {
            status: Mutex::new(ModemStatus {
                state: ModemState::Stopped,
                phase: ModemPhaseStatus::Dead,
                ip_info: None,
                last_error: None,
            }),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
            ppp_error: Mutex::new(None),
        }
    }

    fn update(&self, update: impl FnOnce(&mut ModemStatus)) {
        update(&mut self.status.lock().unwrap());
        self.changed.notify_all();
    }
}

#[derive(Clone)]
pub struct ModemHandle {
    shared: Arc<SharedStatus>,
}

impl ModemHandle {
    pub fn status(&self) -> ModemStatus {
        self.shared.status.lock().unwrap().clone()
    }

    pub fn request_stop(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.changed.notify_all();
    }

    pub fn wait_connected(&self, timeout: Duration) -> Result<ModemStatus, ModemError> {
        let deadline = Instant::now() + timeout;
        let mut status = self.shared.status.lock().unwrap();
        loop {
            match status.state {
                ModemState::Connected => return Ok(status.clone()),
                ModemState::Failed => {
                    return Err(status
                        .last_error
                        .clone()
                        .unwrap_or(ModemError::ConnectionLost))
                }
                _ => {}
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(ModemError::Timeout("waiting for a PPP address"));
            }
            let (next, result) = self
                .shared
                .changed
                .wait_timeout(status, deadline - now)
                .unwrap();
            status = next;
            if result.timed_out() && status.state != ModemState::Connected {
                return Err(ModemError::Timeout("waiting for a PPP address"));
            }
        }
    }
}

/// Factory for a runner with exclusive transport ownership and a clonable handle.
pub struct EspModem;

impl EspModem {
    #[allow(clippy::new_ret_no_self)]
    pub fn new<W, R>(
        config: ModemConfig,
        writer: W,
        reader: R,
        sysloop: EspSystemEventLoop,
    ) -> Result<(ModemRunner<W, R>, ModemHandle), ModemError>
    where
        W: Write + Send + 'static,
        R: TimedRead + Send + 'static,
    {
        config.validate()?;
        let shared = Arc::new(SharedStatus::new());
        let writer = Arc::new(Mutex::new(writer));
        let netif = EspNetif::new(NetifStack::Ppp)?;
        let raw_handle = netif.handle() as usize;

        let ip_shared = shared.clone();
        let ip_subscription = sysloop.subscribe::<IpEvent, _>(move |event| {
            if !event.is_for_handle(raw_handle as _) {
                return;
            }
            match event {
                IpEvent::DhcpIpAssigned(assignment) => ip_shared.update(|status| {
                    status.state = ModemState::Connected;
                    status.ip_info = Some(assignment.ip_info());
                    status.last_error = None;
                }),
                IpEvent::DhcpIpDeassigned(_) => ip_shared.update(|status| {
                    status.ip_info = None;
                    if !ip_shared.stop.load(Ordering::SeqCst) {
                        status.state = ModemState::Recovering;
                    }
                }),
                _ => {}
            }
        })?;

        let ppp_shared = shared.clone();
        let ppp_subscription = sysloop.subscribe::<PppEvent, _>(move |event| {
            let (phase, error) = map_ppp_event(event);
            if let Some(phase) = phase {
                ppp_shared.update(|status| status.phase = phase);
            }
            if let Some(error) = error {
                *ppp_shared.ppp_error.lock().unwrap() = Some(error);
                if error != ModemPPPError::User || !ppp_shared.stop.load(Ordering::SeqCst) {
                    ppp_shared.update(|status| status.last_error = Some(ModemError::Ppp(error)));
                }
            }
        })?;

        let auth = config.authentication.clone();
        let tx_writer = writer.clone();
        let driver = EspNetifDriver::new(
            netif,
            move |netif| {
                netif.set_ppp_conf(&PppConfiguration::default())?;
                if let Some(auth) = &auth {
                    let mut protocols = EnumSet::new();
                    match auth.protocol {
                        PppAuthProtocol::Pap => {
                            protocols.insert(PppAuthentication::Pap);
                        }
                        PppAuthProtocol::Chap => {
                            protocols.insert(PppAuthentication::Chap);
                        }
                        PppAuthProtocol::PapOrChap => {
                            protocols.insert(PppAuthentication::Pap);
                            protocols.insert(PppAuthentication::Chap);
                        }
                    }
                    let username = CString::new(auth.username.as_str()).unwrap();
                    let password = CString::new(auth.password.as_str()).unwrap();
                    netif.set_ppp_auth(protocols, &username, &password)?;
                }
                Ok(())
            },
            move |data| {
                tx_writer
                    .lock()
                    .unwrap()
                    .write_all(data)
                    .map_err(|_| EspError::from_infallible::<{ crate::sys::ESP_FAIL }>())
            },
        )?;

        let handle = ModemHandle {
            shared: shared.clone(),
        };
        Ok((
            ModemRunner {
                config,
                writer,
                reader: BufferedTransport::new(reader, 4096),
                shared,
                _ip_subscription: ip_subscription,
                _ppp_subscription: ppp_subscription,
                driver,
                connected_since: None,
            },
            handle,
        ))
    }
}

pub struct ModemRunner<W, R>
where
    W: Write + Send + 'static,
    R: TimedRead + Send + 'static,
{
    config: ModemConfig,
    writer: Arc<Mutex<W>>,
    reader: BufferedTransport<R>,
    shared: Arc<SharedStatus>,
    _ip_subscription: EspSubscription<'static, System>,
    _ppp_subscription: EspSubscription<'static, System>,
    driver: EspNetifDriver<'static, EspNetif>,
    connected_since: Option<Instant>,
}

impl<W, R> ModemRunner<W, R>
where
    W: Write + Send + 'static,
    R: TimedRead + Send + 'static,
{
    pub fn run(mut self) -> Result<(), ModemError> {
        self.shared.stop.store(false, Ordering::SeqCst);
        let mut retry = 0;

        loop {
            if self.stop_requested() {
                return self.finish_stop();
            }
            *self.shared.ppp_error.lock().unwrap() = None;
            self.connected_since = None;
            self.set_state(ModemState::Initializing, None);

            let error = match self.run_session() {
                Ok(()) => return self.finish_stop(),
                Err(error) => error,
            };
            if self.stop_requested() {
                return self.finish_stop();
            }
            if self
                .connected_since
                .is_some_and(|since| since.elapsed() >= Duration::from_secs(60))
            {
                retry = 0;
            }
            if !error.retryable() || retry == self.config.retry_delays.len() {
                self.set_state(ModemState::Failed, Some(error.clone()));
                return Err(error);
            }

            self.set_state(ModemState::Recovering, Some(error));
            if let Err(error) = self.recover_command_mode() {
                self.set_state(ModemState::Failed, Some(error.clone()));
                return Err(error);
            }
            FreeRtos::delay_ms(self.config.retry_delays[retry].as_millis() as u32);
            retry += 1;
        }
    }

    fn run_session(&mut self) -> Result<(), ModemError> {
        self.synchronize()?;
        self.command("ATE0", self.config.command_timeout)?;
        self.command("AT+CMEE=2", self.config.command_timeout)?;
        self.command("AT+IFC=0,0", self.config.command_timeout)?;
        self.check_sim()?;
        if self.stop_requested() {
            return Ok(());
        }

        self.set_state(ModemState::Registering, None);
        self.wait_for_registration()?;
        if self.stop_requested() {
            return Ok(());
        }
        let context = format!("AT+CGDCONT=1,\"IP\",\"{}\"", self.config.apn);
        self.command(&context, self.config.command_timeout)?;

        self.set_state(ModemState::Dialing, None);
        self.dial()?;
        if self.stop_requested() {
            return Ok(());
        }
        self.set_state(ModemState::NegotiatingPpp, None);
        self.driver.start()?;

        let deadline = Instant::now() + self.config.ppp_timeout;
        let mut was_connected = false;
        let mut buffer = vec![0; 4096];
        loop {
            if self.stop_requested() {
                self.stop_ppp()?;
                return Ok(());
            }
            let ppp_error = self.shared.ppp_error.lock().unwrap().take();
            if let Some(error) = ppp_error {
                if error != ModemPPPError::User {
                    self.stop_ppp()?;
                    return Err(ModemError::Ppp(error));
                }
            }

            let state = self.shared.status.lock().unwrap().state;
            if state == ModemState::Connected {
                was_connected = true;
                self.connected_since.get_or_insert_with(Instant::now);
            } else if state == ModemState::Recovering && was_connected {
                self.stop_ppp()?;
                return Err(ModemError::ConnectionLost);
            } else if !was_connected && Instant::now() >= deadline {
                self.stop_ppp()?;
                return Err(ModemError::Timeout("negotiating PPP"));
            }

            let len = self.reader.read_data(&mut buffer, READ_POLL)?;
            if len > 0 {
                self.driver.rx(&buffer[..len])?;
            }
        }
    }

    fn synchronize(&mut self) -> Result<(), ModemError> {
        for _ in 0..3 {
            if self.command("AT", self.config.command_timeout).is_ok() {
                return Ok(());
            }
        }
        Err(ModemError::Timeout("synchronizing with the modem"))
    }

    fn check_sim(&mut self) -> Result<(), ModemError> {
        let lines = self.command("AT+CPIN?", self.config.command_timeout)?;
        let state = find_value(&lines, "+CPIN:").unwrap_or_default();
        if state == "READY" {
            return Ok(());
        }
        if state != "SIM PIN" {
            return Err(ModemError::SimPinRejected);
        }
        let pin = self.config.pin.clone().ok_or(ModemError::SimPinRequired)?;
        self.command(&format!("AT+CPIN=\"{pin}\""), self.config.command_timeout)
            .map_err(|_| ModemError::SimPinRejected)?;
        Ok(())
    }

    fn wait_for_registration(&mut self) -> Result<(), ModemError> {
        let deadline = Instant::now() + self.config.registration_timeout;
        let mut command = "AT+CEREG?";
        loop {
            match self.command(command, self.config.command_timeout) {
                Ok(lines) => {
                    let prefix = if command == "AT+CEREG?" {
                        "+CEREG:"
                    } else {
                        "+CGREG:"
                    };
                    if let Some(status) = find_value(&lines, prefix).and_then(registration_status) {
                        match status {
                            1 | 5 => return Ok(()),
                            3 => return Err(ModemError::RegistrationDenied),
                            _ => {}
                        }
                    }
                }
                Err(ModemError::At(_)) if command == "AT+CEREG?" => command = "AT+CGREG?",
                Err(error) => return Err(error),
            }
            if self.stop_requested() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(ModemError::Timeout("waiting for cellular registration"));
            }
            FreeRtos::delay_ms(1000);
        }
    }

    fn dial(&mut self) -> Result<(), ModemError> {
        self.write_raw(b"ATD*99***1#\r")?;
        let deadline = Instant::now() + self.config.dial_timeout;
        loop {
            let line = self.reader.read_line(deadline, "dialing the modem")?;
            let line = String::from_utf8_lossy(&line);
            let line = line.trim();
            if line == "CONNECT" || line.starts_with("CONNECT ") {
                return Ok(());
            }
            if is_at_error(line) {
                return Err(ModemError::At(line.into()));
            }
        }
    }

    fn command(&mut self, command: &str, timeout: Duration) -> Result<Vec<String>, ModemError> {
        self.write_raw(command.as_bytes())?;
        self.write_raw(b"\r")?;
        let deadline = Instant::now() + timeout;
        let mut lines = Vec::new();
        loop {
            let line = self
                .reader
                .read_line(deadline, "waiting for an AT response")?;
            let line = String::from(String::from_utf8_lossy(&line).trim());
            if line.is_empty() || line == command {
                continue;
            }
            if line == "OK" {
                return Ok(lines);
            }
            if is_at_error(&line) {
                return Err(ModemError::At(line));
            }
            lines.push(line);
        }
    }

    fn write_raw(&self, data: &[u8]) -> Result<(), ModemError> {
        self.writer
            .lock()
            .unwrap()
            .write_all(data)
            .map_err(|_| ModemError::Io)
    }

    fn stop_ppp(&mut self) -> Result<(), ModemError> {
        if !self.driver.is_started()? {
            return Ok(());
        }
        self.set_state(ModemState::Stopping, None);
        self.driver.stop()?;
        let deadline = Instant::now() + self.config.shutdown_timeout;
        let mut buffer = [0; 512];
        while Instant::now() < deadline {
            if self.shared.status.lock().unwrap().phase == ModemPhaseStatus::Dead {
                return Ok(());
            }
            let len = self.reader.read_data(&mut buffer, READ_POLL)?;
            if len > 0 {
                self.driver.rx(&buffer[..len])?;
            }
        }
        Err(ModemError::ShutdownTimeout)
    }

    fn recover_command_mode(&mut self) -> Result<(), ModemError> {
        if self.driver.is_started()? {
            self.stop_ppp()?;
        }
        self.reader.discard_pending();
        if self.command("AT", Duration::from_secs(1)).is_ok() {
            let _ = self.command("ATH", self.config.command_timeout);
            return Ok(());
        }

        FreeRtos::delay_ms(ESCAPE_GUARD_MS);
        self.write_raw(b"+++")?;
        FreeRtos::delay_ms(ESCAPE_GUARD_MS);
        self.reader.discard_pending();
        self.command("AT", self.config.command_timeout)
            .map_err(|_| ModemError::RecoveryRequired)?;
        let _ = self.command("ATH", self.config.command_timeout);
        Ok(())
    }

    fn finish_stop(&mut self) -> Result<(), ModemError> {
        match self.stop_ppp() {
            Ok(()) => {
                self.set_state(ModemState::Stopped, None);
                Ok(())
            }
            Err(error) => {
                self.set_state(ModemState::Failed, Some(error.clone()));
                Err(error)
            }
        }
    }

    fn set_state(&self, state: ModemState, error: Option<ModemError>) {
        self.shared.update(|status| {
            status.state = state;
            status.last_error = error;
            if state != ModemState::Connected {
                status.ip_info = None;
            }
        });
    }

    fn stop_requested(&self) -> bool {
        self.shared.stop.load(Ordering::SeqCst)
    }
}

struct BufferedTransport<R> {
    inner: R,
    buffer: Vec<u8>,
    start: usize,
    end: usize,
}

impl<R: TimedRead> BufferedTransport<R> {
    fn new(inner: R, capacity: usize) -> Self {
        Self {
            inner,
            buffer: vec![0; capacity],
            start: 0,
            end: 0,
        }
    }

    fn read_line(
        &mut self,
        deadline: Instant,
        operation: &'static str,
    ) -> Result<Vec<u8>, ModemError> {
        loop {
            if let Some(relative_end) = self.buffer[self.start..self.end]
                .iter()
                .position(|byte| *byte == b'\n')
            {
                let line_end = self.start + relative_end + 1;
                let mut content_end = line_end - 1;
                if content_end > self.start && self.buffer[content_end - 1] == b'\r' {
                    content_end -= 1;
                }
                let line = self.buffer[self.start..content_end].to_vec();
                self.start = line_end;
                if self.start == self.end {
                    self.start = 0;
                    self.end = 0;
                }
                return Ok(line);
            }
            if Instant::now() >= deadline {
                return Err(ModemError::Timeout(operation));
            }
            self.compact();
            if self.end == self.buffer.len() {
                return Err(ModemError::BufferOverflow);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = core::cmp::min(READ_POLL, remaining);
            let len = self
                .inner
                .read_timeout(&mut self.buffer[self.end..], timeout)
                .map_err(|_| ModemError::Io)?;
            self.end += len;
        }
    }

    fn read_data(&mut self, output: &mut [u8], timeout: Duration) -> Result<usize, ModemError> {
        if self.start < self.end {
            let len = core::cmp::min(output.len(), self.end - self.start);
            output[..len].copy_from_slice(&self.buffer[self.start..self.start + len]);
            self.start += len;
            if self.start == self.end {
                self.start = 0;
                self.end = 0;
            }
            return Ok(len);
        }
        self.inner
            .read_timeout(output, timeout)
            .map_err(|_| ModemError::Io)
    }

    fn compact(&mut self) {
        if self.start > 0 {
            self.buffer.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
    }

    fn discard_pending(&mut self) {
        self.start = 0;
        self.end = 0;
    }
}

fn validate_at_argument(name: &str, value: &str) -> Result<(), ModemError> {
    if value
        .bytes()
        .any(|byte| matches!(byte, b'"' | b'\r' | b'\n' | 0))
    {
        return Err(ModemError::Configuration(format!(
            "{name} contains a character which cannot be used in an AT command"
        )));
    }
    Ok(())
}

fn is_at_error(line: &str) -> bool {
    line == "ERROR"
        || line == "NO CARRIER"
        || line.starts_with("+CME ERROR")
        || line.starts_with("+CMS ERROR")
}

fn find_value<'a>(lines: &'a [String], prefix: &str) -> Option<&'a str> {
    lines
        .iter()
        .find_map(|line| line.strip_prefix(prefix).map(str::trim))
}

fn registration_status(value: &str) -> Option<u8> {
    value
        .split(',')
        .nth(1)
        .or_else(|| value.split(',').next())?
        .trim()
        .parse()
        .ok()
}

fn map_ppp_event(event: PppEvent) -> (Option<ModemPhaseStatus>, Option<ModemPPPError>) {
    match event {
        PppEvent::NoError => (None, None),
        PppEvent::ParameterError => (None, Some(ModemPPPError::Parameter)),
        PppEvent::OpenError => (None, Some(ModemPPPError::Open)),
        PppEvent::DeviceError => (None, Some(ModemPPPError::Device)),
        PppEvent::AllocError => (None, Some(ModemPPPError::Alloc)),
        PppEvent::UserError => (None, Some(ModemPPPError::User)),
        PppEvent::DisconnectError => (None, Some(ModemPPPError::Disconnect)),
        PppEvent::AuthFailError => (None, Some(ModemPPPError::AuthFail)),
        PppEvent::ProtocolError => (None, Some(ModemPPPError::Protocol)),
        PppEvent::PeerDeadError => (None, Some(ModemPPPError::PeerDead)),
        PppEvent::IdleTimeoutError => (None, Some(ModemPPPError::IdleTimeout)),
        PppEvent::MaxConnectTimeoutError => (None, Some(ModemPPPError::MaxConnectTimeout)),
        PppEvent::LoopbackError => (None, Some(ModemPPPError::Loopback)),
        PppEvent::PhaseDead => (Some(ModemPhaseStatus::Dead), None),
        PppEvent::PhaseMaster => (Some(ModemPhaseStatus::Master), None),
        PppEvent::PhaseHoldoff => (Some(ModemPhaseStatus::Holdoff), None),
        PppEvent::PhaseInitialize => (Some(ModemPhaseStatus::Initialize), None),
        PppEvent::PhaseSerialConnection => (Some(ModemPhaseStatus::SerialConnection), None),
        PppEvent::PhaseDormant => (Some(ModemPhaseStatus::Dormant), None),
        PppEvent::PhaseEstablish => (Some(ModemPhaseStatus::Establish), None),
        PppEvent::PhaseAuthenticate => (Some(ModemPhaseStatus::Authenticate), None),
        PppEvent::PhaseCallback => (Some(ModemPhaseStatus::Callback), None),
        PppEvent::PhaseNetwork => (Some(ModemPhaseStatus::Network), None),
        PppEvent::PhaseRunning => (Some(ModemPhaseStatus::Running), None),
        PppEvent::PhaseTerminate => (Some(ModemPhaseStatus::Terminate), None),
        PppEvent::PhaseDisconnect => (Some(ModemPhaseStatus::Disconnect), None),
        PppEvent::PhaseFailed => (Some(ModemPhaseStatus::Failed), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ScriptedReader {
        chunks: Vec<Vec<u8>>,
    }

    impl ErrorType for ScriptedReader {
        type Error = core::convert::Infallible;
    }

    impl TimedRead for ScriptedReader {
        fn read_timeout(
            &mut self,
            output: &mut [u8],
            _timeout: Duration,
        ) -> Result<usize, Self::Error> {
            if self.chunks.is_empty() {
                return Ok(0);
            }
            let chunk = self.chunks.remove(0);
            let len = core::cmp::min(chunk.len(), output.len());
            output[..len].copy_from_slice(&chunk[..len]);
            if len < chunk.len() {
                self.chunks.insert(0, chunk[len..].to_vec());
            }
            Ok(len)
        }
    }

    #[test]
    fn fragmented_lines_preserve_binary_data_after_connect() {
        let reader = ScriptedReader {
            chunks: vec![b"\r\nCON".to_vec(), b"NECT 115200\r\n~\xff}".to_vec()],
        };
        let mut reader = BufferedTransport::new(reader, 64);
        assert!(reader
            .read_line(Instant::now() + Duration::from_secs(1), "test")
            .unwrap()
            .is_empty());
        assert_eq!(
            reader
                .read_line(Instant::now() + Duration::from_secs(1), "test")
                .unwrap(),
            b"CONNECT 115200"
        );
        let mut ppp = [0; 8];
        let len = reader.read_data(&mut ppp, READ_POLL).unwrap();
        assert_eq!(&ppp[..len], b"~\xff}");
    }

    #[test]
    fn registration_response_parsing_handles_common_forms() {
        assert_eq!(registration_status("0,1"), Some(1));
        assert_eq!(registration_status("5"), Some(5));
        assert_eq!(registration_status("2,3,\"1234\""), Some(3));
    }

    #[test]
    fn at_arguments_reject_command_injection() {
        assert!(validate_at_argument("APN", "internet").is_ok());
        assert!(validate_at_argument("APN", "internet\rAT+CFUN=0").is_err());
        assert!(validate_at_argument("PIN", "12\"34").is_err());
    }
}
