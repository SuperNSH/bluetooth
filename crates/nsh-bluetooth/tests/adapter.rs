//! Adapter state, permissions and the connect lifecycle, driven through the public `Central`,
//! `Device` and `Server` API over the fake and the unsupported backends.

use std::{
    future::{Future, pending, ready},
    sync::Arc,
    time::Duration,
};

use nsh_bluetooth::{
    AdapterState, AdvertiseOptions, AdvertisementData, BackendFuture, BluetoothError, Central,
    CentralBackend, Characteristic, CharacteristicProperties, ConnectOptions, ConnectionEvent,
    Descriptor, Device, DeviceId, DeviceInfo, ErrorKind, FakeBackend, FakeOp, ScanFilter,
    ScanResult, Server, Service, UnsupportedBackend, UnsupportedServer, Uuid, ValueNotification,
    WriteType,
};
use tokio::sync::broadcast;

/// A hang is the bug: every guarded call must resolve within this bound.
const RESOLVE_BOUND: Duration = Duration::from_secs(5);

async fn resolves<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(RESOLVE_BOUND, future)
        .await
        .expect("the call resolves instead of hanging")
}

fn scan_result(id: &str, service: &str) -> ScanResult {
    ScanResult {
        id: DeviceId::new(id),
        name: Some(id.to_owned()),
        rssi: Some(-42),
        advertisement: AdvertisementData {
            service_uuids: vec![Uuid::parse(service).unwrap()],
            ..AdvertisementData::default()
        },
    }
}

fn heart_rate_services() -> Vec<Service> {
    vec![Service {
        uuid: Uuid::parse("180D").unwrap(),
        primary: true,
        characteristics: vec![Characteristic {
            uuid: Uuid::parse("2A37").unwrap(),
            service_uuid: Uuid::parse("180D").unwrap(),
            properties: CharacteristicProperties::from_bits(0x12), // read | notify
            descriptors: vec![Descriptor {
                uuid: Uuid::parse("2902").unwrap(),
                characteristic_uuid: Uuid::parse("2A37").unwrap(),
                service_uuid: Uuid::parse("180D").unwrap(),
            }],
            value: Some(vec![6, 66]),
        }],
        included_services: Vec::new(),
    }]
}

/// A connected watch with its services discovered, over the fake.
async fn connected_watch() -> (Arc<FakeBackend>, Central, Device) {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    fake.add_scanned_device(scan_result("watch", "180D"));
    fake.set_services(&DeviceId::new("watch"), heart_rate_services());
    let device = central.device(&DeviceId::new("watch"));
    resolves(device.connect(ConnectOptions::default()))
        .await
        .unwrap();
    resolves(device.discover_services()).await.unwrap();
    (fake, central, device)
}

/// A backend whose connect never resolves, to race the timeout and the cancel against.
struct PendingBackend;

impl CentralBackend for PendingBackend {
    fn adapter_state(&self) -> BackendFuture<'_, Result<AdapterState, BluetoothError>> {
        Box::pin(ready(Ok(AdapterState::On)))
    }

    fn state_changes(
        &self,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<AdapterState>, BluetoothError>> {
        Box::pin(ready(Ok(broadcast::channel(32).1)))
    }

    fn start_scan(
        &self,
        _filter: ScanFilter,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<ScanResult>, BluetoothError>> {
        Box::pin(ready(Err(BluetoothError::new(ErrorKind::Unsupported))))
    }

    fn stop_scan(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(Ok(())))
    }

    fn system_devices(
        &self,
        _service_uuids: &[Uuid],
    ) -> BackendFuture<'_, Result<Vec<DeviceInfo>, BluetoothError>> {
        Box::pin(ready(Ok(Vec::new())))
    }

    fn connect(&self, _id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(pending())
    }

    fn disconnect(&self, _id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(Ok(())))
    }

    fn is_connected(&self, _id: &DeviceId) -> BackendFuture<'_, Result<bool, BluetoothError>> {
        Box::pin(ready(Ok(true)))
    }

    fn read_rssi(&self, _id: &DeviceId) -> BackendFuture<'_, Result<i8, BluetoothError>> {
        Box::pin(ready(Ok(-60)))
    }

    fn request_mtu(
        &self,
        _id: &DeviceId,
        _mtu: u16,
    ) -> BackendFuture<'_, Result<u16, BluetoothError>> {
        Box::pin(ready(Ok(185)))
    }

    fn discover_services(
        &self,
        _id: &DeviceId,
    ) -> BackendFuture<'_, Result<Vec<Service>, BluetoothError>> {
        Box::pin(ready(Ok(Vec::new())))
    }

    fn read_characteristic(
        &self,
        _id: &DeviceId,
        _characteristic: &Characteristic,
    ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
        Box::pin(ready(Ok(Vec::new())))
    }

    fn write_characteristic(
        &self,
        _id: &DeviceId,
        _characteristic: &Characteristic,
        _data: &[u8],
        _write_type: WriteType,
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(Ok(())))
    }

    fn subscribe_characteristic(
        &self,
        _id: &DeviceId,
        _characteristic: &Characteristic,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<ValueNotification>, BluetoothError>> {
        Box::pin(ready(Ok(broadcast::channel(32).1)))
    }

    fn unsubscribe_characteristic(
        &self,
        _id: &DeviceId,
        _characteristic: &Characteristic,
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(Ok(())))
    }

    fn read_descriptor(
        &self,
        _id: &DeviceId,
        _descriptor: &Descriptor,
    ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
        Box::pin(ready(Ok(Vec::new())))
    }

    fn write_descriptor(
        &self,
        _id: &DeviceId,
        _descriptor: &Descriptor,
        _data: &[u8],
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(Ok(())))
    }

    fn connection_events(
        &self,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<ConnectionEvent>, BluetoothError>> {
        Box::pin(ready(Ok(broadcast::channel(32).1)))
    }
}

#[tokio::test]
async fn bluetooth_off_reports_powered_off() {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    fake.set_adapter_state(AdapterState::PoweredOff);
    assert_eq!(
        resolves(central.state()).await.unwrap(),
        AdapterState::PoweredOff
    );
    // Off is still a usable stack; only Unsupported clears this.
    assert!(resolves(central.is_supported()).await);
    // The fake does not gate scan on the adapter state; the platform bridge does, and its
    // refusal is what fail_next scripts.
    fake.fail_next(
        FakeOp::StartScan,
        BluetoothError::new(ErrorKind::PoweredOff),
    );
    let error = resolves(central.scan(ScanFilter::default()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_POWERED_OFF");
}

#[tokio::test]
async fn scan_refuses_when_unauthorized() {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    fake.add_scanned_device(scan_result("watch", "180D"));
    fake.fail_next(
        FakeOp::StartScan,
        BluetoothError::new(ErrorKind::Unauthorized),
    );
    let error = resolves(central.scan(ScanFilter::default()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_UNAUTHORIZED");

    fake.fail_next(
        FakeOp::Connect,
        BluetoothError::new(ErrorKind::Unauthorized),
    );
    let device = central.device(&DeviceId::new("watch"));
    let error = resolves(device.connect(ConnectOptions::default()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_UNAUTHORIZED");
}

#[tokio::test]
async fn location_disabled_blocks_scan() {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    fake.fail_next(
        FakeOp::StartScan,
        BluetoothError::new(ErrorKind::LocationServicesDisabled),
    );
    let error = resolves(central.scan(ScanFilter::default()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED");
}

#[tokio::test]
async fn unsupported_platform_codes_every_call() {
    let central = Central::new(Arc::new(UnsupportedBackend));
    assert!(!resolves(central.is_supported()).await);
    assert_eq!(
        resolves(central.state()).await.unwrap_err().code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(central.state_changes()).await.unwrap_err().code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(central.scan(ScanFilter::default()))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(central.system_devices(&[]))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );

    let device = central.device(&DeviceId::new("any"));
    assert_eq!(
        resolves(device.connect(ConnectOptions::default()))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(device.disconnect()).await.unwrap_err().code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(device.is_connected()).await.unwrap_err().code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(device.read_rssi()).await.unwrap_err().code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(device.request_mtu(185)).await.unwrap_err().code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(device.discover_services())
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );

    // The discovery guard runs before the backend, so GATT calls report the missing discovery.
    let characteristic = heart_rate_services().remove(0).characteristics.remove(0);
    let descriptor = heart_rate_services()
        .remove(0)
        .characteristics
        .remove(0)
        .descriptors
        .remove(0);
    assert_eq!(
        resolves(device.read_characteristic(&characteristic))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED"
    );
    assert_eq!(
        resolves(device.write_characteristic(&characteristic, &[], WriteType::WithResponse))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED"
    );
    assert_eq!(
        resolves(device.subscribe_characteristic(&characteristic))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED"
    );

    // The backend itself refuses the GATT calls; the wrapper's guard would mask this.
    let backend = UnsupportedBackend;
    let id = DeviceId::new("any");
    assert_eq!(
        resolves(backend.read_characteristic(&id, &characteristic))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(backend.write_characteristic(&id, &characteristic, &[], WriteType::WithResponse))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(backend.subscribe_characteristic(&id, &characteristic))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(backend.unsubscribe_characteristic(&id, &characteristic))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(backend.read_descriptor(&id, &descriptor))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(backend.write_descriptor(&id, &descriptor, &[]))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(central.connection_events())
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );

    let server = Server::new(Arc::new(UnsupportedServer));
    assert_eq!(
        resolves(server.add_service(heart_rate_services().remove(0)))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(server.remove_service(Uuid::parse("180D").unwrap()))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(server.advertise(AdvertiseOptions::default()))
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(server.stop_advertising())
            .await
            .unwrap_err()
            .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(server.set_characteristic_value(
            Uuid::parse("180D").unwrap(),
            Uuid::parse("2A37").unwrap(),
            vec![1]
        ))
        .await
        .unwrap_err()
        .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(server.notify(
            Uuid::parse("180D").unwrap(),
            Uuid::parse("2A37").unwrap(),
            &[1]
        ))
        .await
        .unwrap_err()
        .code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert_eq!(
        resolves(server.requests()).await.unwrap_err().code(),
        "ERR_BLUETOOTH_UNSUPPORTED"
    );
    assert!(!server.is_advertising());
}

#[tokio::test]
async fn invalid_identifier_rejected() {
    for malformed in [
        "",
        "180",
        "180FF",
        "xyz",
        "0000180d00001000800000805f9b34fb",
        "0000180d-0000-1000-8000-00805f9b34fbX",
        "0000180d_0000_1000_8000_00805f9b34fb",
        "-0000180d-0000-1000-8000-00805f9b34fb",
    ] {
        let error = Uuid::parse(malformed).unwrap_err();
        assert_eq!(
            error.code(),
            "ERR_BLUETOOTH_INVALID_IDENTIFIERS",
            "{malformed}"
        );
    }
    // Lookups take typed ids from a scan; an id nothing delivered is refused, not used.
    let central = Central::new(FakeBackend::new());
    let ghost = central.device(&DeviceId::new("ghost"));
    let error = resolves(ghost.connect(ConnectOptions::default()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_DEVICE_NOT_FOUND");
}

#[tokio::test]
async fn connect_timeout_resolves_once() {
    let central = Central::new(Arc::new(PendingBackend));
    let device = central.device(&DeviceId::new("watch"));
    // The backend never answers; the option's deadline resolves the connect exactly once.
    let error = resolves(device.connect(ConnectOptions {
        timeout: Some(Duration::from_millis(25)),
        max_attempts: 9,
    }))
    .await
    .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_TIMEOUT");
    // A zero timeout is a deadline already passed.
    let error = resolves(device.connect(ConnectOptions {
        timeout: Some(Duration::ZERO),
        max_attempts: 1,
    }))
    .await
    .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_TIMEOUT");
}

#[tokio::test]
async fn connect_retries_android_133() {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    fake.add_scanned_device(scan_result("watch", "180D"));
    fake.add_scanned_device(scan_result("ring", "180D"));
    let status_133 = || BluetoothError::new(ErrorKind::ConnectFailed).with_status(133);

    // The first try fails with 133; two attempts succeed on the second.
    fake.fail_next(FakeOp::Connect, status_133());
    let watch = central.device(&DeviceId::new("watch"));
    resolves(watch.connect(ConnectOptions {
        timeout: None,
        max_attempts: 2,
    }))
    .await
    .unwrap();
    assert!(resolves(watch.is_connected()).await.unwrap());

    // One attempt surfaces the 133 instead of retrying it away.
    fake.fail_next(FakeOp::Connect, status_133());
    let ring = central.device(&DeviceId::new("ring"));
    let error = resolves(ring.connect(ConnectOptions {
        timeout: None,
        max_attempts: 1,
    }))
    .await
    .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_CONNECT_FAILED");
    assert_eq!(error.status(), Some(133));
}

#[tokio::test]
async fn connect_twice_already_connected() {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    fake.add_scanned_device(scan_result("watch", "180D"));
    let device = central.device(&DeviceId::new("watch"));
    resolves(device.connect(ConnectOptions::default()))
        .await
        .unwrap();
    let error = resolves(device.connect(ConnectOptions::default()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_ALREADY_CONNECTED");
    assert!(resolves(device.is_connected()).await.unwrap());
}

#[tokio::test]
async fn cancel_connect_returns_cancelled() {
    let central = Central::new(Arc::new(PendingBackend));
    let connecting = central.device(&DeviceId::new("watch"));
    let task = tokio::spawn(async move {
        resolves(connecting.connect(ConnectOptions {
            timeout: None,
            max_attempts: 3,
        }))
        .await
    });
    // Give the connect a turn to register its cancel slot.
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    let cancel_device = central.device(&DeviceId::new("watch"));
    cancel_device.cancel_connect().unwrap();
    let error = resolves(task)
        .await
        .expect("the connect task joins")
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_CANCELLED");
    // With nothing pending, the cancel is still Ok.
    cancel_device.cancel_connect().unwrap();
}

#[tokio::test]
async fn disconnect_is_idempotent() {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    fake.add_scanned_device(scan_result("watch", "180D"));
    let device = central.device(&DeviceId::new("watch"));
    // Never connected: disconnecting is still Ok, twice.
    resolves(device.disconnect()).await.unwrap();
    resolves(device.disconnect()).await.unwrap();
    assert!(!resolves(device.is_connected()).await.unwrap());

    // And after a real connect.
    resolves(device.connect(ConnectOptions::default()))
        .await
        .unwrap();
    resolves(device.disconnect()).await.unwrap();
    resolves(device.disconnect()).await.unwrap();
    assert!(!resolves(device.is_connected()).await.unwrap());
}

#[tokio::test]
async fn lost_connection_fails_operation() {
    let (fake, central, device) = connected_watch().await;
    let mut events = resolves(central.connection_events()).await.unwrap();
    fake.simulate_disconnect(&DeviceId::new("watch"));
    let characteristic = heart_rate_services().remove(0).characteristics.remove(0);
    let error = resolves(device.read_characteristic(&characteristic))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_DISCONNECTED");
    assert_eq!(
        resolves(events.recv()).await.unwrap(),
        ConnectionEvent::Disconnected(DeviceId::new("watch"))
    );
}

#[tokio::test]
async fn state_stream_reports_resetting_and_unknown() {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    let mut changes = resolves(central.state_changes()).await.unwrap();
    fake.set_adapter_state(AdapterState::Resetting);
    assert_eq!(
        resolves(changes.recv()).await.unwrap(),
        AdapterState::Resetting
    );
    fake.set_adapter_state(AdapterState::Unknown);
    assert_eq!(
        resolves(changes.recv()).await.unwrap(),
        AdapterState::Unknown
    );
    assert_eq!(
        resolves(central.state()).await.unwrap(),
        AdapterState::Unknown
    );
}
