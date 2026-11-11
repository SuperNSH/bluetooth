//! GATT operations over a connected device: discovery guards, lookups, scripted failures,
//! subscriptions, write types, MTU and descriptors, through the public `Device` API.

use std::{future::Future, sync::Arc, time::Duration};

use nsh_bluetooth::{
    AdvertisementData, BluetoothError, Central, Characteristic, CharacteristicProperties,
    ConnectOptions, Descriptor, Device, DeviceId, ErrorKind, FakeBackend, FakeOp, ScanResult,
    Service, Uuid, WriteType,
};

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

fn heart_rate_characteristic() -> Characteristic {
    heart_rate_services().remove(0).characteristics.remove(0)
}

fn heart_rate_descriptor() -> Descriptor {
    heart_rate_characteristic().descriptors.remove(0)
}

/// A connected watch whose services are not discovered yet.
async fn connected_watch() -> (Arc<FakeBackend>, Device) {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    fake.add_scanned_device(scan_result("watch", "180D"));
    fake.set_services(&DeviceId::new("watch"), heart_rate_services());
    let device = central.device(&DeviceId::new("watch"));
    resolves(device.connect(ConnectOptions::default()))
        .await
        .unwrap();
    (fake, device)
}

/// A connected watch with its services discovered.
async fn discovered_watch() -> (Arc<FakeBackend>, Device) {
    let (fake, device) = connected_watch().await;
    resolves(device.discover_services()).await.unwrap();
    (fake, device)
}

#[tokio::test]
async fn gatt_before_discovery_fails() {
    let (_fake, device) = connected_watch().await;
    let characteristic = heart_rate_characteristic();
    let descriptor = heart_rate_descriptor();

    let error = resolves(device.read_characteristic(&characteristic))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
    let error =
        resolves(device.write_characteristic(&characteristic, &[1], WriteType::WithResponse))
            .await
            .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
    let error = resolves(device.subscribe_characteristic(&characteristic))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
    let error = resolves(device.unsubscribe_characteristic(&characteristic))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
    let error = resolves(device.read_descriptor(&descriptor))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
    let error = resolves(device.write_descriptor(&descriptor, &[1]))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");

    // The sync lookups carry the same guard.
    let error = device.service(Uuid::parse("180D").unwrap()).unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
    let error = device
        .characteristic(Uuid::parse("2A37").unwrap())
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
    assert!(device.services().is_empty());
}

#[tokio::test]
async fn unknown_uuids_not_found() {
    let (_fake, device) = discovered_watch().await;

    let error = device.service(Uuid::parse("181A").unwrap()).unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICE_NOT_FOUND");
    let error = device
        .characteristic(Uuid::parse("FFFF").unwrap())
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");

    // The whole path exists except the descriptor itself.
    let missing = Descriptor {
        uuid: Uuid::parse("2901").unwrap(),
        characteristic_uuid: Uuid::parse("2A37").unwrap(),
        service_uuid: Uuid::parse("180D").unwrap(),
    };
    let error = resolves(device.read_descriptor(&missing))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND");

    assert!(device.service(Uuid::parse("180D").unwrap()).is_ok());
    assert!(device.characteristic(Uuid::parse("2A37").unwrap()).is_ok());
}

#[tokio::test]
async fn characteristic_read_write_failures() {
    let (fake, device) = discovered_watch().await;
    let characteristic = device
        .characteristic(Uuid::parse("2A37").unwrap())
        .unwrap()
        .clone();

    fake.fail_next(
        FakeOp::ReadCharacteristic,
        BluetoothError::new(ErrorKind::CharacteristicReadFailed).with_status(3),
    );
    let error = resolves(device.read_characteristic(&characteristic))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_CHARACTERISTIC_READ_FAILED");
    assert_eq!(error.status(), Some(3));

    fake.fail_next(
        FakeOp::WriteCharacteristic,
        BluetoothError::new(ErrorKind::CharacteristicWriteFailed).with_status(4),
    );
    let error =
        resolves(device.write_characteristic(&characteristic, &[1], WriteType::WithResponse))
            .await
            .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_CHARACTERISTIC_WRITE_FAILED");
    assert_eq!(error.status(), Some(4));

    // The failures are one-shot: both operations succeed again.
    assert_eq!(
        resolves(device.read_characteristic(&characteristic))
            .await
            .unwrap(),
        vec![6, 66]
    );
    resolves(device.write_characteristic(&characteristic, &[1], WriteType::WithResponse))
        .await
        .unwrap();
}

#[tokio::test]
async fn subscribe_receives_notifications() {
    let (fake, device) = discovered_watch().await;
    let characteristic = device
        .characteristic(Uuid::parse("2A37").unwrap())
        .unwrap()
        .clone();
    let mut notifications = resolves(device.subscribe_characteristic(&characteristic))
        .await
        .unwrap();

    fake.inject_notification(&DeviceId::new("watch"), characteristic.clone(), vec![9]);
    let note = resolves(notifications.recv()).await.unwrap();
    assert_eq!(note.value, vec![9]);
    assert_eq!(note.characteristic.uuid, characteristic.uuid);
}

#[tokio::test]
async fn subscribe_unsubscribe_idempotent() {
    let (fake, device) = discovered_watch().await;
    let characteristic = device
        .characteristic(Uuid::parse("2A37").unwrap())
        .unwrap()
        .clone();
    let mut first = resolves(device.subscribe_characteristic(&characteristic))
        .await
        .unwrap();
    // Subscribing again keeps one logical subscription and returns a fresh receiver.
    let mut second = resolves(device.subscribe_characteristic(&characteristic))
        .await
        .unwrap();

    fake.inject_notification(&DeviceId::new("watch"), characteristic.clone(), vec![1, 2]);
    assert_eq!(resolves(first.recv()).await.unwrap().value, vec![1, 2]);
    assert_eq!(resolves(second.recv()).await.unwrap().value, vec![1, 2]);

    resolves(device.unsubscribe_characteristic(&characteristic))
        .await
        .unwrap();
    // Unsubscribing with nothing subscribed is Ok.
    resolves(device.unsubscribe_characteristic(&characteristic))
        .await
        .unwrap();
}

#[tokio::test]
async fn write_without_response_records_type() {
    let (_fake, device) = discovered_watch().await;
    let characteristic = device
        .characteristic(Uuid::parse("2A37").unwrap())
        .unwrap()
        .clone();
    // The fake stores the written value; it exposes no write-type capture to assert against.
    resolves(device.write_characteristic(&characteristic, &[7, 77], WriteType::WithoutResponse))
        .await
        .unwrap();
    assert_eq!(
        resolves(device.read_characteristic(&characteristic))
            .await
            .unwrap(),
        vec![7, 77]
    );
}

#[tokio::test]
async fn mtu_change_failure() {
    let (fake, device) = connected_watch().await;
    fake.fail_next(
        FakeOp::RequestMtu,
        BluetoothError::new(ErrorKind::MtuChangeFailed),
    );
    let error = resolves(device.request_mtu(247)).await.unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_MTU_CHANGE_FAILED");

    // The refusal costs nothing: the negotiation answers with the configured MTU.
    fake.set_mtu(222);
    assert_eq!(resolves(device.request_mtu(512)).await.unwrap(), 222);
}

#[tokio::test]
async fn descriptor_failures_coded() {
    let (fake, device) = discovered_watch().await;
    let descriptor = heart_rate_descriptor();

    fake.fail_next(
        FakeOp::ReadDescriptor,
        BluetoothError::new(ErrorKind::DescriptorReadFailed).with_status(2),
    );
    let error = resolves(device.read_descriptor(&descriptor))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_DESCRIPTOR_READ_FAILED");
    assert_eq!(error.status(), Some(2));

    fake.fail_next(
        FakeOp::WriteDescriptor,
        BluetoothError::new(ErrorKind::DescriptorWriteFailed).with_status(3),
    );
    let error = resolves(device.write_descriptor(&descriptor, &[1, 0]))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_DESCRIPTOR_WRITE_FAILED");
    assert_eq!(error.status(), Some(3));

    fake.fail_next(
        FakeOp::WriteDescriptor,
        BluetoothError::new(ErrorKind::DescriptorWriteNotAllowed),
    );
    let error = resolves(device.write_descriptor(&descriptor, &[1, 0]))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED");

    // After the scripted refusals, the stored round trip works.
    resolves(device.write_descriptor(&descriptor, &[1, 0]))
        .await
        .unwrap();
    assert_eq!(
        resolves(device.read_descriptor(&descriptor)).await.unwrap(),
        vec![1, 0]
    );
}
