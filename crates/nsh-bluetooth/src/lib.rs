//! Bluetooth LE for Native Shell apps: a central role to scan, connect and use GATT, and a
//! peripheral role to advertise and serve, over per-platform bridges.
//!
//! Every call resolves exactly once: a value, a [`BluetoothError`] with a stable
//! `ERR_BLUETOOTH_*` code, or a cancel. Streams are broadcast receivers of capacity 32; a lagged
//! receiver resynchronises instead of failing. [`Central::platform_central`] and
//! [`Server::platform_server`] pick the target's bridge, falling back to the shared unsupported
//! backends. Tests drive [`FakeBackend`] and [`FakeServerBackend`].

mod backend;
mod central;
mod error;
mod fake;
mod peripheral;
mod platform;
mod unsupported;

pub use backend::{
    AdapterState, AdvertiseOptions, AdvertisementData, BackendFuture, CentralBackend,
    Characteristic, CharacteristicProperties, ConnectionEvent, Descriptor, DeviceId, DeviceInfo,
    GattRequest, GattRequestKind, ScanFilter, ScanResult, ServerBackend, Service, Uuid,
    ValueNotification, WriteType,
};
pub use central::{Central, ConnectOptions, Device, ScanStream};
pub use error::{BluetoothError, ErrorKind};
pub use fake::{FakeBackend, FakeOp, FakeServerBackend, ServerOp};
pub use peripheral::Server;
pub use unsupported::{UnsupportedBackend, UnsupportedServer};
