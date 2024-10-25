use crate::{
    handle::RawHandle,
    netif::{PppConfiguration, PppEvent},
};
use core::{
    borrow::{Borrow, BorrowMut},
    cell::RefCell,
    marker::PhantomData,
};
use esp_idf_hal::{
    delay::BLOCK,
    io::{Error, EspIOError},
    uart::{UartDriver, UartTxDriver},
};
use std::{boxed::Box, rc::Rc, sync::Arc};

use crate::{
    eventloop::{EspEventLoop, EspSubscription, EspSystemEventLoop, System},
    netif::{EspNetif, EspNetifDriver, NetifStack},
    private::mutex,
    sys::*,
};

/// Unable to bypass the current buffered reader or writer because there are buffered bytes.
#[derive(Debug)]
pub struct BypassError;

/// A buffered [`Read`]
///
/// The BufferedRead will read into the provided buffer to avoid small reads to the inner reader.
pub struct BufferedRead<'buf, T: embedded_svc::io::Read> {
    inner: T,
    buf: &'buf mut [u8],
    offset: usize,
    available: usize,
}

impl<'buf, T: embedded_svc::io::Read> BufferedRead<'buf, T> {
    /// Create a new buffered reader
    pub fn new(inner: T, buf: &'buf mut [u8]) -> Self {
        Self {
            inner,
            buf,
            offset: 0,
            available: 0,
        }
    }

    /// Create a new buffered reader with the first `available` bytes readily available at `offset`.
    ///
    /// This is useful if for some reason the inner reader was previously consumed by a greedy reader
    /// in a way such that the BufferedRead must inherit these excess bytes.
    pub fn new_with_data(inner: T, buf: &'buf mut [u8], offset: usize, available: usize) -> Self {
        assert!(offset + available <= buf.len());
        Self {
            inner,
            buf,
            offset,
            available,
        }
    }

    /// Get whether there are any bytes readily available
    pub fn is_empty(&self) -> bool {
        self.available == 0
    }

    /// Get the number of bytes that are readily availbale
    pub fn available(&self) -> usize {
        self.available
    }

    /// Get the inner reader if there are no currently buffered, available bytes
    pub fn bypass(&mut self) -> Result<&mut T, BypassError> {
        match self.available {
            0 => Ok(&mut self.inner),
            _ => Err(BypassError),
        }
    }

    /// Release and get the inner reader
    pub fn release(self) -> T {
        self.inner
    }
}

impl<T: embedded_svc::io::Read> embedded_svc::io::ErrorType for BufferedRead<'_, T> {
    type Error = T::Error;
}

impl<T: embedded_svc::io::Read + embedded_svc::io::Write> embedded_svc::io::Write
    for BufferedRead<'_, T>
{
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.inner.write(buf)
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        self.inner.write_all(buf)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

impl<T: embedded_svc::io::Read> embedded_svc::io::Read for BufferedRead<'_, T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if self.available == 0 {
            if buf.len() >= self.buf.len() {
                // Fast path - bypass local buffer
                return self.inner.read(buf);
            }
            self.offset = 0;
            self.available = self.inner.read(self.buf)?;
        }

        let len = usize::min(self.available, buf.len());
        buf[..len].copy_from_slice(&self.buf[self.offset..self.offset + len]);
        if len < self.available {
            // There are still bytes left
            self.offset += len;
            self.available -= len;
        } else {
            // The buffer is drained
            self.available = 0;
        }

        Ok(len)
    }
}

impl<T: embedded_svc::io::Read> embedded_svc::io::BufRead for BufferedRead<'_, T> {
    fn fill_buf(&mut self) -> Result<&[u8], Self::Error> {
        if self.available == 0 {
            self.offset = 0;
            self.available = self.inner.read(self.buf)?;
        }

        Ok(&self.buf[self.offset..self.offset + self.available])
    }

    fn consume(&mut self, amt: usize) {
        assert!(amt <= self.available);
        self.offset += amt;
        self.available -= amt;
    }
}

pub struct EspModem<'d, T, R, E>
where
    T: embedded_svc::io::Write<Error = E> + Send,
    R: embedded_svc::io::Read<Error = E>,
    EspIOError: From<E>,
{
    writer: Arc<mutex::Mutex<T>>,
    reader: Arc<mutex::Mutex<R>>,
    status: Arc<mutex::Mutex<ModemDriverStatus>>,
    _subscription: EspSubscription<'static, System>,
    netif: Arc<mutex::Mutex<EspNetif>>,
    _d: PhantomData<&'d ()>,
}

impl<'d, T, R, E> EspModem<'d, T, R, E>
where
    T: embedded_svc::io::Write<Error = E> + Send,
    R: embedded_svc::io::Read<Error = E>,
    EspIOError: From<E>, // EspError: From<<T as embedded_svc::io::ErrorType>::Error>,
                         // EspError: From<<R as embedded_svc::io::ErrorType>::Error>,
{
    pub fn new(writer: T, reader: R, sysloop: EspSystemEventLoop) -> Result<Self, EspError> {
        let (status, subscription) = Self::subscribe(&sysloop)?;

        Ok(Self {
            writer: Arc::new(mutex::Mutex::new(writer)),
            reader: Arc::new(mutex::Mutex::new(reader)),
            status,
            _subscription: subscription,
            netif: Arc::new(mutex::Mutex::new(EspNetif::new(NetifStack::Ppp)?)),
            _d: PhantomData,
        })
    }

    /// Run the modem network interface. Blocks until the PPP encounters an error.
    pub fn run(&self, buffer: &mut [u8]) -> Result<(), EspError> {
        self.status.lock().running = true;

        // now in ppp mode.

        let handle = self.netif.as_ref().lock().handle();
        esp!(unsafe {
            esp_event_handler_register(
                IP_EVENT,
                ESP_EVENT_ANY_ID as _,
                Some(Self::raw_on_ip_event),
                handle as *mut core::ffi::c_void,
            )
        })?;

        let netif = Arc::clone(&self.netif);
        let mut netif = (*netif).lock();
        let netif = (*netif).borrow_mut();

        let writer = self.writer.clone();
        let driver = EspNetifDriver::new_nonstatic(
            netif,
            move |x| {
                x.set_ppp_conf(&PppConfiguration {
                    phase_events_enabled: true,
                    error_events_enabled: true,
                })
            },
            move |data| Self::tx(writer.clone(), data),
        )?;

        loop {
            if !self.status.lock().running {
                break;
            }
            let len = self
                .reader
                .lock()
                .read(buffer)
                .map_err(|w| Into::<EspIOError>::into(w).0)?;

            if len > 0 {
                driver.rx(&buffer[..len])?;
            }
        }

        Ok(())
    }

    /// Returns the current [`ModemPPPError`] status, if any.
    pub fn get_error(&self) -> Option<ModemPPPError> {
        self.status.lock().error.clone()
    }

    /// Returns the current [`ModemPhaseStatus`]
    pub fn get_phase_status(&self) -> ModemPhaseStatus {
        self.status.lock().phase.clone()
    }
    pub fn is_connected(&self) -> Result<bool, EspError> {
        let netif = (*self.netif).borrow();
        netif.lock().is_up()
    }
    // /// Returns the underlying [`EspNetif`]
    // pub fn netif(&self) -> &EspNetif {
    //     &self.netif.borrow()
    // }

    // /// Returns the underlying [`EspNetif`], as mutable
    // pub fn netif_mut(&mut self) -> &mut EspNetif {
    //     &mut self.netif
    // }

    /// Callback given to the LWIP API to write data to the PPP server.
    fn tx(writer: Arc<mutex::Mutex<T>>, data: &[u8]) -> Result<(), EspError> {
        writer
            .lock()
            .write_all(data)
            .map_err(|w| Into::<EspIOError>::into(w).0)?;

        Ok(())
    }

    fn subscribe(
        sysloop: &EspEventLoop<System>,
    ) -> Result<
        (
            Arc<mutex::Mutex<ModemDriverStatus>>,
            EspSubscription<'static, System>,
        ),
        EspError,
    > {
        let status = Arc::new(mutex::Mutex::new(ModemDriverStatus {
            error: None,
            phase: ModemPhaseStatus::Disconnect,
            running: false,
        }));

        let s_status = status.clone();

        let subscription = sysloop.subscribe::<PppEvent, _>(move |event| {
            let mut guard = s_status.lock();
            log::info!("Got event PPP: {:?}", event);
            match event {
                PppEvent::NoError => guard.error = None,
                PppEvent::ParameterError => guard.error = Some(ModemPPPError::Parameter),
                PppEvent::OpenError => guard.error = Some(ModemPPPError::Open),
                PppEvent::DeviceError => guard.error = Some(ModemPPPError::Device),
                PppEvent::AllocError => guard.error = Some(ModemPPPError::Alloc),
                PppEvent::UserError => guard.error = Some(ModemPPPError::User),
                PppEvent::DisconnectError => guard.error = Some(ModemPPPError::Disconnect),
                PppEvent::AuthFailError => guard.error = Some(ModemPPPError::AuthFail),
                PppEvent::ProtocolError => guard.error = Some(ModemPPPError::Protocol),
                PppEvent::PeerDeadError => guard.error = Some(ModemPPPError::PeerDead),
                PppEvent::IdleTimeoutError => guard.error = Some(ModemPPPError::IdleTimeout),
                PppEvent::MaxConnectTimeoutError => {
                    guard.error = Some(ModemPPPError::MaxConnectTimeout)
                }
                PppEvent::LoopbackError => guard.error = Some(ModemPPPError::Loopback),
                PppEvent::PhaseDead => guard.phase = ModemPhaseStatus::Dead,
                PppEvent::PhaseMaster => guard.phase = ModemPhaseStatus::Master,
                PppEvent::PhaseHoldoff => guard.phase = ModemPhaseStatus::Holdoff,
                PppEvent::PhaseInitialize => guard.phase = ModemPhaseStatus::Initialize,
                PppEvent::PhaseSerialConnection => guard.phase = ModemPhaseStatus::SerialConnection,
                PppEvent::PhaseDormant => guard.phase = ModemPhaseStatus::Dormant,
                PppEvent::PhaseEstablish => guard.phase = ModemPhaseStatus::Establish,
                PppEvent::PhaseAuthenticate => guard.phase = ModemPhaseStatus::Authenticate,
                PppEvent::PhaseCallback => guard.phase = ModemPhaseStatus::Callback,
                PppEvent::PhaseNetwork => guard.phase = ModemPhaseStatus::Network,
                PppEvent::PhaseRunning => guard.phase = ModemPhaseStatus::Running,
                PppEvent::PhaseTerminate => guard.phase = ModemPhaseStatus::Terminate,
                PppEvent::PhaseDisconnect => guard.phase = ModemPhaseStatus::Disconnect,
                PppEvent::PhaseFailed => guard.phase = ModemPhaseStatus::Failed,
            }
        })?;

        Ok((status, subscription))
    }

    fn on_ip_event(event_id: u32, event_data: *mut ::core::ffi::c_void) {
        use log::info;
        info!("Got event id: {}", event_id);

        if event_id == ip_event_t_IP_EVENT_PPP_GOT_IP {
            let mut dns_info = esp_netif_dns_info_t::default();
            let event_data = unsafe { (event_data as *const ip_event_got_ip_t).as_ref() }.unwrap();
            info!(" ip_info = {:?} ", event_data.ip_info);
            info!("modem connected to ppp server, info: {:?}", event_data);

            let netif = event_data.esp_netif;
            esp!(unsafe { esp_netif_get_dns_info(netif, 0, &mut dns_info) }).unwrap();
            info!(" dns_info = {:?} ", unsafe { dns_info.ip.u_addr.ip4.addr });
        } else if event_id == ip_event_t_IP_EVENT_PPP_LOST_IP {
            info!("Modem disconnected from ppp server");
        }
    }

    unsafe extern "C" fn raw_on_ip_event(
        _event_handler_arg: *mut ::core::ffi::c_void,
        _event_base: esp_event_base_t,
        event_id: i32,
        event_data: *mut ::core::ffi::c_void,
    ) {
        Self::on_ip_event(event_id as _, event_data)
    }
}

unsafe impl<T, R, E> Send for EspModem<'_, T, R, E>
where
    T: embedded_svc::io::Write<Error = E> + Send,
    R: embedded_svc::io::Read<Error = E>,
    EspIOError: From<E>,
{
}

unsafe impl<T, R, E> Sync for EspModem<'_, T, R, E>
where
    T: embedded_svc::io::Write<Error = E> + Send,
    R: embedded_svc::io::Read<Error = E>,
    EspIOError: From<E>,
{
}

impl<'d, T, R, E> Drop for EspModem<'d, T, R, E>
where
    T: embedded_svc::io::Write<Error = E> + Send,
    R: embedded_svc::io::Read<Error = E>,
    EspIOError: From<E>,
{
    fn drop(&mut self) {
        esp!(unsafe {
            esp_event_handler_unregister(
                IP_EVENT,
                ESP_EVENT_ANY_ID as _,
                Some(Self::raw_on_ip_event),
            )
        })
        .unwrap();
    }
}

#[derive(Clone, Debug)]
pub enum ModemPPPError {
    ///  Invalid parameter.
    Parameter,
    ///  Unable to open PPP session.
    Open,
    ///  Invalid I/O device for PPP.
    Device,
    ///  Unable to allocate resources.
    Alloc,
    ///  User interrupt.
    User,
    ///  Connection lost.
    Disconnect,
    ///  Failed authentication challenge.
    AuthFail,
    ///  Failed to meet protocol.
    Protocol,
    ///  Connection timeout
    PeerDead,
    ///  Idle Timeout
    IdleTimeout,
    ///  Max connect time reached
    MaxConnectTimeout,
    ///  Loopback detected
    Loopback,
}
#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
pub struct ModemDriverStatus {
    pub error: Option<ModemPPPError>,
    pub phase: ModemPhaseStatus,
    pub running: bool,
}

pub mod sim {
    //! SimModem
    //!
    //! Models a modem device with a sim card able to serve as a
    //! network interface for the host.

    use embedded_svc::io::{BufRead, Read, Write};

    /// The generic device trait. Implementations of this trait should provide
    /// relevant AT commands and confirm the modem replies to drive the modem
    /// into PPPoS (data mode).
    pub trait SimModem {
        /// The current mode of the sim modem.
        fn get_mode(&self) -> &CommunicationMode;

        /// Initialise the remote modem so that it is in PPPoS mode.
        fn negotiate<T: Write, R: BufRead + Read>(
            &mut self,
            tx: &mut T,
            rx: &mut R,
        ) -> Result<(), ModemError>;
    }

    /// State of the modem.
    ///
    /// In [CommunicationMode::Command] mode, AT commands will function,
    /// serving to put the modem into [CommunicationMode::Data].
    ///
    /// In [CommunicationMode::Data] the modem device will act as a Point-To-Point over Serial (PPPoS)
    /// server.
    pub enum CommunicationMode {
        Command,
        Data,
    }

    #[derive(Debug)]
    pub enum ModemError {
        IO,
        ATParse(at_commands::parser::ParseError),
        ATBuild(usize),
    }

    impl std::fmt::Display for ModemError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "{:?}", self)
        }
    }

    impl std::error::Error for ModemError {}

    impl From<usize> for ModemError {
        fn from(value: usize) -> Self {
            ModemError::ATBuild(value)
        }
    }

    impl From<at_commands::parser::ParseError> for ModemError {
        fn from(value: at_commands::parser::ParseError) -> Self {
            ModemError::ATParse(value)
        }
    }

    pub mod sim7600 {
        //! [super::SimModem] Implementation for the `SIMCOM 76XX` range of
        //! modems.

        use at_commands::{builder::CommandBuilder, parser::CommandParser};
        use core::fmt::Display;
        use embedded_svc::io::{BufRead, Read, Write};

        use super::{CommunicationMode, ModemError, SimModem};
        pub struct SIM7600(CommunicationMode);

        impl SIM7600 {
            pub fn new() -> Self {
                Self(CommunicationMode::Command)
            }
        }

        pub enum BitErrorRate {
            /// < 0.01%
            LT001,
            /// 0.01% - 0.1%
            LT01,
            /// 0.1% - 0.5%
            LT05,
            /// 0.5% - 1%
            LT1,
            /// 1% - 2%
            LT2,
            /// 2% - 4%
            LT4,
            /// 4% - 8%
            LT8,
            /// >=8%
            GT8,
            /// unknown or undetectable
            Unknown,
        }
        impl Display for BitErrorRate {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match *self {
                    BitErrorRate::GT8 => write!(f, ">= 8%"),
                    BitErrorRate::LT001 => write!(f, "< 0.01%"),
                    BitErrorRate::LT01 => write!(f, "0.01% - 0.1%"),
                    BitErrorRate::LT05 => write!(f, "0.1% - 0.5%"),
                    BitErrorRate::LT1 => write!(f, "0.5% - 1%"),
                    BitErrorRate::LT2 => write!(f, "1% - 2%"),
                    BitErrorRate::LT4 => write!(f, "2% - 4%"),
                    BitErrorRate::LT8 => write!(f, "4% - 8%"),
                    BitErrorRate::Unknown => write!(f, "Unknown"),
                }
            }
        }

        impl From<i32> for BitErrorRate {
            fn from(value: i32) -> Self {
                match value {
                    0 => Self::LT001,
                    1 => Self::LT01,
                    2 => Self::LT05,
                    3 => Self::LT1,
                    4 => Self::LT2,
                    5 => Self::LT4,
                    6 => Self::LT8,
                    7 => Self::GT8,
                    _ => Self::Unknown,
                }
            }
        }

        /// Received Signal Strength Indication
        pub enum RSSI {
            /// -113 dBm or less
            DBMLT113,
            /// -111 dBm
            DBM111,
            /// -109 to -53 dBm
            DBM109_53(i32),
            /// -51 dBm or greater
            DBMGT51,
            /// not known or not detectable
            Unknown,
            /// -116 dBm or less
            DBMLT116,
            /// -115 dBm
            DBM115,
            /// -114 to -26 dBm
            DBM114_26(i32),
            /// -25 dBm or greater
            DBMGT25,
        }

        impl Display for RSSI {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match *self {
                    RSSI::DBMLT113 => write!(f, "<= -113 dBm"),
                    RSSI::DBM111 => write!(f, "-111 dBm"),
                    RSSI::DBM109_53(x) => write!(f, "{} dBm", x),
                    RSSI::DBMGT51 => write!(f, ">= -51 dBm"),
                    RSSI::DBM114_26(x) => write!(f, "{} dBm", x),
                    RSSI::DBM115 => write!(f, "-115 dBm"),
                    RSSI::DBMGT25 => write!(f, ">= -25 dBm"),
                    RSSI::DBMLT116 => write!(f, "<= -116 dBm"),
                    RSSI::Unknown => write!(f, "Unknown"),
                }
            }
        }

        impl RSSI {
            pub fn parse(raw: i32) -> RSSI {
                match raw {
                    0 => Self::DBMLT113,
                    1 => Self::DBM111,
                    2..=30 => Self::DBM109_53(RSSI::map2_30_to_109_53(raw)),
                    31 => Self::DBMGT51,
                    99 => Self::Unknown,
                    100 => Self::DBMLT116,
                    101 => Self::DBM115,
                    102..=191 => Self::DBM114_26(RSSI::map102_191_to_114_26(raw)),
                    _ => Self::Unknown,
                }
            }

            fn map2_30_to_109_53(raw: i32) -> i32 {
                const X1: i32 = 2;
                const Y1: i32 = -109;
                const X2: i32 = 30;
                const Y2: i32 = -53;
                const GRAD: i32 = (Y2 - Y1) / (X2 - X1); // 56/28 = 2
                const OFFSET: i32 = Y1 - (GRAD * X1); // -113
                (GRAD * raw) + OFFSET
            }

            fn map102_191_to_114_26(raw: i32) -> i32 {
                const X1: i32 = 102;
                const Y1: i32 = -114;
                // const X2: i32 = 191;
                // const Y2: i32 = -26;
                const GRAD: i32 = 1;
                // requires #![feature(int_roundings)]
                // const GRAD: i32 = (Y2 - Y1).div_ceil((X2 - X1)); // would be 88/89, so truncated to 0
                const OFFSET: i32 = Y1 - (GRAD * X1); // -216
                (GRAD * raw) + OFFSET
            }
        }

        impl Default for SIM7600 {
            fn default() -> Self {
                Self::new()
            }
        }

        impl SimModem for SIM7600 {
            fn negotiate<T: Write, R: BufRead + Read>(
                &mut self,
                tx: &mut T,
                rx: &mut R,
            ) -> Result<(), ModemError> {
                let mut buffer = [0u8; 64];
                reset(tx, rx, &mut buffer)?;

                //disable echo
                set_echo(tx, rx, &mut buffer, false)?;

                // get signal quality
                let (rssi, ber) = get_signal_quality(tx, rx, &mut buffer)?;
                log::info!("RSSI = {rssi}");
                log::info!("BER = {ber}");
                // get iccid
                let iccid = get_iccid(tx, rx, &mut buffer)?;
                log::info!("ICCID = [{}]", iccid);

                // check pdp network reg
                read_gprs_registration_status(tx, rx, &mut buffer)?;

                //configure apn
                set_pdp_context(tx, rx, &mut buffer)?;

                // start ppp
                set_data_mode(tx, rx, &mut buffer)?;

                self.0 = CommunicationMode::Data;
                Ok(())
            }

            fn get_mode(&self) -> &CommunicationMode {
                &self.0
            }
        }

        pub fn get_signal_quality<T: embedded_svc::io::Write, R: embedded_svc::io::BufRead>(
            tx: &mut T,
            rx: &mut R,
            buff: &mut [u8],
        ) -> Result<(RSSI, BitErrorRate), ModemError> {
            let cmd = CommandBuilder::create_execute(buff, true)
                .named("+CSQ")
                .finish()?;

            tx.write(cmd).map_err(|_| ModemError::IO)?;

            let len = rx
                .fill_buf()
                .map_err(|_| ModemError::IO)?
                .read(buff)
                .map_err(|_| ModemError::IO)?;

            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));

            // \r\n+CSQ: 19,99\r\n\r\nOK\r\n
            let (raw_rssi, raw_ber) = CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\n+CSQ: ")
                .expect_int_parameter()
                .expect_int_parameter()
                .expect_identifier(b"\r\n\r\nOK\r\n")
                .finish()?;
            rx.consume(len);

            Ok((RSSI::parse(raw_rssi), raw_ber.into()))
        }

        fn get_iccid<T: embedded_svc::io::Write, R: embedded_svc::io::BufRead>(
            tx: &mut T,
            rx: &mut R,
            buff: &mut [u8],
        ) -> Result<heapless::String<22>, ModemError> {
            let cmd = CommandBuilder::create_execute(buff, true)
                .named("+CICCID")
                .finish()?;

            tx.write(cmd).map_err(|_| ModemError::IO)?;

            let len = rx
                .fill_buf()
                .map_err(|_| ModemError::IO)?
                .read(buff)
                .map_err(|_| ModemError::IO)?;
            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));

            let (ccid,) = CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\n+ICCID: ")
                .expect_raw_string()
                .expect_identifier(b"\r\n\r\nOK\r\n")
                .finish()?;
            rx.consume(len);
            Ok(heapless::String::try_from(ccid).unwrap())
        }

        fn reset<T: Write, R: Read>(
            tx: &mut T,
            rx: &mut R,
            buff: &mut [u8],
        ) -> Result<(), ModemError> {
            let cmd = CommandBuilder::create_execute(buff, false)
                .named("ATZ0")
                .finish()?;
            log::info!("Send Reset");

            tx.write(cmd).map_err(|_| ModemError::IO)?;

            // not sure if I need this or not
            let len = rx.read(buff).map_err(|_| ModemError::IO)?;
            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));
            if CommandParser::parse(&buff[..len])
                .expect_identifier(b"ATZ0\r")
                .expect_identifier(b"\r\nOK\r\n")
                .finish()
                .is_err()
            {
                CommandParser::parse(&buff[..len])
                    .expect_identifier(b"ATZ0\r")
                    .expect_identifier(b"\r\nERROR\r\n")
                    .finish()?
            }
            Ok(())
        }

        fn set_echo<T: Write, R: Read>(
            tx: &mut T,
            rx: &mut R,
            buff: &mut [u8],
            echo: bool,
        ) -> Result<(), ModemError> {
            let cmd = CommandBuilder::create_execute(buff, false)
                .named(format!("ATE{}", i32::from(echo)))
                .finish()?;
            log::info!("Set echo ");
            tx.write(cmd).map_err(|_| ModemError::IO)?;

            let len = rx.read(buff).map_err(|_| ModemError::IO)?;
            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));

            Ok(CommandParser::parse(&buff[..len])
                .expect_identifier(b"ATE0\r")
                .expect_identifier(b"\r\nOK\r\n")
                .finish()?)
        }

        fn read_gprs_registration_status<T: Write, R: Read>(
            tx: &mut T,
            rx: &mut R,
            buff: &mut [u8],
        ) -> Result<(i32, i32, Option<i32>, Option<i32>), ModemError> {
            let cmd = CommandBuilder::create_query(buff, true)
                .named("+CGREG")
                .finish()?;
            log::info!("Get Registration Status");
            tx.write(cmd).map_err(|_| ModemError::IO)?;
            let len = rx.read(buff).map_err(|_| ModemError::IO)?;
            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));

            Ok(CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\n+CGREG: ")
                .expect_int_parameter()
                .expect_int_parameter()
                .expect_optional_int_parameter()
                .expect_optional_int_parameter()
                .expect_identifier(b"\r\n\r\nOK\r\n")
                .finish()?)
        }

        fn set_pdp_context<T: Write, R: Read>(
            tx: &mut T,
            rx: &mut R,
            buff: &mut [u8],
        ) -> Result<(), ModemError> {
            let cmd = CommandBuilder::create_set(buff, true)
                .named("+CGDCONT")
                .with_int_parameter(1) // context id
                .with_string_parameter("IP") // pdp type
                .with_string_parameter("flolive.net") // apn
                .finish()?;
            log::info!("Set PDP Context");
            tx.write(cmd).map_err(|_| ModemError::IO)?;
            let len = rx.read(buff).map_err(|_| ModemError::IO)?;
            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));

            Ok(CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\nOK\r\n")
                .finish()?)
        }

        fn set_data_mode<T: Write, R: BufRead + Read>(
            tx: &mut T,
            rx: &mut R,
            buff: &mut [u8],
        ) -> Result<(), ModemError> {
            let cmd = CommandBuilder::create_execute(buff, false)
                .named("ATD*99#")
                .finish()?;
            log::info!("Set Data mode");
            tx.write(cmd).map_err(|_| ModemError::IO)?;

            let len = rx
                .fill_buf()
                .map_err(|_| ModemError::IO)?
                .read(buff)
                .map_err(|_| ModemError::IO)?;

            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));

            let (connect_parm,) = CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\nCONNECT ")
                .expect_optional_raw_string()
                .expect_identifier(b"\r\n")
                .finish()?;
            log::info!("connect {:?}", connect_parm);
            // consume only pre-PPP bytes from the buffer
            rx.consume(10);
            if let Some(connect_str) = connect_parm {
                rx.consume(connect_str.len());
            }
            rx.consume(2);
            Ok(())
        }
    }
}
