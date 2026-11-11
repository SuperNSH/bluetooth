//! Every error kind is constructible, carries its exact stable code, and displays its message.

use nsh_bluetooth::{BluetoothError, ErrorKind};

#[test]
fn every_error_code_reachable() {
    let table: [(ErrorKind, &str); 35] = [
        (ErrorKind::Unsupported, "ERR_BLUETOOTH_UNSUPPORTED"),
        (ErrorKind::Unauthorized, "ERR_BLUETOOTH_UNAUTHORIZED"),
        (ErrorKind::PoweredOff, "ERR_BLUETOOTH_POWERED_OFF"),
        (ErrorKind::UnknownState, "ERR_BLUETOOTH_UNKNOWN_STATE"),
        (ErrorKind::Resetting, "ERR_BLUETOOTH_RESETTING"),
        (
            ErrorKind::StateChangeFailed,
            "ERR_BLUETOOTH_STATE_CHANGE_FAILED",
        ),
        (
            ErrorKind::LocationServicesDisabled,
            "ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED",
        ),
        (
            ErrorKind::InvalidIdentifiers,
            "ERR_BLUETOOTH_INVALID_IDENTIFIERS",
        ),
        (ErrorKind::InProgress, "ERR_BLUETOOTH_IN_PROGRESS"),
        (
            ErrorKind::ScanStartFailed,
            "ERR_BLUETOOTH_SCAN_START_FAILED",
        ),
        (ErrorKind::ConnectFailed, "ERR_BLUETOOTH_CONNECT_FAILED"),
        (
            ErrorKind::AlreadyConnected,
            "ERR_BLUETOOTH_ALREADY_CONNECTED",
        ),
        (ErrorKind::Disconnected, "ERR_BLUETOOTH_DISCONNECTED"),
        (ErrorKind::DeviceNotFound, "ERR_BLUETOOTH_DEVICE_NOT_FOUND"),
        (
            ErrorKind::DeviceNotConnected,
            "ERR_BLUETOOTH_DEVICE_NOT_CONNECTED",
        ),
        (ErrorKind::Cancelled, "ERR_BLUETOOTH_CANCELLED"),
        (ErrorKind::Timeout, "ERR_BLUETOOTH_TIMEOUT"),
        (
            ErrorKind::MtuChangeFailed,
            "ERR_BLUETOOTH_MTU_CHANGE_FAILED",
        ),
        (
            ErrorKind::ServicesDiscoveryFailed,
            "ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED",
        ),
        (
            ErrorKind::ServicesNotDiscovered,
            "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED",
        ),
        (
            ErrorKind::ServiceNotFound,
            "ERR_BLUETOOTH_SERVICE_NOT_FOUND",
        ),
        (
            ErrorKind::CharacteristicWriteFailed,
            "ERR_BLUETOOTH_CHARACTERISTIC_WRITE_FAILED",
        ),
        (
            ErrorKind::CharacteristicReadFailed,
            "ERR_BLUETOOTH_CHARACTERISTIC_READ_FAILED",
        ),
        (
            ErrorKind::CharacteristicNotifyChangeFailed,
            "ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED",
        ),
        (
            ErrorKind::CharacteristicNotFound,
            "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND",
        ),
        (
            ErrorKind::CharacteristicsNotDiscovered,
            "ERR_BLUETOOTH_CHARACTERISTICS_NOT_DISCOVERED",
        ),
        (
            ErrorKind::CharacteristicInvalidDataFormat,
            "ERR_BLUETOOTH_CHARACTERISTIC_INVALID_DATA_FORMAT",
        ),
        (
            ErrorKind::DescriptorWriteFailed,
            "ERR_BLUETOOTH_DESCRIPTOR_WRITE_FAILED",
        ),
        (
            ErrorKind::DescriptorReadFailed,
            "ERR_BLUETOOTH_DESCRIPTOR_READ_FAILED",
        ),
        (
            ErrorKind::DescriptorNotFound,
            "ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND",
        ),
        (
            ErrorKind::DescriptorsNotDiscovered,
            "ERR_BLUETOOTH_DESCRIPTORS_NOT_DISCOVERED",
        ),
        (
            ErrorKind::DescriptorWriteNotAllowed,
            "ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED",
        ),
        (ErrorKind::AdvertiseFailed, "ERR_BLUETOOTH_ADVERTISE_FAILED"),
        (ErrorKind::ServerFailed, "ERR_BLUETOOTH_SERVER_FAILED"),
        (ErrorKind::Backend, "ERR_BLUETOOTH_BACKEND"),
    ];

    for (kind, code) in table {
        let error = BluetoothError::new(kind);
        assert_eq!(kind.code(), code, "{code}");
        assert_eq!(error.code(), code, "{code}");
        assert!(!kind.message().is_empty(), "{code}");
        assert_eq!(error.to_string(), kind.message(), "{code}");
        assert_eq!(error.kind(), kind, "{code}");
        assert_eq!(error.status(), None, "{code}");
    }
}
