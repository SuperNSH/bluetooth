//! Scan lifecycle, filtering and start failures, driven through the public `Central` API over
//! the fake.

use std::{future::Future, time::Duration};

use nsh_bluetooth::{
    AdvertisementData, BluetoothError, Central, DeviceId, ErrorKind, FakeBackend, FakeOp,
    ScanFilter, ScanResult, Uuid,
};

/// A hang is the bug: every guarded call must resolve within this bound.
const RESOLVE_BOUND: Duration = Duration::from_secs(5);
/// How long to wait for a result that must not arrive.
const QUIET_BOUND: Duration = Duration::from_millis(150);

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

#[tokio::test]
async fn second_scan_fails_in_progress() {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    let mut first = resolves(central.scan(ScanFilter::default())).await.unwrap();
    let error = resolves(central.scan(ScanFilter::default()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_IN_PROGRESS");

    // The refusal leaves the first scan running and delivering.
    fake.add_scanned_device(scan_result("watch", "180D"));
    let result = resolves(first.recv()).await.unwrap();
    assert_eq!(result.id, DeviceId::new("watch"));
    resolves(first.stop()).await;
    assert_eq!(resolves(first.recv()).await, None);
}

#[tokio::test]
async fn scan_filter_matches_service_and_device() {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    let filter = ScanFilter {
        service_uuids: vec![Uuid::parse("180D").unwrap()],
        device_ids: vec![DeviceId::new("watch")],
    };
    let mut stream = resolves(central.scan(filter)).await.unwrap();

    // Neither the device alone nor the service alone is enough.
    fake.add_scanned_device(scan_result("watch", "181A"));
    fake.add_scanned_device(scan_result("phone", "180D"));
    fake.add_scanned_device(scan_result("watch", "180D"));
    let result = resolves(stream.recv()).await.unwrap();
    assert_eq!(result.id, DeviceId::new("watch"));
    assert_eq!(
        result.advertisement.service_uuids,
        vec![Uuid::parse("180D").unwrap()]
    );

    // The non-matches stay undelivered.
    let quiet = tokio::time::timeout(QUIET_BOUND, stream.recv()).await;
    assert!(quiet.is_err(), "only the matching result is delivered");
    resolves(stream.stop()).await;
    assert_eq!(resolves(stream.recv()).await, None);
}

#[tokio::test]
async fn scan_start_failed_carries_status() {
    let fake = FakeBackend::new();
    let central = Central::new(fake.clone());
    fake.fail_next(
        FakeOp::StartScan,
        BluetoothError::new(ErrorKind::ScanStartFailed).with_status(5),
    );
    let error = resolves(central.scan(ScanFilter::default()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_BLUETOOTH_SCAN_START_FAILED");
    assert_eq!(error.status(), Some(5));

    // The failed start freed the gate: the next scan runs.
    let mut stream = resolves(central.scan(ScanFilter::default())).await.unwrap();
    fake.add_scanned_device(scan_result("watch", "180D"));
    assert_eq!(
        resolves(stream.recv()).await.unwrap().id,
        DeviceId::new("watch")
    );
    resolves(stream.stop()).await;
}
