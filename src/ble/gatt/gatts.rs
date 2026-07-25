//! NimBLE GATT server: the service table, its events, and server operations on the [`BleDriver`].

use core::ptr;

use alloc::boxed::Box;
use alloc::vec::Vec;

use enumset::EnumSet;

use crate::sys::*;

use super::super::mbuf::{mbuf_from_slice, Mbuf};
use super::super::{BleDriver, BleError, BleUuid, ConnHandle};
use super::{flags_to_repr, AttrHandle, BleGattCharFlag};

/// A GATT-server event, delivered on the host task to the single
/// [`on_gatts_event`](BleDriver::on_gatts_event) hook.
///
/// There are no per-characteristic callbacks: NimBLE dispatches *every* characteristic read and
/// write through one shared trampoline, and they arrive here as [`Read`](Self::Read) /
/// [`Write`](Self::Write), keyed by the globally-unique `attr_handle`. The
/// [`Register`](Self::Register) variants fire as the service table is registered (at `start`).
/// [`Subscribe`](Self::Subscribe) and [`NotifyTx`](Self::NotifyTx) are server-role connection
/// events that NimBLE delivers on the GAP callback and we demux here.
///
/// The hook returns the ATT status (`0` on success) for `Read`/`Write`; the return is ignored for
/// the others.
pub enum GattsEvent<'a> {
    Register(BleGattRegister),
    Read {
        conn_handle: ConnHandle,
        attr_handle: AttrHandle,
        reply: Mbuf<'a>,
    },
    Write {
        conn_handle: ConnHandle,
        attr_handle: AttrHandle,
        data: Mbuf<'a>,
    },
    /// A peer subscribed to / unsubscribed from one of our characteristics (wrote its CCCD).
    Subscribe {
        conn_handle: ConnHandle,
        attr_handle: AttrHandle,
        cur_indicate: bool,
        cur_notify: bool,
    },
    /// An indication/notification we sent completed (for an indication, `status` is the peer's
    /// confirmation result).
    NotifyTx {
        conn_handle: ConnHandle,
        attr_handle: AttrHandle,
        indication: bool,
        status: i32,
    },
}

impl GattsEvent<'static> {
    /// Build the server-role `Subscribe` / `NotifyTx` events from a raw GAP event. Returns `None`
    /// for any other event type. Called from the GAP trampoline's demux.
    pub(crate) fn from_gap(event: &ble_gap_event) -> Option<Self> {
        let anon = &event.__bindgen_anon_1;

        match event.type_ as u32 {
            BLE_GAP_EVENT_SUBSCRIBE => {
                let subscribe = unsafe { &anon.subscribe };
                Some(Self::Subscribe {
                    conn_handle: subscribe.conn_handle,
                    attr_handle: subscribe.attr_handle,
                    cur_indicate: subscribe.cur_indicate() != 0,
                    cur_notify: subscribe.cur_notify() != 0,
                })
            }
            BLE_GAP_EVENT_NOTIFY_TX => {
                let notify_tx = unsafe { &anon.notify_tx };
                Some(Self::NotifyTx {
                    conn_handle: notify_tx.conn_handle,
                    attr_handle: notify_tx.attr_handle,
                    indication: notify_tx.indication() != 0,
                    status: notify_tx.status,
                })
            }
            _ => None,
        }
    }
}

/// A GATT registration event (the payload of [`GattsEvent::Register`]). Capture the value handles
/// you need (matching on `uuid`) here.
pub enum BleGattRegister {
    Service {
        uuid: BleUuid,
        handle: AttrHandle,
    },
    Characteristic {
        uuid: BleUuid,
        def_handle: AttrHandle,
        val_handle: AttrHandle,
    },
    Descriptor {
        uuid: BleUuid,
        handle: AttrHandle,
    },
    Other,
}

impl From<&ble_gatt_register_ctxt> for BleGattRegister {
    fn from(ctxt: &ble_gatt_register_ctxt) -> Self {
        let anon = &ctxt.__bindgen_anon_1;

        match ctxt.op as u32 {
            BLE_GATT_REGISTER_OP_SVC => {
                let svc = unsafe { &anon.svc };
                Self::Service {
                    uuid: unsafe { BleUuid::from_raw((*svc.svc_def).uuid) },
                    handle: svc.handle,
                }
            }
            BLE_GATT_REGISTER_OP_CHR => {
                let chr = unsafe { &anon.chr };
                Self::Characteristic {
                    uuid: unsafe { BleUuid::from_raw((*chr.chr_def).uuid) },
                    def_handle: chr.def_handle,
                    val_handle: chr.val_handle,
                }
            }
            BLE_GATT_REGISTER_OP_DSC => {
                let dsc = unsafe { &anon.dsc };
                Self::Descriptor {
                    uuid: unsafe { BleUuid::from_raw((*dsc.dsc_def).uuid) },
                    handle: dsc.handle,
                }
            }
            _ => Self::Other,
        }
    }
}

/// A characteristic in a [`BleGattService`] — just its UUID and flags. Reads and writes are
/// serviced by the single [`on_gatts_event`](BleDriver::on_gatts_event) hook (dispatched by the
/// value handle reported via [`BleGattRegister`]), so there is no per-characteristic closure and
/// no per-characteristic allocation.
pub struct BleGattCharacteristic {
    uuid: BleUuid,
    flags: EnumSet<BleGattCharFlag>,
}

impl BleGattCharacteristic {
    pub fn new(uuid: BleUuid, flags: EnumSet<BleGattCharFlag>) -> Self {
        Self { uuid, flags }
    }
}

/// A GATT service definition.
pub struct BleGattService {
    primary: bool,
    uuid: BleUuid,
    characteristics: Vec<BleGattCharacteristic>,
}

impl BleGattService {
    pub fn new(primary: bool, uuid: BleUuid, characteristics: Vec<BleGattCharacteristic>) -> Self {
        Self {
            primary,
            uuid,
            characteristics,
        }
    }
}

/// The GATT service table as the raw NimBLE `ble_gatt_svc_def` tree, ready to hand to
/// [`BleDriver::new_with_services`](crate::ble::BleDriver::new_with_services). Implements
/// `Deref<Target = [ble_gatt_svc_def]>`, so it is one valid `S`; a `&'static` table or a
/// `Box<[ble_gatt_svc_def]>` are others.
pub struct BleGattServices {
    // There are dragons here. The C def arrays hold raw pointers into `_services` (UUIDs) and into
    // `_chr_defs`. Those targets are heap-allocated, so they stay put when this struct's handle
    // moves — which is what keeps the pointers valid. All characteristics share the crate's single
    // access trampoline (`gatts_access_cb`); `arg` and `val_handle` are null.
    _services: Vec<BleGattService>,
    _chr_defs: Vec<Box<[ble_gatt_chr_def]>>,
    svc_defs: Box<[ble_gatt_svc_def]>,
}

impl BleGattServices {
    pub fn new(services: Vec<BleGattService>) -> Self {
        let mut chr_storage: Vec<Box<[ble_gatt_chr_def]>> = Vec::with_capacity(services.len());
        let mut svc_defs: Vec<ble_gatt_svc_def> = Vec::with_capacity(services.len() + 1);

        for service in &services {
            let mut chr_defs: Vec<ble_gatt_chr_def> =
                Vec::with_capacity(service.characteristics.len() + 1);

            for chr in &service.characteristics {
                chr_defs.push(ble_gatt_chr_def {
                    uuid: chr.uuid.as_ptr(),
                    // The one trampoline for every characteristic; `attr_handle` disambiguates.
                    access_cb: Some(super::super::BleSingleton::gatts_access_cb),
                    arg: ptr::null_mut(),
                    flags: flags_to_repr(chr.flags),
                    // Handles are captured from the registration event, not written back here.
                    val_handle: ptr::null_mut(),
                    ..Default::default()
                });
            }
            chr_defs.push(ble_gatt_chr_def::default());

            let chr_defs = chr_defs.into_boxed_slice();
            let chr_ptr = chr_defs.as_ptr();
            chr_storage.push(chr_defs);

            svc_defs.push(ble_gatt_svc_def {
                type_: if service.primary {
                    BLE_GATT_SVC_TYPE_PRIMARY as u8
                } else {
                    BLE_GATT_SVC_TYPE_SECONDARY as u8
                },
                uuid: service.uuid.as_ptr(),
                includes: ptr::null_mut(),
                characteristics: chr_ptr,
            });
        }
        svc_defs.push(ble_gatt_svc_def::default());

        Self {
            _services: services,
            _chr_defs: chr_storage,
            svc_defs: svc_defs.into_boxed_slice(),
        }
    }
}

impl core::ops::Deref for BleGattServices {
    type Target = [ble_gatt_svc_def];

    fn deref(&self) -> &[ble_gatt_svc_def] {
        &self.svc_defs
    }
}

/// GATT-server operations on the [`BleDriver`], available only when the driver was built with a
/// service table (`S: Deref<Target = [ble_gatt_svc_def]>`) via
/// [`new_with_services`](BleDriver::new_with_services). `&self`, so callable re-entrantly.
#[cfg(esp_idf_bt_nimble_gatt_server)]
impl<'d, S> BleDriver<'d, S>
where
    S: core::ops::Deref<Target = [ble_gatt_svc_def]>,
{
    /// Subscribe to GATT-server events ([`GattsEvent`]). Set this **before**
    /// [`start`](BleDriver::start): the `Register` events (carrying the attribute handles NimBLE
    /// assigned) fire during host start.
    pub fn on_gatts_event<F>(&self, callback: F)
    where
        F: for<'a> FnMut(GattsEvent<'a>) -> u8 + Send + 'static,
    {
        unsafe { self.on_gatts_event_nonstatic(callback) }
    }

    /// # Safety
    ///
    /// The non-`'static` counterpart of [`on_gatts_event`](Self::on_gatts_event). See
    /// [`BleDriver::on_sync_nonstatic`](crate::ble::BleDriver::on_sync_nonstatic) for the borrowing
    /// rules and the `core::mem::forget` hazard.
    pub unsafe fn on_gatts_event_nonstatic<F>(&self, callback: F)
    where
        F: for<'a> FnMut(GattsEvent<'a>) -> u8 + Send + 'd,
    {
        unsafe { super::super::SINGLETON.gatts.subscribe_nonstatic(callback) };

        unsafe {
            (*core::ptr::addr_of_mut!(ble_hs_cfg)).gatts_register_cb =
                Some(super::super::BleSingleton::gatts_register_cb);
        }
    }

    /// Stop delivering GATT-server events to the subscribed hook.
    pub fn gatts_unsubscribe(&self) {
        super::super::SINGLETON.gatts.unsubscribe();
    }

    /// Send a "free-form" characteristic indication to `conn_handle`.
    pub fn indicate(
        &self,
        conn_handle: ConnHandle,
        val_handle: AttrHandle,
        data: &[u8],
    ) -> Result<(), BleError> {
        let om = mbuf_from_slice(data)?;

        // `ble_gatts_indicate_custom` takes ownership of `om` and frees it on all paths (no leak, no double-free).
        BleError::from_raw(unsafe { ble_gatts_indicate_custom(conn_handle, val_handle, om) })
    }
}
