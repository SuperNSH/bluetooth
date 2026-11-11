//! Backends for a platform with no bridge: every call, streams included, reports
//! `ERR_BLUETOOTH_UNSUPPORTED`. Nothing here panics or hangs.

use std::future::ready;

use tokio::sync::broadcast;

use crate::backend::{
    AdapterState, AdvertiseOptions, BackendFuture, CentralBackend, Characteristic, ConnectionEvent,
    Descriptor, DeviceId, GattRequest, ScanFilter, ServerBackend, Service, Uuid, WriteType,
};
use crate::error::{BluetoothError, ErrorKind};

fn unsupported<T>() -> Result<T, BluetoothError> {
    Err(BluetoothError::new(ErrorKind::Unsupported))
}

/// A central backend with no stack behind it.
pub struct UnsupportedBackend;

impl CentralBackend for UnsupportedBackend {
    fn adapter_state(&self) -> BackendFuture<'_, Result<AdapterState, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn state_changes(
        &self,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<AdapterState>, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn start_scan(
        &self,
        _filter: ScanFilter,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<crate::backend::ScanResult>, BluetoothError>>
    {
        Box::pin(ready(unsupported()))
    }

    fn stop_scan(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn system_devices(
        &self,
        _service_uuids: &[Uuid],
    ) -> BackendFuture<'_, Result<Vec<crate::backend::DeviceInfo>, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn connect(&self, _id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn disconnect(&self, _id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn is_connected(&self, _id: &DeviceId) -> BackendFuture<'_, Result<bool, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn read_rssi(&self, _id: &DeviceId) -> BackendFuture<'_, Result<i8, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn request_mtu(
        &self,
        _id: &DeviceId,
        _mtu: u16,
    ) -> BackendFuture<'_, Result<u16, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn discover_services(
        &self,
        _id: &DeviceId,
    ) -> BackendFuture<'_, Result<Vec<Service>, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn read_characteristic(
        &self,
        _id: &DeviceId,
        _characteristic: &Characteristic,
    ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn write_characteristic(
        &self,
        _id: &DeviceId,
        _characteristic: &Characteristic,
        _data: &[u8],
        _write_type: WriteType,
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn subscribe_characteristic(
        &self,
        _id: &DeviceId,
        _characteristic: &Characteristic,
    ) -> BackendFuture<
        '_,
        Result<broadcast::Receiver<crate::backend::ValueNotification>, BluetoothError>,
    > {
        Box::pin(ready(unsupported()))
    }

    fn unsubscribe_characteristic(
        &self,
        _id: &DeviceId,
        _characteristic: &Characteristic,
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn read_descriptor(
        &self,
        _id: &DeviceId,
        _descriptor: &Descriptor,
    ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn write_descriptor(
        &self,
        _id: &DeviceId,
        _descriptor: &Descriptor,
        _data: &[u8],
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn connection_events(
        &self,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<ConnectionEvent>, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }
}

/// A server backend with no stack behind it.
pub struct UnsupportedServer;

impl ServerBackend for UnsupportedServer {
    fn add_service(&self, _service: Service) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn remove_service(&self, _uuid: Uuid) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn start_advertising(
        &self,
        _options: AdvertiseOptions,
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn stop_advertising(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn is_advertising(&self) -> BackendFuture<'_, Result<bool, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn set_characteristic_value(
        &self,
        _service: Uuid,
        _characteristic: Uuid,
        _value: Vec<u8>,
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn notify(
        &self,
        _service: Uuid,
        _characteristic: Uuid,
        _data: &[u8],
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        Box::pin(ready(unsupported()))
    }

    fn requests(
        &self,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<GattRequest>, BluetoothError>> {
        Box::pin(ready(unsupported()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_central_call_reports_unsupported() {
        let backend = UnsupportedBackend;
        let id = DeviceId::new("any");
        assert_eq!(
            backend.adapter_state().await.unwrap_err().code(),
            "ERR_BLUETOOTH_UNSUPPORTED"
        );
        assert_eq!(
            backend
                .start_scan(ScanFilter::default())
                .await
                .unwrap_err()
                .code(),
            "ERR_BLUETOOTH_UNSUPPORTED"
        );
        assert_eq!(
            backend.connect(&id).await.unwrap_err().code(),
            "ERR_BLUETOOTH_UNSUPPORTED"
        );
        assert_eq!(
            backend.discover_services(&id).await.unwrap_err().code(),
            "ERR_BLUETOOTH_UNSUPPORTED"
        );
        assert_eq!(
            backend.state_changes().await.unwrap_err().code(),
            "ERR_BLUETOOTH_UNSUPPORTED"
        );
        assert_eq!(
            backend.connection_events().await.unwrap_err().code(),
            "ERR_BLUETOOTH_UNSUPPORTED"
        );
    }

    #[tokio::test]
    async fn every_server_call_reports_unsupported() {
        let backend = UnsupportedServer;
        assert_eq!(
            backend
                .add_service(Service {
                    uuid: Uuid::parse("180D").unwrap(),
                    primary: true,
                    characteristics: Vec::new(),
                    included_services: Vec::new(),
                })
                .await
                .unwrap_err()
                .code(),
            "ERR_BLUETOOTH_UNSUPPORTED"
        );
        assert_eq!(
            backend.is_advertising().await.unwrap_err().code(),
            "ERR_BLUETOOTH_UNSUPPORTED"
        );
    }
}
