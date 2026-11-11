//! Android bridge: the host app's BLE service over JNI. Pure mapping and codec helpers compile
//! and test on every host; the native implementation is Android-only and backs both selectors.

use std::sync::Arc;

use crate::backend::{AdapterState, CentralBackend, ServerBackend};
use crate::error::{BluetoothError, ErrorKind};

/// Host error codes at or above this carry a `BleErrorCode` value from react-native-ble-plx's
/// table (`.refs/react-native-ble-plx/src/BleError.js`), clear of Android's status range.
const BRIDGE_ERROR_BASE: i32 = 10_000;

/// One `BleErrorCode` value to its kind. `700`/`701` are this protocol's own server codes; plx
/// has no peripheral role. Rows without a kind of their own (`202`, `505`) fall back to `Backend`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))] // test-only off-target; the backend uses it on Android
fn ble_error_code_to_kind(code: i32) -> ErrorKind {
    match code {
        2 => ErrorKind::Cancelled,
        3 => ErrorKind::Timeout,
        5 => ErrorKind::InvalidIdentifiers,
        100 => ErrorKind::Unsupported,
        101 => ErrorKind::Unauthorized,
        102 => ErrorKind::PoweredOff,
        103 => ErrorKind::UnknownState,
        104 => ErrorKind::Resetting,
        105 => ErrorKind::StateChangeFailed,
        200 => ErrorKind::ConnectFailed,
        201 => ErrorKind::Disconnected,
        203 => ErrorKind::AlreadyConnected,
        204 => ErrorKind::DeviceNotFound,
        205 => ErrorKind::DeviceNotConnected,
        206 => ErrorKind::MtuChangeFailed,
        300 | 301 => ErrorKind::ServicesDiscoveryFailed,
        302 => ErrorKind::ServiceNotFound,
        303 => ErrorKind::ServicesNotDiscovered,
        // A failed characteristics discovery leaves them undiscovered; there is no finer kind.
        400 | 405 => ErrorKind::CharacteristicsNotDiscovered,
        401 => ErrorKind::CharacteristicWriteFailed,
        402 => ErrorKind::CharacteristicReadFailed,
        403 => ErrorKind::CharacteristicNotifyChangeFailed,
        404 => ErrorKind::CharacteristicNotFound,
        406 => ErrorKind::CharacteristicInvalidDataFormat,
        // Same reasoning as characteristics; and 505 has no descriptor-format kind.
        500 | 504 => ErrorKind::DescriptorsNotDiscovered,
        501 => ErrorKind::DescriptorWriteFailed,
        502 => ErrorKind::DescriptorReadFailed,
        503 => ErrorKind::DescriptorNotFound,
        506 => ErrorKind::DescriptorWriteNotAllowed,
        600 => ErrorKind::ScanStartFailed,
        601 => ErrorKind::LocationServicesDisabled,
        700 => ErrorKind::AdvertiseFailed,
        701 => ErrorKind::ServerFailed,
        _ => ErrorKind::Backend,
    }
}

/// A host status to its kind: `-1` unknown, `133` Android's GATT status, `10000+` a `BleErrorCode`
/// per the plx table, `100..=999` the same table in its raw spelling. Everything else is `Backend`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))] // test-only off-target; the backend uses it on Android
fn android_status_to_kind(status: i32) -> ErrorKind {
    match status {
        -1 => ErrorKind::Backend,
        133 => ErrorKind::ConnectFailed,
        status if status >= BRIDGE_ERROR_BASE => ble_error_code_to_kind(status - BRIDGE_ERROR_BASE),
        100..=999 => ble_error_code_to_kind(status),
        _ => ErrorKind::Backend,
    }
}

/// A host status as a coded error, carrying the status (`133` arrives with `status` set). `-1`
/// keeps no status; it is the host's "unknown".
#[cfg_attr(not(target_os = "android"), allow(dead_code))] // test-only off-target; the backend uses it on Android
fn android_status_to_error(status: i32) -> BluetoothError {
    let error = BluetoothError::new(android_status_to_kind(status));
    if status == -1 {
        error
    } else {
        error.with_status(i64::from(status))
    }
}

/// `BluetoothAdapter.STATE_*` to the crate's adapter state. The turning states are transient,
/// which the crate models as `Resetting`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))] // test-only off-target; the backend uses it on Android
fn android_state_to_adapter_state(state: i32) -> AdapterState {
    match state {
        10 => AdapterState::PoweredOff, // STATE_OFF
        11 => AdapterState::Resetting,  // STATE_TURNING_ON
        12 => AdapterState::On,         // STATE_ON
        13 => AdapterState::Resetting,  // STATE_TURNING_OFF
        _ => AdapterState::Unknown,
    }
}

/// The wire codec shared with the Java host: little-endian numbers, `u16` length prefixes,
/// UTF-8 strings, UUIDs in their canonical form. Every layout here is part of the documented
/// contract in `native` below.
#[cfg_attr(not(target_os = "android"), allow(dead_code))] // test-only off-target; the backend uses it on Android
pub(super) mod wire {
    use crate::backend::{
        AdvertisementData, Characteristic, CharacteristicProperties, Descriptor, DeviceId,
        DeviceInfo, GattRequest, GattRequestKind, ScanFilter, ScanResult, Service, Uuid,
        ValueNotification,
    };
    use crate::error::{BluetoothError, ErrorKind};

    use super::android_status_to_error;

    /// A host notification decoded far enough to route and deliver it.
    #[derive(Debug, PartialEq)]
    pub(super) struct HostNotification {
        /// The device the value arrived from, as the host spells its id.
        pub(super) device: String,
        /// The value and the characteristic it belongs to.
        pub(super) value: ValueNotification,
    }

    /// Cursor over a wire buffer. Every read is total: a short buffer answers `None`.
    struct Reader<'a> {
        bytes: &'a [u8],
        at: usize,
    }

    impl<'a> Reader<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self { bytes, at: 0 }
        }

        fn take(&mut self, len: usize) -> Option<&'a [u8]> {
            let end = self.at.checked_add(len)?;
            let slice = self.bytes.get(self.at..end)?;
            self.at = end;
            Some(slice)
        }

        fn u8(&mut self) -> Option<u8> {
            self.take(1).and_then(|slice| slice.first().copied())
        }

        fn i8(&mut self) -> Option<i8> {
            self.u8().map(|byte| byte as i8)
        }

        fn u16(&mut self) -> Option<u16> {
            let bytes: [u8; 2] = self.take(2)?.try_into().ok()?;
            Some(u16::from_le_bytes(bytes))
        }

        fn u32(&mut self) -> Option<u32> {
            let bytes: [u8; 4] = self.take(4)?.try_into().ok()?;
            Some(u32::from_le_bytes(bytes))
        }

        fn text(&mut self) -> Option<&'a str> {
            let len = self.u16()? as usize;
            self.take(len)
                .and_then(|slice| std::str::from_utf8(slice).ok())
        }

        fn uuid(&mut self) -> Option<Uuid> {
            Uuid::parse(self.text()?).ok()
        }

        fn bytes(&mut self) -> Option<Vec<u8>> {
            let len = self.u16()? as usize;
            self.take(len).map(<[u8]>::to_vec)
        }

        fn optional_text(&mut self) -> Option<Option<String>> {
            if self.u8()? == 0 {
                Some(None)
            } else {
                Some(Some(self.text()?.to_owned()))
            }
        }

        fn optional_i8(&mut self) -> Option<Option<i8>> {
            if self.u8()? == 0 {
                Some(None)
            } else {
                Some(Some(self.i8()?))
            }
        }

        fn optional_bytes(&mut self) -> Option<Option<Vec<u8>>> {
            if self.u8()? == 0 {
                Some(None)
            } else {
                Some(Some(self.bytes()?))
            }
        }
    }

    fn put_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes `text` with its u16 length prefix. `oversize` is the coded error for what the
    /// prefix cannot carry, so an over-long input resolves instead of corrupting the frame.
    fn put_text(out: &mut Vec<u8>, text: &str, oversize: ErrorKind) -> Result<(), BluetoothError> {
        let bytes = text.as_bytes();
        let len = u16::try_from(bytes.len()).map_err(|_| BluetoothError::new(oversize))?;
        put_u16(out, len);
        out.extend_from_slice(bytes);
        Ok(())
    }

    fn put_uuid(out: &mut Vec<u8>, uuid: &Uuid, oversize: ErrorKind) -> Result<(), BluetoothError> {
        put_text(out, uuid.as_str(), oversize)
    }

    /// Writes `bytes` with its u16 length prefix, refusing what the prefix cannot carry.
    fn put_bytes(
        out: &mut Vec<u8>,
        bytes: &[u8],
        oversize: ErrorKind,
    ) -> Result<(), BluetoothError> {
        let len = u16::try_from(bytes.len()).map_err(|_| BluetoothError::new(oversize))?;
        put_u16(out, len);
        out.extend_from_slice(bytes);
        Ok(())
    }

    fn put_count(
        out: &mut Vec<u8>,
        count: usize,
        oversize: ErrorKind,
    ) -> Result<(), BluetoothError> {
        let count = u16::try_from(count).map_err(|_| BluetoothError::new(oversize))?;
        put_u16(out, count);
        Ok(())
    }

    /// A bare list of canonical uuid strings. A list too long to count is a malformed set of
    /// identifiers.
    pub(super) fn encode_uuid_list(uuids: &[Uuid]) -> Result<Vec<u8>, BluetoothError> {
        let oversize = ErrorKind::InvalidIdentifiers;
        let mut out = Vec::new();
        put_count(&mut out, uuids.len(), oversize)?;
        for uuid in uuids {
            put_uuid(&mut out, uuid, oversize)?;
        }
        Ok(out)
    }

    /// A scan filter: the service-uuid list, then the device-id list. Anything too long for the
    /// wire is a malformed identifier, and resolves as one.
    pub(super) fn encode_scan_filter(filter: &ScanFilter) -> Result<Vec<u8>, BluetoothError> {
        let oversize = ErrorKind::InvalidIdentifiers;
        let mut out = encode_uuid_list(&filter.service_uuids)?;
        put_count(&mut out, filter.device_ids.len(), oversize)?;
        for id in &filter.device_ids {
            put_text(&mut out, id.as_str(), oversize)?;
        }
        Ok(out)
    }

    /// A served service tree, characteristic values included; the same layout
    /// `discover_services` answers with. A value the u16 prefix cannot carry does not fit the
    /// expected format; a tree too deep to count cannot be served.
    pub(super) fn encode_service(service: &Service) -> Result<Vec<u8>, BluetoothError> {
        let too_deep = ErrorKind::ServerFailed;
        let mut out = Vec::new();
        put_u16(&mut out, 1);
        put_uuid(&mut out, &service.uuid, too_deep)?;
        out.push(u8::from(service.primary));
        put_count(&mut out, service.included_services.len(), too_deep)?;
        for uuid in &service.included_services {
            put_uuid(&mut out, uuid, too_deep)?;
        }
        put_count(&mut out, service.characteristics.len(), too_deep)?;
        for characteristic in &service.characteristics {
            put_uuid(&mut out, &characteristic.uuid, too_deep)?;
            put_u16(&mut out, characteristic.properties.bits());
            put_count(&mut out, characteristic.descriptors.len(), too_deep)?;
            for descriptor in &characteristic.descriptors {
                put_uuid(&mut out, &descriptor.uuid, too_deep)?;
            }
            match &characteristic.value {
                Some(value) => {
                    out.push(1);
                    put_bytes(&mut out, value, ErrorKind::CharacteristicInvalidDataFormat)?;
                }
                None => out.push(0),
            }
        }
        Ok(out)
    }

    /// The service tree `discover_services` answered with.
    pub(super) fn decode_services(bytes: &[u8]) -> Option<Vec<Service>> {
        let mut reader = Reader::new(bytes);
        let mut services = Vec::new();
        for _ in 0..reader.u16()? {
            let service_uuid = reader.uuid()?;
            let primary = reader.u8()? != 0;
            let mut included_services = Vec::new();
            for _ in 0..reader.u16()? {
                included_services.push(reader.uuid()?);
            }
            let mut characteristics = Vec::new();
            for _ in 0..reader.u16()? {
                let characteristic_uuid = reader.uuid()?;
                let properties = CharacteristicProperties::from_bits(reader.u16()?);
                let mut descriptors = Vec::new();
                for _ in 0..reader.u16()? {
                    descriptors.push(Descriptor {
                        uuid: reader.uuid()?,
                        characteristic_uuid: characteristic_uuid.clone(),
                        service_uuid: service_uuid.clone(),
                    });
                }
                let value = if reader.u8()? != 0 {
                    Some(reader.bytes()?)
                } else {
                    None
                };
                characteristics.push(Characteristic {
                    uuid: characteristic_uuid,
                    service_uuid: service_uuid.clone(),
                    properties,
                    descriptors,
                    value,
                });
            }
            services.push(Service {
                uuid: service_uuid,
                primary,
                characteristics,
                included_services,
            });
        }
        Some(services)
    }

    /// One scan result the host pushed.
    pub(super) fn decode_scan_result(bytes: &[u8]) -> Option<ScanResult> {
        let mut reader = Reader::new(bytes);
        let id = reader.text()?.to_owned();
        let name = reader.optional_text()?;
        let rssi = reader.optional_i8()?;
        let mut service_uuids = Vec::new();
        for _ in 0..reader.u16()? {
            service_uuids.push(reader.uuid()?);
        }
        let manufacturer_data = reader.optional_bytes()?;
        let tx_power = reader.optional_i8()?;
        Some(ScanResult {
            id: DeviceId::new(id),
            name: name.clone(),
            rssi,
            advertisement: AdvertisementData {
                local_name: name,
                manufacturer_data,
                service_uuids,
                tx_power,
            },
        })
    }

    /// The device list `getSystemDevices` answered with.
    pub(super) fn decode_device_infos(bytes: &[u8]) -> Option<Vec<DeviceInfo>> {
        let mut reader = Reader::new(bytes);
        let mut devices = Vec::new();
        for _ in 0..reader.u16()? {
            let id = reader.text()?.to_owned();
            let name = reader.optional_text()?;
            devices.push(DeviceInfo {
                id: DeviceId::new(id),
                name,
            });
        }
        Some(devices)
    }

    /// One characteristic value the host pushed.
    pub(super) fn decode_notification(bytes: &[u8]) -> Option<HostNotification> {
        let mut reader = Reader::new(bytes);
        let device = reader.text()?.to_owned();
        let service_uuid = reader.uuid()?;
        let uuid = reader.uuid()?;
        let properties = CharacteristicProperties::from_bits(reader.u16()?);
        let value = reader.bytes()?;
        Some(HostNotification {
            device,
            value: ValueNotification {
                characteristic: Characteristic {
                    uuid,
                    service_uuid,
                    properties,
                    descriptors: Vec::new(),
                    value: None,
                },
                value,
            },
        })
    }

    /// One request a central sent to the served GATT table.
    pub(super) fn decode_gatt_request(bytes: &[u8]) -> Option<GattRequest> {
        let mut reader = Reader::new(bytes);
        let service_uuid = reader.uuid()?;
        let characteristic_uuid = reader.uuid()?;
        let kind = match reader.u8()? {
            0 => GattRequestKind::Read {
                offset: reader.u32()? as usize,
            },
            1 => {
                let offset = reader.u32()? as usize;
                let data = reader.bytes()?;
                let with_response = reader.u8()? != 0;
                GattRequestKind::Write {
                    offset,
                    data,
                    with_response,
                }
            }
            _ => return None,
        };
        Some(GattRequest {
            service_uuid,
            characteristic_uuid,
            kind,
        })
    }

    /// Splits a value-bearing reply: a u32 status, then the payload. A non-zero status arrives
    /// as the coded error, carrying the status itself.
    pub(super) fn reply_payload(reply: &[u8]) -> Result<&[u8], BluetoothError> {
        let status = reply
            .get(0..4)
            .and_then(|slice| slice.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| BluetoothError::new(ErrorKind::Backend))?;
        let status = i32::try_from(status).unwrap_or(-1);
        if status == 0 {
            Ok(&reply[4..])
        } else {
            Err(android_status_to_error(status))
        }
    }
}

/// The platform's central backend, if this platform has one and it is implemented.
pub fn central_backend() -> Option<Arc<dyn CentralBackend>> {
    #[cfg(not(target_os = "android"))]
    {
        None
    }
    #[cfg(target_os = "android")]
    {
        Some(Arc::new(native::AndroidCentral))
    }
}

/// The platform's server (peripheral role) backend, if this platform has one and it is implemented.
pub fn server_backend() -> Option<Arc<dyn ServerBackend>> {
    #[cfg(not(target_os = "android"))]
    {
        None
    }
    #[cfg(target_os = "android")]
    {
        Some(Arc::new(native::AndroidServer::new()))
    }
}

#[cfg(target_os = "android")]
mod native {
    use std::{
        collections::HashMap,
        future::ready,
        sync::{
            Arc, Mutex, PoisonError,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use jni::{
        Env, JavaVM,
        errors::LogContextErrorAndDefault,
        jni_sig, jni_str,
        objects::{JByteArray, JClass, JObject, JString, JValue},
        sys::{jboolean, jint},
    };
    use tokio::{sync::broadcast, task::spawn_blocking, time::timeout};

    use crate::backend::{
        AdapterState, AdvertiseOptions, BackendFuture, CentralBackend, Characteristic,
        ConnectionEvent, Descriptor, DeviceId, DeviceInfo, GattRequest, ScanFilter, ScanResult,
        ServerBackend, Service, Uuid, ValueNotification, WriteType,
    };
    use crate::error::{BluetoothError, ErrorKind};

    use super::{android_state_to_adapter_state, android_status_to_error, wire};

    // The Java-side contract this bridge speaks. The class does not exist in NSH 0.4.0
    // (docs/decisions.md #3), so on-device today every call resolves Unsupported; the day the
    // host ships BLE, this is the shape it implements. All methods are static on
    // `dev.supernsh.nsh.BluetoothBridge`, answer statuses per `android_status_to_error`
    // (0 ok, -1 unknown, 133 an Android GATT status, 10000+ a BleErrorCode), and never throw.
    //
    //   int      registerCallbacks()                turn on the native dispatch below, idempotent
    //   int      getAdapterState()                  BluetoothAdapter.STATE_* (10..13)
    //   int      startScan(byte[] filter)           filter per `encode_scan_filter`
    //   int      stopScan()
    //   byte[]   getSystemDevices(byte[] uuidFilter)   uuid list in, device-info list out
    //   int      connect(String deviceId)           resolves once the link is up or failed
    //   int      disconnect(String deviceId)        ok when already disconnected
    //   boolean  isConnected(String deviceId)
    //   byte[]   readRssi(String deviceId)          payload: one i8
    //   byte[]   requestMtu(String deviceId, int mtu)   payload: u16 negotiated
    //   byte[]   discoverServices(String deviceId)  payload: service tree, values optional
    //   byte[]   readCharacteristic(String deviceId, String serviceUuid, String characteristicUuid)
    //   int      writeCharacteristic(String deviceId, String serviceUuid, String characteristicUuid, byte[] value, boolean withResponse)
    //   int      setCharacteristicNotification(String deviceId, String serviceUuid, String characteristicUuid, boolean enable)
    //   byte[]   readDescriptor(String deviceId, String serviceUuid, String characteristicUuid, String descriptorUuid)
    //   int      writeDescriptor(String deviceId, String serviceUuid, String characteristicUuid, String descriptorUuid, byte[] value)
    //   int      addService(byte[] service)         service tree per `encode_service`
    //   int      removeService(String serviceUuid)
    //   int      startAdvertising(String localName, byte[] serviceUuids)   null name for none
    //   int      stopAdvertising()
    //   boolean  isAdvertising()
    //   int      setCharacteristicValue(String serviceUuid, String characteristicUuid, byte[] value)
    //   int      notify(String serviceUuid, String characteristicUuid, byte[] value)
    //
    // Value-bearing methods answer a byte[] whose first u32 (little-endian) is the status and
    // the rest the payload. The host calls back through the static natives at the bottom of
    // this module, each spelled Java_dev_supernsh_nsh_BluetoothBridge_<method>.

    /// One deadline under every host call: a stuck host resolves as `Timeout` instead of hanging
    /// the caller, even though the blocked thread may linger.
    const OP_TIMEOUT: Duration = Duration::from_secs(30);
    /// The crate-wide stream capacity; a lagged receiver resynchronises instead of failing.
    const CHANNEL_CAPACITY: usize = 32;
    /// The host bridge class, in the dotted form `ClassLoader.loadClass` takes.
    const BRIDGE_CLASS: &str = "dev.supernsh.nsh.BluetoothBridge";

    /// Host-pushed events, process-wide: the native callbacks have no `self` to carry.
    struct Sink {
        states: broadcast::Sender<AdapterState>,
        scan: broadcast::Sender<ScanResult>,
        connections: broadcast::Sender<ConnectionEvent>,
        requests: broadcast::Sender<GattRequest>,
        subscriptions: Mutex<HashMap<(String, Uuid, Uuid), broadcast::Sender<ValueNotification>>>,
    }

    fn sink() -> &'static Sink {
        static SINK: std::sync::OnceLock<Sink> = std::sync::OnceLock::new();
        SINK.get_or_init(|| Sink {
            states: broadcast::channel(CHANNEL_CAPACITY).0,
            scan: broadcast::channel(CHANNEL_CAPACITY).0,
            connections: broadcast::channel(CHANNEL_CAPACITY).0,
            requests: broadcast::channel(CHANNEL_CAPACITY).0,
            subscriptions: Mutex::new(HashMap::new()),
        })
    }

    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads ndk-context's answer under a mutex: its 0.1 getter panics while uninitialized
    /// (and again after a release), so the probe silences and catches that panic, and the
    /// mutex keeps the hook swap from ever interleaving with itself.
    fn probe_android_context() -> Option<ndk_context::AndroidContext> {
        static PROBE: Mutex<()> = Mutex::new(());
        let _guard = lock(&PROBE);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let probe =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(ndk_context::android_context));
        std::panic::set_hook(previous);
        probe.ok()
    }

    /// The process's JavaVM pointer: once seen it is cached for the process (Android's VM
    /// outlives every activity), while a missed probe retries on the next operation like
    /// `register_callbacks` does. 0 means there is none yet.
    fn android_vm_raw() -> usize {
        static VM: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        if let Some(vm) = VM.get() {
            return *vm;
        }
        let vm = probe_android_context().map_or(0, |context| context.vm() as usize);
        if vm != 0 {
            let _ = VM.set(vm);
        }
        vm
    }

    /// The application context pointer, read fresh each operation: ndk-context may release it
    /// when the activity is destroyed, and a cached pointer would then be stale. 0 means no
    /// context is installed right now.
    fn android_context_raw() -> usize {
        probe_android_context().map_or(0, |context| context.context() as usize)
    }

    /// Attachment and other JNI-layer failures land here; per-call mapping happens in `cleared`.
    impl From<jni::errors::Error> for BluetoothError {
        fn from(_: jni::errors::Error) -> Self {
            BluetoothError::new(ErrorKind::Backend)
        }
    }

    /// Clears a pending exception after a failed Java call and maps the failure to `kind`.
    fn cleared<T>(
        env: &mut Env<'_>,
        result: Result<T, jni::errors::Error>,
        kind: ErrorKind,
    ) -> Result<T, BluetoothError> {
        match result {
            Ok(value) => Ok(value),
            Err(_) => {
                if env.exception_check() {
                    env.exception_clear();
                }
                Err(BluetoothError::new(kind))
            }
        }
    }

    /// Loads the bridge class through the application's class loader; `find_class` from an
    /// attached thread can miss app classes. A missing class resolves `Unsupported`.
    fn bridge_class<'env>(
        env: &mut Env<'env>,
        application: &JObject<'_>,
    ) -> Result<JClass<'env>, BluetoothError> {
        let name: JObject<'env> = env
            .new_string(BRIDGE_CLASS)
            .map_err(|_| BluetoothError::new(ErrorKind::Unsupported))?
            .into();
        let call = env.call_method(
            application,
            jni_str!("getClassLoader"),
            jni_sig!("()Ljava/lang/ClassLoader;"),
            &[],
        );
        let loader = cleared(env, call, ErrorKind::Unsupported)?
            .l()
            .map_err(|_| BluetoothError::new(ErrorKind::Unsupported))?;
        if loader.as_raw().is_null() {
            return Err(BluetoothError::new(ErrorKind::Unsupported));
        }
        let call = env.call_method(
            &loader,
            jni_str!("loadClass"),
            jni_sig!("(Ljava/lang/String;)Ljava/lang/Class;"),
            &[JValue::Object(&name)],
        );
        let class = cleared(env, call, ErrorKind::Unsupported)?
            .l()
            .map_err(|_| BluetoothError::new(ErrorKind::Unsupported))?;
        if class.as_raw().is_null() {
            return Err(BluetoothError::new(ErrorKind::Unsupported));
        }
        // SAFETY: `loadClass` answered a java.lang.Class reference, which is a jclass.
        Ok(unsafe { JClass::from_raw(env, class.as_raw()) })
    }

    static CALLBACKS_REGISTERED: AtomicBool = AtomicBool::new(false);

    /// One `registerCallbacks` per process turns the host's native dispatch on. The host call is
    /// idempotent, so a first failed attempt is simply retried on the next operation.
    fn register_callbacks(env: &mut Env<'_>, class: &JClass<'_>) -> Result<(), BluetoothError> {
        if CALLBACKS_REGISTERED.load(Ordering::Relaxed) {
            return Ok(());
        }
        let call =
            env.call_static_method(class, jni_str!("registerCallbacks"), jni_sig!("()I"), &[]);
        check_status(int_call(env, call)?)?;
        CALLBACKS_REGISTERED.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn check_status(status: i32) -> Result<(), BluetoothError> {
        if status == 0 {
            Ok(())
        } else {
            Err(android_status_to_error(status))
        }
    }

    /// A made Java call answering an int. Per the contract the host never throws, so a pending
    /// exception means a missing method; it is cleared and resolved `Unsupported`.
    fn int_call(
        env: &mut Env<'_>,
        call: Result<jni::objects::JValueOwned<'_>, jni::errors::Error>,
    ) -> Result<i32, BluetoothError> {
        cleared(env, call, ErrorKind::Unsupported)?
            .i()
            .map_err(|_| BluetoothError::new(ErrorKind::Backend))
    }

    /// Calls a static bridge method answering an int status.
    fn status_call<'sig, 'sig_args, N, S>(
        env: &mut Env<'_>,
        class: &JClass<'_>,
        name: N,
        sig: S,
        args: &[JValue],
    ) -> Result<(), BluetoothError>
    where
        N: AsRef<jni::strings::JNIStr>,
        S: AsRef<jni::signature::MethodSignature<'sig, 'sig_args>>,
    {
        let call = env.call_static_method(class, name, sig, args);
        check_status(int_call(env, call)?)
    }

    /// Calls a static bridge method answering a boolean.
    fn bool_call<'sig, 'sig_args, N, S>(
        env: &mut Env<'_>,
        class: &JClass<'_>,
        name: N,
        sig: S,
        args: &[JValue],
    ) -> Result<bool, BluetoothError>
    where
        N: AsRef<jni::strings::JNIStr>,
        S: AsRef<jni::signature::MethodSignature<'sig, 'sig_args>>,
    {
        let call = env.call_static_method(class, name, sig, args);
        cleared(env, call, ErrorKind::Unsupported)?
            .z()
            .map_err(|_| BluetoothError::new(ErrorKind::Backend))
    }

    /// Calls a static bridge method answering `byte[]`: a u32 status, then the payload.
    fn bytes_call<'sig, 'sig_args, N, S>(
        env: &mut Env<'_>,
        class: &JClass<'_>,
        name: N,
        sig: S,
        args: &[JValue],
    ) -> Result<Vec<u8>, BluetoothError>
    where
        N: AsRef<jni::strings::JNIStr>,
        S: AsRef<jni::signature::MethodSignature<'sig, 'sig_args>>,
    {
        let call = env.call_static_method(class, name, sig, args);
        let value = cleared(env, call, ErrorKind::Unsupported)?;
        let object = value
            .l()
            .map_err(|_| BluetoothError::new(ErrorKind::Backend))?;
        if object.as_raw().is_null() {
            return Err(BluetoothError::new(ErrorKind::Backend));
        }
        // SAFETY: the method's contract answers a byte[], so the reference is one.
        let array = unsafe { JByteArray::from_raw(env, object.as_raw()) };
        let reply = env
            .convert_byte_array(&array)
            .map_err(|_| BluetoothError::new(ErrorKind::Backend))?;
        wire::reply_payload(&reply).map(<[u8]>::to_vec)
    }

    fn string_arg<'env>(env: &mut Env<'env>, text: &str) -> Result<JObject<'env>, BluetoothError> {
        env.new_string(text)
            .map(|string| string.into())
            .map_err(|_| BluetoothError::new(ErrorKind::Backend))
    }

    fn bytes_arg<'env>(env: &mut Env<'env>, bytes: &[u8]) -> Result<JObject<'env>, BluetoothError> {
        let array = env
            .byte_array_from_slice(bytes)
            .map_err(|_| BluetoothError::new(ErrorKind::Backend))?;
        Ok(array.into())
    }

    /// Runs `work` attached to the VM with the bridge class resolved and callbacks registered.
    /// Missing pieces resolve coded: no ndk-context, class or method means `Unsupported`.
    fn with_host<R>(
        work: impl FnOnce(&mut Env<'_>, &JClass<'_>) -> Result<R, BluetoothError>,
    ) -> Result<R, BluetoothError> {
        let vm_raw = android_vm_raw();
        if vm_raw == 0 {
            return Err(BluetoothError::new(ErrorKind::Unsupported));
        }
        // Read fresh each call: ndk-context may release the context between operations.
        let context_raw = android_context_raw();
        if context_raw == 0 {
            return Err(BluetoothError::new(ErrorKind::Unsupported));
        }
        // SAFETY: the integer is the JavaVM pointer ndk-context handed out this process;
        // Android's VM is process-lifetime, and `from_raw` only wraps and null-checks it.
        let vm = unsafe { JavaVM::from_raw(vm_raw as *mut _) };
        vm.attach_current_thread(|env| {
            // SAFETY: the jobject is owned by ndk-context (never deleted here; the wrapper
            // has no Drop side effect) and was read fresh for this call, so borrowing it for
            // the duration of this attached closure is as safe as ndk-context allows.
            let application = unsafe { JObject::from_raw(env, context_raw as *mut _) };
            let class = bridge_class(env, &application)?;
            register_callbacks(env, &class)?;
            work(env, &class)
        })
    }

    /// Runs a blocking host call on the blocking pool under the operation deadline.
    async fn host_op<R, F>(work: F) -> Result<R, BluetoothError>
    where
        F: FnOnce() -> Result<R, BluetoothError> + Send + 'static,
        R: Send + 'static,
    {
        match timeout(OP_TIMEOUT, spawn_blocking(work)).await {
            Ok(joined) => joined.map_err(|_| BluetoothError::new(ErrorKind::Backend))?,
            Err(_) => Err(BluetoothError::new(ErrorKind::Timeout)),
        }
    }

    /// The strings the GATT calls address a characteristic by.
    fn gatt_paths(id: &DeviceId, characteristic: &Characteristic) -> (String, String, String) {
        (
            id.as_str().to_owned(),
            characteristic.service_uuid.as_str().to_owned(),
            characteristic.uuid.as_str().to_owned(),
        )
    }

    /// The subscription key of a device characteristic, from the wire strings.
    fn subscription_key(
        device: &str,
        service: &str,
        characteristic: &str,
    ) -> Result<(String, Uuid, Uuid), BluetoothError> {
        let invalid = || BluetoothError::new(ErrorKind::InvalidIdentifiers);
        Ok((
            device.to_owned(),
            Uuid::parse(service).map_err(|_| invalid())?,
            Uuid::parse(characteristic).map_err(|_| invalid())?,
        ))
    }

    pub(super) struct AndroidCentral;

    impl CentralBackend for AndroidCentral {
        fn adapter_state(&self) -> BackendFuture<'_, Result<AdapterState, BluetoothError>> {
            Box::pin(host_op(|| {
                with_host(|env, class| {
                    let call = env.call_static_method(
                        class,
                        jni_str!("getAdapterState"),
                        jni_sig!("()I"),
                        &[],
                    );
                    int_call(env, call).map(android_state_to_adapter_state)
                })
            }))
        }

        fn state_changes(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<AdapterState>, BluetoothError>> {
            Box::pin(host_op(|| {
                with_host(|_env, _class| Ok(sink().states.subscribe()))
            }))
        }

        fn start_scan(
            &self,
            filter: ScanFilter,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ScanResult>, BluetoothError>> {
            Box::pin(host_op(move || {
                let payload = wire::encode_scan_filter(&filter)?;
                with_host(|env, class| {
                    let filter_arg = bytes_arg(env, &payload)?;
                    status_call(
                        env,
                        class,
                        jni_str!("startScan"),
                        jni_sig!("([B)I"),
                        &[JValue::Object(&filter_arg)],
                    )?;
                    Ok(sink().scan.subscribe())
                })
            }))
        }

        fn stop_scan(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(host_op(|| {
                with_host(|env, class| {
                    status_call(env, class, jni_str!("stopScan"), jni_sig!("()I"), &[])
                })
            }))
        }

        fn system_devices(
            &self,
            service_uuids: &[Uuid],
        ) -> BackendFuture<'_, Result<Vec<DeviceInfo>, BluetoothError>> {
            // The borrowed uuid list cannot cross into the blocking closure; encode up front.
            let payload = match wire::encode_uuid_list(service_uuids) {
                Ok(payload) => payload,
                Err(error) => return Box::pin(ready(Err(error))),
            };
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let filter_arg = bytes_arg(env, &payload)?;
                    let reply = bytes_call(
                        env,
                        class,
                        jni_str!("getSystemDevices"),
                        jni_sig!("([B)[B"),
                        &[JValue::Object(&filter_arg)],
                    )?;
                    wire::decode_device_infos(&reply)
                        .ok_or_else(|| BluetoothError::new(ErrorKind::Backend))
                })
            }))
        }

        fn connect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.as_str().to_owned();
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    status_call(
                        env,
                        class,
                        jni_str!("connect"),
                        jni_sig!("(Ljava/lang/String;)I"),
                        &[JValue::Object(&id_arg)],
                    )
                })
            }))
        }

        fn disconnect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.as_str().to_owned();
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    status_call(
                        env,
                        class,
                        jni_str!("disconnect"),
                        jni_sig!("(Ljava/lang/String;)I"),
                        &[JValue::Object(&id_arg)],
                    )
                })
            }))
        }

        fn is_connected(&self, id: &DeviceId) -> BackendFuture<'_, Result<bool, BluetoothError>> {
            let id = id.as_str().to_owned();
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    bool_call(
                        env,
                        class,
                        jni_str!("isConnected"),
                        jni_sig!("(Ljava/lang/String;)Z"),
                        &[JValue::Object(&id_arg)],
                    )
                })
            }))
        }

        fn read_rssi(&self, id: &DeviceId) -> BackendFuture<'_, Result<i8, BluetoothError>> {
            let id = id.as_str().to_owned();
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    let reply = bytes_call(
                        env,
                        class,
                        jni_str!("readRssi"),
                        jni_sig!("(Ljava/lang/String;)[B"),
                        &[JValue::Object(&id_arg)],
                    )?;
                    reply
                        .first()
                        .map(|byte| *byte as i8)
                        .ok_or_else(|| BluetoothError::new(ErrorKind::Backend))
                })
            }))
        }

        fn request_mtu(
            &self,
            id: &DeviceId,
            mtu: u16,
        ) -> BackendFuture<'_, Result<u16, BluetoothError>> {
            let id = id.as_str().to_owned();
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    let reply = bytes_call(
                        env,
                        class,
                        jni_str!("requestMtu"),
                        jni_sig!("(Ljava/lang/String;I)[B"),
                        &[JValue::Object(&id_arg), JValue::Int(jint::from(mtu))],
                    )?;
                    let negotiated = reply
                        .get(0..2)
                        .and_then(|slice| slice.try_into().ok())
                        .map(u16::from_le_bytes)
                        .ok_or_else(|| BluetoothError::new(ErrorKind::Backend))?;
                    Ok(negotiated)
                })
            }))
        }

        fn discover_services(
            &self,
            id: &DeviceId,
        ) -> BackendFuture<'_, Result<Vec<Service>, BluetoothError>> {
            let id = id.as_str().to_owned();
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    let reply = bytes_call(
                        env,
                        class,
                        jni_str!("discoverServices"),
                        jni_sig!("(Ljava/lang/String;)[B"),
                        &[JValue::Object(&id_arg)],
                    )?;
                    wire::decode_services(&reply)
                        .ok_or_else(|| BluetoothError::new(ErrorKind::Backend))
                })
            }))
        }

        fn read_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            let (id, service, char) = gatt_paths(id, characteristic);
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    let service_arg = string_arg(env, &service)?;
                    let char_arg = string_arg(env, &char)?;
                    bytes_call(
                        env,
                        class,
                        jni_str!("readCharacteristic"),
                        jni_sig!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)[B"),
                        &[
                            JValue::Object(&id_arg),
                            JValue::Object(&service_arg),
                            JValue::Object(&char_arg),
                        ],
                    )
                })
            }))
        }

        fn write_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
            data: &[u8],
            write_type: WriteType,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let (id, service, char) = gatt_paths(id, characteristic);
            let data = data.to_vec();
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    let service_arg = string_arg(env, &service)?;
                    let char_arg = string_arg(env, &char)?;
                    let data_arg = bytes_arg(env, &data)?;
                    status_call(
                        env,
                        class,
                        jni_str!("writeCharacteristic"),
                        jni_sig!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;[BZ)I"),
                        &[
                            JValue::Object(&id_arg),
                            JValue::Object(&service_arg),
                            JValue::Object(&char_arg),
                            JValue::Object(&data_arg),
                            JValue::Bool(matches!(write_type, WriteType::WithResponse)),
                        ],
                    )
                })
            }))
        }

        fn subscribe_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ValueNotification>, BluetoothError>>
        {
            let (id, service, char) = gatt_paths(id, characteristic);
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    let service_arg = string_arg(env, &service)?;
                    let char_arg = string_arg(env, &char)?;
                    status_call(
                        env,
                        class,
                        jni_str!("setCharacteristicNotification"),
                        jni_sig!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Z)I"),
                        &[
                            JValue::Object(&id_arg),
                            JValue::Object(&service_arg),
                            JValue::Object(&char_arg),
                            JValue::Bool(true),
                        ],
                    )?;
                    // A fresh receiver on an existing channel keeps one logical subscription.
                    let key = subscription_key(&id, &service, &char)?;
                    let sender = lock(&sink().subscriptions)
                        .entry(key)
                        .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
                        .clone();
                    Ok(sender.subscribe())
                })
            }))
        }

        fn unsubscribe_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let (id, service, char) = gatt_paths(id, characteristic);
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    let service_arg = string_arg(env, &service)?;
                    let char_arg = string_arg(env, &char)?;
                    status_call(
                        env,
                        class,
                        jni_str!("setCharacteristicNotification"),
                        jni_sig!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Z)I"),
                        &[
                            JValue::Object(&id_arg),
                            JValue::Object(&service_arg),
                            JValue::Object(&char_arg),
                            JValue::Bool(false),
                        ],
                    )?;
                    let key = subscription_key(&id, &service, &char)?;
                    lock(&sink().subscriptions).remove(&key);
                    Ok(())
                })
            }))
        }

        fn read_descriptor(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            let id = id.as_str().to_owned();
            let service = descriptor.service_uuid.as_str().to_owned();
            let char = descriptor.characteristic_uuid.as_str().to_owned();
            let descriptor_uuid = descriptor.uuid.as_str().to_owned();
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    let service_arg = string_arg(env, &service)?;
                    let char_arg = string_arg(env, &char)?;
                    let descriptor_arg = string_arg(env, &descriptor_uuid)?;
                    bytes_call(
                        env,
                        class,
                        jni_str!("readDescriptor"),
                        jni_sig!(
                            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)[B"
                        ),
                        &[
                            JValue::Object(&id_arg),
                            JValue::Object(&service_arg),
                            JValue::Object(&char_arg),
                            JValue::Object(&descriptor_arg),
                        ],
                    )
                })
            }))
        }

        fn write_descriptor(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
            data: &[u8],
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.as_str().to_owned();
            let service = descriptor.service_uuid.as_str().to_owned();
            let char = descriptor.characteristic_uuid.as_str().to_owned();
            let descriptor_uuid = descriptor.uuid.as_str().to_owned();
            let data = data.to_vec();
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let id_arg = string_arg(env, &id)?;
                    let service_arg = string_arg(env, &service)?;
                    let char_arg = string_arg(env, &char)?;
                    let descriptor_arg = string_arg(env, &descriptor_uuid)?;
                    let data_arg = bytes_arg(env, &data)?;
                    status_call(
                        env,
                        class,
                        jni_str!("writeDescriptor"),
                        jni_sig!(
                            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;[B)I"
                        ),
                        &[
                            JValue::Object(&id_arg),
                            JValue::Object(&service_arg),
                            JValue::Object(&char_arg),
                            JValue::Object(&descriptor_arg),
                            JValue::Object(&data_arg),
                        ],
                    )
                })
            }))
        }

        fn connection_events(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ConnectionEvent>, BluetoothError>>
        {
            Box::pin(host_op(|| {
                with_host(|_env, _class| Ok(sink().connections.subscribe()))
            }))
        }
    }

    pub(super) struct AndroidServer {
        served: Arc<Mutex<Vec<Uuid>>>,
    }

    impl AndroidServer {
        pub(super) fn new() -> Self {
            Self {
                served: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl ServerBackend for AndroidServer {
        fn add_service(&self, service: Service) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let served = Arc::clone(&self.served);
            Box::pin(host_op(move || {
                if lock(&served).contains(&service.uuid) {
                    return Err(BluetoothError::new(ErrorKind::ServerFailed));
                }
                let payload = wire::encode_service(&service)?;
                let uuid = service.uuid;
                with_host(|env, class| {
                    let service_arg = bytes_arg(env, &payload)?;
                    status_call(
                        env,
                        class,
                        jni_str!("addService"),
                        jni_sig!("([B)I"),
                        &[JValue::Object(&service_arg)],
                    )?;
                    lock(&served).push(uuid);
                    Ok(())
                })
            }))
        }

        fn remove_service(&self, uuid: Uuid) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let served = Arc::clone(&self.served);
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let uuid_arg = string_arg(env, uuid.as_str())?;
                    status_call(
                        env,
                        class,
                        jni_str!("removeService"),
                        jni_sig!("(Ljava/lang/String;)I"),
                        &[JValue::Object(&uuid_arg)],
                    )?;
                    lock(&served).retain(|served| *served != uuid);
                    Ok(())
                })
            }))
        }

        fn start_advertising(
            &self,
            options: AdvertiseOptions,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let local_name = options.local_name;
            let service_uuids = options.service_uuids;
            Box::pin(host_op(move || {
                let payload = wire::encode_uuid_list(&service_uuids)?;
                with_host(|env, class| {
                    let name_arg = match &local_name {
                        Some(name) => string_arg(env, name)?,
                        None => JObject::null(),
                    };
                    let uuids_arg = bytes_arg(env, &payload)?;
                    status_call(
                        env,
                        class,
                        jni_str!("startAdvertising"),
                        jni_sig!("(Ljava/lang/String;[B)I"),
                        &[JValue::Object(&name_arg), JValue::Object(&uuids_arg)],
                    )
                })
            }))
        }

        fn stop_advertising(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(host_op(|| {
                with_host(|env, class| {
                    status_call(
                        env,
                        class,
                        jni_str!("stopAdvertising"),
                        jni_sig!("()I"),
                        &[],
                    )
                })
            }))
        }

        fn is_advertising(&self) -> BackendFuture<'_, Result<bool, BluetoothError>> {
            Box::pin(host_op(|| {
                with_host(|env, class| {
                    bool_call(env, class, jni_str!("isAdvertising"), jni_sig!("()Z"), &[])
                })
            }))
        }

        fn set_characteristic_value(
            &self,
            service: Uuid,
            characteristic: Uuid,
            value: Vec<u8>,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let service_arg = string_arg(env, service.as_str())?;
                    let char_arg = string_arg(env, characteristic.as_str())?;
                    let value_arg = bytes_arg(env, &value)?;
                    status_call(
                        env,
                        class,
                        jni_str!("setCharacteristicValue"),
                        jni_sig!("(Ljava/lang/String;Ljava/lang/String;[B)I"),
                        &[
                            JValue::Object(&service_arg),
                            JValue::Object(&char_arg),
                            JValue::Object(&value_arg),
                        ],
                    )
                })
            }))
        }

        fn notify(
            &self,
            service: Uuid,
            characteristic: Uuid,
            data: &[u8],
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let data = data.to_vec();
            Box::pin(host_op(move || {
                with_host(|env, class| {
                    let service_arg = string_arg(env, service.as_str())?;
                    let char_arg = string_arg(env, characteristic.as_str())?;
                    let data_arg = bytes_arg(env, &data)?;
                    status_call(
                        env,
                        class,
                        jni_str!("notify"),
                        jni_sig!("(Ljava/lang/String;Ljava/lang/String;[B)I"),
                        &[
                            JValue::Object(&service_arg),
                            JValue::Object(&char_arg),
                            JValue::Object(&data_arg),
                        ],
                    )
                })
            }))
        }

        fn requests(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<GattRequest>, BluetoothError>> {
            Box::pin(host_op(|| {
                with_host(|_env, _class| Ok(sink().requests.subscribe()))
            }))
        }
    }

    // The host's dispatch targets: static natives on the bridge class. Each is best-effort — a
    // malformed payload drops, a Java failure logs and resolves, and a panic never unwinds
    // into the JVM.

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_supernsh_nsh_BluetoothBridge_onAdapterState<'caller>(
        mut unowned_env: jni::EnvUnowned<'caller>,
        _class: JClass<'caller>,
        state: jint,
    ) {
        unowned_env
            .with_env(|_env| -> jni::errors::Result<()> {
                let _ = sink().states.send(android_state_to_adapter_state(state));
                Ok(())
            })
            .resolve_with::<LogContextErrorAndDefault, _>(|| {
                "BluetoothBridge callback failed".to_owned()
            })
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_supernsh_nsh_BluetoothBridge_onScanResult<'caller>(
        mut unowned_env: jni::EnvUnowned<'caller>,
        _class: JClass<'caller>,
        payload: JByteArray<'caller>,
    ) {
        unowned_env
            .with_env(|env| -> jni::errors::Result<()> {
                if let Some(result) = env
                    .convert_byte_array(&payload)
                    .ok()
                    .and_then(|bytes| wire::decode_scan_result(&bytes))
                {
                    let _ = sink().scan.send(result);
                }
                Ok(())
            })
            .resolve_with::<LogContextErrorAndDefault, _>(|| {
                "BluetoothBridge callback failed".to_owned()
            })
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_supernsh_nsh_BluetoothBridge_onConnectionState<'caller>(
        mut unowned_env: jni::EnvUnowned<'caller>,
        _class: JClass<'caller>,
        id: JString<'caller>,
        connected: jboolean,
    ) {
        unowned_env
            .with_env(|env| -> jni::errors::Result<()> {
                if let Ok(id) = id.try_to_string(env) {
                    let event = if connected {
                        ConnectionEvent::Connected(DeviceId::new(id))
                    } else {
                        ConnectionEvent::Disconnected(DeviceId::new(id))
                    };
                    let _ = sink().connections.send(event);
                }
                Ok(())
            })
            .resolve_with::<LogContextErrorAndDefault, _>(|| {
                "BluetoothBridge callback failed".to_owned()
            })
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_supernsh_nsh_BluetoothBridge_onCharacteristicNotification<
        'caller,
    >(
        mut unowned_env: jni::EnvUnowned<'caller>,
        _class: JClass<'caller>,
        payload: JByteArray<'caller>,
    ) {
        unowned_env
            .with_env(|env| -> jni::errors::Result<()> {
                if let Some(notification) = env
                    .convert_byte_array(&payload)
                    .ok()
                    .and_then(|bytes| wire::decode_notification(&bytes))
                {
                    let characteristic = &notification.value.characteristic;
                    let key = (
                        notification.device,
                        characteristic.service_uuid.clone(),
                        characteristic.uuid.clone(),
                    );
                    let sender = lock(&sink().subscriptions).get(&key).cloned();
                    if let Some(sender) = sender {
                        let _ = sender.send(notification.value);
                    }
                }
                Ok(())
            })
            .resolve_with::<LogContextErrorAndDefault, _>(|| {
                "BluetoothBridge callback failed".to_owned()
            })
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_supernsh_nsh_BluetoothBridge_onGattRequest<'caller>(
        mut unowned_env: jni::EnvUnowned<'caller>,
        _class: JClass<'caller>,
        payload: JByteArray<'caller>,
    ) {
        unowned_env
            .with_env(|env| -> jni::errors::Result<()> {
                if let Some(request) = env
                    .convert_byte_array(&payload)
                    .ok()
                    .and_then(|bytes| wire::decode_gatt_request(&bytes))
                {
                    let _ = sink().requests.send(request);
                }
                Ok(())
            })
            .resolve_with::<LogContextErrorAndDefault, _>(|| {
                "BluetoothBridge callback failed".to_owned()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::wire;
    use super::*;
    use crate::backend::Uuid;

    #[test]
    fn statuses_map_through_the_plx_table_in_both_spellings() {
        let table = [
            (2_i32, ErrorKind::Cancelled),
            (3, ErrorKind::Timeout),
            (5, ErrorKind::InvalidIdentifiers),
            (100, ErrorKind::Unsupported),
            (101, ErrorKind::Unauthorized),
            (102, ErrorKind::PoweredOff),
            (103, ErrorKind::UnknownState),
            (104, ErrorKind::Resetting),
            (105, ErrorKind::StateChangeFailed),
            (200, ErrorKind::ConnectFailed),
            (201, ErrorKind::Disconnected),
            (203, ErrorKind::AlreadyConnected),
            (204, ErrorKind::DeviceNotFound),
            (205, ErrorKind::DeviceNotConnected),
            (206, ErrorKind::MtuChangeFailed),
            (300, ErrorKind::ServicesDiscoveryFailed),
            (302, ErrorKind::ServiceNotFound),
            (303, ErrorKind::ServicesNotDiscovered),
            (401, ErrorKind::CharacteristicWriteFailed),
            (402, ErrorKind::CharacteristicReadFailed),
            (403, ErrorKind::CharacteristicNotifyChangeFailed),
            (404, ErrorKind::CharacteristicNotFound),
            (406, ErrorKind::CharacteristicInvalidDataFormat),
            (501, ErrorKind::DescriptorWriteFailed),
            (502, ErrorKind::DescriptorReadFailed),
            (503, ErrorKind::DescriptorNotFound),
            (506, ErrorKind::DescriptorWriteNotAllowed),
            (600, ErrorKind::ScanStartFailed),
            (601, ErrorKind::LocationServicesDisabled),
        ];
        for (code, kind) in table {
            assert_eq!(
                android_status_to_kind(code + 10_000),
                kind,
                "offset code {code}"
            );
            // Below 100 the raw spelling collides with Android's own GATT statuses, so only
            // codes from 100 up are accepted in raw form.
            if code >= 100 {
                assert_eq!(android_status_to_kind(code), kind, "raw plx code {code}");
            }
        }
    }

    #[test]
    fn discovery_failures_land_on_not_discovered_and_rows_without_kinds_fall_back() {
        assert_eq!(
            android_status_to_kind(400),
            ErrorKind::CharacteristicsNotDiscovered
        );
        assert_eq!(
            android_status_to_kind(10_405),
            ErrorKind::CharacteristicsNotDiscovered
        );
        assert_eq!(
            android_status_to_kind(500),
            ErrorKind::DescriptorsNotDiscovered
        );
        assert_eq!(
            android_status_to_kind(10_504),
            ErrorKind::DescriptorsNotDiscovered
        );
        // plx rows the crate has no kind for, and the protocol's own server codes.
        assert_eq!(android_status_to_kind(202), ErrorKind::Backend);
        assert_eq!(android_status_to_kind(505), ErrorKind::Backend);
        assert_eq!(android_status_to_kind(10_700), ErrorKind::AdvertiseFailed);
        assert_eq!(android_status_to_kind(10_701), ErrorKind::ServerFailed);
    }

    #[test]
    fn android_statuses_and_unknowns_map_coded() {
        assert_eq!(android_status_to_kind(-1), ErrorKind::Backend);
        assert_eq!(android_status_to_kind(133), ErrorKind::ConnectFailed);
        assert_eq!(android_status_to_kind(8), ErrorKind::Backend);
        assert_eq!(android_status_to_kind(999), ErrorKind::Backend);
        assert_eq!(android_status_to_kind(0), ErrorKind::Backend);
        assert_eq!(
            android_status_to_kind(99_999),
            ErrorKind::Backend,
            "beyond the table, still a kind"
        );
    }

    #[test]
    fn the_error_carries_the_host_status_except_unknown() {
        let error = android_status_to_error(133);
        assert_eq!(error.kind(), ErrorKind::ConnectFailed);
        assert_eq!(error.status(), Some(133));
        assert_eq!(android_status_to_error(10_101).status(), Some(10_101));
        assert_eq!(android_status_to_error(-1).status(), None);
    }

    #[test]
    fn adapter_states_map_and_the_rest_stay_unknown() {
        assert_eq!(android_state_to_adapter_state(10), AdapterState::PoweredOff);
        assert_eq!(android_state_to_adapter_state(11), AdapterState::Resetting);
        assert_eq!(android_state_to_adapter_state(12), AdapterState::On);
        assert_eq!(android_state_to_adapter_state(13), AdapterState::Resetting);
        assert_eq!(android_state_to_adapter_state(-1), AdapterState::Unknown);
        assert_eq!(android_state_to_adapter_state(20), AdapterState::Unknown);
    }

    fn heart_rate_service() -> crate::backend::Service {
        crate::backend::Service {
            uuid: Uuid::parse("180D").unwrap(),
            primary: true,
            characteristics: vec![crate::backend::Characteristic {
                uuid: Uuid::parse("2A37").unwrap(),
                service_uuid: Uuid::parse("180D").unwrap(),
                properties: crate::backend::CharacteristicProperties::from_bits(0x12),
                descriptors: vec![crate::backend::Descriptor {
                    uuid: Uuid::parse("2902").unwrap(),
                    characteristic_uuid: Uuid::parse("2A37").unwrap(),
                    service_uuid: Uuid::parse("180D").unwrap(),
                }],
                value: Some(vec![6, 66]),
            }],
            included_services: vec![Uuid::parse("180F").unwrap()],
        }
    }

    #[test]
    fn the_service_tree_round_trips_through_the_wire() {
        let service = heart_rate_service();
        let encoded = wire::encode_service(&service).unwrap();
        let decoded = wire::decode_services(&encoded).unwrap();
        assert_eq!(decoded, vec![service]);
        // A tree without characteristic values decodes with `value: None`.
        let mut bare = heart_rate_service();
        bare.characteristics[0].value = None;
        assert_eq!(
            wire::decode_services(&wire::encode_service(&bare).unwrap()).unwrap(),
            vec![bare.clone()]
        );
        assert_eq!(bare.characteristics[0].value, None);
        // An empty tree answers empty, not none.
        assert_eq!(wire::decode_services(&[0, 0]), Some(Vec::new()));
    }

    #[test]
    fn malformed_wire_answers_none_instead_of_panicking() {
        assert_eq!(wire::decode_services(&[]), None);
        assert_eq!(wire::decode_services(&[0, 1, 9]), None);
        assert_eq!(wire::decode_scan_result(&[]), None);
        assert_eq!(wire::decode_scan_result(&[0, 1, b'A']), None);
        assert_eq!(wire::decode_device_infos(&[0, 2]), None);
        assert_eq!(wire::decode_notification(&[0, 3, b'a', b'b']), None);
        assert_eq!(wire::decode_gatt_request(&[7]), None);
    }

    #[test]
    fn scan_results_decode_from_the_host_layout() {
        let mut wire_bytes = Vec::new();
        wire_bytes.extend_from_slice(&7u16.to_le_bytes());
        wire_bytes.extend_from_slice(b"watch:1");
        wire_bytes.push(1);
        wire_bytes.extend_from_slice(&4u16.to_le_bytes());
        wire_bytes.extend_from_slice(b"Band");
        wire_bytes.push(1);
        wire_bytes.push((-42i8).to_le_bytes()[0]);
        wire_bytes.extend_from_slice(&1u16.to_le_bytes());
        wire_bytes.extend_from_slice(&4u16.to_le_bytes());
        wire_bytes.extend_from_slice(b"180D");
        wire_bytes.push(0); // no manufacturer data
        wire_bytes.push(0); // no tx power
        let result = wire::decode_scan_result(&wire_bytes).unwrap();
        assert_eq!(result.id.as_str(), "watch:1");
        assert_eq!(result.name.as_deref(), Some("Band"));
        assert_eq!(result.rssi, Some(-42));
        assert_eq!(
            result.advertisement.service_uuids,
            vec![Uuid::parse("180D").unwrap()]
        );
        assert_eq!(result.advertisement.local_name.as_deref(), Some("Band"));
        assert_eq!(result.advertisement.manufacturer_data, None);
        assert_eq!(result.advertisement.tx_power, None);
    }

    #[test]
    fn device_infos_and_notifications_decode() {
        let mut devices = Vec::new();
        devices.extend_from_slice(&1u16.to_le_bytes());
        devices.extend_from_slice(&2u16.to_le_bytes());
        devices.extend_from_slice(b"AA");
        devices.push(0); // no name
        let decoded = wire::decode_device_infos(&devices).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].id.as_str(), "AA");
        assert_eq!(decoded[0].name, None);

        let mut notification = Vec::new();
        notification.extend_from_slice(&4u16.to_le_bytes());
        notification.extend_from_slice(b"watc");
        notification.extend_from_slice(&4u16.to_le_bytes());
        notification.extend_from_slice(b"180D");
        notification.extend_from_slice(&4u16.to_le_bytes());
        notification.extend_from_slice(b"2A37");
        notification.extend_from_slice(&0x12u16.to_le_bytes());
        notification.extend_from_slice(&2u16.to_le_bytes());
        notification.extend_from_slice(&[6, 66]);
        let decoded = wire::decode_notification(&notification).unwrap();
        assert_eq!(decoded.device, "watc");
        assert_eq!(decoded.value.value, vec![6, 66]);
        assert_eq!(
            decoded.value.characteristic.uuid,
            Uuid::parse("2A37").unwrap()
        );
        assert_eq!(
            decoded.value.characteristic.service_uuid,
            Uuid::parse("180D").unwrap()
        );
        assert_eq!(
            decoded.value.characteristic.properties.bits(),
            0x12,
            "short uuids arrive expanded"
        );
    }

    #[test]
    fn gatt_requests_decode_for_both_kinds() {
        let mut read = Vec::new();
        read.extend_from_slice(&4u16.to_le_bytes());
        read.extend_from_slice(b"180D");
        read.extend_from_slice(&4u16.to_le_bytes());
        read.extend_from_slice(b"2A37");
        read.push(0);
        read.extend_from_slice(&3u32.to_le_bytes());
        let request = wire::decode_gatt_request(&read).unwrap();
        assert_eq!(
            request.kind,
            crate::backend::GattRequestKind::Read { offset: 3 }
        );

        let mut write = Vec::new();
        write.extend_from_slice(&4u16.to_le_bytes());
        write.extend_from_slice(b"180D");
        write.extend_from_slice(&4u16.to_le_bytes());
        write.extend_from_slice(b"2A37");
        write.push(1);
        write.extend_from_slice(&2u32.to_le_bytes());
        write.extend_from_slice(&1u16.to_le_bytes());
        write.push(9);
        write.push(1);
        let request = wire::decode_gatt_request(&write).unwrap();
        assert_eq!(
            request.kind,
            crate::backend::GattRequestKind::Write {
                offset: 2,
                data: vec![9],
                with_response: true,
            }
        );
    }

    #[test]
    fn replies_split_the_status_from_the_payload() {
        let ok = [0, 0, 0, 0, 1, 2, 3];
        assert_eq!(wire::reply_payload(&ok).unwrap(), &[1, 2, 3]);
        let empty = 0u32.to_le_bytes();
        assert_eq!(wire::reply_payload(&empty).unwrap(), &[] as &[u8]);

        let error = wire::reply_payload(&133u32.to_le_bytes()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ConnectFailed);
        assert_eq!(error.status(), Some(133));
        let error = wire::reply_payload(&10_601u32.to_le_bytes()).unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED");

        assert!(wire::reply_payload(&[0, 0]).is_err());
    }

    #[test]
    fn filters_and_uuid_lists_encode_to_the_documented_layout() {
        let filter = crate::backend::ScanFilter {
            service_uuids: vec![Uuid::parse("180D").unwrap()],
            device_ids: vec![crate::backend::DeviceId::new("AA")],
        };
        let uuids = wire::encode_uuid_list(&filter.service_uuids).unwrap();
        let mut expected = uuids.clone();
        expected.extend_from_slice(&1u16.to_le_bytes());
        expected.extend_from_slice(&2u16.to_le_bytes());
        expected.extend_from_slice(b"AA");
        assert_eq!(wire::encode_scan_filter(&filter).unwrap(), expected);
        assert_eq!(wire::encode_uuid_list(&[]).unwrap(), vec![0, 0]);
    }

    #[test]
    fn oversize_payloads_resolve_coded_instead_of_a_lying_frame() {
        // A device id the u16 prefix cannot carry is a malformed identifier.
        let filter = crate::backend::ScanFilter {
            service_uuids: Vec::new(),
            device_ids: vec![crate::backend::DeviceId::new("x".repeat(70_000))],
        };
        assert_eq!(
            wire::encode_scan_filter(&filter).unwrap_err().code(),
            "ERR_BLUETOOTH_INVALID_IDENTIFIERS"
        );
        let many = vec![Uuid::parse("180D").unwrap(); 70_000];
        assert_eq!(
            wire::encode_uuid_list(&many).unwrap_err().code(),
            "ERR_BLUETOOTH_INVALID_IDENTIFIERS"
        );

        // A characteristic value the prefix cannot carry does not fit the expected format.
        let mut service = heart_rate_service();
        service.characteristics[0].value = Some(vec![0u8; 70_000]);
        assert_eq!(
            wire::encode_service(&service).unwrap_err().code(),
            "ERR_BLUETOOTH_CHARACTERISTIC_INVALID_DATA_FORMAT"
        );

        // A tree whose entry count overflows the u16 prefix cannot be served.
        let mut deep = heart_rate_service();
        deep.included_services = vec![Uuid::parse("180F").unwrap(); 70_000];
        assert_eq!(
            wire::encode_service(&deep).unwrap_err().code(),
            "ERR_BLUETOOTH_SERVER_FAILED"
        );

        // In-range payloads still encode through the same paths.
        assert!(wire::encode_scan_filter(&crate::backend::ScanFilter::default()).is_ok());
    }
}
