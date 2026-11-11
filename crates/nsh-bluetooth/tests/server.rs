//! The peripheral role: advertising, notifications and the served GATT db, through the public
//! `Server` API over the fake and the unsupported server backends.

use std::{future::Future, sync::Arc, time::Duration};

use nsh_bluetooth::{
    AdvertiseOptions, BluetoothError, Characteristic, CharacteristicProperties, ErrorKind,
    FakeServerBackend, Server, ServerOp, Service, UnsupportedServer, Uuid,
};

/// A hang is the bug: every guarded call must resolve within this bound.
const RESOLVE_BOUND: Duration = Duration::from_secs(5);

async fn resolves<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(RESOLVE_BOUND, future)
        .await
        .expect("the call resolves instead of hanging")
}

fn notify_service() -> Service {
    Service {
        uuid: Uuid::parse("180D").unwrap(),
        primary: true,
        characteristics: vec![Characteristic {
            uuid: Uuid::parse("2A37").unwrap(),
            service_uuid: Uuid::parse("180D").unwrap(),
            properties: CharacteristicProperties::NOTIFY,
            descriptors: Vec::new(),
            value: None,
        }],
        included_services: Vec::new(),
    }
}

fn read_only_service() -> Service {
    Service {
        uuid: Uuid::parse("180F").unwrap(),
        primary: true,
        characteristics: vec![Characteristic {
            uuid: Uuid::parse("2A19").unwrap(),
            service_uuid: Uuid::parse("180F").unwrap(),
            properties: CharacteristicProperties::READ,
            descriptors: Vec::new(),
            value: None,
        }],
        included_services: Vec::new(),
    }
}

async fn served() -> (Arc<FakeServerBackend>, Server) {
    let fake = FakeServerBackend::new();
    let server = Server::new(fake.clone());
    resolves(server.add_service(notify_service()))
        .await
        .unwrap();
    (fake, server)
}

#[tokio::test]
async fn advertise_twice_in_progress() {
    let (_fake, server) = served().await;
    resolves(server.advertise(AdvertiseOptions::default()))
        .await
        .unwrap();
    assert!(server.is_advertising());
    let error = resolves(server.advertise(AdvertiseOptions::default()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_IN_PROGRESS");

    resolves(server.stop_advertising()).await.unwrap();
    // Stopping while not advertising is Ok.
    resolves(server.stop_advertising()).await.unwrap();
    assert!(!server.is_advertising());
}

#[tokio::test]
async fn advertise_unsupported_on_platform() {
    // Every bridge may yet land one; only the absent case carries the code.
    if let Err(error) = Server::platform_server() {
        assert_eq!(error.code(), "ERR_BLUETOOTH_UNSUPPORTED");
    }
    let server = Server::new(Arc::new(UnsupportedServer));
    let error = resolves(server.advertise(AdvertiseOptions::default()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_UNSUPPORTED");
    assert!(!server.is_advertising());
}

#[tokio::test]
async fn advertise_failure_coded() {
    let fake = FakeServerBackend::new();
    let server = Server::new(fake.clone());
    fake.fail_next_server(
        ServerOp::StartAdvertising,
        BluetoothError::new(ErrorKind::AdvertiseFailed).with_status(6),
    );
    let error = resolves(server.advertise(AdvertiseOptions::default()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_ADVERTISE_FAILED");
    assert_eq!(error.status(), Some(6));

    // The failed start freed the slot: the next advertise runs.
    assert!(!server.is_advertising());
    resolves(server.advertise(AdvertiseOptions::default()))
        .await
        .unwrap();
    assert!(server.is_advertising());
}

#[tokio::test]
async fn notify_without_subscriber_fails() {
    let fake = FakeServerBackend::new();
    let server = Server::new(fake.clone());
    resolves(server.add_service(read_only_service()))
        .await
        .unwrap();
    resolves(server.add_service(notify_service()))
        .await
        .unwrap();

    // The fake's simulated central subscribes only to notify- or indicate-capable
    // characteristics; a read-only one has no subscriber.
    let error = resolves(server.notify(
        Uuid::parse("180F").unwrap(),
        Uuid::parse("2A19").unwrap(),
        &[1],
    ))
    .await
    .unwrap_err();
    assert_eq!(
        error.code(),
        "ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED"
    );
    assert_eq!(
        fake.subscriber_count(Uuid::parse("180F").unwrap(), Uuid::parse("2A19").unwrap()),
        0
    );

    // The notify-capable characteristic has the subscriber, so the value is delivered.
    resolves(server.notify(
        Uuid::parse("180D").unwrap(),
        Uuid::parse("2A37").unwrap(),
        &[1],
    ))
    .await
    .unwrap();
    assert_eq!(
        fake.subscriber_count(Uuid::parse("180D").unwrap(), Uuid::parse("2A37").unwrap()),
        1
    );
}

#[tokio::test]
async fn duplicate_service_fails() {
    let (_fake, server) = served().await;
    let error = resolves(server.add_service(notify_service()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_SERVER_FAILED");

    // Removing it once makes the uuid servable again.
    resolves(server.remove_service(Uuid::parse("180D").unwrap()))
        .await
        .unwrap();
    resolves(server.add_service(notify_service()))
        .await
        .unwrap();
}

#[tokio::test]
async fn server_unknown_characteristic() {
    let (fake, server) = served().await;
    let service = Uuid::parse("180D").unwrap();
    let measurement = Uuid::parse("2A37").unwrap();
    let unknown = Uuid::parse("FFFF").unwrap();

    let error =
        resolves(server.set_characteristic_value(service.clone(), unknown.clone(), vec![1]))
            .await
            .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
    let error = resolves(server.notify(service.clone(), unknown.clone(), &[1]))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");

    // The known handle is accepted and captured.
    resolves(server.set_characteristic_value(service.clone(), measurement.clone(), vec![1, 2]))
        .await
        .unwrap();
    assert_eq!(
        fake.captured_values(),
        vec![(service, measurement, vec![1, 2])]
    );
}
