//! The errors a Bluetooth call reports. Each has a stable code; a message says what to do.

use std::fmt;

/// What went wrong, as a stable code; see [`BluetoothError::code`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// No usable Bluetooth stack on this platform, or no bridge for it.
    Unsupported,
    /// The app lacks Bluetooth permission; iOS reports unauthorized, Android 12+ denies the scan
    /// or connect permission.
    Unauthorized,
    /// The adapter is turned off.
    PoweredOff,
    /// The adapter's state is not known yet.
    UnknownState,
    /// The stack is momentarily restarting; iOS reports resetting.
    Resetting,
    /// Turning the adapter on or off failed.
    StateChangeFailed,
    /// Android 11 and older need location services on to scan.
    LocationServicesDisabled,
    /// A UUID or device id is malformed.
    InvalidIdentifiers,
    /// One scan or one advertisement runs at a time.
    InProgress,
    /// The scan did not start.
    ScanStartFailed,
    /// The connection failed; Android's status 133 lands here with `status` set.
    ConnectFailed,
    /// The device is already connected.
    AlreadyConnected,
    /// The connection dropped during the operation.
    Disconnected,
    /// No known device has this id.
    DeviceNotFound,
    /// The device is not connected.
    DeviceNotConnected,
    /// The operation was cancelled through the API.
    Cancelled,
    /// `ConnectOptions::timeout` or a platform deadline passed.
    Timeout,
    /// The MTU change was refused.
    MtuChangeFailed,
    /// Discovering the device's services failed.
    ServicesDiscoveryFailed,
    /// A GATT operation ran before `discover_services`.
    ServicesNotDiscovered,
    /// No discovered service has this UUID.
    ServiceNotFound,
    /// Writing the characteristic failed.
    CharacteristicWriteFailed,
    /// Reading the characteristic failed.
    CharacteristicReadFailed,
    /// Subscribing or unsubscribing failed, or a notify had no subscriber.
    CharacteristicNotifyChangeFailed,
    /// No discovered characteristic has this UUID.
    CharacteristicNotFound,
    /// The service's characteristics are not discovered yet.
    CharacteristicsNotDiscovered,
    /// The characteristic's value does not fit the expected format.
    CharacteristicInvalidDataFormat,
    /// Writing the descriptor failed.
    DescriptorWriteFailed,
    /// Reading the descriptor failed.
    DescriptorReadFailed,
    /// No discovered descriptor has this UUID.
    DescriptorNotFound,
    /// The characteristic's descriptors are not discovered yet.
    DescriptorsNotDiscovered,
    /// The descriptor is read-only.
    DescriptorWriteNotAllowed,
    /// Advertising did not start.
    AdvertiseFailed,
    /// The GATT server failed; a duplicate `add_service` lands here.
    ServerFailed,
    /// The platform service failed with no better code.
    Backend,
}

/// A failed Bluetooth call.
///
/// `Display` is the message. Apps match on the stable code from [`BluetoothError::code`]; `status`
/// holds the platform's own code where one exists, such as Android's 133.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", .kind.message())]
pub struct BluetoothError {
    kind: ErrorKind,
    status: Option<i64>,
}

impl BluetoothError {
    /// An error of `kind`, with no platform status.
    pub fn new(kind: ErrorKind) -> Self {
        Self { kind, status: None }
    }

    /// The same error, carrying the platform's own status code.
    #[must_use]
    pub fn with_status(mut self, status: i64) -> Self {
        self.status = Some(status);
        self
    }

    /// What went wrong.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The platform's status code, when the platform gave one.
    pub fn status(&self) -> Option<i64> {
        self.status
    }

    /// The stable code, such as `ERR_BLUETOOTH_CANCELLED`. Apps match on this.
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
}

impl ErrorKind {
    /// The stable code of this kind.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::Unsupported => "ERR_BLUETOOTH_UNSUPPORTED",
            ErrorKind::Unauthorized => "ERR_BLUETOOTH_UNAUTHORIZED",
            ErrorKind::PoweredOff => "ERR_BLUETOOTH_POWERED_OFF",
            ErrorKind::UnknownState => "ERR_BLUETOOTH_UNKNOWN_STATE",
            ErrorKind::Resetting => "ERR_BLUETOOTH_RESETTING",
            ErrorKind::StateChangeFailed => "ERR_BLUETOOTH_STATE_CHANGE_FAILED",
            ErrorKind::LocationServicesDisabled => "ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED",
            ErrorKind::InvalidIdentifiers => "ERR_BLUETOOTH_INVALID_IDENTIFIERS",
            ErrorKind::InProgress => "ERR_BLUETOOTH_IN_PROGRESS",
            ErrorKind::ScanStartFailed => "ERR_BLUETOOTH_SCAN_START_FAILED",
            ErrorKind::ConnectFailed => "ERR_BLUETOOTH_CONNECT_FAILED",
            ErrorKind::AlreadyConnected => "ERR_BLUETOOTH_ALREADY_CONNECTED",
            ErrorKind::Disconnected => "ERR_BLUETOOTH_DISCONNECTED",
            ErrorKind::DeviceNotFound => "ERR_BLUETOOTH_DEVICE_NOT_FOUND",
            ErrorKind::DeviceNotConnected => "ERR_BLUETOOTH_DEVICE_NOT_CONNECTED",
            ErrorKind::Cancelled => "ERR_BLUETOOTH_CANCELLED",
            ErrorKind::Timeout => "ERR_BLUETOOTH_TIMEOUT",
            ErrorKind::MtuChangeFailed => "ERR_BLUETOOTH_MTU_CHANGE_FAILED",
            ErrorKind::ServicesDiscoveryFailed => "ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED",
            ErrorKind::ServicesNotDiscovered => "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED",
            ErrorKind::ServiceNotFound => "ERR_BLUETOOTH_SERVICE_NOT_FOUND",
            ErrorKind::CharacteristicWriteFailed => "ERR_BLUETOOTH_CHARACTERISTIC_WRITE_FAILED",
            ErrorKind::CharacteristicReadFailed => "ERR_BLUETOOTH_CHARACTERISTIC_READ_FAILED",
            ErrorKind::CharacteristicNotifyChangeFailed => {
                "ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED"
            }
            ErrorKind::CharacteristicNotFound => "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND",
            ErrorKind::CharacteristicsNotDiscovered => {
                "ERR_BLUETOOTH_CHARACTERISTICS_NOT_DISCOVERED"
            }
            ErrorKind::CharacteristicInvalidDataFormat => {
                "ERR_BLUETOOTH_CHARACTERISTIC_INVALID_DATA_FORMAT"
            }
            ErrorKind::DescriptorWriteFailed => "ERR_BLUETOOTH_DESCRIPTOR_WRITE_FAILED",
            ErrorKind::DescriptorReadFailed => "ERR_BLUETOOTH_DESCRIPTOR_READ_FAILED",
            ErrorKind::DescriptorNotFound => "ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND",
            ErrorKind::DescriptorsNotDiscovered => "ERR_BLUETOOTH_DESCRIPTORS_NOT_DISCOVERED",
            ErrorKind::DescriptorWriteNotAllowed => "ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED",
            ErrorKind::AdvertiseFailed => "ERR_BLUETOOTH_ADVERTISE_FAILED",
            ErrorKind::ServerFailed => "ERR_BLUETOOTH_SERVER_FAILED",
            ErrorKind::Backend => "ERR_BLUETOOTH_BACKEND",
        }
    }

    /// What to do about it, in one sentence.
    pub fn message(self) -> &'static str {
        match self {
            ErrorKind::Unsupported => {
                "Bluetooth is not usable on this platform. Check `is_supported` first."
            }
            ErrorKind::Unauthorized => {
                "Grant the app Bluetooth permission in the system settings and try again."
            }
            ErrorKind::PoweredOff => {
                "Bluetooth is off. Turn it on in the system settings and try again."
            }
            ErrorKind::UnknownState => {
                "The adapter state is not known yet. Wait for `state_changes` to report it."
            }
            ErrorKind::Resetting => {
                "The Bluetooth stack is restarting. Wait for `state_changes` and try again."
            }
            ErrorKind::StateChangeFailed => "Turning Bluetooth on or off failed. Try again.",
            ErrorKind::LocationServicesDisabled => {
                "Turn location services on to scan, or scan on Android 12 or newer."
            }
            ErrorKind::InvalidIdentifiers => {
                "A UUID or device id is malformed. Use one from a scan or `system_devices`."
            }
            ErrorKind::InProgress => {
                "One scan or advertisement runs at a time. Stop it before starting another."
            }
            ErrorKind::ScanStartFailed => "The scan did not start. Try again.",
            ErrorKind::ConnectFailed => {
                "The connection failed. Move closer to the device and try again."
            }
            ErrorKind::AlreadyConnected => {
                "The device is already connected. Use it or disconnect it first."
            }
            ErrorKind::Disconnected => "The connection dropped. Reconnect and try again.",
            ErrorKind::DeviceNotFound => {
                "No device with this id is known. Use one from a scan or `system_devices`."
            }
            ErrorKind::DeviceNotConnected => {
                "The device is not connected. Connect it before using it."
            }
            ErrorKind::Cancelled => "The operation was cancelled.",
            ErrorKind::Timeout => "The operation timed out. Try again.",
            ErrorKind::MtuChangeFailed => "The MTU change was refused. Keep the current MTU.",
            ErrorKind::ServicesDiscoveryFailed => {
                "Discovering services failed. Reconnect and try again."
            }
            ErrorKind::ServicesNotDiscovered => "Call `discover_services` before GATT operations.",
            ErrorKind::ServiceNotFound => {
                "No discovered service has this UUID. Discover services again."
            }
            ErrorKind::CharacteristicWriteFailed => {
                "Writing the characteristic failed. Check its properties and try again."
            }
            ErrorKind::CharacteristicReadFailed => {
                "Reading the characteristic failed. Check its properties and try again."
            }
            ErrorKind::CharacteristicNotifyChangeFailed => {
                "The subscription change failed. Check the characteristic's properties."
            }
            ErrorKind::CharacteristicNotFound => "No discovered characteristic has this UUID.",
            ErrorKind::CharacteristicsNotDiscovered => {
                "The service's characteristics are not discovered yet."
            }
            ErrorKind::CharacteristicInvalidDataFormat => {
                "The characteristic's value does not fit the expected format."
            }
            ErrorKind::DescriptorWriteFailed => {
                "Writing the descriptor failed. Check its permissions and try again."
            }
            ErrorKind::DescriptorReadFailed => {
                "Reading the descriptor failed. Check its permissions and try again."
            }
            ErrorKind::DescriptorNotFound => "No discovered descriptor has this UUID.",
            ErrorKind::DescriptorsNotDiscovered => {
                "The characteristic's descriptors are not discovered yet."
            }
            ErrorKind::DescriptorWriteNotAllowed => "This descriptor cannot be written.",
            ErrorKind::AdvertiseFailed => "Advertising did not start. Wait a moment and try again.",
            ErrorKind::ServerFailed => {
                "The GATT server operation failed. Check the service and try again."
            }
            ErrorKind::Backend => "The platform's Bluetooth service failed. Try again.",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_KINDS: [(ErrorKind, &str); 35] = [
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

    #[test]
    fn every_kind_has_its_code_and_a_message() {
        for (kind, code) in ALL_KINDS {
            assert_eq!(kind.code(), code);
            assert_eq!(kind.to_string(), code);
            assert!(!kind.message().is_empty());
        }
    }

    #[test]
    fn every_code_is_distinct() {
        let codes: Vec<&str> = ALL_KINDS.iter().map(|(_, code)| *code).collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len());
    }

    #[test]
    fn the_error_carries_its_code_message_and_status() {
        let error = BluetoothError::new(ErrorKind::Cancelled);
        assert_eq!(error.code(), "ERR_BLUETOOTH_CANCELLED");
        assert_eq!(error.kind(), ErrorKind::Cancelled);
        assert_eq!(error.status(), None);
        assert_eq!(error.to_string(), ErrorKind::Cancelled.message());
        assert_eq!(error.clone().with_status(133).status(), Some(133));
    }
}
