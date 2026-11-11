//! The seam between the high-level roles and a platform: the shared value types, and the two
//! traits a platform bridge implements.

use std::{fmt, future::Future, pin::Pin};

use tokio::sync::broadcast;

use crate::error::{BluetoothError, ErrorKind};

/// A future a backend returns. Each call's future borrows the backend; `Send` so a caller can
/// spawn a call and cancel it from another task.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The state of the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdapterState {
    /// The state is not known yet.
    Unknown,
    /// No usable Bluetooth stack on this platform.
    Unsupported,
    /// The app lacks permission to use Bluetooth.
    Unauthorized,
    /// The adapter is turned off.
    PoweredOff,
    /// The stack is momentarily restarting.
    Resetting,
    /// The adapter is on and usable.
    On,
}

/// A Bluetooth UUID, stored as the canonical lowercase hyphenated form.
///
/// [`Uuid::parse`] accepts the full form and the `XXXX` (16-bit) and `XXXXXXXX` (32-bit) short
/// forms of the Bluetooth base UUID `00000000-0000-1000-8000-00805f9b34fb`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Uuid(String);

impl Uuid {
    /// Parses a full, 32-bit or 16-bit UUID into its full lowercase form.
    pub fn parse(input: &str) -> Result<Self, BluetoothError> {
        let invalid = || BluetoothError::new(ErrorKind::InvalidIdentifiers);
        let lowered = input.to_ascii_lowercase();
        let expanded = match lowered.len() {
            4 => format!("0000{lowered}-0000-1000-8000-00805f9b34fb"),
            8 => format!("{lowered}-0000-1000-8000-00805f9b34fb"),
            _ => lowered,
        };
        let bytes = expanded.as_bytes();
        if bytes.len() != 36 {
            return Err(invalid());
        }
        for (index, byte) in bytes.iter().enumerate() {
            if matches!(index, 8 | 13 | 18 | 23) {
                if *byte != b'-' {
                    return Err(invalid());
                }
            } else if !byte.is_ascii_hexdigit() {
                return Err(invalid());
            }
        }
        Ok(Self(expanded))
    }

    /// The canonical lowercase hyphenated form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A platform's identifier for a device, opaque to the app.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(String);

impl DeviceId {
    /// Wraps an id from a scan or `system_devices`.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The platform's id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A known device: its id and, when the platform has one, its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The device's platform id.
    pub id: DeviceId,
    /// The device's name, when known.
    pub name: Option<String>,
}

/// The GATT properties of a characteristic, as a bit set over the spec's bits. The first eight
/// bits are the spec's; the last two follow the common convention above the byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CharacteristicProperties(u16);

impl CharacteristicProperties {
    /// The value may be broadcast in an advertisement.
    pub const BROADCAST: Self = Self(0x01);
    /// The value may be read.
    pub const READ: Self = Self(0x02);
    /// The value may be written without a response.
    pub const WRITE_NO_RESPONSE: Self = Self(0x04);
    /// The value may be written with a response.
    pub const WRITE: Self = Self(0x08);
    /// The value may be notified.
    pub const NOTIFY: Self = Self(0x10);
    /// The value may be indicated.
    pub const INDICATE: Self = Self(0x20);
    /// Signed writes are allowed.
    pub const AUTHENTICATED_SIGNED_WRITES: Self = Self(0x40);
    /// More properties follow in an extended-properties descriptor.
    pub const EXTENDED_PROPERTIES: Self = Self(0x80);
    /// Reliable writes are allowed.
    pub const RELIABLE_WRITE: Self = Self(0x100);
    /// The value may be written through an auxiliary handle.
    pub const WRITABLE_AUXILIARY: Self = Self(0x200);

    /// Builds the set from a platform's raw bits.
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// The raw bits, for a platform's API.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Whether every bit in `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no property is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for CharacteristicProperties {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for CharacteristicProperties {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// What an advertisement carried, beyond the fields `ScanResult` lifts out.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdvertisementData {
    /// The advertised local name.
    pub local_name: Option<String>,
    /// The manufacturer-specific data.
    pub manufacturer_data: Option<Vec<u8>>,
    /// The advertised service UUIDs.
    pub service_uuids: Vec<Uuid>,
    /// The advertised transmit power.
    pub tx_power: Option<i8>,
}

/// One device seen by a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    /// The device's platform id.
    pub id: DeviceId,
    /// The device's name, when the platform has one.
    pub name: Option<String>,
    /// The latest signal strength.
    pub rssi: Option<i8>,
    /// The advertisement's data.
    pub advertisement: AdvertisementData,
}

/// What a scan looks for. An empty vector matches everything; the two lists combine with and,
/// a match on either list item within a list is enough.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanFilter {
    /// Advertise at least one of these service UUIDs.
    pub service_uuids: Vec<Uuid>,
    /// Be one of these devices.
    pub device_ids: Vec<DeviceId>,
}

impl ScanFilter {
    /// Whether a scan result passes this filter.
    pub fn matches(&self, result: &ScanResult) -> bool {
        let service_match = self.service_uuids.is_empty()
            || self
                .service_uuids
                .iter()
                .any(|uuid| result.advertisement.service_uuids.contains(uuid));
        let device_match = self.device_ids.is_empty() || self.device_ids.contains(&result.id);
        service_match && device_match
    }
}

/// A discovered GATT service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// The service's UUID.
    pub uuid: Uuid,
    /// Whether the service is primary.
    pub primary: bool,
    /// The service's characteristics.
    pub characteristics: Vec<Characteristic>,
    /// UUIDs of services included in this one.
    pub included_services: Vec<Uuid>,
}

/// A discovered characteristic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Characteristic {
    /// The characteristic's UUID.
    pub uuid: Uuid,
    /// The UUID of the service it belongs to.
    pub service_uuid: Uuid,
    /// What the characteristic supports.
    pub properties: CharacteristicProperties,
    /// The characteristic's descriptors.
    pub descriptors: Vec<Descriptor>,
    /// The last value the platform read; may be absent.
    pub value: Option<Vec<u8>>,
}

/// A discovered descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    /// The descriptor's UUID.
    pub uuid: Uuid,
    /// The UUID of the characteristic it belongs to.
    pub characteristic_uuid: Uuid,
    /// The UUID of the service that characteristic belongs to.
    pub service_uuid: Uuid,
}

/// How a write is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteType {
    /// Write with a response: the operation resolves when the device answers.
    WithResponse,
    /// Write without a response: the operation resolves once the value is sent.
    WithoutResponse,
}

/// A new value for a subscribed characteristic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueNotification {
    /// The characteristic the value is for.
    pub characteristic: Characteristic,
    /// The new value.
    pub value: Vec<u8>,
}

/// A device connected or disconnected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    /// The device connected.
    Connected(DeviceId),
    /// The device disconnected.
    Disconnected(DeviceId),
}

/// A request a central sent to the local server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GattRequest {
    /// The service the request targets.
    pub service_uuid: Uuid,
    /// The characteristic the request targets.
    pub characteristic_uuid: Uuid,
    /// Whether the request reads or writes.
    pub kind: GattRequestKind,
}

/// The kind of a [`GattRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GattRequestKind {
    /// The central reads at `offset`.
    Read {
        /// The offset to read from.
        offset: usize,
    },
    /// The central writes `data` at `offset`.
    Write {
        /// The offset to write at.
        offset: usize,
        /// The bytes to write.
        data: Vec<u8>,
        /// Whether the central waits for a response.
        with_response: bool,
    },
}

/// What the server advertises.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdvertiseOptions {
    /// The advertised local name.
    pub local_name: Option<String>,
    /// The advertised service UUIDs.
    pub service_uuids: Vec<Uuid>,
}

/// The central role a platform provides. [`crate::Central`] adds the guards: one scan at a time,
/// connect retries, and the GATT checks.
pub trait CentralBackend: Send + Sync {
    /// The adapter's state now.
    fn adapter_state(&self) -> BackendFuture<'_, Result<AdapterState, BluetoothError>>;

    /// Later adapter states, from the next change on.
    fn state_changes(
        &self,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<AdapterState>, BluetoothError>>;

    /// Starts a scan with `filter` and returns its results. One scan at a time.
    fn start_scan(
        &self,
        filter: ScanFilter,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<ScanResult>, BluetoothError>>;

    /// Stops the running scan. Ok when no scan runs.
    fn stop_scan(&self) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// The devices the system already knows, filtered by `service_uuids` when not empty.
    fn system_devices(
        &self,
        service_uuids: &[Uuid],
    ) -> BackendFuture<'_, Result<Vec<DeviceInfo>, BluetoothError>>;

    /// Connects the device. An `Err` means the connection did not happen.
    fn connect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// Disconnects the device. Ok when the device is already disconnected.
    fn disconnect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// Whether the device is connected.
    fn is_connected(&self, id: &DeviceId) -> BackendFuture<'_, Result<bool, BluetoothError>>;

    /// The device's current signal strength.
    fn read_rssi(&self, id: &DeviceId) -> BackendFuture<'_, Result<i8, BluetoothError>>;

    /// Asks for a larger MTU and returns the negotiated value.
    fn request_mtu(
        &self,
        id: &DeviceId,
        mtu: u16,
    ) -> BackendFuture<'_, Result<u16, BluetoothError>>;

    /// The device's services with their characteristics and descriptors.
    fn discover_services(
        &self,
        id: &DeviceId,
    ) -> BackendFuture<'_, Result<Vec<Service>, BluetoothError>>;

    /// Reads the characteristic's value.
    fn read_characteristic(
        &self,
        id: &DeviceId,
        characteristic: &Characteristic,
    ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>>;

    /// Writes the characteristic's value.
    fn write_characteristic(
        &self,
        id: &DeviceId,
        characteristic: &Characteristic,
        data: &[u8],
        write_type: WriteType,
    ) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// Subscribes to the characteristic's values and returns the stream. Subscribing again keeps
    /// one logical subscription and returns a fresh receiver.
    fn subscribe_characteristic(
        &self,
        id: &DeviceId,
        characteristic: &Characteristic,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<ValueNotification>, BluetoothError>>;

    /// Unsubscribes from the characteristic. Ok when not subscribed.
    fn unsubscribe_characteristic(
        &self,
        id: &DeviceId,
        characteristic: &Characteristic,
    ) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// Reads the descriptor's value.
    fn read_descriptor(
        &self,
        id: &DeviceId,
        descriptor: &Descriptor,
    ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>>;

    /// Writes the descriptor's value.
    fn write_descriptor(
        &self,
        id: &DeviceId,
        descriptor: &Descriptor,
        data: &[u8],
    ) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// Connection events for devices this central uses, from the next one on.
    fn connection_events(
        &self,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<ConnectionEvent>, BluetoothError>>;
}

/// The peripheral (server) role a platform provides. [`crate::Server`] adds the guards: one
/// advertisement at a time.
pub trait ServerBackend: Send + Sync {
    /// Serves `service`. A duplicate service UUID fails with `ServerFailed`.
    fn add_service(&self, service: Service) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// Stops serving the service. Ok when no such service is served.
    fn remove_service(&self, uuid: Uuid) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// Starts advertising. One advertisement at a time.
    fn start_advertising(
        &self,
        options: AdvertiseOptions,
    ) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// Stops advertising. Ok when not advertising.
    fn stop_advertising(&self) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// Whether the server is advertising.
    fn is_advertising(&self) -> BackendFuture<'_, Result<bool, BluetoothError>>;

    /// Sets a served characteristic's value, for the centrals that read it.
    fn set_characteristic_value(
        &self,
        service: Uuid,
        characteristic: Uuid,
        value: Vec<u8>,
    ) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// Pushes a value to the centrals subscribed to the characteristic. No subscriber fails with
    /// `CharacteristicNotifyChangeFailed`.
    fn notify(
        &self,
        service: Uuid,
        characteristic: Uuid,
        data: &[u8],
    ) -> BackendFuture<'_, Result<(), BluetoothError>>;

    /// The requests the centrals send, from the next one on.
    fn requests(
        &self,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<GattRequest>, BluetoothError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_parses_the_full_form_and_normalizes_case() {
        let uuid = Uuid::parse("0000180D-0000-1000-8000-00805F9B34FB").unwrap();
        assert_eq!(uuid.as_str(), "0000180d-0000-1000-8000-00805f9b34fb");
        assert_eq!(uuid.to_string(), "0000180d-0000-1000-8000-00805f9b34fb");
    }

    #[test]
    fn uuid_expands_the_short_forms() {
        let short = Uuid::parse("180D").unwrap();
        assert_eq!(short.as_str(), "0000180d-0000-1000-8000-00805f9b34fb");
        let wide = Uuid::parse("DEADBEEF").unwrap();
        assert_eq!(wide.as_str(), "deadbeef-0000-1000-8000-00805f9b34fb");
        assert_eq!(Uuid::parse("180d").unwrap(), Uuid::parse("180D").unwrap());
    }

    #[test]
    fn uuid_rejects_malformed_input() {
        for input in [
            "",
            "180",
            "180FF",
            "xyz",
            "0000180d00001000800000805f9b34fb",
            "0000180d-0000-1000-8000-00805f9b34fbX",
            "0000180d_0000_1000_8000_00805f9b34fb",
            "-0000180d-0000-1000-8000-00805f9b34fb",
        ] {
            let error = Uuid::parse(input).unwrap_err();
            assert_eq!(error.code(), "ERR_BLUETOOTH_INVALID_IDENTIFIERS", "{input}");
        }
    }

    #[test]
    fn properties_are_a_bit_set() {
        let none = CharacteristicProperties::default();
        assert!(none.is_empty());
        let read_notify = CharacteristicProperties::from_bits(0x12);
        assert!(read_notify.contains(CharacteristicProperties::READ));
        assert!(read_notify.contains(CharacteristicProperties::NOTIFY));
        assert!(!read_notify.contains(CharacteristicProperties::WRITE));
        assert!(!read_notify.is_empty());
        assert_eq!(read_notify.bits(), 0x12);
        let reliable = CharacteristicProperties::RELIABLE_WRITE;
        assert!(reliable.contains(CharacteristicProperties::RELIABLE_WRITE));
        assert!(CharacteristicProperties::from_bits(0).is_empty());
        let union = CharacteristicProperties::READ | CharacteristicProperties::NOTIFY;
        assert!(union.contains(CharacteristicProperties::READ));
        assert!(union.contains(CharacteristicProperties::NOTIFY));
        let mut combined = union;
        combined |= CharacteristicProperties::INDICATE;
        assert!(combined.contains(CharacteristicProperties::INDICATE));
        assert_eq!(combined.bits(), 0x32);
    }

    fn scan_result(id: &str, services: &[&str]) -> ScanResult {
        ScanResult {
            id: DeviceId::new(id),
            name: Some(id.to_owned()),
            rssi: Some(-42),
            advertisement: AdvertisementData {
                service_uuids: services.iter().map(|s| Uuid::parse(s).unwrap()).collect(),
                ..AdvertisementData::default()
            },
        }
    }

    #[test]
    fn an_empty_filter_matches_everything() {
        let filter = ScanFilter::default();
        assert!(filter.matches(&scan_result("a", &[])));
        assert!(filter.matches(&scan_result("b", &["180D"])));
    }

    #[test]
    fn a_filter_matches_on_any_service_or_device_but_needs_both_lists() {
        let filter = ScanFilter {
            service_uuids: vec![Uuid::parse("180D").unwrap()],
            device_ids: vec![DeviceId::new("watch")],
        };
        assert!(filter.matches(&scan_result("watch", &["180D"])));
        assert!(!filter.matches(&scan_result("watch", &["181A"])));
        assert!(!filter.matches(&scan_result("phone", &["180D"])));
        let service_only = ScanFilter {
            service_uuids: vec![Uuid::parse("180D").unwrap()],
            device_ids: Vec::new(),
        };
        assert!(service_only.matches(&scan_result("phone", &["180D"])));
        let device_only = ScanFilter {
            service_uuids: Vec::new(),
            device_ids: vec![DeviceId::new("watch")],
        };
        assert!(device_only.matches(&scan_result("watch", &[])));
    }
}
