//! Safe wrapper for the ESP-IDF NimBLE BLE host.

use core::cell::UnsafeCell;
use core::ffi::{c_int, c_void};
use core::fmt;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, Ordering};

use alloc::boxed::Box;
use alloc::sync::Arc;

use crate::hal::modem::BluetoothModemPeripheral;
use crate::private::mutex::Mutex;
use crate::sys::*;

pub mod gap;
pub mod gatt;
#[cfg(any(esp_idf_bt_nimble_gatt_server, esp_idf_bt_nimble_gatt_client))]
pub mod mbuf;

/// A connection handle (NimBLE's `conn_handle`). A *connection* is a cross-cutting concept — born
/// at the GAP layer and used by the GATT server, the GATT client, and L2CAP — so it lives at the
/// root rather than in any one subsystem module.
pub type ConnHandle = u16;

/// A BLE UUID, either 16-bit (assigned) or 128-bit (vendor-specific).
#[derive(Clone, Copy, Debug)]
pub enum BleUuid {
    Uuid16(ble_uuid16_t),
    Uuid128(ble_uuid128_t),
}

impl BleUuid {
    pub const fn uuid16(uuid: u16) -> Self {
        Self::Uuid16(ble_uuid16_t {
            u: ble_uuid_t {
                type_: BLE_UUID_TYPE_16 as u8,
            },
            value: uuid,
        })
    }

    pub const fn uuid128(uuid: u128) -> Self {
        Self::Uuid128(ble_uuid128_t {
            u: ble_uuid_t {
                type_: BLE_UUID_TYPE_128 as u8,
            },
            value: uuid.to_le_bytes(),
        })
    }

    pub fn as_ptr(&self) -> *const ble_uuid_t {
        match self {
            Self::Uuid16(uuid) => &uuid.u as *const ble_uuid_t,
            Self::Uuid128(uuid) => &uuid.u as *const ble_uuid_t,
        }
    }

    /// # Safety
    ///
    /// `uuid` must point to a valid `ble_uuid_t` header and the concrete
    /// 16-/128-bit body it introduces.
    pub(crate) unsafe fn from_raw(uuid: *const ble_uuid_t) -> Self {
        match unsafe { (*uuid).type_ } as u32 {
            BLE_UUID_TYPE_128 => Self::Uuid128(unsafe { *uuid.cast::<ble_uuid128_t>() }),
            // Only 16- and 128-bit UUIDs are modelled; anything else reads as 16-bit.
            _ => Self::Uuid16(unsafe { *uuid.cast::<ble_uuid16_t>() }),
        }
    }
}

impl PartialEq for BleUuid {
    fn eq(&self, other: &Self) -> bool {
        unsafe { ble_uuid_cmp(self.as_ptr(), other.as_ptr()) == 0 }
    }
}

impl Eq for BleUuid {}

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct BleAddr(ble_addr_t);

impl BleAddr {
    pub const fn new(kind: u8, val: [u8; 6]) -> Self {
        Self(ble_addr_t { type_: kind, val })
    }

    pub const fn raw(&self) -> &ble_addr_t {
        &self.0
    }

    pub const fn kind(&self) -> u8 {
        self.0.type_
    }

    pub const fn val(&self) -> [u8; 6] {
        self.0.val
    }
}

impl fmt::Display for BleAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = &self.0.val;
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            v[5], v[4], v[3], v[2], v[1], v[0]
        )
    }
}

impl From<ble_addr_t> for BleAddr {
    fn from(addr: ble_addr_t) -> Self {
        Self(addr)
    }
}

impl From<BleAddr> for ble_addr_t {
    fn from(addr: BleAddr) -> Self {
        addr.0
    }
}

/// Attempt to configure at least one BLE address; how this is done is hardware-specific.
/// If prefer_random is true, prefer using a random address even if a public address is configured.
pub fn ensure_addr(prefer_random: bool) -> Result<(), BleError> {
    BleError::from_raw(unsafe { ble_hs_util_ensure_addr(prefer_random as c_int) })
}

/// Read back the device's identity address of the given type.
pub fn id_copy_addr(kind: u8) -> Result<BleAddr, BleError> {
    let mut val = [0u8; 6];
    BleError::from_raw(unsafe {
        ble_hs_id_copy_addr(kind, val.as_mut_ptr(), core::ptr::null_mut())
    })?;

    Ok(BleAddr::new(kind, val))
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct BleError(c_int);

impl BleError {
    pub const fn new(rc: c_int) -> Self {
        Self(rc)
    }

    pub const fn code(&self) -> c_int {
        self.0
    }

    pub fn from_raw(rc: c_int) -> Result<(), Self> {
        if rc == 0 {
            Ok(())
        } else {
            Err(Self(rc))
        }
    }

    fn name(&self) -> &'static str {
        match self.0 as u32 {
            BLE_HS_EALREADY => "BLE_HS_EALREADY",
            BLE_HS_EDONE => "BLE_HS_EDONE",
            BLE_HS_ENOMEM => "BLE_HS_ENOMEM",
            BLE_HS_ETIMEOUT => "BLE_HS_ETIMEOUT",
            _ => "BLE_HS_E*",
        }
    }
}

impl fmt::Debug for BleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BleError({}, {})", self.0, self.name())
    }
}

impl fmt::Display for BleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NimBLE error {} ({})", self.0, self.name())
    }
}

#[cfg(feature = "std")]
impl std::error::Error for BleError {}

impl From<BleError> for EspError {
    /// NimBLE host codes (`BLE_HS_E*`) are a separate namespace from `esp_err_t`
    /// with no faithful mapping, so any [`BleError`] collapses to `ESP_FAIL`. Match
    /// on the [`BleError`] directly if you need the specific NimBLE code.
    fn from(_err: BleError) -> Self {
        EspError::from_infallible::<ESP_FAIL>()
    }
}

/// Security Manager (SMP) configuration, applied via
/// [`BleDriver::set_security`](BleDriver::set_security) before the host starts.
#[derive(Clone, Copy)]
pub struct BleSecurity {
    /// Local IO capabilities (`BLE_HS_IO_*`).
    pub io_cap: u8,
    pub oob_data_flag: bool,
    pub bonding: bool,
    pub mitm: bool,
    /// LE Secure Connections.
    pub secure_connections: bool,
    /// Restrict pairing to LE Secure Connections only.
    pub secure_connections_only: bool,
    pub keypress: bool,
    /// Minimum GATT security level (`sm_sec_lvl`); 0 is ignored.
    pub min_sec_level: u8,
    /// Keys we distribute (`BLE_SM_PAIR_KEY_DIST_*` mask).
    pub our_key_dist: u8,
    /// Keys the peer distributes (`BLE_SM_PAIR_KEY_DIST_*` mask).
    pub their_key_dist: u8,
}

impl Default for BleSecurity {
    fn default() -> Self {
        Self {
            io_cap: BLE_HS_IO_NO_INPUT_OUTPUT as u8,
            oob_data_flag: false,
            bonding: false,
            mitm: false,
            secure_connections: false,
            secure_connections_only: false,
            keypress: false,
            min_sec_level: 0,
            our_key_dist: 0,
            their_key_dist: 0,
        }
    }
}

/// Host-lifecycle events, delivered to the [`host_subscribe`](BleDriver::host_subscribe) hook.
pub enum HostEvent {
    /// The host has synchronized with the controller and is ready for BLE operations. Re-entrant:
    /// fires again after a [`Reset`](Self::Reset).
    Sync,
    /// The host reset (e.g. a fatal controller error), carrying the reason code. A
    /// [`Sync`](Self::Sync) follows once the stack re-synchronizes.
    Reset { reason: i32 },
}

#[allow(dead_code)]
#[allow(clippy::type_complexity)]
pub(crate) struct BleCallback<A, R> {
    callback: Mutex<Option<Arc<UnsafeCell<Box<dyn FnMut(A) -> R>>>>>,
    default_result: R,
}

#[allow(dead_code)]
impl<A, R> BleCallback<A, R>
where
    R: Clone,
{
    pub const fn new(default_result: R) -> Self {
        Self {
            callback: Mutex::new(None),
            default_result,
        }
    }

    pub fn subscribe<F>(&self, callback: F)
    where
        F: FnMut(A) -> R + Send + 'static,
    {
        unsafe { self.subscribe_nonstatic(callback) }
    }

    /// # Safety
    ///
    /// The stored slot is `'static`; this erases the callback's lifetime. The
    /// caller must ensure the callback (and everything it borrows) stays valid
    /// until it is unsubscribed via `unsubscribe`.
    pub unsafe fn subscribe_nonstatic<'a, F>(&self, callback: F)
    where
        F: FnMut(A) -> R + Send + 'a,
    {
        let callback: Box<dyn FnMut(A) -> R + 'a> = Box::new(callback);
        let callback: Box<dyn FnMut(A) -> R + 'static> = unsafe { core::mem::transmute(callback) };
        *self.callback.lock() = Some(Arc::new(UnsafeCell::new(callback)));
    }

    pub fn unsubscribe(&self) {
        *self.callback.lock() = None;
    }

    /// # Safety
    ///
    /// Safe to use only from within the NimBLE host task.
    pub unsafe fn call(&self, arg: A) -> R {
        if let Some(callback) = self
            .callback
            .lock()
            .as_ref()
            .map(|callback| callback.clone())
        {
            ((callback.get()).as_mut().unwrap())(arg)
        } else {
            self.default_result.clone()
        }
    }
}

unsafe impl<A, R> Sync for BleCallback<A, R> {}
unsafe impl<A, R> Send for BleCallback<A, R> {}

/// The GATT-server hook. Unlike [`BleCallback`], the argument
/// [`GattsEvent`](gatt::server::GattsEvent) is lifetime-parametrized (its `Read`/`Write` variants
/// borrow the operation's mbuf, valid only for the duration of the call), so the stored closure is
/// higher-ranked over that lifetime. The return value is the ATT status for `Read`/`Write` and is
/// ignored for the registration events.
#[cfg(esp_idf_bt_nimble_gatt_server)]
#[allow(clippy::type_complexity)]
pub(crate) struct GattsCallback {
    callback: Mutex<
        Option<Arc<UnsafeCell<Box<dyn for<'a> FnMut(gatt::server::GattsEvent<'a>) -> u8 + Send>>>>,
    >,
}

#[cfg(esp_idf_bt_nimble_gatt_server)]
impl GattsCallback {
    pub const fn new() -> Self {
        Self {
            callback: Mutex::new(None),
        }
    }

    /// # Safety
    ///
    /// See [`BleCallback::subscribe_nonstatic`]; the stored slot is `'static` and this erases the
    /// callback's capture lifetime.
    // `GattsCallback` is `unsafe impl Send + Sync` below (accessed only from the host task); the
    // `Arc<UnsafeCell<..>>` is the same re-entrancy mechanism as `BleCallback`, which escapes this
    // lint only because it is generic.
    #[allow(clippy::arc_with_non_send_sync)]
    pub unsafe fn subscribe_nonstatic<'a, F>(&self, callback: F)
    where
        F: for<'e> FnMut(gatt::server::GattsEvent<'e>) -> u8 + Send + 'a,
    {
        let callback: Box<dyn for<'e> FnMut(gatt::server::GattsEvent<'e>) -> u8 + Send + 'a> =
            Box::new(callback);
        let callback: Box<dyn for<'e> FnMut(gatt::server::GattsEvent<'e>) -> u8 + Send + 'static> =
            unsafe { core::mem::transmute(callback) };
        *self.callback.lock() = Some(Arc::new(UnsafeCell::new(callback)));
    }

    pub fn unsubscribe(&self) {
        *self.callback.lock() = None;
    }

    /// # Safety
    ///
    /// Safe to use only from within the NimBLE host task.
    pub unsafe fn call(&self, event: gatt::server::GattsEvent<'_>) -> u8 {
        if let Some(callback) = self
            .callback
            .lock()
            .as_ref()
            .map(|callback| callback.clone())
        {
            unsafe { ((callback.get()).as_mut().unwrap())(event) }
        } else {
            0
        }
    }
}

#[cfg(esp_idf_bt_nimble_gatt_server)]
unsafe impl Sync for GattsCallback {}
#[cfg(esp_idf_bt_nimble_gatt_server)]
unsafe impl Send for GattsCallback {}

/// The GATT-client hook — the dual of [`GattsCallback`] for the client side. Its argument
/// [`GattcEvent`](gatt::client::GattcEvent) is likewise higher-ranked (its `ReadComplete`/`Notify`
/// variants borrow an mbuf). It has no return value: the client produces no ATT responses.
#[cfg(esp_idf_bt_nimble_gatt_client)]
#[allow(clippy::type_complexity)]
pub(crate) struct GattcCallback {
    callback:
        Mutex<Option<Arc<UnsafeCell<Box<dyn for<'a> FnMut(gatt::client::GattcEvent<'a>) + Send>>>>>,
}

#[cfg(esp_idf_bt_nimble_gatt_client)]
impl GattcCallback {
    pub const fn new() -> Self {
        Self {
            callback: Mutex::new(None),
        }
    }

    /// # Safety
    ///
    /// See [`GattsCallback::subscribe_nonstatic`].
    #[allow(clippy::arc_with_non_send_sync)]
    pub unsafe fn subscribe_nonstatic<'a, F>(&self, callback: F)
    where
        F: for<'e> FnMut(gatt::client::GattcEvent<'e>) + Send + 'a,
    {
        let callback: Box<dyn for<'e> FnMut(gatt::client::GattcEvent<'e>) + Send + 'a> =
            Box::new(callback);
        let callback: Box<dyn for<'e> FnMut(gatt::client::GattcEvent<'e>) + Send + 'static> =
            unsafe { core::mem::transmute(callback) };
        *self.callback.lock() = Some(Arc::new(UnsafeCell::new(callback)));
    }

    pub fn unsubscribe(&self) {
        *self.callback.lock() = None;
    }

    /// # Safety
    ///
    /// Safe to use only from within the NimBLE host task.
    pub unsafe fn call(&self, event: gatt::client::GattcEvent<'_>) {
        if let Some(callback) = self
            .callback
            .lock()
            .as_ref()
            .map(|callback| callback.clone())
        {
            unsafe { ((callback.get()).as_mut().unwrap())(event) }
        }
    }
}

#[cfg(esp_idf_bt_nimble_gatt_client)]
unsafe impl Sync for GattcCallback {}
#[cfg(esp_idf_bt_nimble_gatt_client)]
unsafe impl Send for GattcCallback {}

/// The NimBLE stack has several globally-singleton things; we enforce that by
/// the calling take/release on this. BleSingleton also wraps the globally singleton state
/// that requires well-known static addresses.
#[allow(dead_code)]
pub(crate) struct BleSingleton {
    initialized: AtomicBool,
    host: BleCallback<HostEvent, ()>,
    gap: BleCallback<gap::GapEvent, i32>,
    #[cfg(esp_idf_bt_nimble_gatt_server)]
    gatts: GattsCallback,
    #[cfg(esp_idf_bt_nimble_gatt_client)]
    gattc: GattcCallback,
}

#[allow(dead_code)]
impl BleSingleton {
    pub const fn new() -> Self {
        Self {
            initialized: AtomicBool::new(false),
            host: BleCallback::new(()),
            gap: BleCallback::new(0),
            #[cfg(esp_idf_bt_nimble_gatt_server)]
            gatts: GattsCallback::new(),
            #[cfg(esp_idf_bt_nimble_gatt_client)]
            gattc: GattcCallback::new(),
        }
    }

    pub fn take(&self) -> Result<(), EspError> {
        self.initialized
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| EspError::from_infallible::<ESP_ERR_INVALID_STATE>())?;

        Ok(())
    }

    pub fn release(&self) -> Result<(), EspError> {
        self.initialized
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| EspError::from_infallible::<ESP_ERR_INVALID_STATE>())?;

        Ok(())
    }

    // The `unsafe extern "C"` trampolines NimBLE calls into. They are grouped here as associated
    // functions — C callbacks take no `self`, so each reads the one global `SINGLETON` — since they
    // all dispatch through it. (They cannot live on `BleDriver`, which is generic.)

    unsafe extern "C" fn host_sync_cb() {
        unsafe { SINGLETON.host.call(HostEvent::Sync) }
    }

    unsafe extern "C" fn host_reset_cb(reason: i32) {
        unsafe { SINGLETON.host.call(HostEvent::Reset { reason }) }
    }

    /// The connection event callback (wired at `adv_start` for a server, at `connect` for a
    /// client). NimBLE multiplexes role-specific events onto it, so we **demux by role**: the
    /// role-agnostic connection events go to the GAP hook, the server-role events
    /// (`Subscribe`/`NotifyComplete`) to the GATTS hook, and the client-role event (`Notify`) to
    /// the GATTC hook.
    unsafe extern "C" fn gap_event_cb(event: *mut ble_gap_event, _arg: *mut c_void) -> c_int {
        let event = unsafe { &*event };

        match event.type_ as u32 {
            #[cfg(esp_idf_bt_nimble_gatt_server)]
            BLE_GAP_EVENT_SUBSCRIBE | BLE_GAP_EVENT_NOTIFY_TX => {
                if let Some(event) = gatt::server::GattsEvent::from_gap(event) {
                    unsafe { SINGLETON.gatts.call(event) };
                }
                0
            }
            #[cfg(esp_idf_bt_nimble_gatt_client)]
            BLE_GAP_EVENT_NOTIFY_RX => {
                unsafe {
                    SINGLETON
                        .gattc
                        .call(gatt::client::GattcEvent::from_notify_rx(event))
                };
                0
            }
            _ => unsafe { SINGLETON.gap.call(gap::GapEvent::from(event)) },
        }
    }

    #[cfg(esp_idf_bt_nimble_gatt_server)]
    unsafe extern "C" fn gatts_register_cb(ctxt: *mut ble_gatt_register_ctxt, _arg: *mut c_void) {
        let event =
            gatt::server::GattsEvent::Register(gatt::server::BleGattRegister::from(unsafe {
                &*ctxt
            }));

        // The registration events carry no reply; the hook's status return is ignored.
        unsafe {
            SINGLETON.gatts.call(event);
        }
    }

    /// The single access trampoline shared by *every* characteristic — NimBLE dispatches reads and
    /// writes here, and we route them to the one [`gatts_subscribe`](BleDriver::gatts_subscribe) hook,
    /// keyed by the (globally unique) `attr_handle`. There are no per-characteristic closures.
    #[cfg(esp_idf_bt_nimble_gatt_server)]
    unsafe extern "C" fn gatts_access_cb(
        conn_handle: u16,
        attr_handle: u16,
        ctxt: *mut ble_gatt_access_ctxt,
        _arg: *mut c_void,
    ) -> c_int {
        let mbuf = mbuf::Mbuf::from_raw(unsafe { (*ctxt).om });

        let event = match unsafe { (*ctxt).op } as u32 {
            BLE_GATT_ACCESS_OP_READ_CHR => gatt::server::GattsEvent::Read {
                conn_handle,
                attr_handle,
                reply: mbuf,
            },
            BLE_GATT_ACCESS_OP_WRITE_CHR => gatt::server::GattsEvent::Write {
                conn_handle,
                attr_handle,
                data: mbuf,
            },
            _ => return BLE_ATT_ERR_UNLIKELY as c_int,
        };

        unsafe { SINGLETON.gatts.call(event) as c_int }
    }

    // The GATT-client per-operation completion trampolines. NimBLE's `ble_gattc_*` calls each take
    // a callback; we pass the matching one of these, and it routes the completion to the single
    // GATTC hook. Discovery fires one event per item, then a final one with a `None` payload.

    #[cfg(esp_idf_bt_nimble_gatt_client)]
    unsafe extern "C" fn gattc_disc_svc_cb(
        conn_handle: u16,
        error: *const ble_gatt_error,
        service: *const ble_gatt_svc,
        _arg: *mut c_void,
    ) -> c_int {
        let status = if error.is_null() {
            0
        } else {
            unsafe { (*error).status }
        };
        let service =
            (!service.is_null()).then(|| gatt::client::GattcService::from(unsafe { &*service }));

        unsafe {
            SINGLETON.gattc.call(gatt::client::GattcEvent::Service {
                conn_handle,
                status,
                service,
            });
        }
        0
    }

    #[cfg(esp_idf_bt_nimble_gatt_client)]
    unsafe extern "C" fn gattc_disc_chr_cb(
        conn_handle: u16,
        error: *const ble_gatt_error,
        chr: *const ble_gatt_chr,
        _arg: *mut c_void,
    ) -> c_int {
        let status = if error.is_null() {
            0
        } else {
            unsafe { (*error).status }
        };
        let chr = (!chr.is_null()).then(|| gatt::client::GattcChr::from(unsafe { &*chr }));

        unsafe {
            SINGLETON
                .gattc
                .call(gatt::client::GattcEvent::Characteristic {
                    conn_handle,
                    status,
                    chr,
                });
        }
        0
    }

    #[cfg(esp_idf_bt_nimble_gatt_client)]
    unsafe extern "C" fn gattc_read_cb(
        conn_handle: u16,
        error: *const ble_gatt_error,
        attr: *mut ble_gatt_attr,
        _arg: *mut c_void,
    ) -> c_int {
        let status = if error.is_null() {
            0
        } else {
            unsafe { (*error).status }
        };
        let (attr_handle, om) = if attr.is_null() {
            (0, core::ptr::null_mut())
        } else {
            unsafe { ((*attr).handle, (*attr).om) }
        };

        unsafe {
            SINGLETON
                .gattc
                .call(gatt::client::GattcEvent::ReadComplete {
                    conn_handle,
                    status,
                    attr_handle,
                    data: mbuf::Mbuf::from_raw(om),
                });
        }
        0
    }

    #[cfg(esp_idf_bt_nimble_gatt_client)]
    unsafe extern "C" fn gattc_write_cb(
        conn_handle: u16,
        error: *const ble_gatt_error,
        attr: *mut ble_gatt_attr,
        _arg: *mut c_void,
    ) -> c_int {
        let status = if error.is_null() {
            0
        } else {
            unsafe { (*error).status }
        };
        let attr_handle = if attr.is_null() {
            0
        } else {
            unsafe { (*attr).handle }
        };

        unsafe {
            SINGLETON
                .gattc
                .call(gatt::client::GattcEvent::WriteComplete {
                    conn_handle,
                    status,
                    attr_handle,
                });
        }
        0
    }

    unsafe extern "C" fn host_task(_arg: *mut c_void) {
        unsafe {
            nimble_port_run();
            nimble_port_freertos_deinit();
        }
    }
}

static SINGLETON: BleSingleton = BleSingleton::new();

/// Shared host initialization for both constructors: `nimble_port_init` + the standard GAP/GATT
/// service init, gated by the singleton `take`. Does **not** start the host task.
fn host_init<M: BluetoothModemPeripheral>(_modem: M) -> Result<(), EspError> {
    SINGLETON.take()?;

    esp!(unsafe { nimble_port_init() })?;

    unsafe {
        ble_svc_gap_init();
        ble_svc_gatt_init();
    }

    Ok(())
}

/// The NimBLE host handle and primary entrypoint to BLE.
///
/// It is **role-agnostic**: the type parameter `S` is the GATT-server service table, defaulting to
/// `()` (no server). A central or broadcaster uses [`new`](Self::new) (`S = ()`); a GATT server
/// uses [`new_with_services`](Self::new_with_services), whose `S: Deref<Target = [ble_gatt_svc_def]>`
/// owns the table and keeps it alive — drop order guarantees `nimble_port_deinit` (in `Drop`) runs
/// before the table field is freed, so NimBLE never sees a dangling pointer.
///
/// The GAP / GATT-server / GATT-client operations are grouped into separate `impl` blocks
/// (`gap.rs`, `gatt/gatts.rs`, and — later — the client), each mirroring a NimBLE subsystem; the
/// GATT ones are `#[cfg]`-gated on the corresponding Kconfig. `start` takes `&self` (interior
/// started-flag).
pub struct BleDriver<'ble, S = ()> {
    started: AtomicBool,
    // Owns the GATT service table (if any). Declared before `_p`; dropped *after* `Drop::drop`
    // runs `nimble_port_deinit`, so the table outlives NimBLE's pointers into it.
    _services: S,
    _p: PhantomData<&'ble mut ()>,
}

impl<'ble> BleDriver<'ble, ()> {
    /// Initialize the NimBLE host with **no GATT server** — the role-agnostic form used by a
    /// central, a broadcaster, or an observer. Performs `nimble_port_init` and the standard
    /// GAP/GATT service init, but does **not** start the host task; configure callbacks/security,
    /// then call [`start`](Self::start).
    pub fn new<M: BluetoothModemPeripheral + 'ble>(modem: M) -> Result<Self, EspError> {
        host_init(modem)?;

        Ok(Self {
            started: AtomicBool::new(false),
            _services: (),
            _p: PhantomData,
        })
    }
}

#[cfg(esp_idf_bt_nimble_gatt_server)]
impl<'ble, S> BleDriver<'ble, S>
where
    S: core::ops::Deref<Target = [ble_gatt_svc_def]>,
{
    /// Initialize the NimBLE host as a **GATT server**, registering `services` in NimBLE's
    /// pre-start window (this is why service registration is a construction concern, not a runtime
    /// one — see [`ble_gatts_add_svcs`]). `S` may be an owned bundle (e.g.
    /// [`BleGattServices`](gatt::server::BleGattServices)), a `Box<[ble_gatt_svc_def]>`, or a
    /// `&'static [ble_gatt_svc_def]`; whatever it is, it must keep the *entire* pointer graph the
    /// table references (characteristics, UUIDs) alive and at stable addresses for as long as it is
    /// held. The driver owns it, so drop order does the rest.
    ///
    /// Does not start the host task; hook [`gatts_subscribe`](Self::gatts_subscribe) (to learn the
    /// assigned attribute handles), configure security/callbacks, then call [`start`](Self::start).
    pub fn new_with_services<M: BluetoothModemPeripheral + 'ble>(
        modem: M,
        services: S,
    ) -> Result<Self, EspError> {
        host_init(modem)?;

        // `?` converts `BleError` to `EspError` via `From<BleError>`.
        let defs = services.deref().as_ptr();
        BleError::from_raw(unsafe { ble_gatts_count_cfg(defs) })?;
        BleError::from_raw(unsafe { ble_gatts_add_svcs(defs) })?;

        Ok(Self {
            started: AtomicBool::new(false),
            _services: services,
            _p: PhantomData,
        })
    }
}

impl<'ble, S> BleDriver<'ble, S> {
    /// Subscribe to host-lifecycle events ([`HostEvent`]): `Sync` when the host and controller are
    /// synchronized (you must delay BLE operations until then), and `Reset` when the host resets.
    /// The hook must be re-entrant — a reset is followed by another `Sync` once re-synced.
    /// See https://mynewt.apache.org/latest/network/ble_setup/ble_sync_cb.html
    pub fn host_subscribe<F>(&self, callback: F)
    where
        F: FnMut(HostEvent) + Send + 'static,
    {
        unsafe { self.host_subscribe_nonstatic(callback) }
    }

    /// # Safety
    ///
    /// The non-`'static` counterpart of [`host_subscribe`](Self::host_subscribe): the callback may
    /// borrow variables that live as long as this [`BleDriver`]. It stays registered with the
    /// running NimBLE host task until the driver is dropped, which un-subscribes it.
    ///
    /// Care must be taken NOT to `core::mem::forget` the driver: that skips the un-subscription,
    /// leaving the host task holding a callback with dangling borrows. This "local borrowing" can
    /// only be expressed safely once/if `!Leak` types are introduced to Rust.
    pub unsafe fn host_subscribe_nonstatic<F>(&self, callback: F)
    where
        F: FnMut(HostEvent) + Send + 'ble,
    {
        unsafe { SINGLETON.host.subscribe_nonstatic(callback) };

        unsafe {
            let cfg = core::ptr::addr_of_mut!(ble_hs_cfg);
            (*cfg).sync_cb = Some(BleSingleton::host_sync_cb);
            (*cfg).reset_cb = Some(BleSingleton::host_reset_cb);
        }
    }

    /// Stop delivering host-lifecycle events to the subscribed hook.
    pub fn host_unsubscribe(&self) {
        SINGLETON.host.unsubscribe();
    }

    /// Configure the Security Manager (SMP) parameters. Must be called before
    /// [`start`](Self::start); the settings take effect once the host task runs.
    pub fn set_security(&self, security: &BleSecurity) {
        unsafe {
            let cfg = core::ptr::addr_of_mut!(ble_hs_cfg);
            (*cfg).sm_io_cap = security.io_cap;
            (*cfg).set_sm_oob_data_flag(security.oob_data_flag as _);
            (*cfg).set_sm_bonding(security.bonding as _);
            (*cfg).set_sm_mitm(security.mitm as _);
            (*cfg).set_sm_sc(security.secure_connections as _);
            (*cfg).set_sm_sc_only(security.secure_connections_only as _);
            (*cfg).set_sm_keypress(security.keypress as _);
            (*cfg).sm_sec_lvl = security.min_sec_level;
            (*cfg).sm_our_key_dist = security.our_key_dist;
            (*cfg).sm_their_key_dist = security.their_key_dist;
        }
    }

    /// Start the NimBLE host task. It runs in the background and calls the
    /// [`Sync`](HostEvent::Sync) via [`host_subscribe`](Self::host_subscribe) callback once the stack is ready for use. Call this once
    /// services, security and callbacks are set up; you must retain the driver, as the BLE stack
    /// is stopped when it drops.
    ///
    /// Takes `&self` (flipping an interior started-flag) rather than consuming the driver, so the
    /// service table it owns and every subscribed callback stay put across the call.
    pub fn start(&self) -> Result<(), EspError> {
        unsafe { nimble_port_freertos_init(Some(BleSingleton::host_task)) };

        self.started.store(true, Ordering::SeqCst);

        Ok(())
    }
}

impl<S> Drop for BleDriver<'_, S> {
    fn drop(&mut self) {
        if self.started.load(Ordering::SeqCst) {
            let _ = unsafe { nimble_port_stop() };
        }

        // Tears down the whole host, including the GATT database — after this NimBLE holds no more
        // pointers into the `_services` table, which is dropped *after* this `Drop::drop` returns.
        esp!(unsafe { nimble_port_deinit() }).unwrap();

        unsafe {
            let cfg = core::ptr::addr_of_mut!(ble_hs_cfg);
            (*cfg).sync_cb = None;
            (*cfg).reset_cb = None;
            #[cfg(esp_idf_bt_nimble_gatt_server)]
            {
                (*cfg).gatts_register_cb = None;
            }
        }

        SINGLETON.host.unsubscribe();
        SINGLETON.gap.unsubscribe();
        #[cfg(esp_idf_bt_nimble_gatt_server)]
        SINGLETON.gatts.unsubscribe();
        #[cfg(esp_idf_bt_nimble_gatt_client)]
        SINGLETON.gattc.unsubscribe();
        let _ = SINGLETON.release();
    }
}
