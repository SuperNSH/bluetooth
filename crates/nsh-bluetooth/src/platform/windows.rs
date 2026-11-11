//! Windows bridge: WinRT BluetoothLE (central and server roles). Pure mapping and codec helpers
//! compile and test on every host; the native implementation is Windows-only and backs both
//! selectors.

use std::sync::Arc;

use crate::backend::{AdapterState, CentralBackend, DeviceId, ServerBackend, Uuid};
use crate::error::ErrorKind;

/// A `Windows.Devices.Radios.RadioState` value to its adapter state: `1` is on, `2` (off) and `3`
/// (disabled) are powered off, everything else is not known yet.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // test-only off-target; the backend uses it on Windows
fn radio_state_to_adapter_state(state: i32) -> AdapterState {
    match state {
        1 => AdapterState::On,
        2 | 3 => AdapterState::PoweredOff,
        _ => AdapterState::Unknown,
    }
}

/// A `GattCommunicationStatus` value to the error it is; `None` (status 0) is success.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // test-only off-target; the backend uses it on Windows
fn gatt_status_to_kind(status: i32) -> Option<ErrorKind> {
    match status {
        0 => None,
        1 => Some(ErrorKind::Disconnected), // Unreachable: the link dropped mid-call
        3 => Some(ErrorKind::Unauthorized), // AccessDenied
        _ => Some(ErrorKind::Backend),      // ProtocolError and anything newer
    }
}

/// An HRESULT to its kind: the few WinRT carries distinctly, `Backend` for the rest.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // test-only off-target; the backend uses it on Windows
fn winrt_error_to_kind(hresult: i32) -> ErrorKind {
    const E_ACCESSDENIED: i32 = 0x8007_0005_u32 as i32;
    const E_NOTFOUND: i32 = 0x8007_0490_u32 as i32;
    const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
    const E_ABORT: i32 = 0x8000_4004_u32 as i32;
    const ERROR_CANCELLED: i32 = 0x8007_04C7_u32 as i32;
    match hresult {
        E_ACCESSDENIED => ErrorKind::Unauthorized,
        E_NOTFOUND => ErrorKind::DeviceNotFound,
        E_INVALIDARG => ErrorKind::InvalidIdentifiers,
        E_ABORT | ERROR_CANCELLED => ErrorKind::Cancelled,
        _ => ErrorKind::Backend,
    }
}

/// The device id is the Bluetooth address in decimal, as the scan builds it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // test-only off-target; the backend uses it on Windows
fn device_address(id: &DeviceId) -> Option<u64> {
    let address = id.as_str().parse().ok()?;
    // A Bluetooth address is six bytes; zero is WinRT's "no address".
    (0 < address && address <= 0xFFFF_FFFF_FFFF).then_some(address)
}

/// The Bluetooth address back into the id form the scan builds.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // test-only off-target; the backend uses it on Windows
fn address_device_id(address: u64) -> DeviceId {
    DeviceId::new(address.to_string())
}

/// A UUID to its big-endian 128-bit value; GUIDs carry exactly that in `from_u128`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // test-only off-target; the backend uses it on Windows
fn uuid_to_u128(uuid: &Uuid) -> Option<u128> {
    u128::from_str_radix(&uuid.as_str().replace('-', ""), 16).ok()
}

/// A big-endian 128-bit value back into the canonical lowercase UUID form.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // test-only off-target; the backend uses it on Windows
fn u128_to_uuid(value: u128) -> Option<Uuid> {
    let text = format!("{value:032x}");
    let parsed = Uuid::parse(&format!(
        "{}-{}-{}-{}-{}",
        &text[0..8],
        &text[8..12],
        &text[12..16],
        &text[16..20],
        &text[20..32]
    ));
    parsed.ok()
}

/// WinRT reports signal strengths as i16; the shared types carry i8.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // test-only off-target; the backend uses it on Windows
fn i16_to_i8(value: i16) -> i8 {
    value.clamp(i8::MIN as i16, i8::MAX as i16) as i8
}

/// The platform's central backend, if this platform has one and it is implemented.
pub fn central_backend() -> Option<Arc<dyn CentralBackend>> {
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
    #[cfg(target_os = "windows")]
    {
        Some(Arc::new(native::WindowsCentral::new()))
    }
}

/// The platform's server (peripheral role) backend, if this platform has one and it is implemented.
pub fn server_backend() -> Option<Arc<dyn ServerBackend>> {
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
    #[cfg(target_os = "windows")]
    {
        Some(Arc::new(native::WindowsServer::new()))
    }
}

#[cfg(target_os = "windows")]
mod native {
    use std::collections::HashMap;
    use std::future::IntoFuture;
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
    use std::time::Duration;

    use tokio::sync::{broadcast, watch};

    use super::{
        address_device_id, device_address, gatt_status_to_kind, i16_to_i8,
        radio_state_to_adapter_state, u128_to_uuid, uuid_to_u128, winrt_error_to_kind,
    };

    use crate::backend::{
        AdapterState, AdvertiseOptions, AdvertisementData, BackendFuture, CentralBackend,
        Characteristic, CharacteristicProperties, ConnectionEvent, Descriptor, DeviceId,
        DeviceInfo, GattRequest, GattRequestKind, ScanFilter, ScanResult, ServerBackend, Service,
        Uuid, ValueNotification, WriteType,
    };
    use crate::error::{BluetoothError, ErrorKind};

    use windows::Devices::Bluetooth::Advertisement::{
        BluetoothLEAdvertisement, BluetoothLEAdvertisementPublisher,
        BluetoothLEAdvertisementPublisherStatus,
        BluetoothLEAdvertisementPublisherStatusChangedEventArgs,
        BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
        BluetoothLEScanningMode,
    };
    use windows::Devices::Bluetooth::GenericAttributeProfile::{
        GattCharacteristic, GattCharacteristicProperties,
        GattClientCharacteristicConfigurationDescriptorValue, GattCommunicationStatus,
        GattDescriptor, GattDeviceService, GattLocalCharacteristic,
        GattLocalCharacteristicParameters, GattReadRequestedEventArgs, GattServiceProvider,
        GattServiceProviderAdvertisementStatus,
        GattServiceProviderAdvertisementStatusChangedEventArgs,
        GattServiceProviderAdvertisingParameters, GattSession, GattValueChangedEventArgs,
        GattWriteOption, GattWriteRequestedEventArgs,
    };
    use windows::Devices::Bluetooth::{
        BluetoothAdapter, BluetoothCacheMode, BluetoothConnectionStatus, BluetoothLEDevice,
    };
    use windows::Devices::Radios::Radio;
    use windows::Foundation::TypedEventHandler;
    use windows::Storage::Streams::{DataReader, DataWriter, IBuffer};
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoIncrementMTAUsage, CoInitializeEx};
    use windows::core::{GUID, HSTRING, IInspectable};

    /// Every stream this backend hands out matches the shared stream capacity.
    const CHANNEL_CAPACITY: usize = 32;
    /// One GATT operation may take this long before the call reports `Timeout`.
    const GATT_OP_TIMEOUT: Duration = Duration::from_secs(10);
    /// Connecting pairs the link and runs one uncached service probe; it gets longer.
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
    /// Advertising must report itself started within this window or the start failed.
    const START_TIMEOUT: Duration = Duration::from_secs(5);
    /// The PDU size reported before the first `MaxPduSizeChanged`.
    const DEFAULT_MTU: u16 = 27;
    /// The 16-bit Client Characteristic Configuration descriptor; Windows rejects direct writes
    /// to it, and subscribe/unsubscribe own it.
    const CCCD_SUFFIX: &str = "00002902-0000-1000-8000-00805f9b34fb";

    // WinRT needs an apartment. The MTA is registered once for the process and never released:
    // tokio worker threads come and go, so a per-thread init cannot own the lifetime.
    static APARTMENT: OnceLock<()> = OnceLock::new();

    fn init_apartment() {
        let _ = APARTMENT.get_or_init(|| unsafe {
            // S_FALSE (already MTA) and RPC_E_CHANGED_MODE (already STA) are both fine.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            // Keeps every uninitialized thread usable as implicit MTA.
            let _ = CoIncrementMTAUsage();
        });
    }

    /// Races a WinRT operation with a deadline. The clone keeps the operation cancellable once
    /// the timeout drops the future; pass an operation binding, never a bare call.
    macro_rules! race {
        ($op:expr, $limit:expr) => {{
            let operation = $op;
            match tokio::time::timeout($limit, operation.clone().into_future()).await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => Err(winrt_err(error)),
                Err(_) => {
                    let _ = operation.Cancel();
                    Err(BluetoothError::new(ErrorKind::Timeout))
                }
            }
        }};
    }

    /// Waits for a request operation inside a WinRT callback: the bounded poll plus one final
    /// GetResults keep a slow completion from hanging the callback thread.
    macro_rules! request_now {
        ($op:expr) => {{
            let operation = $op;
            for _ in 0..50 {
                match operation.Status() {
                    // Status 0 is `Started`; anything else can hand over its results.
                    Ok(status) if status.0 == 0 => std::thread::sleep(Duration::from_millis(2)),
                    _ => break,
                }
            }
            operation.GetResults().ok()
        }};
    }

    fn winrt_err(error: windows::core::Error) -> BluetoothError {
        let code = error.code().0;
        BluetoothError::new(winrt_error_to_kind(code)).with_status(i64::from(code))
    }

    /// A failed GATT status as a coded error, with the platform status attached.
    fn gatt_err(status: i32, fallback: ErrorKind) -> BluetoothError {
        let kind = gatt_status_to_kind(status).unwrap_or(fallback);
        BluetoothError::new(kind).with_status(i64::from(status))
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn buffer_bytes(buffer: &IBuffer) -> windows::core::Result<Vec<u8>> {
        let reader = DataReader::FromBuffer(buffer)?;
        let length = reader.UnconsumedBufferLength()? as usize;
        let mut bytes = vec![0u8; length];
        reader.ReadBytes(&mut bytes)?;
        Ok(bytes)
    }

    fn bytes_buffer(data: &[u8]) -> windows::core::Result<IBuffer> {
        let writer = DataWriter::new()?;
        writer.WriteBytes(data)?;
        writer.DetachBuffer()
    }

    fn guid_of(uuid: &Uuid) -> Option<GUID> {
        uuid_to_u128(uuid).map(GUID::from_u128)
    }

    fn uuid_of(guid: GUID) -> Option<Uuid> {
        u128_to_uuid(guid.to_u128())
    }

    // ---------------------------------------------------------------------------------------------
    // Central role
    // ---------------------------------------------------------------------------------------------

    struct Shared {
        radio: Option<Radio>,
        radio_token: Option<i64>,
        scan: Option<ActiveScan>,
        devices: HashMap<u64, ConnectedDevice>,
        known: HashMap<u64, KnownDevice>,
        state_tx: broadcast::Sender<AdapterState>,
        connection_tx: broadcast::Sender<ConnectionEvent>,
    }

    struct ActiveScan {
        watcher: BluetoothLEAdvertisementWatcher,
        token: i64,
    }

    impl Drop for ActiveScan {
        fn drop(&mut self) {
            // Stop-then-revoke on every path: stop, lost claim, backend drop.
            let _ = self.watcher.Stop();
            let _ = self.watcher.RemoveReceived(self.token);
        }
    }

    #[derive(Default)]
    struct KnownDevice {
        name: Option<String>,
        service_uuids: Vec<Uuid>,
        rssi: Option<i8>,
    }

    struct ConnectedDevice {
        device: BluetoothLEDevice,
        session: GattSession,
        connection_token: i64,
        pdu_token: i64,
        services: HashMap<Uuid, GattDeviceService>,
        characteristics: HashMap<Uuid, (Uuid, GattCharacteristic)>,
        descriptors: HashMap<(Uuid, Uuid), GattDescriptor>,
        notify: HashMap<Uuid, NotifySub>,
        /// Serialises this device's CCCD writes with their map updates, so a subscribe's
        /// write+insert cannot interleave with an unsubscribe's remove+write.
        notify_gate: Arc<tokio::sync::Mutex<()>>,
        discovered: Vec<Service>,
        mtu: u16,
    }

    struct NotifySub {
        characteristic: GattCharacteristic,
        token: i64,
        tx: broadcast::Sender<ValueNotification>,
    }

    impl Drop for NotifySub {
        fn drop(&mut self) {
            // Unsubscribe, replacement and connection teardown all drop a sub exactly once.
            let _ = self.characteristic.RemoveValueChanged(self.token);
        }
    }

    impl Drop for ConnectedDevice {
        fn drop(&mut self) {
            // Every token is revoked before its owner closes, so no handler runs after the drop.
            self.notify.clear();
            for (_, service) in self.services.drain() {
                let _ = service.Close();
            }
            let _ = self.session.RemoveMaxPduSizeChanged(self.pdu_token);
            let _ = self
                .device
                .RemoveConnectionStatusChanged(self.connection_token);
            let _ = self.session.Close();
            let _ = self.device.Close();
        }
    }

    /// The WinRT central: one radio, one watcher, one live connection object per device.
    pub(super) struct WindowsCentral {
        shared: Arc<Mutex<Shared>>,
    }

    impl WindowsCentral {
        pub(super) fn new() -> Self {
            init_apartment();
            Self {
                shared: Arc::new(Mutex::new(Shared {
                    radio: None,
                    radio_token: None,
                    scan: None,
                    devices: HashMap::new(),
                    known: HashMap::new(),
                    state_tx: broadcast::channel(CHANNEL_CAPACITY).0,
                    connection_tx: broadcast::channel(CHANNEL_CAPACITY).0,
                })),
            }
        }

        /// The GATT object of a characteristic, or the code for what is missing. The guard is
        /// released before the value is used, so no lock crosses an await.
        fn gatt_characteristic(
            &self,
            address: u64,
            uuid: &Uuid,
        ) -> Result<(GattCharacteristic, Uuid), BluetoothError> {
            let shared = lock(&self.shared);
            let entry = shared
                .devices
                .get(&address)
                .ok_or_else(|| BluetoothError::new(ErrorKind::DeviceNotConnected))?;
            let (service_uuid, characteristic) = entry
                .characteristics
                .get(uuid)
                .ok_or_else(|| BluetoothError::new(ErrorKind::CharacteristicNotFound))?;
            Ok((characteristic.clone(), service_uuid.clone()))
        }

        /// The GATT object of a descriptor, with the same locking rules as above.
        fn gatt_descriptor(
            &self,
            address: u64,
            descriptor: &Descriptor,
        ) -> Result<GattDescriptor, BluetoothError> {
            let shared = lock(&self.shared);
            let entry = shared
                .devices
                .get(&address)
                .ok_or_else(|| BluetoothError::new(ErrorKind::DeviceNotConnected))?;
            entry
                .descriptors
                .get(&(
                    descriptor.characteristic_uuid.clone(),
                    descriptor.uuid.clone(),
                ))
                .cloned()
                .ok_or_else(|| BluetoothError::new(ErrorKind::DescriptorNotFound))
        }
    }

    /// The adapter's radio, fetched once; the call also installs the state-change handler once.
    async fn radio(shared: &Arc<Mutex<Shared>>) -> Result<Radio, BluetoothError> {
        if let Some(radio) = &lock(shared).radio {
            return Ok(radio.clone());
        }
        let adapter = race!(
            BluetoothAdapter::GetDefaultAsync().map_err(winrt_err)?,
            GATT_OP_TIMEOUT
        )?;
        let radio = race!(adapter.GetRadioAsync().map_err(winrt_err)?, GATT_OP_TIMEOUT)?;
        let weak = Arc::downgrade(shared);
        let handler = TypedEventHandler::<Radio, IInspectable>::new(move |sender, _args| {
            let Some(shared) = weak.upgrade() else {
                return Ok(());
            };
            let state = match sender.ok().and_then(|radio| radio.State()) {
                Ok(state) => radio_state_to_adapter_state(state.0),
                Err(_) => AdapterState::Unknown,
            };
            let _ = lock(&shared).state_tx.send(state);
            Ok(())
        });
        let token = radio.StateChanged(&handler).map_err(winrt_err)?;
        let mut shared = lock(shared);
        if shared.radio.is_none() {
            shared.radio_token = Some(token);
            shared.radio = Some(radio.clone());
            Ok(radio)
        } else {
            // Another call won the race; drop the duplicate registration.
            let _ = radio.RemoveStateChanged(token);
            Ok(shared.radio.as_ref().cloned().unwrap_or(radio))
        }
    }

    /// One advertisement as a scan result; every field is best-effort, like the platform reports.
    fn scan_result(args: &BluetoothLEAdvertisementReceivedEventArgs) -> ScanResult {
        let address = args.BluetoothAddress().unwrap_or(0);
        let mut local_name = None;
        let mut manufacturer_data = None;
        let mut service_uuids = Vec::new();
        if let Ok(advertisement) = args.Advertisement() {
            if let Ok(name) = advertisement.LocalName()
                && !name.is_empty()
            {
                local_name = Some(name.to_string());
            }
            // The shared type carries one blob, so the first manufacturer section stands for all.
            if let Ok(sections) = advertisement.ManufacturerData()
                && sections.Size().unwrap_or(0) > 0
                && let Ok(section) = sections.GetAt(0)
                && let Ok(data) = section.Data()
            {
                manufacturer_data = buffer_bytes(&data).ok();
            }
            if let Ok(uuids) = advertisement.ServiceUuids() {
                for index in 0..uuids.Size().unwrap_or(0) {
                    if let Some(uuid) = uuids.GetAt(index).ok().and_then(uuid_of) {
                        service_uuids.push(uuid);
                    }
                }
            }
        }
        ScanResult {
            id: address_device_id(address),
            name: local_name.clone(),
            rssi: args.RawSignalStrengthInDBm().ok().map(i16_to_i8),
            advertisement: AdvertisementData {
                local_name,
                manufacturer_data,
                service_uuids,
                tx_power: args
                    .TransmitPowerLevelInDBm()
                    .ok()
                    .and_then(|level| level.Value().ok())
                    .map(i16_to_i8),
            },
        }
    }

    impl CentralBackend for WindowsCentral {
        fn adapter_state(&self) -> BackendFuture<'_, Result<AdapterState, BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move {
                let radio = radio(&shared).await?;
                let state = radio.State().map_err(winrt_err)?;
                Ok(radio_state_to_adapter_state(state.0))
            })
        }

        fn state_changes(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<AdapterState>, BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move {
                radio(&shared).await?; // the radio call installs the handler exactly once
                Ok(lock(&shared).state_tx.subscribe())
            })
        }

        fn start_scan(
            &self,
            filter: ScanFilter,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ScanResult>, BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move {
                if lock(&shared).scan.is_some() {
                    return Err(BluetoothError::new(ErrorKind::InProgress));
                }
                let start_failed = || BluetoothError::new(ErrorKind::ScanStartFailed);
                let watcher = BluetoothLEAdvertisementWatcher::new().map_err(|_| start_failed())?;
                watcher
                    .SetScanningMode(BluetoothLEScanningMode::Active)
                    .map_err(|_| start_failed())?;
                let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
                let weak = Arc::downgrade(&shared);
                let handler_tx = tx.clone();
                let handler_filter = filter.clone();
                let handler = TypedEventHandler::<
                    BluetoothLEAdvertisementWatcher,
                    BluetoothLEAdvertisementReceivedEventArgs,
                >::new(move |_sender, args| {
                    let Some(args) = args.as_ref() else {
                        return Ok(());
                    };
                    let result = scan_result(args);
                    if !handler_filter.matches(&result) {
                        return Ok(());
                    }
                    if let Some(shared) = weak.upgrade() {
                        let address = device_address(&result.id).unwrap_or(0);
                        {
                            let mut shared = lock(&shared);
                            let known = shared.known.entry(address).or_default();
                            if let Some(name) = &result.advertisement.local_name {
                                known.name = Some(name.clone());
                            }
                            for uuid in &result.advertisement.service_uuids {
                                if !known.service_uuids.contains(uuid) {
                                    known.service_uuids.push(uuid.clone());
                                }
                            }
                            known.rssi = result.rssi;
                        }
                        let _ = handler_tx.send(result);
                    }
                    Ok(())
                });
                let token = watcher.Received(&handler).map_err(|_| start_failed())?;
                if let Err(_error) = watcher.Start() {
                    let _ = watcher.RemoveReceived(token);
                    return Err(start_failed());
                }
                let receiver = tx.subscribe();
                // The slot is claimed in one lock hold: a concurrent start that slipped past
                // the early check loses here, and its scan drops to a stopped watcher.
                let scan = ActiveScan { watcher, token };
                let mut shared = lock(&shared);
                if shared.scan.is_some() {
                    drop(scan);
                    return Err(BluetoothError::new(ErrorKind::InProgress));
                }
                shared.scan = Some(scan);
                Ok(receiver)
            })
        }

        fn stop_scan(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move {
                // Taking the scan drops it, which stops the watcher and revokes the token.
                lock(&shared).scan.take();
                Ok(())
            })
        }

        fn system_devices(
            &self,
            service_uuids: &[Uuid],
        ) -> BackendFuture<'_, Result<Vec<DeviceInfo>, BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            // The future outlives the caller's slice; it carries its own copy.
            let service_uuids = service_uuids.to_vec();
            Box::pin(async move {
                let shared = lock(&shared);
                let mut devices = Vec::new();
                for (address, known) in &shared.known {
                    let advertised_match = known
                        .service_uuids
                        .iter()
                        .any(|uuid| service_uuids.contains(uuid));
                    let discovered_match = shared.devices.get(address).is_some_and(|entry| {
                        entry
                            .discovered
                            .iter()
                            .any(|service| service_uuids.contains(&service.uuid))
                    });
                    if service_uuids.is_empty() || advertised_match || discovered_match {
                        devices.push(DeviceInfo {
                            id: address_device_id(*address),
                            name: known.name.clone(),
                        });
                    }
                }
                Ok(devices)
            })
        }

        fn connect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            let id = id.clone();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                if lock(&shared).devices.contains_key(&address) {
                    return Err(BluetoothError::new(ErrorKind::AlreadyConnected));
                }
                let device = race!(
                    BluetoothLEDevice::FromBluetoothAddressAsync(address).map_err(winrt_err)?,
                    CONNECT_TIMEOUT
                )?;
                // The link forms on the first GATT use, so the session and an uncached service
                // probe are the connect proper.
                let device_id = device.BluetoothDeviceId().map_err(winrt_err)?;
                let session = match race!(
                    GattSession::FromDeviceIdAsync(&device_id).map_err(winrt_err)?,
                    CONNECT_TIMEOUT
                ) {
                    Ok(session) => session,
                    Err(error) => {
                        let _ = device.Close();
                        return Err(error);
                    }
                };
                let probe = race!(
                    device.GetGattServicesAsync().map_err(winrt_err)?,
                    CONNECT_TIMEOUT
                )?;
                let status = probe.Status().map_err(winrt_err)?;
                if status != GattCommunicationStatus::Success {
                    let _ = session.Close();
                    let _ = device.Close();
                    return Err(BluetoothError::new(ErrorKind::ConnectFailed)
                        .with_status(i64::from(status.0)));
                }

                let weak = Arc::downgrade(&shared);
                let status_handler = TypedEventHandler::<BluetoothLEDevice, IInspectable>::new(
                    move |sender, _args| {
                        let Some(shared) = weak.upgrade() else {
                            return Ok(());
                        };
                        let connected = sender.ok().is_ok_and(|device| {
                            device
                                .ConnectionStatus()
                                .is_ok_and(|status| status == BluetoothConnectionStatus::Connected)
                        });
                        if !connected {
                            // Dropping the entry revokes its handlers and closes the GATT objects.
                            let mut shared = lock(&shared);
                            if shared.devices.remove(&address).is_some() {
                                let _ = shared.connection_tx.send(ConnectionEvent::Disconnected(
                                    address_device_id(address),
                                ));
                            }
                        }
                        Ok(())
                    },
                );
                let connection_token = match device.ConnectionStatusChanged(&status_handler) {
                    Ok(token) => token,
                    Err(error) => {
                        let _ = session.Close();
                        let _ = device.Close();
                        return Err(winrt_err(error));
                    }
                };

                let weak = Arc::downgrade(&shared);
                let pdu_handler =
                    TypedEventHandler::<GattSession, IInspectable>::new(move |sender, _args| {
                        let Some(shared) = weak.upgrade() else {
                            return Ok(());
                        };
                        if let Ok(size) = sender.ok().and_then(|s| s.MaxPduSize())
                            && let Some(entry) = lock(&shared).devices.get_mut(&address)
                        {
                            entry.mtu = size;
                        }
                        Ok(())
                    });
                let pdu_token = match session.MaxPduSizeChanged(&pdu_handler) {
                    Ok(token) => token,
                    Err(error) => {
                        let _ = device.RemoveConnectionStatusChanged(connection_token);
                        let _ = session.Close();
                        let _ = device.Close();
                        return Err(winrt_err(error));
                    }
                };

                if let Ok(name) = device.Name() {
                    let name = name.to_string();
                    if !name.is_empty() {
                        lock(&shared).known.entry(address).or_default().name = Some(name);
                    }
                }
                lock(&shared).devices.insert(
                    address,
                    ConnectedDevice {
                        device,
                        session,
                        connection_token,
                        pdu_token,
                        services: HashMap::new(),
                        characteristics: HashMap::new(),
                        descriptors: HashMap::new(),
                        notify: HashMap::new(),
                        notify_gate: Arc::new(tokio::sync::Mutex::new(())),
                        discovered: Vec::new(),
                        mtu: DEFAULT_MTU,
                    },
                );
                Ok(())
            })
        }

        fn disconnect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            let id = id.clone();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                if lock(&shared).devices.remove(&address).is_some() {
                    let _ = lock(&shared)
                        .connection_tx
                        .send(ConnectionEvent::Disconnected(address_device_id(address)));
                }
                Ok(())
            })
        }

        fn is_connected(&self, id: &DeviceId) -> BackendFuture<'_, Result<bool, BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            let id = id.clone();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                let device = lock(&shared)
                    .devices
                    .get(&address)
                    .map(|entry| entry.device.clone());
                let Some(device) = device else {
                    return Ok(false);
                };
                match device.ConnectionStatus() {
                    Ok(status) if status == BluetoothConnectionStatus::Connected => Ok(true),
                    Ok(_) => {
                        // The event handler may lag; a stale entry is cleaned up here.
                        let mut shared = lock(&shared);
                        if shared.devices.remove(&address).is_some() {
                            let _ = shared
                                .connection_tx
                                .send(ConnectionEvent::Disconnected(address_device_id(address)));
                        }
                        Ok(false)
                    }
                    Err(error) => Err(winrt_err(error)),
                }
            })
        }

        fn read_rssi(&self, id: &DeviceId) -> BackendFuture<'_, Result<i8, BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            let id = id.clone();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                // WinRT reports RSSI only from advertisements; the last one seen stands in.
                lock(&shared)
                    .known
                    .get(&address)
                    .and_then(|known| known.rssi)
                    .ok_or_else(|| BluetoothError::new(ErrorKind::DeviceNotConnected))
            })
        }

        fn request_mtu(
            &self,
            id: &DeviceId,
            _mtu: u16,
        ) -> BackendFuture<'_, Result<u16, BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            let id = id.clone();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                let session = lock(&shared)
                    .devices
                    .get(&address)
                    .map(|entry| entry.session.clone())
                    .ok_or_else(|| BluetoothError::new(ErrorKind::DeviceNotConnected))?;
                // WinRT negotiates the PDU size itself; the live value is the answer.
                session
                    .MaxPduSize()
                    .map_err(|_| BluetoothError::new(ErrorKind::MtuChangeFailed))
            })
        }

        fn discover_services(
            &self,
            id: &DeviceId,
        ) -> BackendFuture<'_, Result<Vec<Service>, BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            let id = id.clone();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                let device = lock(&shared)
                    .devices
                    .get(&address)
                    .map(|entry| entry.device.clone())
                    .ok_or_else(|| BluetoothError::new(ErrorKind::DeviceNotConnected))?;

                let result = race!(
                    device.GetGattServicesAsync().map_err(winrt_err)?,
                    GATT_OP_TIMEOUT
                )?;
                let status = result.Status().map_err(winrt_err)?;
                if status != GattCommunicationStatus::Success {
                    return Err(gatt_err(status.0, ErrorKind::ServicesDiscoveryFailed));
                }
                // Collection views are not Send; they are read out inside a scope of their own.
                let gatt_service_list = {
                    let services_view = result.Services().map_err(winrt_err)?;
                    let mut list = Vec::new();
                    for index in 0..services_view.Size().map_err(winrt_err)? {
                        list.push(services_view.GetAt(index).map_err(winrt_err)?);
                    }
                    list
                };
                let mut services = Vec::new();
                let mut gatt_services = HashMap::new();
                let mut characteristics_map = HashMap::new();
                let mut descriptors_map = HashMap::new();
                for gatt_service in gatt_service_list {
                    let Some(service_uuid) = gatt_service.Uuid().ok().and_then(uuid_of) else {
                        continue;
                    };
                    let chars_result = race!(
                        gatt_service.GetCharacteristicsAsync().map_err(winrt_err)?,
                        GATT_OP_TIMEOUT
                    )?;
                    let chars_status = chars_result.Status().map_err(winrt_err)?;
                    if chars_status != GattCommunicationStatus::Success {
                        return Err(gatt_err(chars_status.0, ErrorKind::ServicesDiscoveryFailed));
                    }
                    let gatt_characteristic_list = {
                        let chars_view = chars_result.Characteristics().map_err(winrt_err)?;
                        let mut list = Vec::new();
                        for char_index in 0..chars_view.Size().map_err(winrt_err)? {
                            list.push(chars_view.GetAt(char_index).map_err(winrt_err)?);
                        }
                        list
                    };
                    let mut characteristics = Vec::new();
                    for gatt_characteristic in gatt_characteristic_list {
                        let Some(characteristic_uuid) =
                            gatt_characteristic.Uuid().ok().and_then(uuid_of)
                        else {
                            continue;
                        };
                        let properties = gatt_characteristic
                            .CharacteristicProperties()
                            .map_err(winrt_err)?;
                        // Descriptors are best-effort: a characteristic stays usable without them.
                        let mut descriptors = Vec::new();
                        if let Ok(operation) = gatt_characteristic.GetDescriptorsAsync()
                            && let Ok(result) = race!(operation, GATT_OP_TIMEOUT)
                            && result
                                .Status()
                                .is_ok_and(|status| status == GattCommunicationStatus::Success)
                            && let Ok(view) = result.Descriptors()
                        {
                            for desc_index in 0..view.Size().unwrap_or(0) {
                                let Ok(gatt_descriptor) = view.GetAt(desc_index) else {
                                    continue;
                                };
                                let Some(descriptor_uuid) =
                                    gatt_descriptor.Uuid().ok().and_then(uuid_of)
                                else {
                                    continue;
                                };
                                descriptors_map.insert(
                                    (characteristic_uuid.clone(), descriptor_uuid.clone()),
                                    gatt_descriptor,
                                );
                                descriptors.push(Descriptor {
                                    uuid: descriptor_uuid,
                                    characteristic_uuid: characteristic_uuid.clone(),
                                    service_uuid: service_uuid.clone(),
                                });
                            }
                        }
                        characteristics_map.insert(
                            characteristic_uuid.clone(),
                            (service_uuid.clone(), gatt_characteristic.clone()),
                        );
                        characteristics.push(Characteristic {
                            uuid: characteristic_uuid,
                            service_uuid: service_uuid.clone(),
                            properties: CharacteristicProperties::from_bits(properties.0 as u16),
                            descriptors,
                            value: None,
                        });
                    }
                    gatt_services.insert(service_uuid.clone(), gatt_service);
                    services.push(Service {
                        uuid: service_uuid,
                        primary: true,
                        characteristics,
                        included_services: Vec::new(),
                    });
                }

                let mut shared = lock(&shared);
                let Some(entry) = shared.devices.get_mut(&address) else {
                    return Err(BluetoothError::new(ErrorKind::Disconnected));
                };
                entry.services = gatt_services;
                entry.characteristics = characteristics_map;
                entry.descriptors = descriptors_map;
                entry.discovered = services.clone();
                Ok(services)
            })
        }

        fn read_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            let id = id.clone();
            let characteristic = characteristic.clone();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                let (gatt, _) = self.gatt_characteristic(address, &characteristic.uuid)?;
                let result = race!(
                    gatt.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)
                        .map_err(winrt_err)?,
                    GATT_OP_TIMEOUT
                )?;
                let status = result.Status().map_err(winrt_err)?;
                if status != GattCommunicationStatus::Success {
                    return Err(gatt_err(status.0, ErrorKind::CharacteristicReadFailed));
                }
                let value = result.Value().map_err(winrt_err)?;
                buffer_bytes(&value).map_err(winrt_err)
            })
        }

        fn write_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
            data: &[u8],
            write_type: WriteType,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.clone();
            let characteristic = characteristic.clone();
            let data = data.to_vec();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                let (gatt, _) = self.gatt_characteristic(address, &characteristic.uuid)?;
                let option = match write_type {
                    WriteType::WithResponse => GattWriteOption::WriteWithResponse,
                    WriteType::WithoutResponse => GattWriteOption::WriteWithoutResponse,
                };
                // The buffer is scoped: an IBuffer is not Send, so it dies before the await.
                let operation = {
                    let buffer = bytes_buffer(&data).map_err(winrt_err)?;
                    gatt.WriteValueWithOptionAsync(&buffer, option)
                        .map_err(winrt_err)?
                };
                let status = race!(operation, GATT_OP_TIMEOUT)?;
                if status != GattCommunicationStatus::Success {
                    return Err(gatt_err(status.0, ErrorKind::CharacteristicWriteFailed));
                }
                Ok(())
            })
        }

        fn subscribe_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ValueNotification>, BluetoothError>>
        {
            let shared = Arc::clone(&self.shared);
            let id = id.clone();
            let characteristic = characteristic.clone();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                let (gatt, service_uuid) =
                    self.gatt_characteristic(address, &characteristic.uuid)?;
                let notify_failed =
                    || BluetoothError::new(ErrorKind::CharacteristicNotifyChangeFailed);
                let properties = gatt.CharacteristicProperties().map_err(winrt_err)?;
                // Indications win when both are offered, matching the CCCD rules.
                let config = if properties.0 & 0x20 != 0 {
                    GattClientCharacteristicConfigurationDescriptorValue::Indicate
                } else if properties.0 & 0x10 != 0 {
                    GattClientCharacteristicConfigurationDescriptorValue::Notify
                } else {
                    return Err(notify_failed());
                };

                let gate = {
                    let shared = lock(&shared);
                    let Some(entry) = shared.devices.get(&address) else {
                        return Err(BluetoothError::new(ErrorKind::DeviceNotConnected));
                    };
                    if let Some(sub) = entry.notify.get(&characteristic.uuid) {
                        // One logical subscription: a fresh receiver over the live stream.
                        let tx = sub.tx.clone();
                        return Ok(tx.subscribe());
                    }
                    entry.notify_gate.clone()
                };

                let snapshot = Characteristic {
                    uuid: characteristic.uuid.clone(),
                    service_uuid,
                    properties: CharacteristicProperties::from_bits(properties.0 as u16),
                    descriptors: Vec::new(),
                    value: None,
                };
                let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
                let handler_tx = tx.clone();
                // The handler is not Send; it is scoped away before the CCCD await.
                let token = {
                    let handler =
                        TypedEventHandler::<GattCharacteristic, GattValueChangedEventArgs>::new(
                            move |_sender, args| {
                                let Some(args) = args.as_ref() else {
                                    return Ok(());
                                };
                                if let Some(value) = args
                                    .CharacteristicValue()
                                    .ok()
                                    .and_then(|b| buffer_bytes(&b).ok())
                                {
                                    let _ = handler_tx.send(ValueNotification {
                                        characteristic: snapshot.clone(),
                                        value,
                                    });
                                }
                                Ok(())
                            },
                        );
                    gatt.ValueChanged(&handler).map_err(|_| notify_failed())?
                };
                // Held through the config write and the insert: an unsubscribe's remove+clear
                // cannot land between them and leave this subscription dead.
                let _gate = gate.lock().await;
                // From here to the insert, any failure leaves no sub: revoke the handler.
                let operation =
                    match gatt.WriteClientCharacteristicConfigurationDescriptorAsync(config) {
                        Ok(operation) => operation,
                        Err(error) => {
                            let _ = gatt.RemoveValueChanged(token);
                            return Err(winrt_err(error));
                        }
                    };
                let status = match race!(operation, GATT_OP_TIMEOUT) {
                    Ok(status) => status,
                    Err(error) => {
                        let _ = gatt.RemoveValueChanged(token);
                        return Err(error);
                    }
                };
                if status != GattCommunicationStatus::Success {
                    let _ = gatt.RemoveValueChanged(token);
                    return Err(gatt_err(
                        status.0,
                        ErrorKind::CharacteristicNotifyChangeFailed,
                    ));
                }
                let mut shared = lock(&shared);
                let Some(entry) = shared.devices.get_mut(&address) else {
                    let _ = gatt.RemoveValueChanged(token);
                    return Err(BluetoothError::new(ErrorKind::DeviceNotConnected));
                };
                // The lock was released across the CCCD await; a concurrent subscribe may
                // have won. It keeps its registration, and ours is revoked by hand.
                if let Some(winner) = entry.notify.get(&characteristic.uuid) {
                    let winner_tx = winner.tx.clone();
                    drop(shared);
                    let _ = gatt.RemoveValueChanged(token);
                    return Ok(winner_tx.subscribe());
                }
                entry.notify.insert(
                    characteristic.uuid.clone(),
                    NotifySub {
                        characteristic: gatt,
                        token,
                        tx: tx.clone(),
                    },
                );
                Ok(tx.subscribe())
            })
        }

        fn unsubscribe_characteristic(
            &self,
            id: &DeviceId,
            characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            let id = id.clone();
            let characteristic = characteristic.clone();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                let gate = {
                    let shared = lock(&shared);
                    let Some(entry) = shared.devices.get(&address) else {
                        return Err(BluetoothError::new(ErrorKind::DeviceNotConnected));
                    };
                    entry.notify_gate.clone()
                };
                // Held through the remove and the CCCD clear, against subscribe's write+insert.
                let _gate = gate.lock().await;
                let sub = {
                    let mut shared = lock(&shared);
                    let Some(entry) = shared.devices.get_mut(&address) else {
                        return Err(BluetoothError::new(ErrorKind::DeviceNotConnected));
                    };
                    entry.notify.remove(&characteristic.uuid)
                };
                let Some(sub) = sub else {
                    return Ok(());
                };
                // The sub drops here however the write ends, revoking its handler token.
                let status = race!(
                    sub.characteristic
                        .WriteClientCharacteristicConfigurationDescriptorAsync(
                            GattClientCharacteristicConfigurationDescriptorValue::None,
                        )
                        .map_err(winrt_err)?,
                    GATT_OP_TIMEOUT
                )?;
                if status != GattCommunicationStatus::Success {
                    return Err(gatt_err(
                        status.0,
                        ErrorKind::CharacteristicNotifyChangeFailed,
                    ));
                }
                Ok(())
            })
        }

        fn read_descriptor(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            let id = id.clone();
            let descriptor = descriptor.clone();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                let gatt = self.gatt_descriptor(address, &descriptor)?;
                let result = race!(
                    gatt.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)
                        .map_err(winrt_err)?,
                    GATT_OP_TIMEOUT
                )?;
                let status = result.Status().map_err(winrt_err)?;
                if status != GattCommunicationStatus::Success {
                    return Err(gatt_err(status.0, ErrorKind::DescriptorReadFailed));
                }
                let value = result.Value().map_err(winrt_err)?;
                buffer_bytes(&value).map_err(winrt_err)
            })
        }

        fn write_descriptor(
            &self,
            id: &DeviceId,
            descriptor: &Descriptor,
            data: &[u8],
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let id = id.clone();
            let descriptor = descriptor.clone();
            let data = data.to_vec();
            Box::pin(async move {
                let Some(address) = device_address(&id) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                if descriptor.uuid.as_str().ends_with(CCCD_SUFFIX) {
                    // The CCCD belongs to subscribe/unsubscribe on Windows.
                    return Err(BluetoothError::new(ErrorKind::DescriptorWriteNotAllowed));
                }
                let gatt = self.gatt_descriptor(address, &descriptor)?;
                // The buffer is scoped: an IBuffer is not Send, so it dies before the await.
                let operation = {
                    let buffer = bytes_buffer(&data).map_err(winrt_err)?;
                    gatt.WriteValueAsync(&buffer).map_err(winrt_err)?
                };
                let status = race!(operation, GATT_OP_TIMEOUT)?;
                if status != GattCommunicationStatus::Success {
                    return Err(gatt_err(status.0, ErrorKind::DescriptorWriteFailed));
                }
                Ok(())
            })
        }

        fn connection_events(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ConnectionEvent>, BluetoothError>>
        {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move { Ok(lock(&shared).connection_tx.subscribe()) })
        }
    }

    impl Drop for WindowsCentral {
        fn drop(&mut self) {
            let mut shared = lock(&self.shared);
            shared.scan.take(); // ActiveScan's Drop stops the watcher and revokes the token.
            shared.devices.clear();
            if let (Some(radio), Some(token)) = (shared.radio.clone(), shared.radio_token.take()) {
                let _ = radio.RemoveStateChanged(token);
            }
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Server role
    // ---------------------------------------------------------------------------------------------

    struct ServerShared {
        services: HashMap<Uuid, ServedService>,
        advertising: bool,
        publisher_status: i32,
        publisher: Option<ActivePublisher>,
        status_tx: watch::Sender<u64>,
        requests_tx: broadcast::Sender<GattRequest>,
    }

    struct ServedService {
        provider: GattServiceProvider,
        status_token: i64,
        status: i32,
        characteristics: HashMap<Uuid, LocalCharacteristic>,
    }

    impl Drop for ServedService {
        fn drop(&mut self) {
            let _ = self
                .provider
                .RemoveAdvertisementStatusChanged(self.status_token);
        }
    }

    struct LocalCharacteristic {
        characteristic: GattLocalCharacteristic,
        read_token: i64,
        write_token: i64,
        value: Vec<u8>,
    }

    impl Drop for LocalCharacteristic {
        fn drop(&mut self) {
            let _ = self.characteristic.RemoveReadRequested(self.read_token);
            let _ = self.characteristic.RemoveWriteRequested(self.write_token);
        }
    }

    struct ActivePublisher {
        publisher: BluetoothLEAdvertisementPublisher,
        token: i64,
    }

    impl Drop for ActivePublisher {
        fn drop(&mut self) {
            let _ = self.publisher.Stop();
            let _ = self.publisher.RemoveStatusChanged(self.token);
        }
    }

    /// The WinRT server: `GattServiceProvider`s per service plus a publisher for the local name.
    pub(super) struct WindowsServer {
        shared: Arc<Mutex<ServerShared>>,
    }

    /// Registers the read and write handlers of one served characteristic.
    fn build_local_characteristic(
        shared: &Arc<Mutex<ServerShared>>,
        service_uuid: Uuid,
        characteristic_uuid: Uuid,
        characteristic: GattLocalCharacteristic,
        value: Vec<u8>,
    ) -> Result<LocalCharacteristic, BluetoothError> {
        let weak = Arc::downgrade(shared);
        // Each handler takes its own copy; the read closure moves the originals in.
        let write_service_uuid = service_uuid.clone();
        let write_characteristic_uuid = characteristic_uuid.clone();
        let read_handler =
            TypedEventHandler::<GattLocalCharacteristic, GattReadRequestedEventArgs>::new(
                move |_sender, args| {
                    let Some(shared) = weak.upgrade() else {
                        return Ok(());
                    };
                    let Some(args) = args.as_ref() else {
                        return Ok(());
                    };
                    // The deferral holds the request window open: without it the peer's
                    // request can die before this handler fetched it.
                    let deferral = args.GetDeferral().ok();
                    let Some(request) = (match args.GetRequestAsync() {
                        Ok(operation) => request_now!(operation),
                        Err(_) => None,
                    }) else {
                        // No request means nothing to answer through; closing the deferral
                        // ends our window so the stack's own error path resolves the peer.
                        if let Some(deferral) = &deferral {
                            let _ = deferral.Complete();
                        }
                        return Ok(());
                    };
                    let offset = request.Offset().unwrap_or(0) as usize;
                    let value = lock(&shared)
                        .services
                        .get(&service_uuid)
                        .and_then(|service| service.characteristics.get(&characteristic_uuid))
                        .map(|local| local.value.clone())
                        .unwrap_or_default();
                    let _ = lock(&shared).requests_tx.send(GattRequest {
                        service_uuid: service_uuid.clone(),
                        characteristic_uuid: characteristic_uuid.clone(),
                        kind: GattRequestKind::Read { offset },
                    });
                    // An offset past the value is the protocol's invalid-offset error.
                    if offset > value.len() {
                        let _ = request.RespondWithProtocolError(0x07);
                    } else if let Ok(buffer) = bytes_buffer(&value[offset..]) {
                        let _ = request.RespondWithValue(&buffer);
                    } else {
                        let _ = request.RespondWithProtocolError(0x02);
                    }
                    if let Some(deferral) = &deferral {
                        let _ = deferral.Complete();
                    }
                    Ok(())
                },
            );
        let read_token = characteristic
            .ReadRequested(&read_handler)
            .map_err(winrt_err)?;

        let weak = Arc::downgrade(shared);
        let write_handler =
            TypedEventHandler::<GattLocalCharacteristic, GattWriteRequestedEventArgs>::new(
                move |_sender, args| {
                    let Some(shared) = weak.upgrade() else {
                        return Ok(());
                    };
                    let Some(args) = args.as_ref() else {
                        return Ok(());
                    };
                    // The deferral holds the request window open: without it the peer's
                    // request can die before this handler fetched it.
                    let deferral = args.GetDeferral().ok();
                    let Some(request) = (match args.GetRequestAsync() {
                        Ok(operation) => request_now!(operation),
                        Err(_) => None,
                    }) else {
                        // No request means nothing to answer through; closing the deferral
                        // ends our window so the stack's own error path resolves the peer.
                        if let Some(deferral) = &deferral {
                            let _ = deferral.Complete();
                        }
                        return Ok(());
                    };
                    let offset = request.Offset().unwrap_or(0) as usize;
                    let with_response = request
                        .Option()
                        .is_ok_and(|option| option == GattWriteOption::WriteWithResponse);
                    let data = request
                        .Value()
                        .ok()
                        .and_then(|buffer| buffer_bytes(&buffer).ok())
                        .unwrap_or_default();
                    {
                        let mut shared = lock(&shared);
                        if let Some(local) =
                            shared
                                .services
                                .get_mut(&write_service_uuid)
                                .and_then(|service| {
                                    service.characteristics.get_mut(&write_characteristic_uuid)
                                })
                        {
                            // A write replaces the served value; the read responder hands it back.
                            local.value = data.clone();
                        }
                        let _ = shared.requests_tx.send(GattRequest {
                            service_uuid: write_service_uuid.clone(),
                            characteristic_uuid: write_characteristic_uuid.clone(),
                            kind: GattRequestKind::Write {
                                offset,
                                data,
                                with_response,
                            },
                        });
                    }
                    let _ = request.Respond();
                    if let Some(deferral) = &deferral {
                        let _ = deferral.Complete();
                    }
                    Ok(())
                },
            );
        let write_token = characteristic
            .WriteRequested(&write_handler)
            .map_err(winrt_err)?;

        Ok(LocalCharacteristic {
            characteristic,
            read_token,
            write_token,
            value,
        })
    }

    /// Stops every provider and the publisher, and clears the advertising claim.
    fn stop_all(shared: &Arc<Mutex<ServerShared>>) {
        let mut shared = lock(shared);
        shared.advertising = false;
        shared.publisher_status = -1;
        for served in shared.services.values() {
            let _ = served.provider.StopAdvertising();
        }
        shared.publisher.take();
    }

    impl WindowsServer {
        pub(super) fn new() -> Self {
            init_apartment();
            Self {
                shared: Arc::new(Mutex::new(ServerShared {
                    services: HashMap::new(),
                    advertising: false,
                    publisher_status: -1,
                    publisher: None,
                    status_tx: watch::channel(0u64).0,
                    requests_tx: broadcast::channel(CHANNEL_CAPACITY).0,
                })),
            }
        }
    }

    impl ServerBackend for WindowsServer {
        fn add_service(&self, service: Service) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move {
                let server_failed = || BluetoothError::new(ErrorKind::ServerFailed);
                if lock(&shared).services.contains_key(&service.uuid) {
                    return Err(server_failed());
                }
                let Some(service_guid) = guid_of(&service.uuid) else {
                    return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                };
                let result = race!(
                    GattServiceProvider::CreateAsync(service_guid).map_err(winrt_err)?,
                    GATT_OP_TIMEOUT
                )?;
                // WinRT reports creation failures as a value, not a failed operation.
                let error = result.Error().map_err(winrt_err)?;
                if error.0 != 0 {
                    return Err(server_failed().with_status(i64::from(error.0)));
                }
                let provider = result.ServiceProvider().map_err(winrt_err)?;
                let local_service = provider.Service().map_err(winrt_err)?;

                let mut characteristics = HashMap::new();
                for spec in &service.characteristics {
                    let Some(char_guid) = guid_of(&spec.uuid) else {
                        return Err(BluetoothError::new(ErrorKind::InvalidIdentifiers));
                    };
                    let parameters = GattLocalCharacteristicParameters::new().map_err(winrt_err)?;
                    parameters
                        .SetCharacteristicProperties(GattCharacteristicProperties(
                            spec.properties.bits() as u32,
                        ))
                        .map_err(winrt_err)?;
                    let initial = spec.value.clone().unwrap_or_default();
                    if !initial.is_empty() {
                        parameters
                            .SetStaticValue(&bytes_buffer(&initial).map_err(winrt_err)?)
                            .map_err(winrt_err)?;
                    }
                    let char_result = race!(
                        local_service
                            .CreateCharacteristicAsync(char_guid, &parameters)
                            .map_err(winrt_err)?,
                        GATT_OP_TIMEOUT
                    )?;
                    let error = char_result.Error().map_err(winrt_err)?;
                    if error.0 != 0 {
                        return Err(server_failed().with_status(i64::from(error.0)));
                    }
                    let characteristic = char_result.Characteristic().map_err(winrt_err)?;
                    let local = build_local_characteristic(
                        &shared,
                        service.uuid.clone(),
                        spec.uuid.clone(),
                        characteristic,
                        initial,
                    )?;
                    characteristics.insert(spec.uuid.clone(), local);
                }

                let weak = Arc::downgrade(&shared);
                let status_service_uuid = service.uuid.clone();
                let status_handler = TypedEventHandler::<
                    GattServiceProvider,
                    GattServiceProviderAdvertisementStatusChangedEventArgs,
                >::new(move |_sender, args| {
                    let Some(shared) = weak.upgrade() else {
                        return Ok(());
                    };
                    let status = args
                        .ok()
                        .and_then(|args| args.Status())
                        .map(|status| status.0)
                        .unwrap_or(-1);
                    {
                        let mut shared = lock(&shared);
                        if let Some(served) = shared.services.get_mut(&status_service_uuid) {
                            served.status = status;
                        }
                        shared.status_tx.send_modify(|version| *version += 1);
                    }
                    Ok(())
                });
                let status_token = provider
                    .AdvertisementStatusChanged(&status_handler)
                    .map_err(winrt_err)?;

                lock(&shared).services.insert(
                    service.uuid.clone(),
                    ServedService {
                        status: provider
                            .AdvertisementStatus()
                            .map(|status| status.0)
                            .unwrap_or(0),
                        provider,
                        status_token,
                        characteristics,
                    },
                );
                Ok(())
            })
        }

        fn remove_service(&self, uuid: Uuid) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move {
                lock(&shared).services.remove(&uuid);
                Ok(())
            })
        }

        fn start_advertising(
            &self,
            options: AdvertiseOptions,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move {
                let advertise_failed = || BluetoothError::new(ErrorKind::AdvertiseFailed);
                {
                    let mut shared = lock(&shared);
                    if shared.advertising {
                        return Err(BluetoothError::new(ErrorKind::InProgress));
                    }
                    shared.advertising = true;
                }
                let providers = lock(&shared)
                    .services
                    .values()
                    .map(|served| served.provider.clone())
                    .collect::<Vec<_>>();
                let parameters = match GattServiceProviderAdvertisingParameters::new() {
                    Ok(parameters) => parameters,
                    Err(error) => {
                        stop_all(&shared);
                        return Err(winrt_err(error));
                    }
                };
                let _ = parameters.SetIsConnectable(true);
                let _ = parameters.SetIsDiscoverable(true);
                for provider in &providers {
                    if let Err(_error) = provider.StartAdvertisingWithParameters(&parameters) {
                        stop_all(&shared);
                        return Err(advertise_failed());
                    }
                }

                // The publisher carries the local name; the providers carry the service UUIDs.
                if options.local_name.is_some() || !options.service_uuids.is_empty() {
                    let publisher = match BluetoothLEAdvertisementPublisher::new() {
                        Ok(publisher) => publisher,
                        Err(_) => {
                            stop_all(&shared);
                            return Err(advertise_failed());
                        }
                    };
                    if let Err(_error) = fill_advertisement(
                        &publisher,
                        options.local_name.as_deref(),
                        &options.service_uuids,
                    ) {
                        stop_all(&shared);
                        return Err(advertise_failed());
                    }
                    let weak = Arc::downgrade(&shared);
                    let handler = TypedEventHandler::<
                        BluetoothLEAdvertisementPublisher,
                        BluetoothLEAdvertisementPublisherStatusChangedEventArgs,
                    >::new(move |_sender, args| {
                        let Some(shared) = weak.upgrade() else {
                            return Ok(());
                        };
                        let status = args
                            .ok()
                            .and_then(|args| args.Status())
                            .map(|status| status.0)
                            .unwrap_or(-1);
                        {
                            let mut shared = lock(&shared);
                            shared.publisher_status = status;
                            shared.status_tx.send_modify(|version| *version += 1);
                        }
                        Ok(())
                    });
                    let token = match publisher.StatusChanged(&handler) {
                        Ok(token) => token,
                        Err(_) => {
                            stop_all(&shared);
                            return Err(advertise_failed());
                        }
                    };
                    if publisher.Start().is_err() {
                        let _ = publisher.RemoveStatusChanged(token);
                        stop_all(&shared);
                        return Err(advertise_failed());
                    }
                    lock(&shared).publisher = Some(ActivePublisher { publisher, token });
                }

                // Providers report Started (or StartedWithoutAllAdvertisementData); the wait is
                // bounded, and the watch cannot miss a change made after the subscribe.
                let provider_started = GattServiceProviderAdvertisementStatus::Started.0;
                let provider_partial =
                    GattServiceProviderAdvertisementStatus::StartedWithoutAllAdvertisementData.0;
                let provider_aborted = GattServiceProviderAdvertisementStatus::Aborted.0;
                let publisher_started = BluetoothLEAdvertisementPublisherStatus::Started.0;
                let publisher_aborted = BluetoothLEAdvertisementPublisherStatus::Aborted.0;
                let started = tokio::time::timeout(START_TIMEOUT, async {
                    let mut version = lock(&shared).status_tx.subscribe();
                    loop {
                        {
                            let shared = lock(&shared);
                            let mut all_started = true;
                            for served in shared.services.values() {
                                if served.status == provider_aborted {
                                    return false;
                                }
                                if served.status != provider_started
                                    && served.status != provider_partial
                                {
                                    all_started = false;
                                }
                            }
                            if shared.publisher.is_some() {
                                if shared.publisher_status == publisher_aborted {
                                    return false;
                                }
                                if shared.publisher_status != publisher_started {
                                    all_started = false;
                                }
                            }
                            if all_started {
                                return true;
                            }
                        }
                        let _ = version.changed().await;
                    }
                })
                .await
                .unwrap_or(false);
                if !started {
                    stop_all(&shared);
                    return Err(advertise_failed());
                }
                Ok(())
            })
        }

        fn stop_advertising(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move {
                stop_all(&shared);
                Ok(())
            })
        }

        fn is_advertising(&self) -> BackendFuture<'_, Result<bool, BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move { Ok(lock(&shared).advertising) })
        }

        fn set_characteristic_value(
            &self,
            service: Uuid,
            characteristic: Uuid,
            value: Vec<u8>,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move {
                let mut shared = lock(&shared);
                let Some(served) = shared.services.get_mut(&service) else {
                    return Err(BluetoothError::new(ErrorKind::ServiceNotFound));
                };
                let Some(local) = served.characteristics.get_mut(&characteristic) else {
                    return Err(BluetoothError::new(ErrorKind::CharacteristicNotFound));
                };
                // WinRT has no live static-value update; the read responder serves this value.
                local.value = value;
                Ok(())
            })
        }

        fn notify(
            &self,
            service: Uuid,
            characteristic: Uuid,
            data: &[u8],
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            let data = data.to_vec();
            Box::pin(async move {
                let no_subscriber =
                    || BluetoothError::new(ErrorKind::CharacteristicNotifyChangeFailed);
                let gatt = {
                    let shared = lock(&shared);
                    let served = shared
                        .services
                        .get(&service)
                        .ok_or_else(|| BluetoothError::new(ErrorKind::ServiceNotFound))?;
                    served
                        .characteristics
                        .get(&characteristic)
                        .map(|local| local.characteristic.clone())
                        .ok_or_else(|| BluetoothError::new(ErrorKind::CharacteristicNotFound))?
                };
                if gatt
                    .SubscribedClients()
                    .map_err(|_| no_subscriber())?
                    .Size()
                    .map_err(|_| no_subscriber())?
                    == 0
                {
                    return Err(no_subscriber());
                }
                // The buffer is scoped: an IBuffer is not Send, so it dies before the await.
                let operation = {
                    let buffer = bytes_buffer(&data).map_err(winrt_err)?;
                    gatt.NotifyValueAsync(&buffer).map_err(winrt_err)?
                };
                race!(operation, GATT_OP_TIMEOUT)?;
                Ok(())
            })
        }

        fn requests(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<GattRequest>, BluetoothError>> {
            let shared = Arc::clone(&self.shared);
            Box::pin(async move { Ok(lock(&shared).requests_tx.subscribe()) })
        }
    }

    impl Drop for WindowsServer {
        fn drop(&mut self) {
            stop_all(&self.shared);
            lock(&self.shared).services.clear();
        }
    }

    /// Puts the advertised name and any extra service UUIDs on the publisher's advertisement.
    fn fill_advertisement(
        publisher: &BluetoothLEAdvertisementPublisher,
        local_name: Option<&str>,
        service_uuids: &[Uuid],
    ) -> windows::core::Result<()> {
        let advertisement: BluetoothLEAdvertisement = publisher.Advertisement()?;
        if let Some(name) = local_name {
            advertisement.SetLocalName(&HSTRING::from(name))?;
        }
        let uuids = advertisement.ServiceUuids()?;
        for uuid in service_uuids {
            if let Some(value) = uuid_to_u128(uuid) {
                uuids.Append(GUID::from_u128(value))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radio_states_map_to_adapter_states() {
        assert_eq!(radio_state_to_adapter_state(1), AdapterState::On);
        assert_eq!(radio_state_to_adapter_state(2), AdapterState::PoweredOff);
        assert_eq!(radio_state_to_adapter_state(3), AdapterState::PoweredOff);
        assert_eq!(radio_state_to_adapter_state(0), AdapterState::Unknown);
        assert_eq!(radio_state_to_adapter_state(99), AdapterState::Unknown);
    }

    #[test]
    fn gatt_statuses_map_to_kinds_and_success_is_none() {
        assert_eq!(gatt_status_to_kind(0), None);
        assert_eq!(gatt_status_to_kind(1), Some(ErrorKind::Disconnected));
        assert_eq!(gatt_status_to_kind(2), Some(ErrorKind::Backend));
        assert_eq!(gatt_status_to_kind(3), Some(ErrorKind::Unauthorized));
        assert_eq!(gatt_status_to_kind(9), Some(ErrorKind::Backend));
    }

    #[test]
    fn known_hresults_map_to_kinds_and_the_rest_to_backend() {
        assert_eq!(
            winrt_error_to_kind(0x8007_0005_u32 as i32),
            ErrorKind::Unauthorized
        );
        assert_eq!(
            winrt_error_to_kind(0x8007_0490_u32 as i32),
            ErrorKind::DeviceNotFound
        );
        assert_eq!(
            winrt_error_to_kind(0x8007_0057_u32 as i32),
            ErrorKind::InvalidIdentifiers
        );
        assert_eq!(
            winrt_error_to_kind(0x8000_4004_u32 as i32),
            ErrorKind::Cancelled
        );
        assert_eq!(
            winrt_error_to_kind(0x8007_04C7_u32 as i32),
            ErrorKind::Cancelled
        );
        assert_eq!(
            winrt_error_to_kind(0x8000_4008_u32 as i32),
            ErrorKind::Backend
        );
        assert_eq!(winrt_error_to_kind(0), ErrorKind::Backend);
    }

    #[test]
    fn the_device_id_round_trips_a_bluetooth_address() {
        let address = 0x0011_2233_4455_u64;
        assert_eq!(device_address(&address_device_id(address)), Some(address));
        assert_eq!(device_address(&DeviceId::new("watch")), None);
        assert_eq!(device_address(&address_device_id(0)), None);
        // One past the six-byte address range.
        assert_eq!(device_address(&DeviceId::new("281474976710656")), None);
    }

    #[test]
    fn uuids_round_trip_through_their_128_bit_form() {
        let short = Uuid::parse("180D").unwrap();
        let round = u128_to_uuid(uuid_to_u128(&short).unwrap()).unwrap();
        assert_eq!(round, short);
        let full = Uuid::parse("10B201FF-5B3B-45A1-9508-CF3EFCD7BBAF").unwrap();
        let round = u128_to_uuid(uuid_to_u128(&full).unwrap()).unwrap();
        assert_eq!(round, full);
    }

    #[test]
    fn signal_strengths_clamp_into_i8() {
        assert_eq!(i16_to_i8(-42), -42);
        assert_eq!(i16_to_i8(-200), -128);
        assert_eq!(i16_to_i8(200), 127);
        assert_eq!(i16_to_i8(0), 0);
    }
}
