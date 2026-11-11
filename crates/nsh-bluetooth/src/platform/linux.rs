//! Linux bridge: BlueZ over D-Bus through bluer. The pure mapping helpers compile and test on
//! every host; the native implementation is Linux-only and backs both selectors below.

use std::sync::Arc;

use crate::backend::{AdapterState, CentralBackend, CharacteristicProperties, ServerBackend};
use crate::error::ErrorKind;

/// Maps a BlueZ error name (with or without the `org.bluez.Error.` prefix, and in the
/// `bluer::ErrorKind` variant-name form) to its specific kind, or `None` when the caller's
/// context-specific fallback fits better.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // test-only off-target; the backend uses it on Linux
fn bluez_error_to_kind(name: &str) -> Option<ErrorKind> {
    let name = name.strip_prefix("org.bluez.Error.").unwrap_or(name);
    match name {
        "AlreadyConnected" => Some(ErrorKind::AlreadyConnected),
        "AlreadyExists" => Some(ErrorKind::ServerFailed),
        "AuthenticationCanceled"
        | "AuthenticationFailed"
        | "AuthenticationRejected"
        | "AuthenticationTimeout"
        | "NotAuthorized" => Some(ErrorKind::Unauthorized),
        "ConnectionAttemptFailed" => Some(ErrorKind::ConnectFailed),
        "DoesNotExist" | "NotFound" => Some(ErrorKind::DeviceNotFound),
        "InProgress" => Some(ErrorKind::InProgress),
        "InvalidArguments" | "InvalidLength" | "InvalidOffset" => {
            Some(ErrorKind::InvalidIdentifiers)
        }
        "NotAvailable" | "NotSupported" => Some(ErrorKind::Unsupported),
        "NotReady" => Some(ErrorKind::PoweredOff),
        "ServicesUnresolved" => Some(ErrorKind::ServicesDiscoveryFailed),
        "NotificationSessionStopped" => Some(ErrorKind::CharacteristicNotifyChangeFailed),
        "IndicationUnconfirmed" => Some(ErrorKind::Timeout),
        _ => None,
    }
}

/// BlueZ models adapter state as one powered bit; off means `PoweredOff`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // test-only off-target; the backend uses it on Linux
fn bluez_powered_to_adapter_state(powered: bool) -> AdapterState {
    if powered {
        AdapterState::On
    } else {
        AdapterState::PoweredOff
    }
}

/// Builds the property bit set from BlueZ's characteristic flag names, in the same spelling the
/// D-Bus `Flags` property uses. Unknown names are ignored.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // test-only off-target; the backend uses it on Linux
fn bluez_flags_to_properties(flags: &[(&str, bool)]) -> CharacteristicProperties {
    let mut bits = 0u16;
    for (name, set) in flags {
        if !set {
            continue;
        }
        bits |= match *name {
            "broadcast" => 0x01,
            "read" => 0x02,
            "write-without-response" => 0x04,
            "write" => 0x08,
            "notify" => 0x10,
            "indicate" => 0x20,
            "authenticated-signed-writes" => 0x40,
            "extended-properties" => 0x80,
            "reliable-write" => 0x100,
            "writable-auxiliaries" => 0x200,
            _ => 0,
        };
    }
    CharacteristicProperties::from_bits(bits)
}

/// Flattens manufacturer data entries into one blob: entries sorted by manufacturer id, each as
/// the id in little-endian order followed by its payload.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // test-only off-target; the backend uses it on Linux
fn manufacturer_data_to_bytes(entries: &[(u16, Vec<u8>)]) -> Option<Vec<u8>> {
    if entries.is_empty() {
        return None;
    }
    let mut sorted: Vec<&(u16, Vec<u8>)> = entries.iter().collect();
    sorted.sort_by_key(|(id, _)| *id);
    let mut bytes = Vec::new();
    for (id, data) in sorted {
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(data);
    }
    Some(bytes)
}

/// BlueZ reports signal strengths as `i16`; our values carry them as `i8`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // test-only off-target; the backend uses it on Linux
fn clamp_dbm(value: i16) -> i8 {
    value.clamp(-128, 127) as i8
}

/// The platform's central backend, if this platform has one and it is implemented.
pub fn central_backend() -> Option<Arc<dyn CentralBackend>> {
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
    #[cfg(target_os = "linux")]
    {
        Some(Arc::new(native::LinuxCentral::new()))
    }
}

/// The platform's server (peripheral role) backend, if this platform has one and it is implemented.
pub fn server_backend() -> Option<Arc<dyn ServerBackend>> {
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
    #[cfg(target_os = "linux")]
    {
        Some(Arc::new(native::LinuxServer::new()))
    }
}

#[cfg(target_os = "linux")]
mod native {
    use std::{
        collections::{BTreeSet, HashMap, HashSet},
        future::{Future, ready},
        sync::{Arc, Mutex, PoisonError},
        time::Duration,
    };

    use bluer::{
        Adapter, AdapterEvent, AdapterProperty, Address, DeviceEvent, DeviceProperty,
        DiscoveryFilter, DiscoveryTransport, Error as BluezError, ErrorKind as BluezErrorKind,
        InternalErrorKind, Session,
        adv::{Advertisement, AdvertisementHandle, Type as AdvertisementType},
        gatt::{
            WriteOp,
            local::{
                Application, ApplicationHandle, Characteristic as LocalCharacteristic,
                CharacteristicNotifier, CharacteristicNotify, CharacteristicNotifyMethod,
                CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod, ReqError,
                Service as LocalService,
            },
            remote::{
                Characteristic as RemoteCharacteristic, CharacteristicWriteRequest,
                Descriptor as RemoteDescriptor,
            },
        },
    };
    use futures::StreamExt;
    use tokio::{
        sync::{Mutex as AsyncMutex, OnceCell, broadcast, mpsc},
        task::JoinHandle,
        time::{sleep, timeout},
    };

    use crate::backend::{
        AdapterState, AdvertiseOptions, AdvertisementData, BackendFuture, CentralBackend,
        Characteristic, CharacteristicProperties, ConnectionEvent, Descriptor, DeviceId,
        DeviceInfo, GattRequest, GattRequestKind, ScanFilter, ScanResult, ServerBackend, Service,
        Uuid, ValueNotification, WriteType,
    };
    use crate::error::{BluetoothError, ErrorKind};

    /// bluer races D-Bus replies against an internal 120 s timeout; a shorter deadline here keeps
    /// every backend call resolving well before that.
    const OP_TIMEOUT: Duration = Duration::from_secs(30);
    const NOTIFY_POLL_INTERVAL: Duration = Duration::from_millis(500);
    /// The crate-wide stream capacity; a lagged receiver resynchronises instead of failing.
    const CHANNEL_CAPACITY: usize = 32;

    /// Served characteristic values, shared between the GATT callbacks and the app-facing calls.
    type ValueMap = Arc<Mutex<HashMap<(Uuid, Uuid), Vec<u8>>>>;
    /// One notification slot per served characteristic; `None` means no central subscribes.
    type NotifierEntry = Arc<AsyncMutex<Option<CharacteristicNotifier>>>;

    fn error_name(error: &BluezError) -> String {
        match &error.kind {
            // Raw D-Bus names keep their prefix; parsed kinds carry the BlueZ name as the variant.
            BluezErrorKind::Internal(InternalErrorKind::DBus(name)) => name.clone(),
            kind => format!("{kind:?}"),
        }
    }

    /// Converts a bluer failure into a coded error: a specific kind when the name maps, otherwise
    /// the operation's `fallback`.
    fn bluez<T>(result: Result<T, BluezError>, fallback: ErrorKind) -> Result<T, BluetoothError> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => match bluez_error_to_kind(&error_name(&error)) {
                Some(kind) => Err(BluetoothError::new(kind)),
                None => Err(BluetoothError::new(fallback)),
            },
        }
    }

    /// Puts one deadline under every operation so a stalled daemon resolves as `Timeout`.
    async fn raced<T, F>(future: F) -> Result<T, BluetoothError>
    where
        F: Future<Output = Result<T, BluetoothError>>,
    {
        match timeout(OP_TIMEOUT, future).await {
            Ok(outcome) => outcome,
            Err(_) => Err(BluetoothError::new(ErrorKind::Timeout)),
        }
    }

    fn to_bluer_uuid(uuid: &Uuid) -> Result<bluer::Uuid, BluetoothError> {
        bluer::Uuid::parse_str(uuid.as_str())
            .map_err(|_| BluetoothError::new(ErrorKind::InvalidIdentifiers))
    }

    fn to_our_uuid(uuid: &bluer::Uuid) -> Result<Uuid, BluetoothError> {
        Uuid::parse(&uuid.to_string())
            .map_err(|_| BluetoothError::new(ErrorKind::InvalidIdentifiers))
    }

    /// Resolves the default adapter once and caches it: no session or adapter means there is no
    /// usable stack on this machine, which the crate reports as `Unsupported`.
    async fn default_adapter(cell: &OnceCell<Adapter>) -> Result<Adapter, BluetoothError> {
        cell.get_or_try_init(|| async {
            let session = Session::new()
                .await
                .map_err(|_| BluetoothError::new(ErrorKind::Unsupported))?;
            match session.default_adapter().await {
                Ok(adapter) => Ok(adapter),
                Err(error) => match error.kind {
                    BluezErrorKind::NotFound => Err(BluetoothError::new(ErrorKind::Unsupported)),
                    _ => Err(BluetoothError::new(ErrorKind::Backend)),
                },
            }
        })
        .await
        .map(Adapter::clone)
    }

    /// Spawns `task` unless a live one already pumps the same channel.
    fn ensure_task(slot: &Mutex<Option<JoinHandle<()>>>, task: impl FnOnce() -> JoinHandle<()>) {
        let mut guard = slot.lock().unwrap_or_else(PoisonError::into_inner);
        let running = match guard.as_ref() {
            Some(handle) => !handle.is_finished(),
            None => false,
        };
        if !running {
            *guard = Some(task());
        }
    }

    #[derive(Default)]
    struct DiscoveredServices {
        services: Vec<Service>,
        characteristics: HashMap<Uuid, RemoteCharacteristic>,
        descriptors: HashMap<(Uuid, Uuid), RemoteDescriptor>,
    }

    /// Walks a connected device's resolved GATT tree into the crate's value types and a handle
    /// cache for later operations.
    async fn walk_services(
        device: &bluer::Device,
        fallback: ErrorKind,
    ) -> Result<DiscoveredServices, BluetoothError> {
        let mut out = DiscoveredServices::default();
        for remote_service in bluez(device.services().await, fallback)? {
            let service_uuid = to_our_uuid(&bluez(remote_service.uuid().await, fallback)?)?;
            let primary = bluez(remote_service.primary().await, fallback)?;
            let mut included_services = Vec::new();
            for id in bluez(remote_service.includes().await, fallback)? {
                if let Ok(included) = device.service(id) {
                    if let Ok(uuid) = included.uuid().await {
                        if let Ok(ours) = to_our_uuid(&uuid) {
                            included_services.push(ours);
                        }
                    }
                }
            }
            let mut characteristics = Vec::new();
            for remote_char in bluez(remote_service.characteristics().await, fallback)? {
                let char_uuid = to_our_uuid(&bluez(remote_char.uuid().await, fallback)?)?;
                let flags = bluez(remote_char.flags().await, fallback)?;
                let properties = bluez_flags_to_properties(&[
                    ("broadcast", flags.broadcast),
                    ("read", flags.read),
                    ("write-without-response", flags.write_without_response),
                    ("write", flags.write),
                    ("notify", flags.notify),
                    ("indicate", flags.indicate),
                    (
                        "authenticated-signed-writes",
                        flags.authenticated_signed_writes,
                    ),
                    ("extended-properties", flags.extended_properties),
                    ("reliable-write", flags.reliable_write),
                    ("writable-auxiliaries", flags.writable_auxiliaries),
                ]);
                let mut descriptors = Vec::new();
                for remote_desc in bluez(remote_char.descriptors().await, fallback)? {
                    let descriptor = Descriptor {
                        uuid: to_our_uuid(&bluez(remote_desc.uuid().await, fallback)?)?,
                        characteristic_uuid: char_uuid.clone(),
                        service_uuid: service_uuid.clone(),
                    };
                    out.descriptors.insert(
                        (char_uuid.clone(), descriptor.uuid.clone()),
                        remote_desc.clone(),
                    );
                    descriptors.push(descriptor);
                }
                let value = bluez(remote_char.cached_value().await, fallback).ok();
                out.characteristics
                    .insert(char_uuid.clone(), remote_char.clone());
                characteristics.push(Characteristic {
                    uuid: char_uuid,
                    service_uuid: service_uuid.clone(),
                    properties,
                    descriptors,
                    value,
                });
            }
            out.services.push(Service {
                uuid: service_uuid,
                primary,
                characteristics,
                included_services,
            });
        }
        Ok(out)
    }

    /// Reads one device's advertisement-facing properties into a scan result; `None` for a device
    /// BlueZ knows but no longer sees (it reports no RSSI then).
    async fn scan_snapshot(device: &bluer::Device) -> Result<Option<ScanResult>, BluezError> {
        let Some(rssi) = device.rssi().await? else {
            return Ok(None);
        };
        let name = device.name().await?;
        let alias = device.alias().await.ok().filter(|alias| !alias.is_empty());
        let tx_power = device.tx_power().await?;
        let uuids = device.uuids().await?.unwrap_or_default();
        let manufacturer: Vec<(u16, Vec<u8>)> = device
            .manufacturer_data()
            .await?
            .unwrap_or_default()
            .into_iter()
            .collect();
        let advertisement = AdvertisementData {
            local_name: name.clone().or(alias),
            manufacturer_data: manufacturer_data_to_bytes(&manufacturer),
            service_uuids: uuids
                .iter()
                .filter_map(|uuid| Uuid::parse(&uuid.to_string()).ok())
                .collect(),
            tx_power: tx_power.map(clamp_dbm),
        };
        Ok(Some(ScanResult {
            id: DeviceId::new(device.address().to_string()),
            name,
            rssi: Some(clamp_dbm(rssi)),
            advertisement,
        }))
    }

    /// Forwards one device's connected-state changes into `events`.
    fn watch_device(
        adapter: &Adapter,
        address: Address,
        events: &mpsc::UnboundedSender<ConnectionEvent>,
    ) -> Option<JoinHandle<()>> {
        let device = adapter.device(address).ok()?;
        let events = events.clone();
        Some(tokio::spawn(async move {
            let Ok(changes) = device.events().await else {
                return;
            };
            // bluer does not promise Unpin streams; pinning keeps StreamExt usable.
            let mut changes = Box::pin(changes);
            while let Some(change) = changes.next().await {
                if let DeviceEvent::PropertyChanged(DeviceProperty::Connected(connected)) = change {
                    let id = DeviceId::new(address.to_string());
                    let event = if connected {
                        ConnectionEvent::Connected(id)
                    } else {
                        ConnectionEvent::Disconnected(id)
                    };
                    let _ = events.send(event);
                }
            }
        }))
    }

    struct ScanRun {
        task: JoinHandle<()>,
    }

    struct Subscription {
        values: broadcast::Sender<ValueNotification>,
        task: JoinHandle<()>,
    }

    #[derive(Default)]
    struct DeviceGatt {
        characteristics: HashMap<Uuid, RemoteCharacteristic>,
        descriptors: HashMap<(Uuid, Uuid), RemoteDescriptor>,
    }

    pub(super) struct LinuxCentral {
        adapter: OnceCell<Adapter>,
        state_tx: broadcast::Sender<AdapterState>,
        state_task: Mutex<Option<JoinHandle<()>>>,
        events_tx: broadcast::Sender<ConnectionEvent>,
        events_task: Mutex<Option<JoinHandle<()>>>,
        scan: Mutex<Option<ScanRun>>,
        subscriptions: Mutex<HashMap<(String, Uuid), Subscription>>,
        gatt: Mutex<HashMap<String, DeviceGatt>>,
    }

    impl LinuxCentral {
        pub(super) fn new() -> Self {
            let (state_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
            let (events_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
            Self {
                adapter: OnceCell::new(),
                state_tx,
                state_task: Mutex::new(None),
                events_tx,
                events_task: Mutex::new(None),
                scan: Mutex::new(None),
                subscriptions: Mutex::new(HashMap::new()),
                gatt: Mutex::new(HashMap::new()),
            }
        }

        fn lock_map<T>(map: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
            map.lock().unwrap_or_else(PoisonError::into_inner)
        }

        async fn adapter(&self) -> Result<Adapter, BluetoothError> {
            default_adapter(&self.adapter).await
        }

        async fn device_for(&self, id: &DeviceId) -> Result<bluer::Device, BluetoothError> {
            let address: Address = id
                .as_str()
                .parse()
                .map_err(|_| BluetoothError::new(ErrorKind::InvalidIdentifiers))?;
            let adapter = self.adapter().await?;
            adapter
                .device(address)
                .map_err(|_| BluetoothError::new(ErrorKind::InvalidIdentifiers))
        }

        /// Finds the remote handle for a discovered characteristic, walking the device's tree when
        /// the cache holds nothing yet.
        async fn remote_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> Result<RemoteCharacteristic, BluetoothError> {
            if let Some(found) = Self::lock_map(&self.gatt)
                .get(id.as_str())
                .and_then(|gatt| gatt.characteristics.get(&characteristic.uuid))
            {
                return Ok(found.clone());
            }
            let device = self.device_for(id).await?;
            let discovered = walk_services(&device, ErrorKind::Backend).await?;
            let found = discovered
                .characteristics
                .get(&characteristic.uuid)
                .cloned()
                .ok_or_else(|| BluetoothError::new(ErrorKind::CharacteristicNotFound))?;
            Self::lock_map(&self.gatt).insert(
                id.as_str().to_owned(),
                DeviceGatt {
                    characteristics: discovered.characteristics,
                    descriptors: discovered.descriptors,
                },
            );
            Ok(found)
        }

        async fn remote_descriptor(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
        ) -> Result<RemoteDescriptor, BluetoothError> {
            if let Some(found) = Self::lock_map(&self.gatt)
                .get(id.as_str())
                .and_then(|gatt| {
                    gatt.descriptors.get(&(
                        descriptor.characteristic_uuid.clone(),
                        descriptor.uuid.clone(),
                    ))
                })
            {
                return Ok(found.clone());
            }
            let device = self.device_for(id).await?;
            let discovered = walk_services(&device, ErrorKind::Backend).await?;
            let found = discovered
                .descriptors
                .get(&(
                    descriptor.characteristic_uuid.clone(),
                    descriptor.uuid.clone(),
                ))
                .cloned()
                .ok_or_else(|| BluetoothError::new(ErrorKind::DescriptorNotFound))?;
            Self::lock_map(&self.gatt).insert(
                id.as_str().to_owned(),
                DeviceGatt {
                    characteristics: discovered.characteristics,
                    descriptors: discovered.descriptors,
                },
            );
            Ok(found)
        }
    }

    impl CentralBackend for LinuxCentral {
        fn adapter_state(&self) -> BackendFuture<'_, Result<AdapterState, BluetoothError>> {
            Box::pin(raced(async move {
                let adapter = self.adapter().await?;
                let powered = bluez(adapter.is_powered().await, ErrorKind::Backend)?;
                Ok(bluez_powered_to_adapter_state(powered))
            }))
        }

        fn state_changes(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<AdapterState>, BluetoothError>> {
            Box::pin(raced(async move {
                let adapter = self.adapter().await?;
                ensure_task(&self.state_task, || {
                    let adapter = adapter.clone();
                    let state_tx = self.state_tx.clone();
                    tokio::spawn(async move {
                        let Ok(events) = adapter.events().await else {
                            return;
                        };
                        // bluer does not promise Unpin streams; pinning keeps StreamExt usable.
                        let mut events = Box::pin(events);
                        while let Some(event) = events.next().await {
                            if let AdapterEvent::PropertyChanged(AdapterProperty::Powered(
                                powered,
                            )) = event
                            {
                                let _ = state_tx.send(bluez_powered_to_adapter_state(powered));
                            }
                        }
                    })
                });
                Ok(self.state_tx.subscribe())
            }))
        }

        fn start_scan(
            &self,
            filter: ScanFilter,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ScanResult>, BluetoothError>> {
            Box::pin(raced(async move {
                if Self::lock_map(&self.scan).is_some() {
                    return Err(BluetoothError::new(ErrorKind::InProgress));
                }
                let adapter = self.adapter().await?;
                let mut filter_uuids = HashSet::new();
                for uuid in &filter.service_uuids {
                    filter_uuids.insert(to_bluer_uuid(uuid)?);
                }
                let discovery_filter = DiscoveryFilter {
                    uuids: filter_uuids,
                    transport: DiscoveryTransport::Le,
                    duplicate_data: true,
                    ..DiscoveryFilter::default()
                };
                bluez(
                    adapter.set_discovery_filter(discovery_filter).await,
                    ErrorKind::ScanStartFailed,
                )?;
                let discovery = bluez(
                    adapter.discover_devices_with_changes().await,
                    ErrorKind::ScanStartFailed,
                )?;
                let (results, receiver) = broadcast::channel(CHANNEL_CAPACITY);
                let scan_adapter = adapter.clone();
                let task = tokio::spawn(async move {
                    // bluer does not promise Unpin streams; pinning keeps StreamExt usable.
                    let mut discovery = Box::pin(discovery);
                    while let Some(event) = discovery.next().await {
                        if results.receiver_count() == 0 {
                            break;
                        }
                        let AdapterEvent::DeviceAdded(address) = event else {
                            continue;
                        };
                        let Ok(device) = scan_adapter.device(address) else {
                            continue;
                        };
                        // One deadline per snapshot so a stuck reply cannot stall the scan.
                        let snapshot = match timeout(OP_TIMEOUT, scan_snapshot(&device)).await {
                            Ok(Ok(Some(result))) => result,
                            _ => continue,
                        };
                        if filter.matches(&snapshot) {
                            let _ = results.send(snapshot);
                        }
                    }
                });
                *Self::lock_map(&self.scan) = Some(ScanRun { task });
                Ok(receiver)
            }))
        }

        fn stop_scan(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
            // Aborting the task drops bluer's discovery stream, which stops discovery for us.
            if let Some(run) = Self::lock_map(&self.scan).take() {
                run.task.abort();
            }
            Box::pin(ready(Ok(())))
        }

        fn system_devices(
            &self,
            service_uuids: &[Uuid],
        ) -> BackendFuture<'_, Result<Vec<DeviceInfo>, BluetoothError>> {
            Box::pin(raced(async move {
                let adapter = self.adapter().await?;
                let mut wanted = HashSet::new();
                for uuid in service_uuids {
                    wanted.insert(to_bluer_uuid(uuid)?);
                }
                let mut devices = Vec::new();
                for address in bluez(adapter.device_addresses().await, ErrorKind::Backend)? {
                    let Ok(device) = adapter.device(address) else {
                        continue;
                    };
                    let name = bluez(device.name().await, ErrorKind::Backend)?;
                    if !wanted.is_empty() {
                        let advertised =
                            bluez(device.uuids().await, ErrorKind::Backend)?.unwrap_or_default();
                        if !advertised.iter().any(|uuid| wanted.contains(uuid)) {
                            continue;
                        }
                    }
                    devices.push(DeviceInfo {
                        id: DeviceId::new(address.to_string()),
                        name,
                    });
                }
                Ok(devices)
            }))
        }

        fn connect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(raced(async move {
                let device = self.device_for(id).await?;
                bluez(device.connect().await, ErrorKind::ConnectFailed).map(|_| ())
            }))
        }

        fn disconnect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(raced(async move {
                let device = self.device_for(id).await?;
                match bluez(device.is_connected().await, ErrorKind::Backend) {
                    Ok(false) => Ok(()),
                    Ok(true) => bluez(device.disconnect().await, ErrorKind::Backend).map(|_| ()),
                    Err(error) if error.kind() == ErrorKind::DeviceNotFound => Ok(()),
                    Err(error) => Err(error),
                }
            }))
        }

        fn is_connected(&self, id: &DeviceId) -> BackendFuture<'_, Result<bool, BluetoothError>> {
            Box::pin(raced(async move {
                let device = self.device_for(id).await?;
                bluez(device.is_connected().await, ErrorKind::Backend)
            }))
        }

        fn read_rssi(&self, id: &DeviceId) -> BackendFuture<'_, Result<i8, BluetoothError>> {
            Box::pin(raced(async move {
                let device = self.device_for(id).await?;
                match bluez(device.rssi().await, ErrorKind::Backend)? {
                    Some(rssi) => Ok(clamp_dbm(rssi)),
                    // No RSSI means BlueZ knows the device but no longer sees it.
                    None => Err(BluetoothError::new(ErrorKind::Disconnected)),
                }
            }))
        }

        fn request_mtu(
            &self,
            id: &DeviceId,
            _mtu: u16,
        ) -> BackendFuture<'_, Result<u16, BluetoothError>> {
            Box::pin(raced(async move {
                // BlueZ exchanges the ATT MTU during connect and exposes no request call; the
                // asked-for value is advisory and the effective one is read from a characteristic.
                let device = self.device_for(id).await?;
                let services = device
                    .services()
                    .await
                    .map_err(|_| BluetoothError::new(ErrorKind::MtuChangeFailed))?;
                for service in services {
                    let Ok(characteristics) = service.characteristics().await else {
                        continue;
                    };
                    for characteristic in characteristics {
                        if let Ok(mtu) = characteristic.mtu().await {
                            return Ok(mtu.clamp(0, u16::MAX as usize) as u16);
                        }
                    }
                }
                Err(BluetoothError::new(ErrorKind::MtuChangeFailed))
            }))
        }

        fn discover_services(
            &self,
            id: &DeviceId,
        ) -> BackendFuture<'_, Result<Vec<Service>, BluetoothError>> {
            Box::pin(raced(async move {
                let device = self.device_for(id).await?;
                let discovered = walk_services(&device, ErrorKind::ServicesDiscoveryFailed).await?;
                Self::lock_map(&self.gatt).insert(
                    id.as_str().to_owned(),
                    DeviceGatt {
                        characteristics: discovered.characteristics,
                        descriptors: discovered.descriptors,
                    },
                );
                Ok(discovered.services)
            }))
        }

        fn read_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            Box::pin(raced(async move {
                let remote = self.remote_characteristic(id, characteristic).await?;
                bluez(remote.read().await, ErrorKind::CharacteristicReadFailed)
            }))
        }

        fn write_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
            data: &[u8],
            write_type: WriteType,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(raced(async move {
                let remote = self.remote_characteristic(id, characteristic).await?;
                let op_type = match write_type {
                    WriteType::WithResponse => WriteOp::Request,
                    WriteType::WithoutResponse => WriteOp::Command,
                };
                let request = CharacteristicWriteRequest {
                    op_type,
                    ..CharacteristicWriteRequest::default()
                };
                bluez(
                    remote.write_ext(data, &request).await,
                    ErrorKind::CharacteristicWriteFailed,
                )
                .map(|_| ())
            }))
        }

        fn subscribe_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ValueNotification>, BluetoothError>>
        {
            Box::pin(raced(async move {
                let key = (id.as_str().to_owned(), characteristic.uuid.clone());
                if let Some(subscription) = Self::lock_map(&self.subscriptions).get(&key) {
                    if !subscription.task.is_finished() {
                        return Ok(subscription.values.subscribe());
                    }
                }
                let remote = self.remote_characteristic(id, characteristic).await?;
                let changes = bluez(
                    remote.notify().await,
                    ErrorKind::CharacteristicNotifyChangeFailed,
                )?;
                let (values, receiver) = broadcast::channel(CHANNEL_CAPACITY);
                let notified = characteristic.clone();
                let task = tokio::spawn(async move {
                    // bluer does not promise Unpin streams; pinning keeps StreamExt usable.
                    let mut changes = Box::pin(changes);
                    while let Some(value) = changes.next().await {
                        if values.receiver_count() == 0 {
                            break;
                        }
                        let _ = values.send(ValueNotification {
                            characteristic: notified.clone(),
                            value,
                        });
                    }
                });
                Self::lock_map(&self.subscriptions).insert(key, Subscription { values, task });
                Ok(receiver)
            }))
        }

        fn unsubscribe_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            // Aborting the task drops bluer's notify stream, which sends StopNotify for us.
            let key = (id.as_str().to_owned(), characteristic.uuid.clone());
            if let Some(subscription) = Self::lock_map(&self.subscriptions).remove(&key) {
                subscription.task.abort();
            }
            Box::pin(ready(Ok(())))
        }

        fn read_descriptor(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            Box::pin(raced(async move {
                let remote = self.remote_descriptor(id, descriptor).await?;
                bluez(remote.read().await, ErrorKind::DescriptorReadFailed)
            }))
        }

        fn write_descriptor(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
            data: &[u8],
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(raced(async move {
                let remote = self.remote_descriptor(id, descriptor).await?;
                bluez(remote.write(data).await, ErrorKind::DescriptorWriteFailed).map(|_| ())
            }))
        }

        fn connection_events(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ConnectionEvent>, BluetoothError>>
        {
            Box::pin(raced(async move {
                let adapter = self.adapter().await?;
                ensure_task(&self.events_task, || {
                    let events_tx = self.events_tx.clone();
                    tokio::spawn(async move {
                        let Ok(events) = adapter.events().await else {
                            return;
                        };
                        // bluer does not promise Unpin streams; pinning keeps StreamExt usable.
                        let mut adapter_events = Box::pin(events);
                        let (device_tx, mut device_rx) = mpsc::unbounded_channel();
                        let mut watchers: HashMap<Address, JoinHandle<()>> = HashMap::new();
                        for address in adapter.device_addresses().await.unwrap_or_default() {
                            if let Some(handle) = watch_device(&adapter, address, &device_tx) {
                                watchers.insert(address, handle);
                            }
                        }
                        loop {
                            tokio::select! {
                                event = adapter_events.next() => {
                                    match event {
                                        Some(AdapterEvent::DeviceAdded(address)) => {
                                            if !watchers.contains_key(&address) {
                                                if let Some(handle) =
                                                    watch_device(&adapter, address, &device_tx)
                                                {
                                                    watchers.insert(address, handle);
                                                }
                                            }
                                        }
                                        Some(AdapterEvent::DeviceRemoved(address)) => {
                                            if let Some(handle) = watchers.remove(&address) {
                                                handle.abort();
                                            }
                                        }
                                        Some(_) => {}
                                        None => break,
                                    }
                                }
                                event = device_rx.recv() => {
                                    match event {
                                        Some(event) => {
                                            let _ = events_tx.send(event);
                                        }
                                        None => break,
                                    }
                                }
                            }
                        }
                        for handle in watchers.into_values() {
                            handle.abort();
                        }
                    })
                });
                Ok(self.events_tx.subscribe())
            }))
        }
    }

    /// Builds the bluer-side definition of one served characteristic: reads and writes come from
    /// and land in `values`, each request surfaces on `requests`, and notifications reach the
    /// subscribers through `notifier`.
    fn local_characteristic(
        service_uuid: Uuid,
        characteristic: &Characteristic,
        requests: broadcast::Sender<GattRequest>,
        values: ValueMap,
        notifier: Option<NotifierEntry>,
    ) -> Result<LocalCharacteristic, BluetoothError> {
        let properties = characteristic.properties;
        let characteristic_uuid = characteristic.uuid.clone();

        let read = properties
            .contains(CharacteristicProperties::READ)
            .then(|| {
                let (requests, values) = (requests.clone(), Arc::clone(&values));
                let (service_uuid, characteristic_uuid) =
                    (service_uuid.clone(), characteristic_uuid.clone());
                CharacteristicRead {
                    read: true,
                    fun: Box::new(move |request| {
                        Box::pin(async move {
                            let _ = requests.send(GattRequest {
                                service_uuid: service_uuid.clone(),
                                characteristic_uuid: characteristic_uuid.clone(),
                                kind: GattRequestKind::Read {
                                    offset: request.offset as usize,
                                },
                            });
                            let value = values
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .get(&(service_uuid.clone(), characteristic_uuid.clone()))
                                .cloned()
                                .unwrap_or_default();
                            if request.offset as usize > value.len() {
                                Err(ReqError::InvalidOffset)
                            } else {
                                Ok(value[request.offset as usize..].to_vec())
                            }
                        })
                    }),
                    ..CharacteristicRead::default()
                }
            });

        let write = (properties.contains(CharacteristicProperties::WRITE)
            || properties.contains(CharacteristicProperties::WRITE_NO_RESPONSE)
            || properties.contains(CharacteristicProperties::AUTHENTICATED_SIGNED_WRITES))
        .then(|| {
            let (requests, values) = (requests.clone(), Arc::clone(&values));
            let (service_uuid, characteristic_uuid) =
                (service_uuid.clone(), characteristic_uuid.clone());
            CharacteristicWrite {
                write: properties.contains(CharacteristicProperties::WRITE),
                write_without_response: properties
                    .contains(CharacteristicProperties::WRITE_NO_RESPONSE),
                reliable_write: properties.contains(CharacteristicProperties::RELIABLE_WRITE),
                authenticated_signed_writes: properties
                    .contains(CharacteristicProperties::AUTHENTICATED_SIGNED_WRITES),
                method: CharacteristicWriteMethod::Fun(Box::new(
                    move |data, request: bluer::gatt::local::CharacteristicWriteRequest| {
                        Box::pin(async move {
                            let _ = requests.send(GattRequest {
                                service_uuid: service_uuid.clone(),
                                characteristic_uuid: characteristic_uuid.clone(),
                                kind: GattRequestKind::Write {
                                    offset: request.offset as usize,
                                    data: data.clone(),
                                    with_response: matches!(
                                        request.op_type,
                                        WriteOp::Request | WriteOp::Reliable
                                    ),
                                },
                            });
                            let mut map = values.lock().unwrap_or_else(PoisonError::into_inner);
                            let value = map
                                .entry((service_uuid.clone(), characteristic_uuid.clone()))
                                .or_default();
                            let offset = request.offset as usize;
                            if offset > value.len() {
                                return Err(ReqError::InvalidOffset);
                            }
                            if offset + data.len() > value.len() {
                                value.resize(offset + data.len(), 0);
                            }
                            value[offset..offset + data.len()].copy_from_slice(&data);
                            Ok(())
                        })
                    },
                )),
                ..CharacteristicWrite::default()
            }
        });

        let notify = notifier
            .filter(|_| {
                properties.contains(CharacteristicProperties::NOTIFY)
                    || properties.contains(CharacteristicProperties::INDICATE)
            })
            .map(|entry| {
                CharacteristicNotify {
                    notify: properties.contains(CharacteristicProperties::NOTIFY),
                    indicate: properties.contains(CharacteristicProperties::INDICATE)
                        && !properties.contains(CharacteristicProperties::NOTIFY),
                    method: CharacteristicNotifyMethod::Fun(Box::new(move |session| {
                        Box::pin(async move {
                            *entry.lock().await = Some(session);
                            // bluer replies to StartNotify only after this future finishes, so the
                            // watch loop runs detached; it polls because the notifier must stay in
                            // the slot for the app's notify calls.
                            tokio::spawn(async move {
                                loop {
                                    sleep(NOTIFY_POLL_INTERVAL).await;
                                    let mut slot = entry.lock().await;
                                    let stopped = match slot.as_ref() {
                                        Some(notifier) => notifier.is_stopped(),
                                        None => true,
                                    };
                                    if stopped {
                                        *slot = None;
                                        break;
                                    }
                                }
                            });
                        })
                    })),
                    ..CharacteristicNotify::default()
                }
            });

        Ok(LocalCharacteristic {
            uuid: to_bluer_uuid(&characteristic_uuid)?,
            read,
            write,
            notify,
            ..LocalCharacteristic::default()
        })
    }

    struct ServedService {
        // Dropping the application handle makes bluer unregister the service asynchronously.
        application: ApplicationHandle,
        characteristics: Vec<Uuid>,
    }

    pub(super) struct LinuxServer {
        adapter: OnceCell<Adapter>,
        served: Mutex<HashMap<Uuid, ServedService>>,
        values: ValueMap,
        notifiers: Mutex<HashMap<(Uuid, Uuid), NotifierEntry>>,
        requests: broadcast::Sender<GattRequest>,
        advertising: Mutex<Option<AdvertisementHandle>>,
    }

    impl LinuxServer {
        pub(super) fn new() -> Self {
            let (requests, _) = broadcast::channel(CHANNEL_CAPACITY);
            Self {
                adapter: OnceCell::new(),
                served: Mutex::new(HashMap::new()),
                values: Arc::new(Mutex::new(HashMap::new())),
                notifiers: Mutex::new(HashMap::new()),
                requests,
                advertising: Mutex::new(None),
            }
        }

        fn lock_map<T>(map: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
            map.lock().unwrap_or_else(PoisonError::into_inner)
        }

        async fn adapter(&self) -> Result<Adapter, BluetoothError> {
            default_adapter(&self.adapter).await
        }
    }

    impl ServerBackend for LinuxServer {
        fn add_service(&self, service: Service) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(raced(async move {
                if Self::lock_map(&self.served).contains_key(&service.uuid) {
                    return Err(BluetoothError::new(ErrorKind::ServerFailed));
                }
                let adapter = self.adapter().await?;
                let mut characteristics = Vec::new();
                let mut local_characteristics = Vec::new();
                let mut notifier_entries = Vec::new();
                let mut initial_values = Vec::new();
                for characteristic in &service.characteristics {
                    initial_values.push((
                        (service.uuid.clone(), characteristic.uuid.clone()),
                        characteristic.value.clone().unwrap_or_default(),
                    ));
                    let notifier = if characteristic
                        .properties
                        .contains(CharacteristicProperties::NOTIFY)
                        || characteristic
                            .properties
                            .contains(CharacteristicProperties::INDICATE)
                    {
                        let entry: NotifierEntry = Arc::new(AsyncMutex::new(None));
                        notifier_entries.push((
                            (service.uuid.clone(), characteristic.uuid.clone()),
                            Arc::clone(&entry),
                        ));
                        Some(entry)
                    } else {
                        None
                    };
                    local_characteristics.push(local_characteristic(
                        service.uuid.clone(),
                        characteristic,
                        self.requests.clone(),
                        Arc::clone(&self.values),
                        notifier,
                    )?);
                    characteristics.push(characteristic.uuid.clone());
                }
                let local_service = LocalService {
                    uuid: to_bluer_uuid(&service.uuid)?,
                    primary: service.primary,
                    characteristics: local_characteristics,
                    ..LocalService::default()
                };
                let application = Application {
                    services: vec![local_service],
                    ..Application::default()
                };
                let handle = bluez(
                    adapter.serve_gatt_application(application).await,
                    ErrorKind::ServerFailed,
                )?;
                // Registration succeeded; publish the bookkeeping the callbacks rely on.
                Self::lock_map(&self.served).insert(
                    service.uuid.clone(),
                    ServedService {
                        application: handle,
                        characteristics,
                    },
                );
                let mut values = Self::lock_map(&self.values);
                for (key, value) in initial_values {
                    values.insert(key, value);
                }
                drop(values);
                let mut notifiers = Self::lock_map(&self.notifiers);
                for (key, entry) in notifier_entries {
                    notifiers.insert(key, entry);
                }
                Ok(())
            }))
        }

        fn remove_service(&self, uuid: Uuid) -> BackendFuture<'_, Result<(), BluetoothError>> {
            if let Some(served) = Self::lock_map(&self.served).remove(&uuid) {
                {
                    let mut values = Self::lock_map(&self.values);
                    for characteristic in &served.characteristics {
                        values.remove(&(uuid.clone(), characteristic.clone()));
                    }
                }
                let mut notifiers = Self::lock_map(&self.notifiers);
                for characteristic in &served.characteristics {
                    notifiers.remove(&(uuid.clone(), characteristic.clone()));
                }
                drop(notifiers);
                // Dropping the handle unregisters the application asynchronously.
                drop(served.application);
            }
            Box::pin(ready(Ok(())))
        }

        fn start_advertising(
            &self,
            options: AdvertiseOptions,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(raced(async move {
                if Self::lock_map(&self.advertising).is_some() {
                    return Err(BluetoothError::new(ErrorKind::InProgress));
                }
                let adapter = self.adapter().await?;
                let mut service_uuids = BTreeSet::new();
                for uuid in &options.service_uuids {
                    service_uuids.insert(to_bluer_uuid(uuid)?);
                }
                let advertisement = Advertisement {
                    advertisement_type: AdvertisementType::Peripheral,
                    service_uuids,
                    local_name: options.local_name.clone(),
                    ..Advertisement::default()
                };
                let handle = bluez(
                    adapter.advertise(advertisement).await,
                    ErrorKind::AdvertiseFailed,
                )?;
                *Self::lock_map(&self.advertising) = Some(handle);
                Ok(())
            }))
        }

        fn stop_advertising(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
            // Dropping the handle makes bluer unregister the advertisement asynchronously.
            drop(Self::lock_map(&self.advertising).take());
            Box::pin(ready(Ok(())))
        }

        fn is_advertising(&self) -> BackendFuture<'_, Result<bool, BluetoothError>> {
            Box::pin(ready(Ok(Self::lock_map(&self.advertising).is_some())))
        }

        fn set_characteristic_value(
            &self,
            service: Uuid,
            characteristic: Uuid,
            value: Vec<u8>,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(raced(async move {
                let served = Self::lock_map(&self.served);
                match served.get(&service) {
                    None => return Err(BluetoothError::new(ErrorKind::ServiceNotFound)),
                    Some(served) => {
                        if !served.characteristics.contains(&characteristic) {
                            return Err(BluetoothError::new(ErrorKind::CharacteristicNotFound));
                        }
                    }
                }
                drop(served);
                Self::lock_map(&self.values).insert((service, characteristic), value);
                Ok(())
            }))
        }

        fn notify(
            &self,
            service: Uuid,
            characteristic: Uuid,
            data: &[u8],
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(raced(async move {
                let served = Self::lock_map(&self.served);
                match served.get(&service) {
                    None => return Err(BluetoothError::new(ErrorKind::ServiceNotFound)),
                    Some(served) => {
                        if !served.characteristics.contains(&characteristic) {
                            return Err(BluetoothError::new(ErrorKind::CharacteristicNotFound));
                        }
                    }
                }
                drop(served);
                let entry = Self::lock_map(&self.notifiers)
                    .get(&(service.clone(), characteristic.clone()))
                    .map(Arc::clone);
                let Some(entry) = entry else {
                    return Err(BluetoothError::new(
                        ErrorKind::CharacteristicNotifyChangeFailed,
                    ));
                };
                let mut slot = entry.lock().await;
                let Some(notifier) = slot.as_mut() else {
                    return Err(BluetoothError::new(
                        ErrorKind::CharacteristicNotifyChangeFailed,
                    ));
                };
                bluez(
                    notifier.notify(data.to_vec()).await,
                    ErrorKind::CharacteristicNotifyChangeFailed,
                )
                .map(|_| ())
            }))
        }

        fn requests(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<GattRequest>, BluetoothError>> {
            Box::pin(ready(Ok(self.requests.subscribe())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powered_maps_to_adapter_state() {
        assert_eq!(bluez_powered_to_adapter_state(true), AdapterState::On);
        assert_eq!(
            bluez_powered_to_adapter_state(false),
            AdapterState::PoweredOff
        );
    }

    #[test]
    fn bluez_error_names_map_to_specific_kinds() {
        assert_eq!(
            bluez_error_to_kind("AlreadyConnected"),
            Some(ErrorKind::AlreadyConnected)
        );
        assert_eq!(
            bluez_error_to_kind("ConnectionAttemptFailed"),
            Some(ErrorKind::ConnectFailed)
        );
        assert_eq!(bluez_error_to_kind("NotReady"), Some(ErrorKind::PoweredOff));
        assert_eq!(
            bluez_error_to_kind("DoesNotExist"),
            Some(ErrorKind::DeviceNotFound)
        );
        assert_eq!(
            bluez_error_to_kind("NotFound"),
            Some(ErrorKind::DeviceNotFound)
        );
        assert_eq!(
            bluez_error_to_kind("InProgress"),
            Some(ErrorKind::InProgress)
        );
        assert_eq!(
            bluez_error_to_kind("AlreadyExists"),
            Some(ErrorKind::ServerFailed)
        );
        assert_eq!(
            bluez_error_to_kind("org.bluez.Error.NotAuthorized"),
            Some(ErrorKind::Unauthorized)
        );
        assert_eq!(
            bluez_error_to_kind("org.bluez.Error.AuthenticationTimeout"),
            Some(ErrorKind::Unauthorized)
        );
        assert_eq!(
            bluez_error_to_kind("NotAvailable"),
            Some(ErrorKind::Unsupported)
        );
        assert_eq!(
            bluez_error_to_kind("InvalidArguments"),
            Some(ErrorKind::InvalidIdentifiers)
        );
        assert_eq!(
            bluez_error_to_kind("ServicesUnresolved"),
            Some(ErrorKind::ServicesDiscoveryFailed)
        );
        assert_eq!(
            bluez_error_to_kind("NotificationSessionStopped"),
            Some(ErrorKind::CharacteristicNotifyChangeFailed)
        );
        assert_eq!(
            bluez_error_to_kind("IndicationUnconfirmed"),
            Some(ErrorKind::Timeout)
        );
    }

    #[test]
    fn unknown_bluez_errors_have_no_specific_kind() {
        assert_eq!(bluez_error_to_kind("Failed"), None);
        assert_eq!(bluez_error_to_kind("NotPermitted"), None);
        assert_eq!(bluez_error_to_kind(""), None);
        assert_eq!(
            bluez_error_to_kind("org.freedesktop.DBus.Error.ServiceUnknown"),
            None
        );
    }

    #[test]
    fn bluez_flag_names_map_to_property_bits() {
        let properties = bluez_flags_to_properties(&[
            ("read", true),
            ("notify", true),
            ("write", false),
            ("nonsense", true),
        ]);
        assert!(properties.contains(CharacteristicProperties::READ));
        assert!(properties.contains(CharacteristicProperties::NOTIFY));
        assert!(!properties.contains(CharacteristicProperties::WRITE));
        assert_eq!(properties.bits(), 0x12);
        assert!(bluez_flags_to_properties(&[]).is_empty());
        let writer = bluez_flags_to_properties(&[
            ("write", true),
            ("write-without-response", true),
            ("reliable-write", true),
            ("indicate", true),
            ("authenticated-signed-writes", true),
            ("extended-properties", true),
            ("writable-auxiliaries", true),
            ("broadcast", true),
        ]);
        assert_eq!(
            writer.bits(),
            0x08 | 0x04 | 0x100 | 0x20 | 0x40 | 0x80 | 0x200 | 0x01
        );
    }

    #[test]
    fn manufacturer_data_flattens_sorted_entries() {
        assert_eq!(manufacturer_data_to_bytes(&[]), None);
        assert_eq!(
            manufacturer_data_to_bytes(&[(0x004C, vec![1, 2])]),
            Some(vec![0x4C, 0x00, 1, 2])
        );
        // Entries sort by manufacturer id so the blob is deterministic.
        let flat = manufacturer_data_to_bytes(&[(2, vec![9]), (1, vec![7, 8])]).unwrap();
        assert_eq!(flat, vec![1, 0, 7, 8, 2, 0, 9]);
    }

    #[test]
    fn dbm_values_clamp_into_i8() {
        assert_eq!(clamp_dbm(-42), -42);
        assert_eq!(clamp_dbm(-200), -128);
        assert_eq!(clamp_dbm(300), 127);
    }
}
