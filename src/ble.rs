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
pub mod mbuf;

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
/// [`GattsEvent`](gatt::gatts::GattsEvent) is lifetime-parametrized (its `Read`/`Write` variants
/// borrow the operation's mbuf, valid only for the duration of the call), so the stored closure is
/// higher-ranked over that lifetime. The return value is the ATT status for `Read`/`Write` and is
/// ignored for the registration events.
#[allow(clippy::type_complexity)]
pub(crate) struct GattsCallback {
    callback: Mutex<
        Option<Arc<UnsafeCell<Box<dyn for<'a> FnMut(gatt::gatts::GattsEvent<'a>) -> u8 + Send>>>>,
    >,
}

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
        F: for<'e> FnMut(gatt::gatts::GattsEvent<'e>) -> u8 + Send + 'a,
    {
        let callback: Box<dyn for<'e> FnMut(gatt::gatts::GattsEvent<'e>) -> u8 + Send + 'a> =
            Box::new(callback);
        let callback: Box<dyn for<'e> FnMut(gatt::gatts::GattsEvent<'e>) -> u8 + Send + 'static> =
            unsafe { core::mem::transmute(callback) };
        *self.callback.lock() = Some(Arc::new(UnsafeCell::new(callback)));
    }

    pub fn unsubscribe(&self) {
        *self.callback.lock() = None;
    }

    /// # Safety
    ///
    /// Safe to use only from within the NimBLE host task.
    pub unsafe fn call(&self, event: gatt::gatts::GattsEvent<'_>) -> u8 {
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

unsafe impl Sync for GattsCallback {}
unsafe impl Send for GattsCallback {}

/// The NimBLE stack has several globally-singleton things; we enforce that by
/// the calling take/release on this. BleSingleton also wraps the globally singleton state
/// that requires well-known static addresses.
#[allow(dead_code)]
pub(crate) struct BleSingleton {
    initialized: AtomicBool,
    sync: BleCallback<(), ()>,
    reset: BleCallback<i32, ()>,
    gap_event: BleCallback<gap::BleGapEvent, i32>,
    gatts: GattsCallback,
}

#[allow(dead_code)]
impl BleSingleton {
    pub const fn new() -> Self {
        Self {
            initialized: AtomicBool::new(false),
            sync: BleCallback::new(()),
            reset: BleCallback::new(()),
            gap_event: BleCallback::new(0),
            gatts: GattsCallback::new(),
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
        unsafe { SINGLETON.sync.call(()) }
    }

    unsafe extern "C" fn host_reset_cb(reason: i32) {
        unsafe { SINGLETON.reset.call(reason) }
    }

    unsafe extern "C" fn gap_event_cb(event: *mut ble_gap_event, _arg: *mut c_void) -> c_int {
        let event = gap::BleGapEvent::from(unsafe { &*event });

        unsafe { SINGLETON.gap_event.call(event) }
    }

    unsafe extern "C" fn gatts_register_cb(ctxt: *mut ble_gatt_register_ctxt, _arg: *mut c_void) {
        let event = gatt::gatts::GattsEvent::Register(gatt::gatts::BleGattRegister::from(unsafe {
            &*ctxt
        }));

        // The registration events carry no reply; the hook's status return is ignored.
        unsafe {
            SINGLETON.gatts.call(event);
        }
    }

    /// The single access trampoline shared by *every* characteristic — NimBLE dispatches reads and
    /// writes here, and we route them to the one [`on_gatts_event`](BleDriver::on_gatts_event) hook,
    /// keyed by the (globally unique) `attr_handle`. There are no per-characteristic closures.
    unsafe extern "C" fn gatts_access_cb(
        conn_handle: u16,
        attr_handle: u16,
        ctxt: *mut ble_gatt_access_ctxt,
        _arg: *mut c_void,
    ) -> c_int {
        let mbuf = mbuf::Mbuf::from_raw(unsafe { (*ctxt).om });

        let event = match unsafe { (*ctxt).op } as u32 {
            BLE_GATT_ACCESS_OP_READ_CHR => gatt::gatts::GattsEvent::Read {
                conn_handle,
                attr_handle,
                reply: mbuf,
            },
            BLE_GATT_ACCESS_OP_WRITE_CHR => gatt::gatts::GattsEvent::Write {
                conn_handle,
                attr_handle,
                data: mbuf,
            },
            _ => return BLE_ATT_ERR_UNLIKELY as c_int,
        };

        unsafe { SINGLETON.gatts.call(event) as c_int }
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
    /// [`BleGattServices`](gatt::gatts::BleGattServices)), a `Box<[ble_gatt_svc_def]>`, or a
    /// `&'static [ble_gatt_svc_def]`; whatever it is, it must keep the *entire* pointer graph the
    /// table references (characteristics, UUIDs) alive and at stable addresses for as long as it is
    /// held. The driver owns it, so drop order does the rest.
    ///
    /// Does not start the host task; hook [`on_gatts_event`](Self::on_gatts_event) (to learn the
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
    /// Once the BLE stack is "in sync", this gets called; you should delay BLE operations until
    /// this gets called. Note the stack can also go out-of-sync for various reasons, indicated by
    /// on_reset being called followed by this being called again, so the callback must be re-entrant.
    /// See https://mynewt.apache.org/latest/network/ble_setup/ble_sync_cb.html
    pub fn on_sync<F>(&self, callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        unsafe { self.on_sync_nonstatic(callback) }
    }

    /// # Safety
    ///
    /// This method - in contrast to `on_sync` - allows a non-`'static` callback,
    /// so it may borrow variables that live as long as this [`BleDriver`]. The callback stays
    /// registered with the running NimBLE host task until the driver is dropped, which
    /// un-subscribes it.
    ///
    /// Care must be taken NOT to `core::mem::forget` the driver: that skips the
    /// un-subscription, leaving the host task holding a callback with dangling
    /// borrows. This "local borrowing" can only be expressed safely once/if `!Leak`
    /// types are introduced to Rust.
    pub unsafe fn on_sync_nonstatic<F>(&self, mut callback: F)
    where
        F: FnMut() + Send + 'ble,
    {
        unsafe { SINGLETON.sync.subscribe_nonstatic(move |()| callback()) };

        unsafe {
            (*core::ptr::addr_of_mut!(ble_hs_cfg)).sync_cb = Some(BleSingleton::host_sync_cb);
        }
    }

    /// Called if the BLE host goes out of sync with the controller after on_sync has been called.
    /// See https://mynewt.apache.org/latest/network/ble_setup/ble_sync_cb.html
    pub fn on_reset<F>(&self, callback: F)
    where
        F: FnMut(i32) + Send + 'static,
    {
        unsafe { self.on_reset_nonstatic(callback) }
    }

    /// # Safety
    ///
    /// The non-`'static` counterpart of `on_reset`. See
    /// [`on_sync_nonstatic`](Self::on_sync_nonstatic) for the borrowing rules and
    /// the `core::mem::forget` hazard.
    pub unsafe fn on_reset_nonstatic<F>(&self, callback: F)
    where
        F: FnMut(i32) + Send + 'ble,
    {
        unsafe { SINGLETON.reset.subscribe_nonstatic(callback) };

        unsafe {
            (*core::ptr::addr_of_mut!(ble_hs_cfg)).reset_cb = Some(BleSingleton::host_reset_cb);
        }
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
    /// [`on_sync`](Self::on_sync) callback once the stack is ready for use. Call this once
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
            (*cfg).gatts_register_cb = None;
        }

        SINGLETON.sync.unsubscribe();
        SINGLETON.reset.unsubscribe();
        SINGLETON.gap_event.unsubscribe();
        SINGLETON.gatts.unsubscribe();
        let _ = SINGLETON.release();
    }
}
