//! Apple bridge (CoreBluetooth): a central over `CBCentralManager` and a GATT server over
//! `CBPeripheralManager`, each with its delegate on a dedicated dispatch queue. Every async call
//! races a timeout, so nothing hangs; the pure mappings below are tested on every host.

use std::sync::Arc;

use crate::backend::{AdapterState, CentralBackend, CharacteristicProperties, ServerBackend, Uuid};
use crate::error::ErrorKind;

/// The adapter state a `CBManagerState` raw value maps to. Unknown values stay unknown.
#[cfg_attr(not(target_vendor = "apple"), cfg_attr(not(test), expect(dead_code)))]
fn cb_manager_state_to_adapter_state(state: isize) -> AdapterState {
    match state {
        1 => AdapterState::Resetting,
        2 => AdapterState::Unsupported,
        3 => AdapterState::Unauthorized,
        4 => AdapterState::PoweredOff,
        5 => AdapterState::On,
        _ => AdapterState::Unknown,
    }
}

/// The coded kind a `CBError` code maps to; codes with no better home are the platform's own
/// failure.
#[cfg_attr(not(target_vendor = "apple"), cfg_attr(not(test), expect(dead_code)))]
fn cb_error_to_kind(code: isize) -> ErrorKind {
    match code {
        1 | 8 => ErrorKind::InvalidIdentifiers,
        2 | 7 | 14 | 15 => ErrorKind::Disconnected,
        3 => ErrorKind::DeviceNotConnected,
        5 => ErrorKind::Cancelled,
        6 | 10 | 11 => ErrorKind::ConnectFailed,
        9 => ErrorKind::InProgress,
        12 => ErrorKind::DeviceNotFound,
        13 => ErrorKind::Unsupported,
        17 | 18 => ErrorKind::CharacteristicNotifyChangeFailed,
        _ => ErrorKind::Backend,
    }
}

/// The GATT server's authorisation failure: `None` when the app may serve, the kind to fail with
/// otherwise. `CBManagerAuthorization` raw values.
#[cfg_attr(not(target_vendor = "apple"), cfg_attr(not(test), expect(dead_code)))]
fn cb_server_authorization_to_kind(status: isize) -> Option<ErrorKind> {
    match status {
        3 => None,
        _ => Some(ErrorKind::Unauthorized),
    }
}

/// Our characteristic properties from CoreBluetooth's raw bits. CoreBluetooth's two high bits mean
/// "encryption required", not our reliable-write bits, so only the spec's byte carries over.
#[cfg_attr(not(target_vendor = "apple"), cfg_attr(not(test), expect(dead_code)))]
fn cb_characteristic_properties_from_cb(bits: usize) -> CharacteristicProperties {
    CharacteristicProperties::from_bits((bits & 0xFF) as u16)
}

/// CoreBluetooth property bits from ours; CoreBluetooth has no bits above the spec's byte.
#[cfg_attr(not(target_vendor = "apple"), cfg_attr(not(test), expect(dead_code)))]
fn cb_properties_bits_from_ours(properties: CharacteristicProperties) -> usize {
    (properties.bits() & 0xFF) as usize
}

/// The waiter-key form of a UUID string: the crate's canonical lowercase full form. CBUUID hands
/// out uppercase 4/8-character short strings while registration uses our canonical form, so every
/// waiter key must pass through here; `None` when the string is not a UUID.
#[cfg_attr(not(target_vendor = "apple"), cfg_attr(not(test), expect(dead_code)))]
fn cb_uuid_key(text: &str) -> Option<String> {
    Uuid::parse(text).ok().map(|uuid| uuid.as_str().to_owned())
}

/// The device half of a waiter key: NSUUID-shaped ids collapse to their canonical form so a
/// caller's differently-cased DeviceId and the callback's spelling land on one key; anything
/// else passes through verbatim.
#[cfg_attr(not(target_vendor = "apple"), cfg_attr(not(test), expect(dead_code)))]
fn cb_device_key(device: &str) -> String {
    match Uuid::parse(device) {
        Ok(uuid) => uuid.as_str().to_owned(),
        Err(_) => device.to_owned(),
    }
}

/// The platform's central backend, if this platform has one and it is implemented.
pub fn central_backend() -> Option<Arc<dyn CentralBackend>> {
    #[cfg(not(target_vendor = "apple"))]
    {
        None
    }
    #[cfg(target_vendor = "apple")]
    {
        Some(native::central_backend())
    }
}

/// The platform's server (peripheral role) backend, if this platform has one and it is implemented.
pub fn server_backend() -> Option<Arc<dyn ServerBackend>> {
    #[cfg(not(target_vendor = "apple"))]
    {
        None
    }
    #[cfg(target_vendor = "apple")]
    {
        Some(native::server_backend())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_manager_state_maps_to_its_adapter_state() {
        assert_eq!(cb_manager_state_to_adapter_state(0), AdapterState::Unknown);
        assert_eq!(
            cb_manager_state_to_adapter_state(1),
            AdapterState::Resetting
        );
        assert_eq!(
            cb_manager_state_to_adapter_state(2),
            AdapterState::Unsupported
        );
        assert_eq!(
            cb_manager_state_to_adapter_state(3),
            AdapterState::Unauthorized
        );
        assert_eq!(
            cb_manager_state_to_adapter_state(4),
            AdapterState::PoweredOff
        );
        assert_eq!(cb_manager_state_to_adapter_state(5), AdapterState::On);
        assert_eq!(cb_manager_state_to_adapter_state(99), AdapterState::Unknown);
    }

    #[test]
    fn every_core_bluetooth_error_maps_to_one_kind() {
        let cases: [(isize, ErrorKind); 14] = [
            (0, ErrorKind::Backend),
            (1, ErrorKind::InvalidIdentifiers),
            (2, ErrorKind::Disconnected),
            (3, ErrorKind::DeviceNotConnected),
            (4, ErrorKind::Backend),
            (5, ErrorKind::Cancelled),
            (6, ErrorKind::ConnectFailed),
            (7, ErrorKind::Disconnected),
            (9, ErrorKind::InProgress),
            (10, ErrorKind::ConnectFailed),
            (12, ErrorKind::DeviceNotFound),
            (13, ErrorKind::Unsupported),
            (17, ErrorKind::CharacteristicNotifyChangeFailed),
            (99, ErrorKind::Backend),
        ];
        for (code, kind) in cases {
            assert_eq!(cb_error_to_kind(code), kind, "code {code}");
        }
    }

    #[test]
    fn server_authorization_fails_unauthorized_in_every_non_allowed_status() {
        assert_eq!(
            cb_server_authorization_to_kind(0),
            Some(ErrorKind::Unauthorized)
        );
        assert_eq!(
            cb_server_authorization_to_kind(1),
            Some(ErrorKind::Unauthorized)
        );
        assert_eq!(
            cb_server_authorization_to_kind(2),
            Some(ErrorKind::Unauthorized)
        );
        assert_eq!(cb_server_authorization_to_kind(3), None);
        assert_eq!(
            cb_server_authorization_to_kind(99),
            Some(ErrorKind::Unauthorized)
        );
    }

    #[test]
    fn property_bits_keep_the_spec_byte_and_drop_the_rest() {
        let read_notify = cb_characteristic_properties_from_cb(0x12);
        assert!(read_notify.contains(CharacteristicProperties::READ));
        assert!(read_notify.contains(CharacteristicProperties::NOTIFY));
        // CoreBluetooth's notify-encryption bit must not become our reliable-write bit.
        let encrypted_notify = cb_characteristic_properties_from_cb(0x112);
        assert!(!encrypted_notify.contains(CharacteristicProperties::RELIABLE_WRITE));
        assert!(encrypted_notify.contains(CharacteristicProperties::NOTIFY));
        assert_eq!(
            cb_properties_bits_from_ours(CharacteristicProperties::from_bits(0x1FF)),
            0xFF
        );
    }

    #[test]
    fn every_uuid_spelling_lands_on_one_waiter_key() {
        let canonical = "0000180d-0000-1000-8000-00805f9b34fb";
        assert_eq!(cb_uuid_key("180D").as_deref(), Some(canonical));
        assert_eq!(cb_uuid_key("180d").as_deref(), Some(canonical));
        assert_eq!(cb_uuid_key(canonical).as_deref(), Some(canonical));
        assert_eq!(
            cb_uuid_key("0000180D-0000-1000-8000-00805F9B34FB").as_deref(),
            Some(canonical)
        );
        let wide = "deadbeef-0000-1000-8000-00805f9b34fb";
        assert_eq!(cb_uuid_key("DEADBEEF").as_deref(), Some(wide));
        assert_eq!(cb_uuid_key(wide).as_deref(), Some(wide));
        assert_eq!(cb_uuid_key("not-a-uuid"), None);
        assert_eq!(cb_uuid_key(""), None);
    }

    #[test]
    fn every_device_id_spelling_lands_on_one_waiter_key() {
        let upper = "68753A44-37D1-4A50-A1C4-2AF1B1D5F1BB";
        let canonical = "68753a44-37d1-4a50-a1c4-2af1b1d5f1bb";
        assert_eq!(cb_device_key(upper), canonical);
        assert_eq!(cb_device_key(canonical), canonical);
        // Non-UUID ids have no canonical form and pass through untouched.
        assert_eq!(cb_device_key("watch"), "watch");
        assert_eq!(cb_device_key(""), "");
    }
}

#[cfg(target_vendor = "apple")]
mod native {
    use std::collections::HashMap;
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::future::ready;
    use std::slice;
    use std::sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    };
    use std::time::Duration;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol};
    use objc2::{AnyThread, ClassType, DefinedClass, define_class, msg_send};
    use objc2_core_bluetooth::{
        CBATTError, CBAdvertisementDataLocalNameKey, CBAdvertisementDataManufacturerDataKey,
        CBAdvertisementDataServiceUUIDsKey, CBAdvertisementDataTxPowerLevelKey,
        CBAttributePermissions, CBCentralManager, CBCharacteristic,
        CBCharacteristicProperties as CbProps, CBCharacteristicWriteType, CBDescriptor,
        CBMutableCharacteristic, CBMutableDescriptor, CBMutableService, CBPeripheral,
        CBPeripheralManager, CBPeripheralState, CBService, CBUUID,
    };
    use tokio::sync::{broadcast, oneshot};
    use tokio::time::timeout;

    use crate::backend::{
        AdapterState, AdvertiseOptions, AdvertisementData, BackendFuture, CentralBackend,
        Characteristic, ConnectionEvent, Descriptor, DeviceId, DeviceInfo, GattRequest,
        GattRequestKind, ScanFilter, ScanResult, ServerBackend, Service, Uuid, ValueNotification,
        WriteType,
    };
    use crate::error::{BluetoothError, ErrorKind};

    use super::{
        cb_characteristic_properties_from_cb, cb_device_key, cb_error_to_kind,
        cb_manager_state_to_adapter_state, cb_properties_bits_from_ours,
        cb_server_authorization_to_kind, cb_uuid_key,
    };

    /// How long an operation waits for its CoreBluetooth callback before reporting `Timeout`.
    const GATT_TIMEOUT: Duration = Duration::from_secs(10);
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
    const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);
    const STATE_WAIT: Duration = Duration::from_secs(3);
    /// Ceiling on a served characteristic value; a write whose end passes it is refused with
    /// InvalidOffset rather than trusted to resize.
    const MAX_SERVED_VALUE: usize = 1024 * 1024;

    fn backend_error() -> BluetoothError {
        BluetoothError::new(ErrorKind::Backend)
    }

    fn locked<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
        lock.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // A serial queue the managers deliver delegate callbacks on; CoreBluetooth retains it while
    // the manager lives, so the `Retained` from creation can drop after init.
    unsafe extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> *mut c_void;
    }

    fn new_queue(label: &CStr) -> Option<Retained<AnyObject>> {
        unsafe {
            let queue = dispatch_queue_create(label.as_ptr(), std::ptr::null());
            if queue.is_null() {
                return None;
            }
            Retained::from_raw(queue.cast::<AnyObject>())
        }
    }

    /// A CoreBluetooth object handle. The managers are thread-safe and every Rust-side field they
    /// reach is lock-protected, so sharing the handle across the async runtime is sound.
    struct CbPtr<T>(Retained<T>);

    // SAFETY: see the struct comment; the ObjC runtime serialises the message sends themselves.
    unsafe impl<T> Send for CbPtr<T> {}
    // SAFETY: every access goes through `msg_send!` or lock-protected state.
    unsafe impl<T> Sync for CbPtr<T> {}

    impl<T: objc2::Message> Clone for CbPtr<T> {
        fn clone(&self) -> Self {
            Self(self.0.clone())
        }
    }

    impl<T> CbPtr<T> {
        fn get(&self) -> &T {
            &self.0
        }
    }

    impl<T> std::ops::Deref for CbPtr<T> {
        type Target = Retained<T>;

        fn deref(&self) -> &Retained<T> {
            &self.0
        }
    }

    /// Casts a delegate parameter to its documented CoreBluetooth class.
    unsafe fn as_cb<T>(object: &AnyObject) -> &T {
        unsafe { &*(object as *const AnyObject as *const T) }
    }

    // ---- Foundation values as plain objects: this crate must not name Foundation types. ----

    fn class(name: &CStr) -> Option<&'static AnyClass> {
        AnyClass::get(name)
    }

    fn ns_string(text: &str) -> Option<Retained<AnyObject>> {
        let cstring = CString::new(text).ok()?;
        let cls = class(c"NSString")?;
        unsafe { msg_send![cls, stringWithUTF8String: cstring.as_ptr()] }
    }

    unsafe fn string_value<O: objc2::Message + ?Sized>(object: &O) -> String {
        unsafe {
            let utf8: *const c_char = msg_send![object, UTF8String];
            if utf8.is_null() {
                return String::new();
            }
            CStr::from_ptr(utf8).to_string_lossy().into_owned()
        }
    }

    fn mutable_dictionary() -> Option<Retained<AnyObject>> {
        let cls = class(c"NSMutableDictionary")?;
        unsafe { msg_send![cls, dictionary] }
    }

    fn ns_data(bytes: &[u8]) -> Option<CbPtr<AnyObject>> {
        let cls = class(c"NSData")?;
        let data: Option<Retained<AnyObject>> = unsafe {
            msg_send![
                cls,
                dataWithBytes: bytes.as_ptr().cast::<c_void>(),
                length: bytes.len()
            ]
        };
        data.map(CbPtr)
    }

    unsafe fn data_bytes<O: objc2::Message + ?Sized>(object: &O) -> Vec<u8> {
        unsafe {
            let length: usize = msg_send![object, length];
            if length == 0 {
                return Vec::new();
            }
            let bytes: *const c_void = msg_send![object, bytes];
            if bytes.is_null() {
                return Vec::new();
            }
            slice::from_raw_parts(bytes.cast::<u8>(), length).to_vec()
        }
    }

    /// A descriptor value as bytes: data stays bytes, strings and numbers become their textual
    /// form, anything else is empty.
    unsafe fn foundation_bytes<O: objc2::Message + ?Sized>(object: &O) -> Vec<u8> {
        unsafe {
            let Some(data) = class(c"NSData") else {
                return Vec::new();
            };
            let is_data: bool = msg_send![object, isKindOfClass: data];
            if is_data {
                return data_bytes(object);
            }
            let Some(number) = class(c"NSNumber") else {
                return Vec::new();
            };
            let is_number: bool = msg_send![object, isKindOfClass: number];
            if is_number {
                let text: Option<Retained<AnyObject>> = msg_send![object, stringValue];
                return text
                    .map(|t| string_value(&*t))
                    .unwrap_or_default()
                    .into_bytes();
            }
            let Some(text) = class(c"NSString") else {
                return Vec::new();
            };
            let is_text: bool = msg_send![object, isKindOfClass: text];
            if is_text {
                return string_value(object).into_bytes();
            }
            Vec::new()
        }
    }

    fn ns_array<T: objc2::Message>(items: &[Retained<T>]) -> Option<Retained<AnyObject>> {
        let cls = class(c"NSArray")?;
        let handles: Vec<*const T> = items.iter().map(Retained::as_ptr).collect();
        unsafe {
            msg_send![
                cls,
                arrayWithObjects: handles.as_ptr(),
                count: handles.len()
            ]
        }
    }

    unsafe fn array_count<O: objc2::Message + ?Sized>(array: &O) -> usize {
        unsafe { msg_send![array, count] }
    }

    unsafe fn array_item<O: objc2::Message + ?Sized, T: objc2::Message>(
        array: &O,
        index: usize,
    ) -> Option<Retained<T>> {
        unsafe { msg_send![array, objectAtIndex: index] }
    }

    unsafe fn array_items<O: objc2::Message + ?Sized, T: objc2::Message>(
        array: &O,
    ) -> Vec<Retained<T>> {
        unsafe {
            let count = array_count(array);
            let mut items = Vec::with_capacity(count);
            for index in 0..count {
                let item: Option<Retained<T>> = msg_send![array, objectAtIndex: index];
                if let Some(item) = item {
                    items.push(item);
                }
            }
            items
        }
    }

    unsafe fn dict_value<O, K>(dict: &O, key: &K) -> Option<Retained<AnyObject>>
    where
        O: objc2::Message + ?Sized,
        K: objc2::Message + ?Sized,
    {
        unsafe { msg_send![dict, objectForKey: key] }
    }

    unsafe fn cb_uuid_string<O: objc2::Message + ?Sized>(object: &O) -> String {
        unsafe {
            let text: Option<Retained<AnyObject>> = msg_send![object, UUIDString];
            text.map(|t| string_value(&*t)).unwrap_or_default()
        }
    }

    fn parse_uuid(text: &str) -> Result<Uuid, BluetoothError> {
        Uuid::parse(text)
    }

    unsafe fn cb_uuid(uuid: &Uuid) -> Result<Retained<CBUUID>, BluetoothError> {
        unsafe {
            let text = ns_string(uuid.as_str()).ok_or_else(backend_error)?;
            let uuid: Option<Retained<CBUUID>> = msg_send![CBUUID::class(), UUIDWithString: &*text];
            uuid.ok_or_else(backend_error)
        }
    }

    /// The peripheral's device key: NSUUID spelling canonicalised here, so every callback-side
    /// key and map entry matches what registration built from the caller's DeviceId.
    unsafe fn peripheral_id<O: objc2::Message + ?Sized>(peripheral: &O) -> Option<String> {
        unsafe {
            let identifier: Option<Retained<AnyObject>> = msg_send![peripheral, identifier];
            let identifier = identifier?;
            let text: Option<Retained<AnyObject>> = msg_send![&*identifier, UUIDString];
            Some(cb_device_key(&string_value(&*text?)))
        }
    }

    unsafe fn ns_error_code(error: &AnyObject) -> i64 {
        unsafe {
            let code: isize = msg_send![error, code];
            code as i64
        }
    }

    /// The coded error a delegate callback's `NSError` maps to; `fallback` covers the no-error
    /// case, which callers use when the callback itself implies the failure.
    fn ns_error_to_bt(error: Option<&AnyObject>, fallback: ErrorKind) -> BluetoothError {
        match error {
            Some(error) => {
                let code = unsafe { ns_error_code(error) };
                BluetoothError::new(cb_error_to_kind(code as isize)).with_status(code)
            }
            None => BluetoothError::new(fallback),
        }
    }

    // ---- One-shot waiters, keyed by `"<device>|<uuid>"` (or a bare word on the server). ----

    type SlotWaiter<T> = (u64, oneshot::Sender<Result<T, BluetoothError>>);
    type SlotMap<T> = HashMap<String, Vec<SlotWaiter<T>>>;

    struct Slots<T> {
        next_id: AtomicU64,
        waiters: Mutex<SlotMap<T>>,
    }

    impl<T> Slots<T> {
        fn new() -> Self {
            Self {
                next_id: AtomicU64::new(0),
                waiters: Mutex::new(HashMap::new()),
            }
        }

        /// Registers a waiter under `key`; the id identifies it for a timed-out `discard`.
        fn register(&self, key: &str) -> (u64, oneshot::Receiver<Result<T, BluetoothError>>) {
            let (sender, receiver) = oneshot::channel();
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            locked(&self.waiters)
                .entry(key.to_owned())
                .or_default()
                .push((id, sender));
            (id, receiver)
        }

        fn resolve(&self, key: &str, outcome: Result<T, BluetoothError>)
        where
            T: Clone,
        {
            if let Some(waiters) = locked(&self.waiters).remove(key) {
                for (_, waiter) in waiters {
                    let _ = waiter.send(outcome.clone());
                }
            }
        }

        /// Drops one timed-out waiter so the slot map cannot grow without bound.
        fn discard(&self, key: &str, id: u64) {
            let mut map = locked(&self.waiters);
            if let Some(waiters) = map.get_mut(key) {
                waiters.retain(|(waiter_id, _)| *waiter_id != id);
                if waiters.is_empty() {
                    map.remove(key);
                }
            }
        }

        fn fail_prefix(&self, prefix: &str, outcome: Result<T, BluetoothError>)
        where
            T: Clone,
        {
            let keys: Vec<String> = locked(&self.waiters)
                .keys()
                .filter(|key| key.starts_with(prefix))
                .cloned()
                .collect();
            for key in keys {
                self.resolve(&key, outcome.clone());
            }
        }
    }

    /// Races a waiter against a deadline; a dropped slot is the platform failing silently and a
    /// timeout deregisters the dead sender.
    async fn settle<T>(
        slots: &Slots<T>,
        key: &str,
        id: u64,
        receiver: oneshot::Receiver<Result<T, BluetoothError>>,
        limit: Duration,
    ) -> Result<T, BluetoothError> {
        match timeout(limit, receiver).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_dropped)) => Err(backend_error()),
            Err(_elapsed) => {
                slots.discard(key, id);
                Err(BluetoothError::new(ErrorKind::Timeout))
            }
        }
    }

    /// The failure a device drop resolves every waiter of that device with.
    fn dropped<T>() -> Result<T, BluetoothError> {
        Err(BluetoothError::new(ErrorKind::Disconnected))
    }

    // ---- The central role. ----

    /// Every characteristic and descriptor handle a discovery built, with its portable copy.
    #[derive(Default)]
    struct DeviceObjects {
        characteristics: HashMap<String, (CbPtr<CBCharacteristic>, Characteristic)>,
        descriptors: HashMap<String, (CbPtr<CBDescriptor>, Descriptor)>,
    }

    #[derive(Default)]
    struct DiscoveryProgress {
        pending: usize,
        failed: bool,
    }

    struct Waiters {
        connect: Slots<()>,
        services: Slots<Vec<Service>>,
        value: Slots<Vec<u8>>,
        write: Slots<()>,
        notify_change: Slots<()>,
        descriptor_read: Slots<Vec<u8>>,
        descriptor_write: Slots<()>,
        rssi: Slots<i8>,
        disconnect: Slots<()>,
    }

    struct CentralShared {
        state: Mutex<AdapterState>,
        state_tx: broadcast::Sender<AdapterState>,
        scan: Mutex<Option<(broadcast::Sender<ScanResult>, ScanFilter)>>,
        events_tx: broadcast::Sender<ConnectionEvent>,
        peripherals: Mutex<HashMap<String, CbPtr<CBPeripheral>>>,
        objects: Mutex<HashMap<String, DeviceObjects>>,
        discovery: Mutex<HashMap<String, DiscoveryProgress>>,
        notify_tx: Mutex<HashMap<String, broadcast::Sender<ValueNotification>>>,
        waiters: Waiters,
    }

    impl CentralShared {
        fn new() -> Self {
            let (state_tx, _) = broadcast::channel(32);
            let (events_tx, _) = broadcast::channel(32);
            Self {
                state: Mutex::new(AdapterState::Unknown),
                state_tx,
                scan: Mutex::new(None),
                events_tx,
                peripherals: Mutex::new(HashMap::new()),
                objects: Mutex::new(HashMap::new()),
                discovery: Mutex::new(HashMap::new()),
                notify_tx: Mutex::new(HashMap::new()),
                waiters: Waiters {
                    connect: Slots::new(),
                    services: Slots::new(),
                    value: Slots::new(),
                    write: Slots::new(),
                    notify_change: Slots::new(),
                    descriptor_read: Slots::new(),
                    descriptor_write: Slots::new(),
                    rssi: Slots::new(),
                    disconnect: Slots::new(),
                },
            }
        }

        /// Ends every pending operation of a device that disconnected.
        fn fail_device(&self, device: &str) {
            let prefix = format!("{device}|");
            self.waiters.connect.fail_prefix(&prefix, dropped());
            self.waiters.services.fail_prefix(&prefix, dropped());
            self.waiters.value.fail_prefix(&prefix, dropped());
            self.waiters.write.fail_prefix(&prefix, dropped());
            self.waiters.notify_change.fail_prefix(&prefix, dropped());
            self.waiters.descriptor_read.fail_prefix(&prefix, dropped());
            self.waiters
                .descriptor_write
                .fail_prefix(&prefix, dropped());
            self.waiters.rssi.fail_prefix(&prefix, dropped());
            self.waiters.disconnect.fail_prefix(&prefix, dropped());
            locked(&self.objects).remove(device);
            // A dropped link stops the discovery chain; the waiter above got its answer.
            locked(&self.discovery).remove(device);
            locked(&self.notify_tx).retain(|key, _| !key.starts_with(&prefix));
        }
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and no callback drops a CoreBluetooth
        // object the backend still owns.
        #[unsafe(super(NSObject))]
        #[thread_kind = AnyThread]
        #[ivars = Arc<CentralShared>]
        struct CentralDelegate;

        unsafe impl NSObjectProtocol for CentralDelegate {}

        impl CentralDelegate {
            #[unsafe(method(centralManagerDidUpdateState:))]
            fn did_update_state(&self, central: &AnyObject) {
                let raw: isize = unsafe { msg_send![central, state] };
                let state = cb_manager_state_to_adapter_state(raw);
                let shared = self.ivars();
                *locked(&shared.state) = state;
                let _ = shared.state_tx.send(state);
            }

            #[unsafe(method(centralManager:didDiscoverPeripheral:advertisementData:RSSI:))]
            fn did_discover(
                &self,
                _central: &AnyObject,
                peripheral: &AnyObject,
                advertisement: &AnyObject,
                rssi: &AnyObject,
            ) {
                let shared = self.ivars();
                let device = unsafe { peripheral_id(peripheral) };
                let Some(device) = device else {
                    return;
                };
                let handle = unsafe {
                    Retained::retain(peripheral as *const AnyObject as *mut CBPeripheral)
                };
                let Some(handle) = handle else {
                    return;
                };
                locked(&shared.peripherals).insert(device.clone(), CbPtr(handle));
                let result = unsafe { scan_result(&device, peripheral, advertisement, rssi) };
                let scan = locked(&shared.scan);
                if let Some((sender, filter)) = scan.as_ref()
                    && filter.matches(&result)
                {
                    let _ = sender.send(result);
                }
            }

            #[unsafe(method(centralManager:didConnectPeripheral:))]
            fn did_connect(&self, _central: &AnyObject, peripheral: &AnyObject) {
                let shared = self.ivars();
                let device = unsafe { peripheral_id(peripheral) };
                let Some(device) = device else {
                    return;
                };
                // The peripheral's delegate only receives callbacks once set here.
                let _: () = unsafe { msg_send![peripheral, setDelegate: self] };
                shared.waiters.connect.resolve(&connect_key(&device), Ok(()));
                let _ = shared
                    .events_tx
                    .send(ConnectionEvent::Connected(DeviceId::new(device)));
            }

            #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
            fn did_fail_to_connect(
                &self,
                _central: &AnyObject,
                peripheral: &AnyObject,
                error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let device = unsafe { peripheral_id(peripheral) };
                let Some(device) = device else {
                    return;
                };
                let failure = ns_error_to_bt(error, ErrorKind::ConnectFailed);
                let failure = if failure.kind() == ErrorKind::Backend {
                    // A connect failure with no mapped code is still a connect failure.
                    BluetoothError::new(ErrorKind::ConnectFailed)
                        .with_status(failure.status().unwrap_or_default())
                } else {
                    failure
                };
                shared
                    .waiters
                    .connect
                    .resolve(&connect_key(&device), Err(failure));
            }

            #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
            fn did_disconnect(
                &self,
                _central: &AnyObject,
                peripheral: &AnyObject,
                _error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let device = unsafe { peripheral_id(peripheral) };
                let Some(device) = device else {
                    return;
                };
                shared
                    .waiters
                    .disconnect
                    .resolve(&disconnect_key(&device), Ok(()));
                // A link drop also ends any connect still waiting on this device.
                shared.waiters.connect.resolve(
                    &connect_key(&device),
                    Err(BluetoothError::new(ErrorKind::Disconnected)),
                );
                shared.fail_device(&device);
                let _ = shared
                    .events_tx
                    .send(ConnectionEvent::Disconnected(DeviceId::new(device)));
            }

            #[unsafe(method(peripheral:didDiscoverServices:))]
            fn did_discover_services(
                &self,
                peripheral: &AnyObject,
                error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let device = unsafe { peripheral_id(peripheral) };
                let Some(device) = device else {
                    return;
                };
                let services = unsafe {
                    let list: Option<Retained<AnyObject>> = msg_send![peripheral, services];
                    list.as_ref()
                        .map(|list| array_items::<_, CBService>(&**list))
                        .unwrap_or_default()
                };
                let mut progress = locked(&shared.discovery);
                // Only a discovery the backend started has an entry; a late callback with none
                // in flight belongs to no waiter and is ignored.
                let Some(entry) = progress.get_mut(&device) else {
                    return;
                };
                if error.is_some() {
                    entry.failed = true;
                }
                let none: Option<&AnyObject> = None;
                for service in &services {
                    let _: () = unsafe {
                        msg_send![
                            peripheral,
                            discoverCharacteristics: none,
                            forService: &**service
                        ]
                    };
                }
                entry.pending += services.len();
                let done = entry.pending == 0;
                drop(progress);
                if done {
                    finish_discovery(shared, peripheral, &device);
                }
            }

            #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
            fn did_discover_characteristics(
                &self,
                peripheral: &AnyObject,
                service: &AnyObject,
                error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let device = unsafe { peripheral_id(peripheral) };
                let Some(device) = device else {
                    return;
                };
                let characteristics = unsafe {
                    let list: Option<Retained<AnyObject>> = msg_send![service, characteristics];
                    list.as_ref()
                        .map(|list| array_items::<_, CBCharacteristic>(&**list))
                        .unwrap_or_default()
                };
                let cb_peripheral: &CBPeripheral = unsafe { as_cb(peripheral) };
                let mut progress = locked(&shared.discovery);
                let Some(entry) = progress.get_mut(&device) else {
                    return;
                };
                if error.is_some() {
                    entry.failed = true;
                }
                // Descriptor discovery closes the chain; each of its callbacks decrements.
                for characteristic in &characteristics {
                    unsafe { cb_peripheral.discoverDescriptorsForCharacteristic(characteristic) };
                }
                entry.pending += characteristics.len();
                entry.pending = entry.pending.saturating_sub(1);
                let done = entry.pending == 0;
                drop(progress);
                if done {
                    finish_discovery(shared, peripheral, &device);
                }
            }

            #[unsafe(method(peripheral:didDiscoverDescriptorsForCharacteristic:error:))]
            fn did_discover_descriptors(
                &self,
                peripheral: &AnyObject,
                _characteristic: &AnyObject,
                _error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let device = unsafe { peripheral_id(peripheral) };
                let Some(device) = device else {
                    return;
                };
                let mut progress = locked(&shared.discovery);
                let Some(entry) = progress.get_mut(&device) else {
                    return;
                };
                entry.pending = entry.pending.saturating_sub(1);
                let done = entry.pending == 0;
                drop(progress);
                if done {
                    finish_discovery(shared, peripheral, &device);
                }
            }

            #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
            fn did_update_value(
                &self,
                _peripheral: &AnyObject,
                characteristic: &AnyObject,
                error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let Some((device, key)) = characteristic_keys(characteristic) else {
                    return;
                };
                let slot_key = format!("{device}|{key}");
                if let Some(error) = error {
                    shared.waiters.value.resolve(
                        &slot_key,
                        Err(BluetoothError::new(ErrorKind::CharacteristicReadFailed)
                            .with_status(unsafe { ns_error_code(error) })),
                    );
                    return;
                }
                let bytes = unsafe {
                    let value: Option<Retained<AnyObject>> = msg_send![characteristic, value];
                    value.as_ref().map(|value| data_bytes(&**value)).unwrap_or_default()
                };
                shared.waiters.value.resolve(&slot_key, Ok(bytes.clone()));
                let streams = locked(&shared.notify_tx);
                let mut objects = locked(&shared.objects);
                if let Some(objects) = objects.get_mut(&device)
                    && let Some((_, copy)) = objects.characteristics.get_mut(&key)
                    && let Some(sender) = streams.get(&slot_key)
                {
                    copy.value = Some(bytes.clone());
                    let _ = sender.send(ValueNotification {
                        characteristic: copy.clone(),
                        value: bytes,
                    });
                }
            }

            #[unsafe(method(peripheral:didWriteValueForCharacteristic:error:))]
            fn did_write_value(
                &self,
                _peripheral: &AnyObject,
                characteristic: &AnyObject,
                error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let Some((device, key)) = characteristic_keys(characteristic) else {
                    return;
                };
                let outcome = match error {
                    Some(error) => Err(BluetoothError::new(ErrorKind::CharacteristicWriteFailed)
                        .with_status(unsafe { ns_error_code(error) })),
                    None => Ok(()),
                };
                shared
                    .waiters
                    .write
                    .resolve(&format!("{device}|{key}"), outcome);
            }

            #[unsafe(method(peripheral:didUpdateNotificationStateForCharacteristic:error:))]
            fn did_update_notification_state(
                &self,
                _peripheral: &AnyObject,
                characteristic: &AnyObject,
                error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let Some((device, key)) = characteristic_keys(characteristic) else {
                    return;
                };
                let outcome = match error {
                    Some(error) => Err(BluetoothError::new(
                        ErrorKind::CharacteristicNotifyChangeFailed,
                    )
                    .with_status(unsafe { ns_error_code(error) })),
                    None => Ok(()),
                };
                shared
                    .waiters
                    .notify_change
                    .resolve(&format!("{device}|{key}"), outcome);
            }

            #[unsafe(method(peripheral:didReadRSSI:error:))]
            fn did_read_rssi(
                &self,
                peripheral: &AnyObject,
                rssi: &AnyObject,
                error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let device = unsafe { peripheral_id(peripheral) };
                let Some(device) = device else {
                    return;
                };
                let raw: isize = unsafe { msg_send![rssi, integerValue] };
                let outcome = match error {
                    Some(error) => Err(ns_error_to_bt(Some(error), ErrorKind::Backend)),
                    None if (-128..=127).contains(&raw) => Ok(raw as i8),
                    None => Err(backend_error()),
                };
                shared
                    .waiters
                    .rssi
                    .resolve(&format!("{device}|rssi"), outcome);
            }

            #[unsafe(method(peripheral:didUpdateValueForDescriptor:error:))]
            fn did_update_descriptor_value(
                &self,
                _peripheral: &AnyObject,
                descriptor: &AnyObject,
                error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let Some((device, key)) = descriptor_keys(descriptor) else {
                    return;
                };
                let outcome = match error {
                    Some(error) => Err(BluetoothError::new(ErrorKind::DescriptorReadFailed)
                        .with_status(unsafe { ns_error_code(error) })),
                    None => {
                        let bytes = unsafe {
                            let value: Option<Retained<AnyObject>> = msg_send![descriptor, value];
                            value
                                .as_ref()
                                .map(|value| foundation_bytes(&**value))
                                .unwrap_or_default()
                        };
                        Ok(bytes)
                    }
                };
                shared
                    .waiters
                    .descriptor_read
                    .resolve(&format!("{device}|{key}"), outcome);
            }

            #[unsafe(method(peripheral:didWriteValueForDescriptor:error:))]
            fn did_write_descriptor_value(
                &self,
                _peripheral: &AnyObject,
                descriptor: &AnyObject,
                error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let Some((device, key)) = descriptor_keys(descriptor) else {
                    return;
                };
                let outcome = match error {
                    Some(error) => Err(BluetoothError::new(ErrorKind::DescriptorWriteFailed)
                        .with_status(unsafe { ns_error_code(error) })),
                    None => Ok(()),
                };
                shared
                    .waiters
                    .descriptor_write
                    .resolve(&format!("{device}|{key}"), outcome);
            }
        }
    );

    impl CentralDelegate {
        fn new(shared: Arc<CentralShared>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(shared);
            unsafe { msg_send![super(this), init] }
        }
    }

    /// The connect waiter's key; registration and every callback must build it identically, so
    /// the device half canonicalises on both paths.
    fn connect_key(device: &str) -> String {
        format!("{}|connect", cb_device_key(device))
    }

    fn disconnect_key(device: &str) -> String {
        format!("{}|disconnect", cb_device_key(device))
    }

    /// The device and characteristic key a callback's object identifies, through the
    /// characteristic's service back to its peripheral. The uuid passes through `cb_uuid_key` so
    /// CBUUID's short uppercase spelling matches the registration's canonical form.
    fn characteristic_keys(characteristic: &AnyObject) -> Option<(String, String)> {
        let device = unsafe {
            let service: Option<Retained<AnyObject>> = msg_send![characteristic, service];
            let peripheral: Option<Retained<AnyObject>> = match &service {
                Some(service) => msg_send![&**service, peripheral],
                None => None,
            };
            peripheral.as_ref().and_then(|p| peripheral_id(&**p))
        };
        let key = cb_uuid_key(&unsafe { cb_uuid_string(characteristic) })?;
        Some((device?, key))
    }

    fn descriptor_keys(descriptor: &AnyObject) -> Option<(String, String)> {
        let device = unsafe {
            let characteristic: Option<Retained<AnyObject>> = msg_send![descriptor, characteristic];
            match &characteristic {
                Some(characteristic) => {
                    characteristic_keys(characteristic).map(|(device, _)| device)
                }
                None => None,
            }
        };
        let key = cb_uuid_key(&unsafe { cb_uuid_string(descriptor) })?;
        Some((device?, key))
    }

    unsafe fn scan_result(
        device: &str,
        peripheral: &AnyObject,
        advertisement: &AnyObject,
        rssi: &AnyObject,
    ) -> ScanResult {
        unsafe {
            let local_name: Option<String> =
                dict_value(advertisement, CBAdvertisementDataLocalNameKey)
                    .map(|name| string_value(&*name));
            let name = match local_name.clone() {
                Some(name) => Some(name),
                None => {
                    let peripheral_name: Option<Retained<AnyObject>> = msg_send![peripheral, name];
                    peripheral_name.map(|name| string_value(&*name))
                }
            };
            let raw_rssi: isize = msg_send![rssi, integerValue];
            // 127 is CoreBluetooth's "not available" marker.
            let rssi = if (-128..=127).contains(&raw_rssi) && raw_rssi != 127 {
                Some(raw_rssi as i8)
            } else {
                None
            };
            let manufacturer_data =
                dict_value(advertisement, CBAdvertisementDataManufacturerDataKey)
                    .map(|data| data_bytes(&*data));
            let service_uuids = dict_value(advertisement, CBAdvertisementDataServiceUUIDsKey)
                .map(|services| {
                    array_items::<_, CBUUID>(&*services)
                        .iter()
                        .filter_map(|uuid| parse_uuid(&cb_uuid_string(&**uuid)).ok())
                        .collect()
                })
                .unwrap_or_default();
            let tx_power =
                dict_value(advertisement, CBAdvertisementDataTxPowerLevelKey).map(|power| {
                    let raw: isize = msg_send![&*power, integerValue];
                    raw.clamp(i8::MIN as isize, i8::MAX as isize) as i8
                });
            ScanResult {
                id: DeviceId::new(device),
                name,
                rssi,
                advertisement: AdvertisementData {
                    local_name,
                    manufacturer_data,
                    service_uuids,
                    tx_power,
                },
            }
        }
    }

    /// Ends a discovery: one walk builds the portable tree and captures the live handles.
    unsafe fn collect_services(
        peripheral: &AnyObject,
    ) -> Result<(Vec<Service>, DeviceObjects), BluetoothError> {
        unsafe {
            let mut services_built = Vec::new();
            let mut objects = DeviceObjects::default();
            let services: Option<Retained<AnyObject>> = msg_send![peripheral, services];
            let Some(services) = services else {
                return Ok((services_built, objects));
            };
            for service in array_items::<_, CBService>(&*services) {
                let service_uuid = parse_uuid(&cb_uuid_string(&*service))?;
                let primary: bool = msg_send![&*service, isPrimary];
                let included: Option<Retained<AnyObject>> = msg_send![&*service, includedServices];
                let included_services = included
                    .as_ref()
                    .map(|list| {
                        array_items::<_, CBService>(&**list)
                            .iter()
                            .filter_map(|service| parse_uuid(&cb_uuid_string(&**service)).ok())
                            .collect()
                    })
                    .unwrap_or_default();
                let characteristics: Option<Retained<AnyObject>> =
                    msg_send![&*service, characteristics];
                let mut characteristics_built = Vec::new();
                if let Some(list) = &characteristics {
                    for characteristic in array_items::<_, CBCharacteristic>(&**list) {
                        let uuid = parse_uuid(&cb_uuid_string(&*characteristic))?;
                        let properties = cb_characteristic_properties_from_cb(
                            characteristic.properties().bits(),
                        );
                        let value = characteristic.value().map(|value| data_bytes(&*value));
                        let descriptors: Option<Retained<AnyObject>> =
                            msg_send![&*characteristic, descriptors];
                        let mut descriptors_built = Vec::new();
                        if let Some(list) = &descriptors {
                            for descriptor in array_items::<_, CBDescriptor>(&**list) {
                                let descriptor_uuid = parse_uuid(&cb_uuid_string(&*descriptor))?;
                                objects.descriptors.insert(
                                    descriptor_uuid.to_string(),
                                    (
                                        CbPtr(descriptor.clone()),
                                        Descriptor {
                                            uuid: descriptor_uuid.clone(),
                                            characteristic_uuid: uuid.clone(),
                                            service_uuid: service_uuid.clone(),
                                        },
                                    ),
                                );
                                descriptors_built.push(Descriptor {
                                    uuid: descriptor_uuid,
                                    characteristic_uuid: uuid.clone(),
                                    service_uuid: service_uuid.clone(),
                                });
                            }
                        }
                        let built = Characteristic {
                            uuid: uuid.clone(),
                            service_uuid: service_uuid.clone(),
                            properties,
                            descriptors: descriptors_built,
                            value,
                        };
                        objects
                            .characteristics
                            .insert(uuid.to_string(), (CbPtr(characteristic), built.clone()));
                        characteristics_built.push(built);
                    }
                }
                services_built.push(Service {
                    uuid: service_uuid,
                    primary,
                    characteristics: characteristics_built,
                    included_services,
                });
            }
            Ok((services_built, objects))
        }
    }

    fn finish_discovery(shared: &CentralShared, peripheral: &AnyObject, device: &str) {
        let failed = locked(&shared.discovery)
            .remove(device)
            .is_some_and(|progress| progress.failed);
        let outcome = if failed {
            Err(BluetoothError::new(ErrorKind::ServicesDiscoveryFailed))
        } else {
            unsafe { collect_services(peripheral) }.map(|(services, objects)| {
                locked(&shared.objects).insert(device.to_owned(), objects);
                services
            })
        };
        shared
            .waiters
            .services
            .resolve(&format!("{device}|services"), outcome);
    }

    /// The central backend over a `CBCentralManager`.
    pub struct CoreBluetoothCentral {
        manager: Option<CbPtr<CBCentralManager>>,
        // Managers hold their delegate weakly; this pin keeps it delivering callbacks.
        _delegate: CbPtr<CentralDelegate>,
        shared: Arc<CentralShared>,
    }

    impl CoreBluetoothCentral {
        fn new() -> Self {
            let shared = Arc::new(CentralShared::new());
            let delegate = CentralDelegate::new(Arc::clone(&shared));
            let manager = new_queue(c"nsh-bluetooth.central").and_then(|queue| {
                let manager: Option<Retained<CBCentralManager>> = unsafe {
                    msg_send![
                        CBCentralManager::alloc(),
                        initWithDelegate: &*delegate,
                        queue: &*queue
                    ]
                };
                manager
            });
            Self {
                manager: manager.map(CbPtr),
                _delegate: CbPtr(delegate),
                shared,
            }
        }

        fn manager(&self) -> Result<&CbPtr<CBCentralManager>, BluetoothError> {
            self.manager.as_ref().ok_or_else(backend_error)
        }

        fn peripheral(&self, id: &DeviceId) -> Result<CbPtr<CBPeripheral>, BluetoothError> {
            locked(&self.shared.peripherals)
                .get(&cb_device_key(id.as_str()))
                .cloned()
                .ok_or_else(|| BluetoothError::new(ErrorKind::DeviceNotFound))
        }

        /// The adapter state, waiting briefly for the first callback when it is not known yet.
        async fn adapter_state_ready(&self) -> Result<AdapterState, BluetoothError> {
            let manager = self.manager()?;
            let read_now = || -> AdapterState {
                let raw: isize = unsafe { msg_send![manager.get(), state] };
                cb_manager_state_to_adapter_state(raw)
            };
            let mut state = read_now();
            if state == AdapterState::Unknown {
                let mut changes = self.shared.state_tx.subscribe();
                let waited = timeout(STATE_WAIT, async {
                    loop {
                        match changes.recv().await {
                            Ok(next) if next != AdapterState::Unknown => return next,
                            Ok(_) => continue,
                            Err(_) => return AdapterState::Unknown,
                        }
                    }
                })
                .await;
                state = waited.unwrap_or(AdapterState::Unknown);
                if state == AdapterState::Unknown {
                    state = read_now();
                }
            }
            Ok(state)
        }

        fn gate(state: AdapterState) -> Result<(), BluetoothError> {
            let kind = match state {
                AdapterState::On => return Ok(()),
                AdapterState::PoweredOff => ErrorKind::PoweredOff,
                AdapterState::Unauthorized => ErrorKind::Unauthorized,
                AdapterState::Unsupported => ErrorKind::Unsupported,
                AdapterState::Resetting => ErrorKind::Resetting,
                AdapterState::Unknown => ErrorKind::UnknownState,
            };
            Err(BluetoothError::new(kind))
        }
    }

    impl CentralBackend for CoreBluetoothCentral {
        fn adapter_state(&self) -> BackendFuture<'_, Result<AdapterState, BluetoothError>> {
            Box::pin(async { self.adapter_state_ready().await })
        }

        fn state_changes(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<AdapterState>, BluetoothError>> {
            Box::pin(ready(Ok(self.shared.state_tx.subscribe())))
        }

        fn start_scan(
            &self,
            filter: ScanFilter,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ScanResult>, BluetoothError>> {
            Box::pin(async move {
                let manager = self.manager()?;
                Self::gate(self.adapter_state_ready().await?)?;
                let receiver = {
                    let mut scan = locked(&self.shared.scan);
                    if scan.is_some() {
                        return Err(BluetoothError::new(ErrorKind::InProgress));
                    }
                    let (sender, receiver) = broadcast::channel(32);
                    *scan = Some((sender, filter));
                    receiver
                };
                let services: Option<&AnyObject> = None;
                let options: Option<&AnyObject> = None;
                let _: () = unsafe {
                    msg_send![
                        manager.get(),
                        scanForPeripheralsWithServices: services,
                        options: options
                    ]
                };
                Ok(receiver)
            })
        }

        fn stop_scan(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(async {
                let manager = self.manager()?;
                *locked(&self.shared.scan) = None;
                let _: () = unsafe { msg_send![manager.get(), stopScan] };
                Ok(())
            })
        }

        fn system_devices(
            &self,
            service_uuids: &[Uuid],
        ) -> BackendFuture<'_, Result<Vec<DeviceInfo>, BluetoothError>> {
            let uuids = service_uuids.to_vec();
            Box::pin(async move {
                let manager = self.manager()?;
                if uuids.is_empty() {
                    return Ok(Vec::new());
                }
                let cb_uuids = uuids
                    .iter()
                    .map(|uuid| unsafe { cb_uuid(uuid) })
                    .collect::<Result<Vec<_>, _>>()?;
                let list = ns_array(&cb_uuids);
                let Some(list) = list else {
                    return Err(backend_error());
                };
                let peripherals: Option<Retained<AnyObject>> = unsafe {
                    msg_send![manager.get(), retrieveConnectedPeripheralsWithServices: &*list]
                };
                let mut devices = Vec::new();
                if let Some(peripherals) = &peripherals {
                    for peripheral in unsafe { array_items::<_, CBPeripheral>(&**peripherals) } {
                        let id = unsafe { peripheral_id(&*peripheral) };
                        let Some(id) = id else {
                            continue;
                        };
                        let name = unsafe {
                            let name: Option<Retained<AnyObject>> = msg_send![&*peripheral, name];
                            name.map(|name| string_value(&*name))
                        };
                        devices.push(DeviceInfo {
                            id: DeviceId::new(id),
                            name,
                        });
                    }
                }
                Ok(devices)
            })
        }

        fn connect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.clone();
            Box::pin(async move {
                let manager = self.manager()?;
                let peripheral = self.retrieve_peripheral(&id)?;
                let state: isize = unsafe { msg_send![peripheral.get(), state] };
                if state == CBPeripheralState::Connected.0 {
                    return Err(BluetoothError::new(ErrorKind::AlreadyConnected));
                }
                Self::gate(self.adapter_state_ready().await?)?;
                let key = connect_key(id.as_str());
                let (waiter_id, waiter) = self.shared.waiters.connect.register(&key);
                let options: Option<&AnyObject> = None;
                let _: () = unsafe {
                    msg_send![
                        manager.get(),
                        connectPeripheral: peripheral.get(),
                        options: options
                    ]
                };
                let outcome = settle(
                    &self.shared.waiters.connect,
                    &key,
                    waiter_id,
                    waiter,
                    CONNECT_TIMEOUT,
                )
                .await;
                if matches!(outcome, Err(ref error) if error.kind() == ErrorKind::Timeout) {
                    // CoreBluetooth never times a pending connect out itself.
                    let _: () = unsafe {
                        msg_send![manager.get(), cancelPeripheralConnection: peripheral.get()]
                    };
                }
                outcome
            })
        }

        fn disconnect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.clone();
            Box::pin(async move {
                let manager = self.manager()?;
                let Ok(peripheral) = self.peripheral(&id) else {
                    return Ok(());
                };
                let state: isize = unsafe { msg_send![peripheral.get(), state] };
                if state != CBPeripheralState::Connected.0 {
                    return Ok(());
                }
                let key = disconnect_key(id.as_str());
                let (waiter_id, waiter) = self.shared.waiters.disconnect.register(&key);
                let _: () = unsafe {
                    msg_send![manager.get(), cancelPeripheralConnection: peripheral.get()]
                };
                match settle(
                    &self.shared.waiters.disconnect,
                    &key,
                    waiter_id,
                    waiter,
                    GATT_TIMEOUT,
                )
                .await
                {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == ErrorKind::Timeout => {
                        let state: isize = unsafe { msg_send![peripheral.get(), state] };
                        if state != CBPeripheralState::Connected.0 {
                            Ok(())
                        } else {
                            Err(error)
                        }
                    }
                    Err(error) => Err(error),
                }
            })
        }

        fn is_connected(&self, id: &DeviceId) -> BackendFuture<'_, Result<bool, BluetoothError>> {
            let id = id.clone();
            Box::pin(async move {
                match self.peripheral(&id) {
                    Ok(peripheral) => {
                        let state: isize = unsafe { msg_send![peripheral.get(), state] };
                        Ok(state == CBPeripheralState::Connected.0)
                    }
                    Err(_) => Ok(false),
                }
            })
        }

        fn read_rssi(&self, id: &DeviceId) -> BackendFuture<'_, Result<i8, BluetoothError>> {
            let id = id.clone();
            Box::pin(async move {
                let peripheral = self.peripheral(&id)?;
                let state: isize = unsafe { msg_send![peripheral.get(), state] };
                if state != CBPeripheralState::Connected.0 {
                    return Err(BluetoothError::new(ErrorKind::DeviceNotConnected));
                }
                let key = format!("{}|rssi", cb_device_key(id.as_str()));
                let (waiter_id, waiter) = self.shared.waiters.rssi.register(&key);
                unsafe { peripheral.get().readRSSI() };
                settle(
                    &self.shared.waiters.rssi,
                    &key,
                    waiter_id,
                    waiter,
                    GATT_TIMEOUT,
                )
                .await
            })
        }

        fn request_mtu(
            &self,
            id: &DeviceId,
            _mtu: u16,
        ) -> BackendFuture<'_, Result<u16, BluetoothError>> {
            let id = id.clone();
            Box::pin(async move {
                let peripheral = self.peripheral(&id)?;
                // iOS has no MTU request; the write ceiling is the negotiated ATT payload.
                let ceiling = unsafe {
                    peripheral
                        .get()
                        .maximumWriteValueLengthForType(CBCharacteristicWriteType::WithoutResponse)
                };
                Ok((ceiling.min(u16::MAX as usize) as u16).saturating_add(3))
            })
        }

        fn discover_services(
            &self,
            id: &DeviceId,
        ) -> BackendFuture<'_, Result<Vec<Service>, BluetoothError>> {
            let id = id.clone();
            Box::pin(async move {
                self.manager()?;
                let peripheral = self.peripheral(&id)?;
                let state: isize = unsafe { msg_send![peripheral.get(), state] };
                if state != CBPeripheralState::Connected.0 {
                    return Err(BluetoothError::new(ErrorKind::DeviceNotConnected));
                }
                Self::gate(self.adapter_state_ready().await?)?;
                let device = cb_device_key(id.as_str());
                let key = format!("{device}|services");
                {
                    // One discovery per device: a second would reset the first's counters.
                    let mut discovery = locked(&self.shared.discovery);
                    if discovery.contains_key(&device) {
                        return Err(BluetoothError::new(ErrorKind::InProgress));
                    }
                    discovery.insert(device.clone(), DiscoveryProgress::default());
                }
                let (waiter_id, waiter) = self.shared.waiters.services.register(&key);
                let none: Option<&AnyObject> = None;
                let _: () = unsafe { msg_send![peripheral.get(), discoverServices: none] };
                let outcome = settle(
                    &self.shared.waiters.services,
                    &key,
                    waiter_id,
                    waiter,
                    DISCOVERY_TIMEOUT,
                )
                .await;
                if matches!(outcome, Err(ref error) if error.kind() == ErrorKind::Timeout) {
                    // The chain will not complete on its own; free the retry gate.
                    locked(&self.shared.discovery).remove(&device);
                }
                outcome
            })
        }

        fn read_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            let id = id.clone();
            let characteristic = characteristic.clone();
            Box::pin(async move {
                let handle = self.characteristic_handle(&id, &characteristic)?;
                let peripheral = self.peripheral(&id)?;
                let key = format!(
                    "{}|{}",
                    cb_device_key(id.as_str()),
                    characteristic.uuid.as_str()
                );
                let (waiter_id, waiter) = self.shared.waiters.value.register(&key);
                unsafe { peripheral.get().readValueForCharacteristic(handle.get()) };
                settle(
                    &self.shared.waiters.value,
                    &key,
                    waiter_id,
                    waiter,
                    GATT_TIMEOUT,
                )
                .await
            })
        }

        fn write_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
            data: &[u8],
            write_type: WriteType,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.clone();
            let characteristic = characteristic.clone();
            let data = data.to_vec();
            Box::pin(async move {
                let handle = self.characteristic_handle(&id, &characteristic)?;
                let peripheral = self.peripheral(&id)?;
                let cb_type = match write_type {
                    WriteType::WithResponse => CBCharacteristicWriteType::WithResponse,
                    WriteType::WithoutResponse => CBCharacteristicWriteType::WithoutResponse,
                };
                if write_type == WriteType::WithoutResponse {
                    let ready: bool = unsafe { peripheral.get().canSendWriteWithoutResponse() };
                    if !ready {
                        return Err(BluetoothError::new(ErrorKind::CharacteristicWriteFailed));
                    }
                }
                let payload = ns_data(&data)
                    .ok_or_else(|| BluetoothError::new(ErrorKind::CharacteristicWriteFailed))?;
                let write_key = format!(
                    "{}|{}",
                    cb_device_key(id.as_str()),
                    characteristic.uuid.as_str()
                );
                let waiter = match write_type {
                    WriteType::WithResponse => {
                        let (waiter_id, waiter) = self.shared.waiters.write.register(&write_key);
                        Some((waiter_id, waiter))
                    }
                    WriteType::WithoutResponse => None,
                };
                let _: () = unsafe {
                    msg_send![
                        peripheral.get(),
                        writeValue: &**payload,
                        forCharacteristic: handle.get(),
                        type: cb_type
                    ]
                };
                match waiter {
                    // Without-response resolves once the value is sent.
                    None => Ok(()),
                    Some((waiter_id, waiter)) => {
                        settle(
                            &self.shared.waiters.write,
                            &write_key,
                            waiter_id,
                            waiter,
                            GATT_TIMEOUT,
                        )
                        .await
                    }
                }
            })
        }

        fn subscribe_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ValueNotification>, BluetoothError>>
        {
            let id = id.clone();
            let characteristic = characteristic.clone();
            Box::pin(async move {
                let handle = self.characteristic_handle(&id, &characteristic)?;
                let peripheral = self.peripheral(&id)?;
                let key = format!(
                    "{}|{}",
                    cb_device_key(id.as_str()),
                    characteristic.uuid.as_str()
                );
                let (waiter_id, waiter) = self.shared.waiters.notify_change.register(&key);
                unsafe {
                    peripheral
                        .get()
                        .setNotifyValue_forCharacteristic(true, handle.get())
                };
                settle(
                    &self.shared.waiters.notify_change,
                    &key,
                    waiter_id,
                    waiter,
                    GATT_TIMEOUT,
                )
                .await?;
                let mut streams = locked(&self.shared.notify_tx);
                if !streams.contains_key(&key) {
                    let (sender, _) = broadcast::channel(32);
                    streams.insert(key.clone(), sender);
                }
                Ok(streams
                    .get(&key)
                    .map(|sender| sender.subscribe())
                    .unwrap_or_else(|| broadcast::channel(32).1))
            })
        }

        fn unsubscribe_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.clone();
            let characteristic = characteristic.clone();
            Box::pin(async move {
                let handle = self.characteristic_handle(&id, &characteristic)?;
                let peripheral = self.peripheral(&id)?;
                let key = format!(
                    "{}|{}",
                    cb_device_key(id.as_str()),
                    characteristic.uuid.as_str()
                );
                let subscribed = locked(&self.shared.notify_tx).contains_key(&key);
                if !subscribed {
                    return Ok(());
                }
                let (waiter_id, waiter) = self.shared.waiters.notify_change.register(&key);
                unsafe {
                    peripheral
                        .get()
                        .setNotifyValue_forCharacteristic(false, handle.get())
                };
                settle(
                    &self.shared.waiters.notify_change,
                    &key,
                    waiter_id,
                    waiter,
                    GATT_TIMEOUT,
                )
                .await?;
                locked(&self.shared.notify_tx).remove(&key);
                Ok(())
            })
        }

        fn read_descriptor(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            let id = id.clone();
            let descriptor = descriptor.clone();
            Box::pin(async move {
                let handle = self.descriptor_handle(&id, &descriptor)?;
                let peripheral = self.peripheral(&id)?;
                let key = format!(
                    "{}|{}",
                    cb_device_key(id.as_str()),
                    descriptor.uuid.as_str()
                );
                let (waiter_id, waiter) = self.shared.waiters.descriptor_read.register(&key);
                unsafe { peripheral.get().readValueForDescriptor(handle.get()) };
                settle(
                    &self.shared.waiters.descriptor_read,
                    &key,
                    waiter_id,
                    waiter,
                    GATT_TIMEOUT,
                )
                .await
            })
        }

        fn write_descriptor(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
            data: &[u8],
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.clone();
            let descriptor = descriptor.clone();
            let data = data.to_vec();
            Box::pin(async move {
                let handle = self.descriptor_handle(&id, &descriptor)?;
                let peripheral = self.peripheral(&id)?;
                let payload = ns_data(&data)
                    .ok_or_else(|| BluetoothError::new(ErrorKind::DescriptorWriteFailed))?;
                let key = format!(
                    "{}|{}",
                    cb_device_key(id.as_str()),
                    descriptor.uuid.as_str()
                );
                let (waiter_id, waiter) = self.shared.waiters.descriptor_write.register(&key);
                let _: () = unsafe {
                    msg_send![
                        peripheral.get(),
                        writeValue: &**payload,
                        forDescriptor: handle.get()
                    ]
                };
                settle(
                    &self.shared.waiters.descriptor_write,
                    &key,
                    waiter_id,
                    waiter,
                    GATT_TIMEOUT,
                )
                .await
            })
        }

        fn connection_events(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ConnectionEvent>, BluetoothError>>
        {
            Box::pin(ready(Ok(self.shared.events_tx.subscribe())))
        }
    }

    impl CoreBluetoothCentral {
        fn characteristic_handle(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> Result<CbPtr<CBCharacteristic>, BluetoothError> {
            locked(&self.shared.objects)
                .get(&cb_device_key(id.as_str()))
                .and_then(|objects| objects.characteristics.get(characteristic.uuid.as_str()))
                .map(|(handle, _)| handle.clone())
                .ok_or_else(|| BluetoothError::new(ErrorKind::CharacteristicNotFound))
        }

        fn descriptor_handle(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
        ) -> Result<CbPtr<CBDescriptor>, BluetoothError> {
            locked(&self.shared.objects)
                .get(&cb_device_key(id.as_str()))
                .and_then(|objects| objects.descriptors.get(descriptor.uuid.as_str()))
                .map(|(handle, _)| handle.clone())
                .ok_or_else(|| BluetoothError::new(ErrorKind::DescriptorNotFound))
        }

        /// Finds the peripheral by id, retrieving it from the system when only known there.
        fn retrieve_peripheral(
            &self,
            id: &DeviceId,
        ) -> Result<CbPtr<CBPeripheral>, BluetoothError> {
            let device = cb_device_key(id.as_str());
            if let Some(peripheral) = locked(&self.shared.peripherals).get(&device).cloned() {
                return Ok(peripheral);
            }
            let manager = self.manager()?;
            // NSUUID rejects short spellings, so retrieval uses the canonical form the map key
            // uses; non-UUID ids still fail the NSUUID construction with a coded error.
            let uuid = ns_string(&device).ok_or_else(backend_error)?;
            let nsuuid_class = class(c"NSUUID").ok_or_else(backend_error)?;
            let nsuuid: Option<Retained<AnyObject>> =
                unsafe { msg_send![nsuuid_class, UUIDWithString: &*uuid] };
            let Some(nsuuid) = nsuuid else {
                return Err(backend_error());
            };
            let list = ns_array(&[nsuuid]).ok_or_else(backend_error)?;
            let peripherals: Option<Retained<AnyObject>> =
                unsafe { msg_send![manager.get(), retrievePeripheralsWithIdentifiers: &*list] };
            let found = peripherals
                .as_ref()
                .map(|list| unsafe { array_items::<_, CBPeripheral>(&**list) })
                .unwrap_or_default();
            let Some(peripheral) = found.into_iter().next() else {
                return Err(BluetoothError::new(ErrorKind::DeviceNotFound));
            };
            let handle = CbPtr(peripheral);
            locked(&self.shared.peripherals).insert(device, handle.clone());
            Ok(handle)
        }
    }

    pub fn central_backend() -> Arc<dyn CentralBackend> {
        Arc::new(CoreBluetoothCentral::new())
    }

    // ---- The server role. ----

    struct ServerShared {
        state: Mutex<AdapterState>,
        state_tx: broadcast::Sender<AdapterState>,
        services: Mutex<HashMap<String, CbPtr<CBMutableService>>>,
        characteristics: Mutex<HashMap<String, CbPtr<CBMutableCharacteristic>>>,
        values: Mutex<HashMap<String, Vec<u8>>>,
        subscribers: Mutex<HashMap<String, usize>>,
        requests_tx: broadcast::Sender<GattRequest>,
        added: Slots<()>,
        advertising: Slots<()>,
    }

    impl ServerShared {
        fn new() -> Self {
            let (state_tx, _) = broadcast::channel(32);
            let (requests_tx, _) = broadcast::channel(32);
            Self {
                state: Mutex::new(AdapterState::Unknown),
                state_tx,
                services: Mutex::new(HashMap::new()),
                characteristics: Mutex::new(HashMap::new()),
                values: Mutex::new(HashMap::new()),
                subscribers: Mutex::new(HashMap::new()),
                requests_tx,
                added: Slots::new(),
                advertising: Slots::new(),
            }
        }

        fn char_key(service: &Uuid, characteristic: &Uuid) -> String {
            format!("{service}|{characteristic}")
        }
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and no callback drops a CoreBluetooth
        // object the backend still owns.
        #[unsafe(super(NSObject))]
        #[thread_kind = AnyThread]
        #[ivars = Arc<ServerShared>]
        struct ServerDelegate;

        unsafe impl NSObjectProtocol for ServerDelegate {}

        impl ServerDelegate {
            #[unsafe(method(peripheralManagerDidUpdateState:))]
            fn did_update_state(&self, manager: &AnyObject) {
                let raw: isize = unsafe { msg_send![manager, state] };
                let state = cb_manager_state_to_adapter_state(raw);
                let shared = self.ivars();
                *locked(&shared.state) = state;
                let _ = shared.state_tx.send(state);
                if state != AdapterState::On {
                    // A state below poweredOn stops an advertisement we may be starting.
                    let kind = match state {
                        AdapterState::Unauthorized => ErrorKind::Unauthorized,
                        _ => ErrorKind::AdvertiseFailed,
                    };
                    shared
                        .advertising
                        .resolve("advertising", Err(BluetoothError::new(kind)));
                }
            }

            #[unsafe(method(peripheralManagerDidStartAdvertising:error:))]
            fn did_start_advertising(
                &self,
                _manager: &AnyObject,
                error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                let outcome = match error {
                    Some(error) => Err(BluetoothError::new(ErrorKind::AdvertiseFailed)
                        .with_status(unsafe { ns_error_code(error) })),
                    None => Ok(()),
                };
                shared.advertising.resolve("advertising", outcome);
            }

            #[unsafe(method(peripheralManager:didAddService:error:))]
            fn did_add_service(
                &self,
                _manager: &AnyObject,
                service: &AnyObject,
                error: Option<&AnyObject>,
            ) {
                let shared = self.ivars();
                // The key must land on the canonical form registration used.
                let Some(uuid) = cb_uuid_key(&unsafe { cb_uuid_string(service) }) else {
                    return;
                };
                let outcome = match error {
                    Some(error) => Err(BluetoothError::new(ErrorKind::ServerFailed)
                        .with_status(unsafe { ns_error_code(error) })),
                    None => Ok(()),
                };
                shared.added.resolve(&uuid, outcome);
            }

            #[unsafe(method(peripheralManager:central:didSubscribeToCharacteristic:))]
            fn did_subscribe(
                &self,
                _manager: &AnyObject,
                _central: &AnyObject,
                characteristic: &AnyObject,
            ) {
                let shared = self.ivars();
                if let Some((service, characteristic)) = server_char_keys(characteristic) {
                    let key = ServerShared::char_key(&service, &characteristic);
                    *locked(&shared.subscribers).entry(key).or_insert(0) += 1;
                }
            }

            #[unsafe(method(peripheralManager:central:didUnsubscribeFromCharacteristic:))]
            fn did_unsubscribe(
                &self,
                _manager: &AnyObject,
                _central: &AnyObject,
                characteristic: &AnyObject,
            ) {
                let shared = self.ivars();
                if let Some((service, characteristic)) = server_char_keys(characteristic) {
                    let key = ServerShared::char_key(&service, &characteristic);
                    if let Some(count) = locked(&shared.subscribers).get_mut(&key) {
                        *count = count.saturating_sub(1);
                    }
                }
            }

            #[unsafe(method(peripheralManager:didReceiveReadRequest:))]
            fn did_receive_read(&self, manager: &AnyObject, request: &AnyObject) {
                let shared = self.ivars();
                let offset: usize = unsafe { msg_send![request, offset] };
                let characteristic: Option<Retained<AnyObject>> =
                    unsafe { msg_send![request, characteristic] };
                let Some(characteristic) = characteristic else {
                    unsafe { respond(manager, Some(request), CBATTError::UnlikelyError.0) };
                    return;
                };
                let keys = server_char_keys(&characteristic);
                let value = keys
                    .as_ref()
                    .map(|(service, characteristic)| {
                        let key = ServerShared::char_key(service, characteristic);
                        locked(&shared.values).get(&key).cloned().unwrap_or_default()
                    })
                    .unwrap_or_default();
                let result = if offset > value.len() {
                    CBATTError::InvalidOffset.0
                } else {
                    match ns_data(&value[offset..]) {
                        Some(data) => {
                            let _: () = unsafe { msg_send![request, setValue: Some(&**data)] };
                            CBATTError::Success.0
                        }
                        None => CBATTError::UnlikelyError.0,
                    }
                };
                unsafe { respond(manager, Some(request), result) };
                if let Some((service, characteristic)) = keys {
                    let _ = shared.requests_tx.send(GattRequest {
                        service_uuid: service,
                        characteristic_uuid: characteristic,
                        kind: GattRequestKind::Read { offset },
                    });
                }
            }

            #[unsafe(method(peripheralManager:didReceiveWriteRequests:))]
            fn did_receive_writes(&self, manager: &AnyObject, requests: &AnyObject) {
                let shared = self.ivars();
                let count = unsafe { array_count(requests) };
                let mut last: Option<Retained<AnyObject>> = None;
                let mut invalid_offset = false;
                for index in 0..count {
                    let request = unsafe { array_item::<AnyObject, AnyObject>(requests, index) };
                    let Some(request) = request else {
                        continue;
                    };
                    last = Some(request.clone());
                    let characteristic: Option<Retained<AnyObject>> =
                        unsafe { msg_send![&*request, characteristic] };
                    let offset: usize = unsafe { msg_send![&*request, offset] };
                    let value: Option<Retained<AnyObject>> = unsafe { msg_send![&*request, value] };
                    let data = unsafe { value.as_ref().map(|value| data_bytes(&**value)) }
                        .unwrap_or_default();
                    let Some((service, characteristic)) = characteristic
                        .as_ref()
                        .and_then(|c| server_char_keys(c))
                    else {
                        continue;
                    };
                    let key = ServerShared::char_key(&service, &characteristic);
                    // A hostile offset must not reach the resize or the slicing; ATT allows at
                    // most the current length, and the sum is checked before it is trusted.
                    let stored_len = locked(&shared.values).get(&key).map_or(0, Vec::len);
                    let Some(end) = offset.checked_add(data.len()) else {
                        invalid_offset = true;
                        continue;
                    };
                    if offset > stored_len || end > MAX_SERVED_VALUE {
                        invalid_offset = true;
                        continue;
                    }
                    {
                        let mut values = locked(&shared.values);
                        let stored = values.entry(key).or_default();
                        // A write extends the stored value; the ack must cover the bytes.
                        if stored.len() < end {
                            stored.resize(end, 0);
                        }
                        stored[offset..end].copy_from_slice(&data);
                    }
                    let _ = shared.requests_tx.send(GattRequest {
                        service_uuid: service.clone(),
                        characteristic_uuid: characteristic.clone(),
                        kind: GattRequestKind::Write {
                            offset,
                            data,
                            with_response: true,
                        },
                    });
                }
                // CoreBluetooth wants exactly one answer for the whole batch; with no usable
                // request a nil response still satisfies the contract without touching state.
                let result = if invalid_offset {
                    CBATTError::InvalidOffset.0
                } else {
                    CBATTError::Success.0
                };
                match last {
                    Some(request) => unsafe { respond(manager, Some(&*request), result) },
                    None => unsafe {
                        respond(manager, None, CBATTError::InvalidPdu.0);
                    },
                }
            }
        }
    );

    impl ServerDelegate {
        fn new(shared: Arc<ServerShared>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(shared);
            unsafe { msg_send![super(this), init] }
        }
    }

    /// A nil request is a no-op on the ObjC side, so an unusable batch still gets one answer.
    unsafe fn respond(manager: &AnyObject, request: Option<&AnyObject>, result: isize) {
        unsafe {
            let _: () = msg_send![manager, respondToRequest: request, withResult: result];
        }
    }

    /// The service and characteristic uuids a served characteristic carries, both in the
    /// canonical form the registration keys use.
    fn server_char_keys(characteristic: &AnyObject) -> Option<(Uuid, Uuid)> {
        let service_uuid = unsafe {
            let service: Option<Retained<AnyObject>> = msg_send![characteristic, service];
            service.as_ref().map(|service| cb_uuid_string(&**service))
        };
        let characteristic_uuid = unsafe { cb_uuid_string(characteristic) };
        Some((
            parse_uuid(&service_uuid?).ok()?,
            parse_uuid(&characteristic_uuid).ok()?,
        ))
    }

    /// The GATT server backend over a `CBPeripheralManager`.
    pub struct CoreBluetoothServer {
        manager: Option<CbPtr<CBPeripheralManager>>,
        // Managers hold their delegate weakly; this pin keeps it delivering callbacks.
        _delegate: CbPtr<ServerDelegate>,
        shared: Arc<ServerShared>,
    }

    impl CoreBluetoothServer {
        fn new() -> Self {
            let shared = Arc::new(ServerShared::new());
            let delegate = ServerDelegate::new(Arc::clone(&shared));
            let manager = new_queue(c"nsh-bluetooth.server").and_then(|queue| {
                let manager: Option<Retained<CBPeripheralManager>> = unsafe {
                    msg_send![
                        CBPeripheralManager::alloc(),
                        initWithDelegate: &*delegate,
                        queue: &*queue
                    ]
                };
                manager
            });
            Self {
                manager: manager.map(CbPtr),
                _delegate: CbPtr(delegate),
                shared,
            }
        }

        fn manager(&self) -> Result<&CbPtr<CBPeripheralManager>, BluetoothError> {
            self.manager.as_ref().ok_or_else(backend_error)
        }

        /// The adapter state, waiting briefly for the first callback when it is not known yet.
        async fn state_ready(&self) -> Result<AdapterState, BluetoothError> {
            let manager = self.manager()?;
            let read_now = || -> AdapterState {
                let raw: isize = unsafe { msg_send![manager.get(), state] };
                cb_manager_state_to_adapter_state(raw)
            };
            let mut state = read_now();
            if state == AdapterState::Unknown {
                let mut changes = self.shared.state_tx.subscribe();
                let waited = timeout(STATE_WAIT, async {
                    loop {
                        match changes.recv().await {
                            Ok(next) if next != AdapterState::Unknown => return next,
                            Ok(_) => continue,
                            Err(_) => return AdapterState::Unknown,
                        }
                    }
                })
                .await;
                state = waited.unwrap_or(AdapterState::Unknown);
                if state == AdapterState::Unknown {
                    state = read_now();
                }
            }
            Ok(state)
        }
    }

    impl ServerBackend for CoreBluetoothServer {
        fn add_service(&self, service: Service) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(async move {
                let manager = self.manager()?;
                let state = self.state_ready().await?;
                if state != AdapterState::On {
                    let kind = match state {
                        AdapterState::Unauthorized => ErrorKind::Unauthorized,
                        AdapterState::PoweredOff => ErrorKind::PoweredOff,
                        AdapterState::Resetting => ErrorKind::Resetting,
                        AdapterState::Unsupported => ErrorKind::Unsupported,
                        AdapterState::Unknown => ErrorKind::UnknownState,
                        AdapterState::On => return Ok(()),
                    };
                    return Err(BluetoothError::new(kind));
                }
                if locked(&self.shared.services).contains_key(service.uuid.as_str()) {
                    return Err(BluetoothError::new(ErrorKind::ServerFailed));
                }
                // The build's Foundation handles are not Send; their scope ends before the await.
                let (mutable, built_handles) = {
                    let built = service
                        .characteristics
                        .iter()
                        .map(|characteristic| unsafe {
                            build_mutable_characteristic(characteristic)
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let mutable = CbPtr(unsafe {
                        let cb_uuid = cb_uuid(&service.uuid)?;
                        CBMutableService::initWithType_primary(
                            CBMutableService::alloc(),
                            &cb_uuid,
                            service.primary,
                        )
                    });
                    if let Some(list) = ns_array(&built) {
                        let _: () =
                            unsafe { msg_send![&**mutable, setCharacteristics: Some(&*list)] };
                    }
                    let handles = built.into_iter().map(CbPtr).collect::<Vec<_>>();
                    (mutable, handles)
                };
                let add_key = service.uuid.as_str().to_owned();
                let (waiter_id, waiter) = self.shared.added.register(&add_key);
                unsafe { manager.get().addService(mutable.get()) };
                settle(
                    &self.shared.added,
                    &add_key,
                    waiter_id,
                    waiter,
                    GATT_TIMEOUT,
                )
                .await?;
                locked(&self.shared.services).insert(service.uuid.to_string(), mutable);
                {
                    let mut characteristics = locked(&self.shared.characteristics);
                    let mut values = locked(&self.shared.values);
                    for (portable, handle) in service.characteristics.iter().zip(built_handles) {
                        let key = ServerShared::char_key(&service.uuid, &portable.uuid);
                        characteristics.insert(key.clone(), handle);
                        values.insert(key, portable.value.clone().unwrap_or_default());
                    }
                }
                Ok(())
            })
        }

        fn remove_service(&self, uuid: Uuid) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(async move {
                let manager = self.manager()?;
                let Some(handle) = locked(&self.shared.services).remove(uuid.as_str()) else {
                    return Ok(());
                };
                let prefix = format!("{uuid}|");
                locked(&self.shared.characteristics).retain(|key, _| !key.starts_with(&prefix));
                locked(&self.shared.values).retain(|key, _| !key.starts_with(&prefix));
                locked(&self.shared.subscribers).retain(|key, _| !key.starts_with(&prefix));
                unsafe { manager.get().removeService(handle.get()) };
                Ok(())
            })
        }

        fn start_advertising(
            &self,
            options: AdvertiseOptions,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(async move {
                let manager = self.manager()?;
                let authorization: isize =
                    unsafe { msg_send![CBPeripheralManager::class(), authorization] };
                if let Some(kind) = cb_server_authorization_to_kind(authorization) {
                    return Err(BluetoothError::new(kind));
                }
                let state = self.state_ready().await?;
                if state != AdapterState::On {
                    let kind = match state {
                        AdapterState::Unauthorized => ErrorKind::Unauthorized,
                        _ => ErrorKind::AdvertiseFailed,
                    };
                    return Err(BluetoothError::new(kind));
                }
                let advertise_key = "advertising";
                let (waiter_id, waiter) = self.shared.advertising.register(advertise_key);
                // The dictionary is Foundation-owned and not Send; scope it to the send.
                {
                    let dict = mutable_dictionary().ok_or_else(backend_error)?;
                    if let Some(name) = &options.local_name {
                        let name = ns_string(name)
                            .ok_or_else(|| BluetoothError::new(ErrorKind::AdvertiseFailed))?;
                        let _: () = unsafe {
                            msg_send![
                                &*dict,
                                setObject: &*name,
                                forKey: CBAdvertisementDataLocalNameKey
                            ]
                        };
                    }
                    if !options.service_uuids.is_empty() {
                        let uuids = options
                            .service_uuids
                            .iter()
                            .map(|uuid| unsafe { cb_uuid(uuid) })
                            .collect::<Result<Vec<_>, _>>()?;
                        let list = ns_array(&uuids)
                            .ok_or_else(|| BluetoothError::new(ErrorKind::AdvertiseFailed))?;
                        let _: () = unsafe {
                            msg_send![
                                &*dict,
                                setObject: &*list,
                                forKey: CBAdvertisementDataServiceUUIDsKey
                            ]
                        };
                    }
                    let _: () = unsafe { msg_send![manager.get(), startAdvertising: Some(&*dict)] };
                }
                settle(
                    &self.shared.advertising,
                    advertise_key,
                    waiter_id,
                    waiter,
                    GATT_TIMEOUT,
                )
                .await
            })
        }

        fn stop_advertising(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(async {
                let manager = self.manager()?;
                let _: () = unsafe { msg_send![manager.get(), stopAdvertising] };
                Ok(())
            })
        }

        fn is_advertising(&self) -> BackendFuture<'_, Result<bool, BluetoothError>> {
            Box::pin(async {
                let manager = self.manager()?;
                Ok(unsafe { manager.get().isAdvertising() })
            })
        }

        fn set_characteristic_value(
            &self,
            service: Uuid,
            characteristic: Uuid,
            value: Vec<u8>,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(async move {
                self.manager()?;
                let key = ServerShared::char_key(&service, &characteristic);
                let prefix = format!("{service}|");
                let characteristics = locked(&self.shared.characteristics);
                if !characteristics.keys().any(|key| key.starts_with(&prefix)) {
                    return Err(BluetoothError::new(ErrorKind::ServiceNotFound));
                }
                if !characteristics.contains_key(&key) {
                    return Err(BluetoothError::new(ErrorKind::CharacteristicNotFound));
                }
                drop(characteristics);
                locked(&self.shared.values).insert(key, value);
                Ok(())
            })
        }

        fn notify(
            &self,
            service: Uuid,
            characteristic: Uuid,
            data: &[u8],
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let data = data.to_vec();
            Box::pin(async move {
                let manager = self.manager()?;
                let key = ServerShared::char_key(&service, &characteristic);
                let handle = locked(&self.shared.characteristics)
                    .get(&key)
                    .cloned()
                    .ok_or_else(|| BluetoothError::new(ErrorKind::CharacteristicNotFound))?;
                let subscribers = locked(&self.shared.subscribers)
                    .get(&key)
                    .copied()
                    .unwrap_or(0);
                if subscribers == 0 {
                    return Err(BluetoothError::new(
                        ErrorKind::CharacteristicNotifyChangeFailed,
                    ));
                }
                let payload = ns_data(&data).ok_or_else(|| {
                    BluetoothError::new(ErrorKind::CharacteristicNotifyChangeFailed)
                })?;
                let centrals: Option<&AnyObject> = None;
                let sent: bool = unsafe {
                    msg_send![
                        manager.get(),
                        updateValue: &**payload,
                        forCharacteristic: handle.get(),
                        onSubscribedCentrals: centrals
                    ]
                };
                if !sent {
                    return Err(BluetoothError::new(
                        ErrorKind::CharacteristicNotifyChangeFailed,
                    ));
                }
                locked(&self.shared.values).insert(key, data.to_vec());
                Ok(())
            })
        }

        fn requests(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<GattRequest>, BluetoothError>> {
            Box::pin(ready(Ok(self.shared.requests_tx.subscribe())))
        }
    }

    unsafe fn build_mutable_characteristic(
        characteristic: &Characteristic,
    ) -> Result<Retained<CBMutableCharacteristic>, BluetoothError> {
        unsafe {
            let uuid = cb_uuid(&characteristic.uuid)?;
            let properties =
                CbProps::from_bits_retain(cb_properties_bits_from_ours(characteristic.properties));
            let mut permissions = CBAttributePermissions::empty();
            if properties.contains(CbProps::Read) {
                permissions |= CBAttributePermissions::Readable;
            }
            if properties.intersects(CbProps::Write.union(CbProps::WriteWithoutResponse)) {
                permissions |= CBAttributePermissions::Writeable;
            }
            // A nil value makes every read and write arrive as a request we serve from state.
            let built = CBMutableCharacteristic::initWithType_properties_value_permissions(
                CBMutableCharacteristic::alloc(),
                &uuid,
                properties,
                None,
                permissions,
            );
            let descriptors = characteristic
                .descriptors
                .iter()
                .map(|descriptor| build_mutable_descriptor(&descriptor.uuid))
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(list) = ns_array(&descriptors) {
                let _: () = msg_send![&*built, setDescriptors: Some(&*list)];
            }
            Ok(built)
        }
    }

    /// Our `Descriptor` carries no value, so served descriptors publish an empty string.
    unsafe fn build_mutable_descriptor(
        uuid: &Uuid,
    ) -> Result<Retained<CBMutableDescriptor>, BluetoothError> {
        unsafe {
            let cb_uuid = cb_uuid(uuid)?;
            let value = ns_string("").ok_or_else(backend_error)?;
            Ok(CBMutableDescriptor::initWithType_value(
                CBMutableDescriptor::alloc(),
                &cb_uuid,
                Some(&*value),
            ))
        }
    }

    pub fn server_backend() -> Arc<dyn ServerBackend> {
        Arc::new(CoreBluetoothServer::new())
    }
}
