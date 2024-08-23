use crate::handle::RawHandle;
use core::{borrow::BorrowMut, ffi, marker::PhantomData, mem::MaybeUninit};
use esp_idf_hal::{
    delay::BLOCK,
    uart::{UartDriver, UartTxDriver},
};
use std::sync::Arc;

use crate::{
    eventloop::{
        EspEventDeserializer, EspEventLoop, EspEventSource, EspSubscription, EspSystemEventLoop,
        System,
    },
    netif::{EspNetif, EspNetifDriver, NetifStack},
    private::mutex,
    sys::*,
};

pub struct EspModem<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    serial: T,
    status: Arc<mutex::Mutex<ModemDriverStatus>>,
    _subscription: EspSubscription<'static, System>,
    netif: EspNetif,
    _d: PhantomData<&'d ()>,
}

impl<'d, T> EspModem<'d, T>
where
    T: BorrowMut<UartDriver<'d>> + Send,
{
    pub fn new(serial: T, sysloop: EspSystemEventLoop) -> Result<Self, EspError> {
        let (status, subscription) = Self::subscribe(&sysloop)?;

        Ok(Self {
            serial,
            status,
            _subscription: subscription,
            netif: EspNetif::new(NetifStack::Ppp)?,
            _d: PhantomData,
        })
    }

    /// Run the modem network interface. Blocks until the PPP encounters an error.
    pub fn run(&mut self, mut buffer: [u8; 64]) -> Result<(), EspError> {
        self.status.lock().running = true;

        // now in ppp mode.
        // let netif = EspNetif::new(NetifStack::Ppp)?;

        // subscribe to user event
        esp!(unsafe {
            esp_event_handler_register(
                IP_EVENT,
                ESP_EVENT_ANY_ID as _,
                Some(Self::raw_on_ip_event),
                self.netif.handle() as *mut core::ffi::c_void,
            )
        })?;
        esp!(unsafe {
            esp_event_handler_register(
                NETIF_PPP_STATUS,
                ESP_EVENT_ANY_ID as _,
                Some(Self::raw_on_ppp_changed),
                self.netif.handle() as *mut core::ffi::c_void,
            )
        })?;
        let (mut tx, rx) = self.serial.borrow_mut().split();
        let driver = unsafe {
            EspNetifDriver::new_nonstatic_ppp(&self.netif, move |x| Self::tx(&mut tx, x))?
        };

        loop {
            if !self.status.lock().running {
                break;
            }
            let len = rx.read(&mut buffer, BLOCK)?;
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

    /// Returns the underlying [`EspNetif`]
    pub fn netif(&self) -> &EspNetif {
        &self.netif
    }

    /// Returns the underlying [`EspNetif`], as mutable
    pub fn netif_mut(&mut self) -> &mut EspNetif {
        &mut self.netif
    }

    /// Callback given to the LWIP API to write data to the PPP server.
    fn tx(writer: &mut UartTxDriver, data: &[u8]) -> Result<(), EspError> {
        esp_idf_hal::io::Write::write_all(writer, data).map_err(|w| w.0)?;

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

        let subscription = sysloop.subscribe::<ModemEvent, _>(move |event| {
            let mut guard = s_status.lock();
            log::info!("Got event PPP: {:?}", event);
            match event {
                ModemEvent::ErrorNone => guard.error = None,
                ModemEvent::ErrorParameter => guard.error = Some(ModemPPPError::Parameter),
                ModemEvent::ErrorOpen => guard.error = Some(ModemPPPError::Open),
                ModemEvent::ErrorDevice => guard.error = Some(ModemPPPError::Device),
                ModemEvent::ErrorAlloc => guard.error = Some(ModemPPPError::Alloc),
                ModemEvent::ErrorUser => guard.error = Some(ModemPPPError::User),
                ModemEvent::ErrorDisconnect => guard.error = Some(ModemPPPError::Disconnect),
                ModemEvent::ErrorAuthFail => guard.error = Some(ModemPPPError::AuthFail),
                ModemEvent::ErrorProtocol => guard.error = Some(ModemPPPError::Protocol),
                ModemEvent::ErrorPeerDead => guard.error = Some(ModemPPPError::PeerDead),
                ModemEvent::ErrorIdleTimeout => guard.error = Some(ModemPPPError::IdleTimeout),
                ModemEvent::ErrorMaxConnectTimeout => {
                    guard.error = Some(ModemPPPError::MaxConnectTimeout)
                }
                ModemEvent::ErrorLoopback => guard.error = Some(ModemPPPError::Loopback),
                ModemEvent::PhaseDead => guard.phase = ModemPhaseStatus::Dead,
                ModemEvent::PhaseMaster => guard.phase = ModemPhaseStatus::Master,
                ModemEvent::PhaseHoldoff => guard.phase = ModemPhaseStatus::Holdoff,
                ModemEvent::PhaseInitialize => guard.phase = ModemPhaseStatus::Initialize,
                ModemEvent::PhaseSerialConnection => {
                    guard.phase = ModemPhaseStatus::SerialConnection
                }
                ModemEvent::PhaseDormant => guard.phase = ModemPhaseStatus::Dormant,
                ModemEvent::PhaseEstablish => guard.phase = ModemPhaseStatus::Establish,
                ModemEvent::PhaseAuthenticate => guard.phase = ModemPhaseStatus::Authenticate,
                ModemEvent::PhaseCallback => guard.phase = ModemPhaseStatus::Callback,
                ModemEvent::PhaseNetwork => guard.phase = ModemPhaseStatus::Network,
                ModemEvent::PhaseRunning => guard.phase = ModemPhaseStatus::Running,
                ModemEvent::PhaseTerminate => guard.phase = ModemPhaseStatus::Terminate,
                ModemEvent::PhaseDisconnect => guard.phase = ModemPhaseStatus::Disconnect,
                ModemEvent::PhaseFailed => guard.phase = ModemPhaseStatus::Failed,
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

    fn on_ppp_changed(event_id: u32, _event_data: *mut ::core::ffi::c_void) {
        use log::info;
        info!("Got event id ppp changed: {}", event_id);

        if event_id == esp_netif_ppp_status_event_t_NETIF_PPP_ERRORUSER {
            info!("user interrupted event from netif");
        }
    }

    unsafe extern "C" fn raw_on_ppp_changed(
        _event_handler_arg: *mut ::core::ffi::c_void,
        _event_base: esp_event_base_t,
        event_id: i32,
        event_data: *mut ::core::ffi::c_void,
    ) {
        Self::on_ppp_changed(event_id as _, event_data)
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

#[derive(Debug)]
pub enum ModemEvent {
    ErrorNone,
    ErrorParameter,
    ErrorOpen,
    ErrorDevice,
    ErrorAlloc,
    ErrorUser,
    ErrorDisconnect,
    ErrorAuthFail,
    ErrorProtocol,
    ErrorPeerDead,
    ErrorIdleTimeout,
    ErrorMaxConnectTimeout,
    ErrorLoopback,
    PhaseDead,
    PhaseMaster,
    PhaseHoldoff,
    PhaseInitialize,
    PhaseSerialConnection,
    PhaseDormant,
    PhaseEstablish,
    PhaseAuthenticate,
    PhaseCallback,
    PhaseNetwork,
    PhaseRunning,
    PhaseTerminate,
    PhaseDisconnect,
    PhaseFailed,
}

unsafe impl EspEventSource for ModemEvent {
    fn source() -> Option<&'static core::ffi::CStr> {
        Some(unsafe { ffi::CStr::from_ptr(NETIF_PPP_STATUS) })
    }
}

impl EspEventDeserializer for ModemEvent {
    type Data<'a> = ModemEvent;

    #[allow(non_upper_case_globals, non_snake_case)]
    fn deserialize<'a>(data: &crate::eventloop::EspEvent<'a>) -> Self::Data<'a> {
        let event_id = data.event_id as u32;

        match event_id {
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORNONE => ModemEvent::ErrorNone,
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORPARAM => ModemEvent::ErrorParameter,
            esp_netif_ppp_status_event_t_NETIF_PPP_ERROROPEN => ModemEvent::ErrorOpen,
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORDEVICE => ModemEvent::ErrorDevice,
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORALLOC => ModemEvent::ErrorAlloc,
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORUSER => ModemEvent::ErrorUser,
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORCONNECT => ModemEvent::ErrorDisconnect,
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORAUTHFAIL => ModemEvent::ErrorAuthFail,
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORPROTOCOL => ModemEvent::ErrorProtocol,
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORPEERDEAD => ModemEvent::ErrorPeerDead,
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORIDLETIMEOUT => ModemEvent::ErrorIdleTimeout,
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORCONNECTTIME => {
                ModemEvent::ErrorMaxConnectTimeout
            }
            esp_netif_ppp_status_event_t_NETIF_PPP_ERRORLOOPBACK => ModemEvent::ErrorLoopback,
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_DEAD => ModemEvent::PhaseDead,
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_MASTER => ModemEvent::PhaseMaster,
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_HOLDOFF => ModemEvent::PhaseHoldoff,
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_INITIALIZE => ModemEvent::PhaseInitialize,
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_SERIALCONN => {
                ModemEvent::PhaseSerialConnection
            }
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_DORMANT => ModemEvent::PhaseDormant,
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_ESTABLISH => ModemEvent::PhaseEstablish,
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_AUTHENTICATE => {
                ModemEvent::PhaseAuthenticate
            }
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_CALLBACK => ModemEvent::PhaseCallback,
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_NETWORK => ModemEvent::PhaseNetwork,
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_RUNNING => ModemEvent::PhaseRunning,
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_TERMINATE => ModemEvent::PhaseTerminate,
            esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_DISCONNECT => ModemEvent::PhaseDisconnect,
            esp_netif_ppp_status_event_t_NETIF_PPP_CONNECT_FAILED => ModemEvent::PhaseFailed,
            _ => panic!("Unknown event ID: {}", event_id),
        }
    }
}

pub mod sim {
    //! SimModem
    //!
    //! Models a modem device with a sim card able to serve as a
    //! network interface for the host.
    use esp_idf_hal::uart::UartDriver;

    /// The generic device trait. Implementations of this trait should provide
    /// relevant AT commands and confirm the modem replies to drive the modem
    /// into PPPoS (data mode).
    pub trait SimModem {
        /// The current mode of the sim modem.
        fn get_mode(&self) -> &CommunicationMode;

        /// Initialise the remote modem so that it is in PPPoS mode.
        fn negotiate(&mut self, comm: &mut UartDriver, buffer: [u8; 64]) -> Result<(), ModemError>;
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

        use core::{fmt::Display, time::Duration};

        use at_commands::{builder::CommandBuilder, parser::CommandParser};
        use esp_idf_hal::{delay::TickType, uart::UartDriver};

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
            fn negotiate(
                &mut self,
                comm: &mut UartDriver,
                mut buffer: [u8; 64],
            ) -> Result<(), ModemError> {
                reset(comm, &mut buffer)?;

                //disable echo
                set_echo(comm, &mut buffer, false)?;

                // get signal quality
                let (rssi, ber) = get_signal_quality(comm, &mut buffer)?;
                log::info!("RSSI = {rssi}");
                log::info!("BER = {ber}");
                // get iccid
                let iccid = get_iccid(comm, &mut buffer)?;
                log::info!("ICCID = [{}]", iccid);

                // check pdp network reg
                read_gprs_registration_status(comm, &mut buffer)?;

                //configure apn
                set_pdp_context(comm, &mut buffer)?;

                // start ppp
                set_data_mode(comm, &mut buffer)?;

                self.0 = CommunicationMode::Data;
                Ok(())
            }

            fn get_mode(&self) -> &CommunicationMode {
                &self.0
            }
        }

        pub fn get_signal_quality(
            comm: &mut UartDriver,
            buff: &mut [u8],
        ) -> Result<(RSSI, BitErrorRate), ModemError> {
            let cmd = CommandBuilder::create_execute(buff, true)
                .named("+CSQ")
                .finish()?;

            comm.write(cmd).map_err(|_| ModemError::IO)?;

            let len = comm
                .read(buff, TickType::new_millis(1000).ticks())
                .map_err(|_| ModemError::IO)?;

            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));

            // \r\n+CSQ: 19,99\r\n\r\nOK\r\n
            let (raw_rssi, raw_ber) = CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\n+CSQ: ")
                .expect_int_parameter()
                .expect_int_parameter()
                .expect_identifier(b"\r\n\r\nOK\r\n")
                .finish()?;

            Ok((RSSI::parse(raw_rssi), raw_ber.into()))
        }

        fn get_iccid(
            comm: &mut UartDriver,
            buff: &mut [u8],
        ) -> Result<heapless::String<22>, ModemError> {
            let cmd = CommandBuilder::create_execute(buff, true)
                .named("+CICCID")
                .finish()?;

            comm.write(cmd).map_err(|_| ModemError::IO)?;

            let len = comm
                .read(buff, TickType::new_millis(1000).ticks())
                .map_err(|_| ModemError::IO)?;
            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));

            let (ccid,) = CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\n+ICCID: ")
                .expect_raw_string()
                .expect_identifier(b"\r\n\r\nOK\r\n")
                .finish()?;

            Ok(heapless::String::try_from(ccid).unwrap())
        }

        fn reset(comm: &mut UartDriver, buff: &mut [u8]) -> Result<(), ModemError> {
            let cmd = CommandBuilder::create_execute(buff, false)
                .named("ATZ0")
                .finish()?;
            log::info!("Send Reset");

            comm.write(cmd).map_err(|_| ModemError::IO)?;

            let len = comm
                .read(buff, TickType::new_millis(1000).ticks())
                .map_err(|_| ModemError::IO)?;
            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));
            CommandParser::parse(&buff[..len])
                .expect_identifier(b"ATZ0\r")
                .expect_identifier(b"\r\nOK\r\n")
                .finish()?;
            Ok(())
        }

        fn set_echo(comm: &mut UartDriver, buff: &mut [u8], echo: bool) -> Result<(), ModemError> {
            let cmd = CommandBuilder::create_execute(buff, false)
                .named(format!("ATE{}", i32::from(echo)))
                .finish()?;
            log::info!("Set echo ");
            comm.write(cmd).map_err(|_| ModemError::IO)?;
            let len = comm
                .read(buff, TickType::new_millis(1000).ticks())
                .map_err(|_| ModemError::IO)?;
            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));

            CommandParser::parse(&buff[..len])
                .expect_identifier(b"ATE0\r")
                .expect_identifier(b"\r\nOK\r\n")
                .finish()?;
            Ok(())
        }

        fn read_gprs_registration_status(
            comm: &mut UartDriver,
            buff: &mut [u8],
        ) -> Result<(i32, i32, Option<i32>, Option<i32>), ModemError> {
            let cmd = CommandBuilder::create_query(buff, true)
                .named("+CGREG")
                .finish()?;
            log::info!("Get Registration Status");
            comm.write(cmd).map_err(|_| ModemError::IO)?;
            let len = comm
                .read(buff, TickType::new_millis(1000).ticks())
                .map_err(|_| ModemError::IO)?;
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

        fn set_pdp_context(comm: &mut UartDriver, buff: &mut [u8]) -> Result<(), ModemError> {
            let cmd = CommandBuilder::create_set(buff, true)
                .named("+CGDCONT")
                .with_int_parameter(1) // context id
                .with_string_parameter("IP") // pdp type
                .with_string_parameter("flolive.net") // apn
                .finish()?;
            log::info!("Set PDP Context");
            comm.write(cmd).map_err(|_| ModemError::IO)?;
            let len = comm
                .read(buff, TickType::new_millis(1000).ticks())
                .map_err(|_| ModemError::IO)?;
            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));

            CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\nOK\r\n")
                .finish()?;
            Ok(())
        }

        fn set_data_mode(comm: &mut UartDriver, buff: &mut [u8]) -> Result<(), ModemError> {
            let cmd = CommandBuilder::create_execute(buff, false)
                .named("ATD*99#")
                .finish()?;
            log::info!("Set Data mode");
            comm.write(cmd).map_err(|_| ModemError::IO)?;
            let len = comm
                .read(buff, TickType::new_millis(1000).ticks())
                .map_err(|_| ModemError::IO)?;
            log::info!("got response{:?}", std::str::from_utf8(&buff[..len]));

            let (connect_parm,) = CommandParser::parse(&buff[..len])
                .expect_identifier(b"\r\nCONNECT ")
                .expect_optional_raw_string()
                .expect_identifier(b"\r\n")
                .finish()?;
            log::info!("connect {:?}", connect_parm);
            Ok(())
        }
    }
}
