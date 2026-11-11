//! Web bridge (Web Bluetooth): a central role only, entered through a user gesture.
//!
//! The platform gives no background scan, no peripheral role and no RSSI reads, and it fixes the
//! ATT MTU at 23. A device enters through `native::request_device`, which wraps the browser's
//! device chooser; `start_scan` reports `Unsupported` to point there. Every JS promise settles by
//! browser contract and a rejection maps to a coded error, so no call panics or hangs.

use std::sync::Arc;

use crate::backend::{CentralBackend, CharacteristicProperties, ServerBackend};
use crate::error::ErrorKind;

/// The error kind a Web Bluetooth rejection name maps to; every other name is the platform's own
/// failure. Pure so every host build tests it.
#[cfg_attr(not(target_family = "wasm"), cfg_attr(not(test), expect(dead_code)))]
fn web_error_name_to_kind(name: &str) -> ErrorKind {
    match name {
        "NotFoundError" => ErrorKind::DeviceNotFound,
        "SecurityError" => ErrorKind::Unauthorized,
        "InvalidStateError" => ErrorKind::DeviceNotConnected,
        _ => ErrorKind::Backend,
    }
}

/// The spec bit a Web Bluetooth characteristic property flag maps to; flags the platform does not
/// carry have none. Pure so every host build tests it.
#[cfg_attr(not(target_family = "wasm"), cfg_attr(not(test), expect(dead_code)))]
fn web_property_bit(name: &str) -> Option<u16> {
    match name {
        "broadcast" => Some(CharacteristicProperties::BROADCAST.bits()),
        "read" => Some(CharacteristicProperties::READ.bits()),
        "writeWithoutResponse" => Some(CharacteristicProperties::WRITE_NO_RESPONSE.bits()),
        "write" => Some(CharacteristicProperties::WRITE.bits()),
        "notify" => Some(CharacteristicProperties::NOTIFY.bits()),
        "indicate" => Some(CharacteristicProperties::INDICATE.bits()),
        "authenticatedSignedWrites" => {
            Some(CharacteristicProperties::AUTHENTICATED_SIGNED_WRITES.bits())
        }
        "reliableWrite" => Some(CharacteristicProperties::RELIABLE_WRITE.bits()),
        _ => None,
    }
}

/// The platform's central backend, if this platform has one and it is implemented.
pub fn central_backend() -> Option<Arc<dyn CentralBackend>> {
    #[cfg(not(target_family = "wasm"))]
    {
        None
    }
    #[cfg(target_family = "wasm")]
    {
        Some(native::central_backend())
    }
}

/// The platform's server (peripheral role) backend, if this platform has one and it is implemented.
pub fn server_backend() -> Option<Arc<dyn ServerBackend>> {
    // Web Bluetooth has no peripheral role: no advertising and no GATT server, on any target.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_error_names_map_to_their_kinds() {
        assert_eq!(
            web_error_name_to_kind("NotFoundError"),
            ErrorKind::DeviceNotFound
        );
        assert_eq!(
            web_error_name_to_kind("SecurityError"),
            ErrorKind::Unauthorized
        );
        assert_eq!(
            web_error_name_to_kind("InvalidStateError"),
            ErrorKind::DeviceNotConnected
        );
        assert_eq!(web_error_name_to_kind("NetworkError"), ErrorKind::Backend);
        assert_eq!(web_error_name_to_kind(""), ErrorKind::Backend);
    }

    #[test]
    fn web_property_flags_map_to_spec_bits() {
        let bit = |flag: &str| web_property_bit(flag).map(CharacteristicProperties::from_bits);
        assert_eq!(bit("broadcast"), Some(CharacteristicProperties::BROADCAST));
        assert_eq!(bit("read"), Some(CharacteristicProperties::READ));
        assert_eq!(
            bit("writeWithoutResponse"),
            Some(CharacteristicProperties::WRITE_NO_RESPONSE)
        );
        assert_eq!(bit("write"), Some(CharacteristicProperties::WRITE));
        assert_eq!(bit("notify"), Some(CharacteristicProperties::NOTIFY));
        assert_eq!(bit("indicate"), Some(CharacteristicProperties::INDICATE));
        assert_eq!(
            bit("authenticatedSignedWrites"),
            Some(CharacteristicProperties::AUTHENTICATED_SIGNED_WRITES)
        );
        assert_eq!(
            bit("reliableWrite"),
            Some(CharacteristicProperties::RELIABLE_WRITE)
        );
        assert_eq!(web_property_bit("writableAuxiliary"), None);
        assert_eq!(web_property_bit(""), None);
    }
}

#[cfg(target_family = "wasm")]
mod native {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::future::{Future, ready};
    use std::pin::Pin;
    use std::rc::Rc;
    use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
    use std::task::{Context, Poll, Waker};

    use tokio::sync::broadcast;
    use wasm_bindgen::{JsCast, closure::Closure, prelude::*};

    use crate::backend::{
        AdapterState, BackendFuture, CentralBackend, Characteristic, CharacteristicProperties,
        ConnectionEvent, Descriptor, DeviceId, DeviceInfo, ScanFilter, ScanResult, Service, Uuid,
        ValueNotification, WriteType,
    };
    use crate::error::{BluetoothError, ErrorKind};

    use super::{web_error_name_to_kind, web_property_bit};

    // Hand-written externs for the Web Bluetooth surface this bridge uses; the crate does not
    // depend on web-sys's Bluetooth features.
    #[wasm_bindgen]
    extern "C" {
        /// `navigator.bluetooth`, the entry point of Web Bluetooth.
        type Bluetooth;

        #[wasm_bindgen(method, catch)]
        fn get_availability(this: &Bluetooth) -> Result<js_sys::Promise, JsValue>;

        #[wasm_bindgen(method, catch)]
        fn request_device(
            this: &Bluetooth,
            options: &RequestDeviceOptions,
        ) -> Result<js_sys::Promise, JsValue>;

        /// The `requestDevice` options dictionary, built as a plain object.
        type RequestDeviceOptions;

        #[wasm_bindgen(method, setter)]
        fn set_filters(this: &RequestDeviceOptions, filters: &js_sys::Array);

        #[wasm_bindgen(method, setter)]
        fn set_optional_services(this: &RequestDeviceOptions, services: &js_sys::Array);

        #[wasm_bindgen(method, setter)]
        fn set_accept_all_devices(this: &RequestDeviceOptions, accept: bool);

        /// A device the origin gained access to through the chooser.
        #[derive(Clone)]
        type BluetoothDevice;

        #[wasm_bindgen(method, getter)]
        fn gatt(this: &BluetoothDevice) -> BluetoothRemoteGATTServer;

        #[wasm_bindgen(method, getter)]
        fn id(this: &BluetoothDevice) -> String;

        #[wasm_bindgen(method, getter)]
        fn name(this: &BluetoothDevice) -> Option<String>;

        #[wasm_bindgen(method, catch)]
        fn add_event_listener(
            this: &BluetoothDevice,
            type_: &str,
            listener: &js_sys::Function,
        ) -> Result<(), JsValue>;

        #[wasm_bindgen(method, catch)]
        fn remove_event_listener(
            this: &BluetoothDevice,
            type_: &str,
            listener: &js_sys::Function,
        ) -> Result<(), JsValue>;

        /// The device's GATT server.
        type BluetoothRemoteGATTServer;

        #[wasm_bindgen(method, getter)]
        fn connected(this: &BluetoothRemoteGATTServer) -> bool;

        #[wasm_bindgen(method, catch)]
        fn connect(this: &BluetoothRemoteGATTServer) -> Result<js_sys::Promise, JsValue>;

        #[wasm_bindgen(method)]
        fn disconnect(this: &BluetoothRemoteGATTServer);

        #[wasm_bindgen(method, catch)]
        fn get_primary_services(
            this: &BluetoothRemoteGATTServer,
        ) -> Result<js_sys::Promise, JsValue>;

        /// A GATT service of a connected device.
        #[derive(Clone)]
        type BluetoothRemoteGATTService;

        #[wasm_bindgen(method, getter, js_name = "uuid")]
        fn service_uuid(this: &BluetoothRemoteGATTService) -> String;

        #[wasm_bindgen(method, getter)]
        fn is_primary(this: &BluetoothRemoteGATTService) -> bool;

        #[wasm_bindgen(method, catch)]
        fn get_characteristics(
            this: &BluetoothRemoteGATTService,
        ) -> Result<js_sys::Promise, JsValue>;

        /// A characteristic of a GATT service.
        #[derive(Clone)]
        type BluetoothRemoteGATTCharacteristic;

        #[wasm_bindgen(method, getter, js_name = "uuid")]
        fn characteristic_uuid(this: &BluetoothRemoteGATTCharacteristic) -> String;

        /// The characteristic's properties bag, read field by field.
        #[wasm_bindgen(method, getter)]
        fn properties(this: &BluetoothRemoteGATTCharacteristic) -> JsValue;

        /// The last value the platform holds for the characteristic; a DataView or null.
        #[wasm_bindgen(method, getter)]
        fn value(this: &BluetoothRemoteGATTCharacteristic) -> JsValue;

        #[wasm_bindgen(method, catch)]
        fn read_value(this: &BluetoothRemoteGATTCharacteristic)
        -> Result<js_sys::Promise, JsValue>;

        #[wasm_bindgen(method, catch)]
        fn write_value_with_response(
            this: &BluetoothRemoteGATTCharacteristic,
            value: &js_sys::Object,
        ) -> Result<js_sys::Promise, JsValue>;

        #[wasm_bindgen(method, catch)]
        fn write_value_without_response(
            this: &BluetoothRemoteGATTCharacteristic,
            value: &js_sys::Object,
        ) -> Result<js_sys::Promise, JsValue>;

        #[wasm_bindgen(method, catch)]
        fn get_descriptors(
            this: &BluetoothRemoteGATTCharacteristic,
        ) -> Result<js_sys::Promise, JsValue>;

        #[wasm_bindgen(method, catch)]
        fn start_notifications(
            this: &BluetoothRemoteGATTCharacteristic,
        ) -> Result<js_sys::Promise, JsValue>;

        #[wasm_bindgen(method, catch)]
        fn stop_notifications(
            this: &BluetoothRemoteGATTCharacteristic,
        ) -> Result<js_sys::Promise, JsValue>;

        #[wasm_bindgen(method, catch, js_name = "addEventListener")]
        fn add_value_listener(
            this: &BluetoothRemoteGATTCharacteristic,
            type_: &str,
            listener: &js_sys::Function,
        ) -> Result<(), JsValue>;

        #[wasm_bindgen(method, catch, js_name = "removeEventListener")]
        fn remove_value_listener(
            this: &BluetoothRemoteGATTCharacteristic,
            type_: &str,
            listener: &js_sys::Function,
        ) -> Result<(), JsValue>;

        /// A descriptor of a characteristic.
        #[derive(Clone)]
        type BluetoothRemoteGATTDescriptor;

        #[wasm_bindgen(method, getter)]
        fn uuid(this: &BluetoothRemoteGATTDescriptor) -> String;

        #[wasm_bindgen(method, catch, js_name = "readValue")]
        fn read(this: &BluetoothRemoteGATTDescriptor) -> Result<js_sys::Promise, JsValue>;

        #[wasm_bindgen(method, catch, js_name = "writeValue")]
        fn write(
            this: &BluetoothRemoteGATTDescriptor,
            value: &js_sys::Object,
        ) -> Result<js_sys::Promise, JsValue>;

        /// A JS promise this bridge only silences: the `then` derivation of a settle future.
        type QuietPromise;

        #[wasm_bindgen(method, catch, js_name = "catch")]
        fn quiet_catch(this: &QuietPromise, on_reject: &js_sys::Function) -> Result<(), JsValue>;
    }

    // One no-op rejection handler for the page's life; a per-call closure would die with the
    // settle future it quiets for.
    static QUIET_HANDLER: LazyLock<JsMutex<Option<js_sys::Function>>> =
        LazyLock::new(|| JsMutex::new(None));

    /// The shared no-op rejection handler, created once and never dropped.
    fn quiet_handler() -> js_sys::Function {
        let mut handler = QUIET_HANDLER.lock();
        handler
            .get_or_insert_with(|| {
                let noop: Closure<dyn FnMut(JsValue)> = Closure::own_aborting(|_| ());
                js_sys::Function::from(noop.into_js_value())
            })
            .clone()
    }

    /// A mutex guarding JS-owned state, carrying the `Send + Sync` the traits demand.
    struct JsMutex<T>(Mutex<T>);

    // SAFETY: wasm32 runs all Rust and JS on one thread; the guarded data cannot race.
    unsafe impl<T> Send for JsMutex<T> {}

    // SAFETY: the same single thread; JS callbacks run between polls, never during one.
    unsafe impl<T> Sync for JsMutex<T> {}

    impl<T> JsMutex<T> {
        fn new(value: T) -> Self {
            Self(Mutex::new(value))
        }

        fn lock(&self) -> MutexGuard<'_, T> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }

    /// The settled outcome of one promise, set by its callbacks and read by `poll`.
    struct PromiseSlot {
        outcome: Option<Result<JsValue, JsValue>>,
        waker: Option<Waker>,
    }

    /// A future over one JS promise: the `then` callbacks set the slot and wake the poller.
    struct PromiseFuture {
        slot: Rc<RefCell<PromiseSlot>>,
        // Alive exactly as long as the future; dropping the future first invalidates them, and
        // a late settlement then throws inside a quieted derived promise, not the console.
        _on_fulfilled: Closure<dyn FnMut(JsValue)>,
        _on_rejected: Closure<dyn FnMut(JsValue)>,
    }

    /// Wraps a promise as a future. The closures are one-shot by promise semantics and never
    /// panic; `own_aborting` states both without asking for unwind safety.
    fn promise_future(promise: &js_sys::Promise) -> PromiseFuture {
        let slot = Rc::new(RefCell::new(PromiseSlot {
            outcome: None,
            waker: None,
        }));
        let fulfilled = Rc::clone(&slot);
        let on_fulfilled: Closure<dyn FnMut(JsValue)> =
            Closure::own_aborting(move |value: JsValue| {
                finish_slot(&fulfilled, Ok(value));
            });
        let rejected = Rc::clone(&slot);
        let on_rejected: Closure<dyn FnMut(JsValue)> =
            Closure::own_aborting(move |error: JsValue| {
                finish_slot(&rejected, Err(error));
            });
        let derived = promise.then2(&on_fulfilled, &on_rejected);
        // A dropped settle future's late settlement throws inside the derived promise; the
        // shared no-op catch keeps that rejection out of the browser's console.
        let quiet: QuietPromise = derived.unchecked_into();
        let _ = quiet.quiet_catch(&quiet_handler());
        PromiseFuture {
            slot,
            _on_fulfilled: on_fulfilled,
            _on_rejected: on_rejected,
        }
    }

    /// Records the settled outcome and wakes whoever polls the slot.
    fn finish_slot(slot: &Rc<RefCell<PromiseSlot>>, outcome: Result<JsValue, JsValue>) {
        let waker = {
            let mut slot = slot.borrow_mut();
            slot.outcome = Some(outcome);
            slot.waker.take()
        };
        // Wake outside the borrow: a waker that polls synchronously would re-enter poll.
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    impl Future for PromiseFuture {
        type Output = Result<JsValue, JsValue>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let mut slot = self.slot.borrow_mut();
            match slot.outcome.take() {
                Some(outcome) => Poll::Ready(outcome),
                None => {
                    slot.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }

    /// A JS-backed async block wearing the `Send` the backend traits demand.
    struct OnMain<F>(Pin<Box<F>>);

    // SAFETY: wasm32 runs all Rust and JS on one thread; this future never crosses to another.
    unsafe impl<F> Send for OnMain<F> {}

    impl<F: Future> Future for OnMain<F> {
        type Output = F::Output;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            self.get_mut().0.as_mut().poll(cx)
        }
    }

    /// Boxes a JS-backed async block as the `Send` future a backend method returns.
    fn on_main<F: Future>(block: F) -> OnMain<F> {
        OnMain(Box::pin(block))
    }

    /// Reads `name` off a JS value; a throw, a non-object or a missing field reads as absence.
    fn js_get(target: &JsValue, name: &str) -> Option<JsValue> {
        if !target.is_object() {
            return None;
        }
        js_sys::Reflect::get(
            &js_sys::Object::from(target.clone()),
            &JsValue::from_str(name),
        )
        .ok()
    }

    /// `navigator.bluetooth`, or `None` when the browser has no Web Bluetooth at all.
    fn web_bluetooth() -> Option<Bluetooth> {
        let navigator = js_get(&js_sys::global(), "navigator")?;
        let bluetooth = js_get(&navigator, "bluetooth")?;
        if bluetooth.is_undefined() || bluetooth.is_null() {
            return None;
        }
        Some(bluetooth.unchecked_into::<Bluetooth>())
    }

    /// The rejection's `name` field when it carries one, else the empty string.
    fn error_name(error: &JsValue) -> String {
        js_get(error, "name")
            .and_then(|name| name.as_string())
            .unwrap_or_default()
    }

    /// A web rejection as an error: the kind its name maps to, else the failure kind of the
    /// operation that got it.
    fn web_error_or(error: JsValue, fallback: ErrorKind) -> BluetoothError {
        match web_error_name_to_kind(&error_name(&error)) {
            ErrorKind::Backend => BluetoothError::new(fallback),
            kind => BluetoothError::new(kind),
        }
    }

    /// A synchronous JS throw, mapped the same way a rejection is.
    fn js_throw(error: JsValue) -> BluetoothError {
        web_error_or(error, ErrorKind::Backend)
    }

    /// Awaits one promise; a rejection maps to the kind its name maps to, else `fallback`.
    async fn settle(
        promise: js_sys::Promise,
        fallback: ErrorKind,
    ) -> Result<JsValue, BluetoothError> {
        match promise_future(&promise).await {
            Ok(value) => Ok(value),
            Err(error) => Err(web_error_or(error, fallback)),
        }
    }

    /// The characteristic property flags the platform carries, in `web_property_bit`'s terms.
    const WEB_PROPERTY_FLAGS: [&str; 8] = [
        "broadcast",
        "read",
        "writeWithoutResponse",
        "write",
        "notify",
        "indicate",
        "authenticatedSignedWrites",
        "reliableWrite",
    ];

    /// The characteristic's properties bag as the spec bit set.
    fn web_properties(properties: &JsValue) -> CharacteristicProperties {
        let bag = js_sys::Object::from(properties.clone());
        let mut bits = 0;
        for flag in WEB_PROPERTY_FLAGS {
            let set = js_get(&bag, flag)
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if set {
                bits |= web_property_bit(flag).unwrap_or(0);
            }
        }
        CharacteristicProperties::from_bits(bits)
    }

    /// A read value's bytes; a value that is not a DataView fails with `malformed`'s kind.
    fn value_bytes(value: &JsValue, malformed: ErrorKind) -> Result<Vec<u8>, BluetoothError> {
        if !value.is_instance_of::<js_sys::DataView>() {
            return Err(BluetoothError::new(malformed));
        }
        let view = js_sys::DataView::from(value.clone());
        let buffer = view.buffer();
        let bytes = js_sys::Uint8Array::new_with_byte_offset_and_length(
            &buffer,
            view.byte_offset() as u32,
            view.byte_length() as u32,
        );
        Ok(bytes.to_vec())
    }

    /// The bytes to write, as the BufferSource the write calls take.
    fn js_value_of(data: &[u8]) -> js_sys::Object {
        js_sys::Object::from(js_sys::Uint8Array::from(data))
    }

    /// The platform's uuid string as a [`Uuid`]; anything else fails the discovery it came from.
    fn parse_uuid(value: String) -> Result<Uuid, BluetoothError> {
        Uuid::parse(&value).map_err(|_| BluetoothError::new(ErrorKind::ServicesDiscoveryFailed))
    }

    /// One requested device: its JS handle and the listener that reports its GATT disconnects.
    struct WebDevice {
        device: BluetoothDevice,
        // Kept alive so the device's disconnect event keeps a working listener; read to retire
        // that listener when a re-pick replaces this entry.
        disconnected: Closure<dyn FnMut(JsValue)>,
    }

    /// One live characteristic subscription: the JS handle, the value listener and the stream's
    /// sender.
    struct Subscription {
        characteristic: BluetoothRemoteGATTCharacteristic,
        values: Closure<dyn FnMut(JsValue)>,
        sender: broadcast::Sender<ValueNotification>,
    }

    /// The JS handles discovery found, keyed by the uuids the app sees.
    #[derive(Default)]
    struct GattHandles {
        services: HashMap<(DeviceId, Uuid), BluetoothRemoteGATTService>,
        characteristics: HashMap<(DeviceId, Uuid, Uuid), BluetoothRemoteGATTCharacteristic>,
        descriptors: HashMap<(DeviceId, Uuid, Uuid, Uuid), BluetoothRemoteGATTDescriptor>,
    }

    /// The Web Bluetooth central: the devices the user picked, their GATT handles and streams.
    pub struct WebCentral {
        events: broadcast::Sender<ConnectionEvent>,
        devices: JsMutex<HashMap<DeviceId, WebDevice>>,
        handles: JsMutex<GattHandles>,
        subscriptions: JsMutex<HashMap<(DeviceId, Uuid, Uuid), Subscription>>,
    }

    // One instance backs every call: the device registry has to outlive any one caller's
    // `Arc<dyn CentralBackend>`.
    static CENTRAL: LazyLock<Arc<WebCentral>> = LazyLock::new(|| {
        let (events, _) = broadcast::channel(32);
        Arc::new(WebCentral {
            events,
            devices: JsMutex::new(HashMap::new()),
            handles: JsMutex::new(GattHandles::default()),
            subscriptions: JsMutex::new(HashMap::new()),
        })
    });

    /// The platform's central backend; one shared instance of the concrete [`WebCentral`].
    pub fn central_backend() -> Arc<dyn CentralBackend> {
        (*CENTRAL).clone()
    }

    /// Opens the browser's device chooser and returns the device the user picked.
    ///
    /// This is the web platform's only way in: there is no scan. An empty filter accepts every
    /// device, but Web Bluetooth then grants access to no services. A filter naming devices by id
    /// is invalid here; the platform's chooser cannot target an id.
    #[expect(dead_code)] // app-facing entry; the crate-root re-export is not this file's to add
    pub async fn request_device(filter: ScanFilter) -> Result<DeviceId, BluetoothError> {
        CENTRAL.request_device(filter).await
    }

    impl WebCentral {
        /// The requested device's JS handle, or the not-found error.
        fn device(&self, id: &DeviceId) -> Result<BluetoothDevice, BluetoothError> {
            self.devices
                .lock()
                .get(id)
                .map(|entry| entry.device.clone())
                .ok_or_else(|| BluetoothError::new(ErrorKind::DeviceNotFound))
        }

        /// The discovered characteristic's JS handle, or the not-found error.
        fn characteristic_handle(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> Result<BluetoothRemoteGATTCharacteristic, BluetoothError> {
            self.handles
                .lock()
                .characteristics
                .get(&(
                    id.clone(),
                    characteristic.service_uuid.clone(),
                    characteristic.uuid.clone(),
                ))
                .cloned()
                .ok_or_else(|| BluetoothError::new(ErrorKind::CharacteristicNotFound))
        }

        /// The discovered descriptor's JS handle, or the not-found error.
        fn descriptor_handle(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
        ) -> Result<BluetoothRemoteGATTDescriptor, BluetoothError> {
            self.handles
                .lock()
                .descriptors
                .get(&(
                    id.clone(),
                    descriptor.service_uuid.clone(),
                    descriptor.characteristic_uuid.clone(),
                    descriptor.uuid.clone(),
                ))
                .cloned()
                .ok_or_else(|| BluetoothError::new(ErrorKind::DescriptorNotFound))
        }

        /// Drops the device's cached JS handles; a rediscovery invalidates the previous walk.
        fn drop_handles(&self, id: &DeviceId) {
            self.handles.lock().services.retain(|key, _| key.0 != *id);
            self.handles
                .lock()
                .characteristics
                .retain(|key, _| key.0 != *id);
            self.handles
                .lock()
                .descriptors
                .retain(|key, _| key.0 != *id);
        }

        /// Opens the chooser and registers the picked device with its disconnect listener.
        async fn request_device(&self, filter: ScanFilter) -> Result<DeviceId, BluetoothError> {
            if !filter.device_ids.is_empty() {
                // The chooser picks by services, never by a device id.
                return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
            }
            let bluetooth =
                web_bluetooth().ok_or_else(|| BluetoothError::new(ErrorKind::Unsupported))?;
            let services = js_sys::Array::new();
            for uuid in &filter.service_uuids {
                services.push(&JsValue::from_str(uuid.as_str()));
            }
            let options: RequestDeviceOptions = js_sys::Object::new().unchecked_into();
            if services.length() == 0 {
                options.set_accept_all_devices(true);
            } else {
                let matches = js_sys::Object::new();
                js_sys::Reflect::set(&matches, &JsValue::from_str("services"), &services)
                    .map_err(|_| BluetoothError::new(ErrorKind::InvalidIdentifiers))?;
                let filters = js_sys::Array::new();
                filters.push(&matches);
                options.set_filters(&filters);
            }
            // The filters grant their own services; nothing extra is requested.
            options.set_optional_services(&js_sys::Array::new());
            let promise = bluetooth
                .request_device(&options)
                .map_err(|_| BluetoothError::new(ErrorKind::InvalidIdentifiers))?;
            let picked = settle(promise, ErrorKind::InvalidIdentifiers).await?;
            let device: BluetoothDevice = picked.unchecked_into();
            let id = DeviceId::new(device.id());
            let events = self.events.clone();
            let disconnected: Closure<dyn FnMut(JsValue)> =
                Closure::own_aborting(move |event: JsValue| {
                    let id = js_get(&event, "target").and_then(|target| js_get(&target, "id"));
                    if let Some(id) = id.and_then(|id| id.as_string()) {
                        let _ = events.send(ConnectionEvent::Disconnected(DeviceId::new(id)));
                    }
                });
            let listener = js_sys::Function::from(disconnected.as_js_value().clone());
            device
                .add_event_listener("gattserverdisconnected", &listener)
                .map_err(js_throw)?;
            let retired = {
                let mut devices = self.devices.lock();
                devices.insert(
                    id.clone(),
                    WebDevice {
                        device,
                        disconnected,
                    },
                )
            };
            if let Some(retired) = retired {
                // A re-pick replaces the entry; the old listener must come off first, or a
                // later disconnect fires the retired closure and aborts.
                let old = js_sys::Function::from(retired.disconnected.as_js_value().clone());
                let _ = retired
                    .device
                    .remove_event_listener("gattserverdisconnected", &old);
            }
            Ok(id)
        }

        /// Connects the device's GATT server.
        async fn connect_inner(&self, id: &DeviceId) -> Result<(), BluetoothError> {
            let device = self.device(id)?;
            let gatt = device.gatt();
            if gatt.connected() {
                return Err(BluetoothError::new(ErrorKind::AlreadyConnected));
            }
            let promise = gatt.connect().map_err(js_throw)?;
            settle(promise, ErrorKind::ConnectFailed).await?;
            let _ = self.events.send(ConnectionEvent::Connected(id.clone()));
            Ok(())
        }

        /// Disconnects the device; an unknown or already disconnected device is still Ok.
        fn disconnect_inner(&self, id: &DeviceId) {
            let device = self
                .devices
                .lock()
                .get(id)
                .map(|entry| entry.device.clone());
            if let Some(device) = device {
                device.gatt().disconnect();
            }
        }

        /// Whether the device's GATT server reports a live connection.
        fn connected_now(&self, id: &DeviceId) -> bool {
            self.devices
                .lock()
                .get(id)
                .is_some_and(|entry| entry.device.gatt().connected())
        }

        /// Walks the device's GATT tree into the crate's service types, caching the JS handles.
        async fn discover_inner(&self, id: &DeviceId) -> Result<Vec<Service>, BluetoothError> {
            let device = self.device(id)?;
            let gatt = device.gatt();
            if !gatt.connected() {
                return Err(BluetoothError::new(ErrorKind::DeviceNotConnected));
            }
            self.drop_handles(id);
            let services_js = settle(
                gatt.get_primary_services().map_err(js_throw)?,
                ErrorKind::ServicesDiscoveryFailed,
            )
            .await?;
            let services_array = js_sys::Array::from(&services_js);
            let mut services = Vec::with_capacity(services_array.length() as usize);
            for index in 0..services_array.length() {
                let service: BluetoothRemoteGATTService =
                    services_array.get(index).unchecked_into();
                let service_uuid = parse_uuid(service.service_uuid())?;
                let characteristics_js = settle(
                    service.get_characteristics().map_err(js_throw)?,
                    ErrorKind::ServicesDiscoveryFailed,
                )
                .await?;
                let characteristics_array = js_sys::Array::from(&characteristics_js);
                let mut characteristics =
                    Vec::with_capacity(characteristics_array.length() as usize);
                for index in 0..characteristics_array.length() {
                    let characteristic: BluetoothRemoteGATTCharacteristic =
                        characteristics_array.get(index).unchecked_into();
                    characteristics.push(
                        self.characteristic_of(id, &service_uuid, characteristic)
                            .await?,
                    );
                }
                self.handles
                    .lock()
                    .services
                    .insert((id.clone(), service_uuid.clone()), service.clone());
                services.push(Service {
                    uuid: service_uuid,
                    primary: service.is_primary(),
                    characteristics,
                    // The platform exposes no included-service listing.
                    included_services: Vec::new(),
                });
            }
            Ok(services)
        }

        /// One characteristic with its descriptors as the crate's type; caches the JS handles.
        async fn characteristic_of(
            &self,
            id: &DeviceId,
            service_uuid: &Uuid,
            characteristic: BluetoothRemoteGATTCharacteristic,
        ) -> Result<Characteristic, BluetoothError> {
            let characteristic_uuid = parse_uuid(characteristic.characteristic_uuid())?;
            let descriptors_js = settle(
                characteristic.get_descriptors().map_err(js_throw)?,
                ErrorKind::ServicesDiscoveryFailed,
            )
            .await?;
            let descriptors_array = js_sys::Array::from(&descriptors_js);
            let mut descriptors = Vec::with_capacity(descriptors_array.length() as usize);
            for index in 0..descriptors_array.length() {
                let descriptor: BluetoothRemoteGATTDescriptor =
                    descriptors_array.get(index).unchecked_into();
                let descriptor_uuid = parse_uuid(descriptor.uuid())?;
                self.handles.lock().descriptors.insert(
                    (
                        id.clone(),
                        service_uuid.clone(),
                        characteristic_uuid.clone(),
                        descriptor_uuid.clone(),
                    ),
                    descriptor,
                );
                descriptors.push(Descriptor {
                    uuid: descriptor_uuid,
                    characteristic_uuid: characteristic_uuid.clone(),
                    service_uuid: service_uuid.clone(),
                });
            }
            let current = characteristic.value();
            let value = (!current.is_null() && !current.is_undefined())
                .then(|| value_bytes(&current, ErrorKind::CharacteristicInvalidDataFormat))
                .transpose()?;
            let properties = web_properties(&characteristic.properties());
            self.handles.lock().characteristics.insert(
                (
                    id.clone(),
                    service_uuid.clone(),
                    characteristic_uuid.clone(),
                ),
                characteristic,
            );
            Ok(Characteristic {
                uuid: characteristic_uuid,
                service_uuid: service_uuid.clone(),
                properties,
                descriptors,
                value,
            })
        }

        /// Reads the characteristic's value.
        async fn read_characteristic_inner(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> Result<Vec<u8>, BluetoothError> {
            let handle = self.characteristic_handle(id, characteristic)?;
            let value = settle(
                handle.read_value().map_err(js_throw)?,
                ErrorKind::CharacteristicReadFailed,
            )
            .await?;
            value_bytes(&value, ErrorKind::CharacteristicInvalidDataFormat)
        }

        /// Writes the characteristic's value the way the write type asks.
        async fn write_characteristic_inner(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
            data: &[u8],
            write_type: WriteType,
        ) -> Result<(), BluetoothError> {
            let handle = self.characteristic_handle(id, characteristic)?;
            let value = js_value_of(data);
            let promise = match write_type {
                WriteType::WithResponse => handle.write_value_with_response(&value),
                WriteType::WithoutResponse => handle.write_value_without_response(&value),
            }
            .map_err(js_throw)?;
            settle(promise, ErrorKind::CharacteristicWriteFailed).await?;
            Ok(())
        }

        /// Starts notifications and returns the stream; a second call returns a fresh receiver of
        /// the one logical subscription.
        async fn subscribe_characteristic_inner(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> Result<broadcast::Receiver<ValueNotification>, BluetoothError> {
            let key = (
                id.clone(),
                characteristic.service_uuid.clone(),
                characteristic.uuid.clone(),
            );
            if let Some(subscription) = self.subscriptions.lock().get(&key) {
                return Ok(subscription.sender.subscribe());
            }
            let handle = self.characteristic_handle(id, characteristic)?;
            settle(
                handle.start_notifications().map_err(js_throw)?,
                ErrorKind::CharacteristicNotifyChangeFailed,
            )
            .await?;
            let sender = broadcast::channel(32).0;
            let receiver = sender.subscribe();
            let notified = characteristic.clone();
            let events = sender.clone();
            let values: Closure<dyn FnMut(JsValue)> =
                Closure::own_aborting(move |event: JsValue| {
                    let value =
                        js_get(&event, "target").and_then(|target| js_get(&target, "value"));
                    // A malformed notification carries nothing; dropping it keeps the stream sound
                    // instead of erroring with nowhere to send the error.
                    let value = value.and_then(|value| {
                        value_bytes(&value, ErrorKind::CharacteristicInvalidDataFormat).ok()
                    });
                    if let Some(bytes) = value {
                        let _ = events.send(ValueNotification {
                            characteristic: notified.clone(),
                            value: bytes,
                        });
                    }
                });
            let listener = js_sys::Function::from(values.as_js_value().clone());
            handle
                .add_value_listener("characteristicvaluechanged", &listener)
                .map_err(js_throw)?;
            // A racing second subscribe attaches a second listener before either inserts; the
            // loser drops its own and returns the winner's stream, so one listener survives.
            let winner = {
                let mut subscriptions = self.subscriptions.lock();
                match subscriptions.get(&key) {
                    Some(existing) => Some(existing.sender.subscribe()),
                    None => {
                        subscriptions.insert(
                            key,
                            Subscription {
                                characteristic: handle.clone(),
                                values,
                                sender,
                            },
                        );
                        None
                    }
                }
            };
            if let Some(winner) = winner {
                let _ = handle.remove_value_listener("characteristicvaluechanged", &listener);
                return Ok(winner);
            }
            Ok(receiver)
        }

        /// Stops notifications; not being subscribed is already unsubscribed.
        async fn unsubscribe_characteristic_inner(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> Result<(), BluetoothError> {
            let key = (
                id.clone(),
                characteristic.service_uuid.clone(),
                characteristic.uuid.clone(),
            );
            let Some(subscription) = self.subscriptions.lock().remove(&key) else {
                return Ok(());
            };
            // The listener comes off before anything can fail, so the closure never drops
            // still attached.
            let listener = js_sys::Function::from(subscription.values.as_js_value().clone());
            let _ = subscription
                .characteristic
                .remove_value_listener("characteristicvaluechanged", &listener);
            let outcome = settle(
                subscription
                    .characteristic
                    .stop_notifications()
                    .map_err(js_throw)?,
                ErrorKind::CharacteristicNotifyChangeFailed,
            )
            .await;
            outcome.map(|_| ())
        }

        /// Reads the descriptor's value.
        async fn read_descriptor_inner(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
        ) -> Result<Vec<u8>, BluetoothError> {
            let handle = self.descriptor_handle(id, descriptor)?;
            let value = settle(
                handle.read().map_err(js_throw)?,
                ErrorKind::DescriptorReadFailed,
            )
            .await?;
            value_bytes(&value, ErrorKind::DescriptorReadFailed)
        }

        /// Writes the descriptor's value.
        async fn write_descriptor_inner(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
            data: &[u8],
        ) -> Result<(), BluetoothError> {
            let handle = self.descriptor_handle(id, descriptor)?;
            let value = js_value_of(data);
            settle(
                handle.write(&value).map_err(js_throw)?,
                ErrorKind::DescriptorWriteFailed,
            )
            .await?;
            Ok(())
        }
    }

    impl CentralBackend for WebCentral {
        fn adapter_state(&self) -> BackendFuture<'_, Result<AdapterState, BluetoothError>> {
            Box::pin(on_main(async move {
                let Some(bluetooth) = web_bluetooth() else {
                    // No Web Bluetooth at all reads as the platform not supporting it.
                    return Ok(AdapterState::Unsupported);
                };
                let promise = bluetooth.get_availability().map_err(js_throw)?;
                let available = settle(promise, ErrorKind::Backend).await?;
                // The availability flag is the platform's whole adapter state.
                Ok(if available.as_bool() == Some(true) {
                    AdapterState::On
                } else {
                    AdapterState::Unsupported
                })
            }))
        }

        fn state_changes(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<AdapterState>, BluetoothError>> {
            // The platform has no state-change signal; getAvailability is a polled snapshot, so
            // the stream closes at once instead of pretending to wait.
            Box::pin(ready(Ok(broadcast::channel(32).1)))
        }

        fn start_scan(
            &self,
            _filter: ScanFilter,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ScanResult>, BluetoothError>> {
            // No background scan exists on the web; `request_device` above is the way in.
            Box::pin(ready(Err(BluetoothError::new(ErrorKind::Unsupported))))
        }

        fn stop_scan(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
            // No scan can run, so stopping is already done.
            Box::pin(ready(Ok(())))
        }

        fn system_devices(
            &self,
            _service_uuids: &[Uuid],
        ) -> BackendFuture<'_, Result<Vec<DeviceInfo>, BluetoothError>> {
            // The session's picked devices are the known ones; their services are unknown before
            // a connection, so the uuid filter has nothing to filter on.
            Box::pin(on_main(async move {
                let devices = self
                    .devices
                    .lock()
                    .iter()
                    .map(|(id, entry)| DeviceInfo {
                        id: id.clone(),
                        name: entry.device.name(),
                    })
                    .collect();
                Ok(devices)
            }))
        }

        fn connect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.clone();
            Box::pin(on_main(async move { self.connect_inner(&id).await }))
        }

        fn disconnect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.clone();
            Box::pin(on_main(async move {
                self.disconnect_inner(&id);
                Ok(())
            }))
        }

        fn is_connected(&self, id: &DeviceId) -> BackendFuture<'_, Result<bool, BluetoothError>> {
            let id = id.clone();
            Box::pin(on_main(async move { Ok(self.connected_now(&id)) }))
        }

        fn read_rssi(&self, _id: &DeviceId) -> BackendFuture<'_, Result<i8, BluetoothError>> {
            // The platform has no RSSI read; this is a capability it simply lacks.
            Box::pin(ready(Err(BluetoothError::new(ErrorKind::Unsupported))))
        }

        fn request_mtu(
            &self,
            _id: &DeviceId,
            _mtu: u16,
        ) -> BackendFuture<'_, Result<u16, BluetoothError>> {
            // Web Bluetooth fixes the ATT MTU at 23; no negotiation exists to fail.
            Box::pin(ready(Ok(23)))
        }

        fn discover_services(
            &self,
            id: &DeviceId,
        ) -> BackendFuture<'_, Result<Vec<Service>, BluetoothError>> {
            let id = id.clone();
            Box::pin(on_main(async move { self.discover_inner(&id).await }))
        }

        fn read_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            let id = id.clone();
            let characteristic = characteristic.clone();
            Box::pin(on_main(async move {
                self.read_characteristic_inner(&id, &characteristic).await
            }))
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
            Box::pin(on_main(async move {
                self.write_characteristic_inner(&id, &characteristic, &data, write_type)
                    .await
            }))
        }

        fn subscribe_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ValueNotification>, BluetoothError>>
        {
            let id = id.clone();
            let characteristic = characteristic.clone();
            Box::pin(on_main(async move {
                self.subscribe_characteristic_inner(&id, &characteristic)
                    .await
            }))
        }

        fn unsubscribe_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.clone();
            let characteristic = characteristic.clone();
            Box::pin(on_main(async move {
                self.unsubscribe_characteristic_inner(&id, &characteristic)
                    .await
            }))
        }

        fn read_descriptor(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            let id = id.clone();
            let descriptor = descriptor.clone();
            Box::pin(on_main(async move {
                self.read_descriptor_inner(&id, &descriptor).await
            }))
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
            Box::pin(on_main(async move {
                self.write_descriptor_inner(&id, &descriptor, &data).await
            }))
        }

        fn connection_events(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ConnectionEvent>, BluetoothError>>
        {
            Box::pin(ready(Ok(self.events.subscribe())))
        }
    }
}
