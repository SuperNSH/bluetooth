//! `Central` and `Device`: the calls an app makes, with one scan at a time, connect retries, and
//! the GATT guards on top of a backend.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::sync::{broadcast, oneshot};

use crate::backend::{
    AdapterState, CentralBackend, Characteristic, ConnectionEvent, Descriptor, DeviceId,
    DeviceInfo, ScanFilter, ScanResult, Service, Uuid, ValueNotification,
};
use crate::error::{BluetoothError, ErrorKind};

/// What a connect may do: give up after `timeout`, and retry an Android status 133 up to
/// `max_attempts` total tries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectOptions {
    /// The deadline for the whole connect, retries included; `None` waits for the platform.
    pub timeout: Option<Duration>,
    /// How many tries the connect gets; a `ConnectFailed` with status 133 is retried, other
    /// failures return at once. Values below 1 count as 1.
    pub max_attempts: u8,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            timeout: None,
            max_attempts: 1,
        }
    }
}

/// One scan per `Central`. `active` names the generation that owns the gate, so an old stream's
/// drop cannot end a newer scan; `owed_stops` counts backend stops a dropped stream still owes.
#[derive(Default)]
struct ScanGate {
    next: u64,
    active: Option<u64>,
    owed_stops: u32,
}

/// The cancel signals of the connects waiting on one device, keyed by registration.
type CancelMap = HashMap<DeviceId, Vec<(u64, oneshot::Sender<()>)>>;

struct CentralShared {
    backend: Arc<dyn CentralBackend>,
    scan: Mutex<ScanGate>,
    devices: Mutex<HashMap<DeviceId, Arc<DeviceState>>>,
    connects: Mutex<CancelMap>,
    next_id: AtomicU64,
}

impl CentralShared {
    fn gate(&self) -> MutexGuard<'_, ScanGate> {
        self.scan.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Releases the scan gate when `generation` still owns it.
    fn end_scan(&self, generation: u64) {
        let mut gate = self.gate();
        if gate.active == Some(generation) {
            gate.active = None;
        }
    }

    /// Releases the gate and records the backend stop `generation` still owes; the next scan
    /// pays it, so the gate is only ever free with the backend stopped or the stop pending here.
    fn owe_scan_stop(&self, generation: u64) {
        let mut gate = self.gate();
        if gate.active == Some(generation) {
            gate.active = None;
            gate.owed_stops = gate.owed_stops.saturating_add(1);
        }
    }
}

/// The services one device handle discovered. Shared by every handle for the same device, so
/// two `device` calls see one cache.
struct DeviceState {
    services: std::sync::OnceLock<Vec<Service>>,
}

/// The central role: scan, connect, and GATT operations over a backend.
///
/// Clones share one scan gate and one per-device state. [`Central::platform_central`] returns the
/// target's backend, with the shared fallback for platforms without one.
#[derive(Clone)]
pub struct Central {
    shared: Arc<CentralShared>,
}

impl Central {
    /// A central over `backend`. Tests and fakes use this.
    pub fn new(backend: Arc<dyn CentralBackend>) -> Self {
        Self {
            shared: Arc::new(CentralShared {
                backend,
                scan: Mutex::new(ScanGate::default()),
                devices: Mutex::new(HashMap::new()),
                connects: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(0),
            }),
        }
    }

    /// The target's central. On a platform without a bridge this carries an
    /// [`crate::UnsupportedBackend`], which reports `ERR_BLUETOOTH_UNSUPPORTED`.
    pub fn platform_central() -> Self {
        Self::new(crate::platform::central_backend())
    }

    /// Whether the platform has a usable Bluetooth stack: false when the backend reports no
    /// support, true even when a query fails for another reason.
    pub async fn is_supported(&self) -> bool {
        match self.shared.backend.adapter_state().await {
            Ok(AdapterState::Unsupported) => false,
            Err(error) => error.kind() != ErrorKind::Unsupported,
            _ => true,
        }
    }

    /// The adapter's state now.
    pub async fn state(&self) -> Result<AdapterState, BluetoothError> {
        self.shared.backend.adapter_state().await
    }

    /// Later adapter states, from the next change on.
    pub async fn state_changes(&self) -> Result<broadcast::Receiver<AdapterState>, BluetoothError> {
        self.shared.backend.state_changes().await
    }

    /// Starts a scan. One scan runs per `Central` at a time; a second gets
    /// `ERR_BLUETOOTH_IN_PROGRESS`. Stopping the returned stream, or dropping it, ends the scan;
    /// a dropped stream's backend stop is paid here, before the new scan starts. Dropping the
    /// call itself frees the claim, so an abandoned start cannot wedge the gate.
    pub async fn scan(&self, filter: ScanFilter) -> Result<ScanStream, BluetoothError> {
        let generation = {
            let mut gate = self.shared.gate();
            if gate.active.is_some() {
                return Err(BluetoothError::new(ErrorKind::InProgress));
            }
            let generation = gate.next;
            gate.next += 1;
            gate.active = Some(generation);
            generation
        };
        let mut claim = ScanClaim {
            shared: Arc::clone(&self.shared),
            generation,
            armed: true,
        };
        // Pay stop-first, count after: an abandoned start never erases an unpaid debt.
        while self.shared.gate().owed_stops > 0 {
            let _ = self.shared.backend.stop_scan().await;
            self.shared.gate().owed_stops -= 1;
        }
        match self.shared.backend.start_scan(filter).await {
            Ok(receiver) => {
                claim.disarm();
                Ok(ScanStream {
                    shared: Arc::clone(&self.shared),
                    receiver,
                    generation,
                    done: false,
                })
            }
            // The backend refused the start; release the gate without owing a stop.
            Err(error) => {
                claim.disarm();
                self.shared.end_scan(generation);
                Err(error)
            }
        }
    }

    /// The devices the system already knows, filtered by `service_uuids` when not empty.
    pub async fn system_devices(
        &self,
        service_uuids: &[Uuid],
    ) -> Result<Vec<DeviceInfo>, BluetoothError> {
        self.shared.backend.system_devices(service_uuids).await
    }

    /// A handle for the device. Handles for one id share their discovery cache.
    pub fn device(&self, id: &DeviceId) -> Device {
        let state = {
            let mut devices = self
                .shared
                .devices
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            devices
                .entry(id.clone())
                .or_insert_with(|| {
                    Arc::new(DeviceState {
                        services: std::sync::OnceLock::new(),
                    })
                })
                .clone()
        };
        Device {
            shared: Arc::clone(&self.shared),
            id: id.clone(),
            state,
        }
    }

    /// Connection events for the devices this central uses, from the next one on.
    pub async fn connection_events(
        &self,
    ) -> Result<broadcast::Receiver<ConnectionEvent>, BluetoothError> {
        self.shared.backend.connection_events().await
    }
}

/// The results of one scan. `recv` yields each result once the filter passed; after `stop`, or
/// once the platform ends the scan, it yields `None`. A lagged receiver resynchronises instead of
/// failing.
pub struct ScanStream {
    shared: Arc<CentralShared>,
    receiver: broadcast::Receiver<ScanResult>,
    generation: u64,
    done: bool,
}

impl std::fmt::Debug for ScanStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanStream")
            .field("generation", &self.generation)
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl ScanStream {
    /// The next scan result, or `None` once the scan is over.
    pub async fn recv(&mut self) -> Option<ScanResult> {
        if self.done {
            return None;
        }
        loop {
            match self.receiver.recv().await {
                Ok(result) => return Some(result),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    // The platform ended the scan; free the gate and owe the stop, or every
                    // later scan on this Central would meet InProgress forever.
                    self.done = true;
                    self.shared.owe_scan_stop(self.generation);
                    return None;
                }
            }
        }
    }

    /// Ends the scan: the backend stop completes before the gate is released, so a scan started
    /// right after never meets a backend that is still scanning. A failed stop still frees the
    /// gate; its error is swallowed. Later `recv` calls return `None`.
    pub async fn stop(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        let _ = self.shared.backend.stop_scan().await;
        self.shared.end_scan(self.generation);
    }
}

impl Drop for ScanStream {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // Drop cannot await; the next Central::scan pays the owed stop before it starts.
        self.done = true;
        self.shared.owe_scan_stop(self.generation);
    }
}

/// Releases the scan claim when a start is abandoned — dropped or failed — before a stream
/// exists to stop.
struct ScanClaim {
    shared: Arc<CentralShared>,
    generation: u64,
    armed: bool,
}

impl ScanClaim {
    /// Hands the claim to the constructed stream; its stop and drop own the gate from here.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ScanClaim {
    fn drop(&mut self) {
        if self.armed {
            // An abandoned start may have reached the backend; the next scan pays a defensive
            // stop, which backends answer with Ok when no scan runs.
            self.shared.owe_scan_stop(self.generation);
        }
    }
}

/// Removes its own registration from the cancel map when the connect ends, however it ends.
struct CancelSlot {
    shared: Arc<CentralShared>,
    id: DeviceId,
    key: u64,
}

impl CancelSlot {
    fn register(shared: &Arc<CentralShared>, id: &DeviceId, sender: oneshot::Sender<()>) -> Self {
        let key = shared.next_id.fetch_add(1, Ordering::Relaxed);
        shared
            .connects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(id.clone())
            .or_default()
            .push((key, sender));
        Self {
            shared: Arc::clone(shared),
            id: id.clone(),
            key,
        }
    }
}

impl Drop for CancelSlot {
    fn drop(&mut self) {
        let mut connects = self
            .shared
            .connects
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(waiting) = connects.get_mut(&self.id) {
            waiting.retain(|(key, _)| *key != self.key);
            if waiting.is_empty() {
                connects.remove(&self.id);
            }
        }
    }
}

/// A connected device: the connection, its services, and the GATT operations on them.
#[derive(Clone)]
pub struct Device {
    shared: Arc<CentralShared>,
    id: DeviceId,
    state: Arc<DeviceState>,
}

impl Device {
    /// The device's platform id.
    pub fn id(&self) -> &DeviceId {
        &self.id
    }

    /// Connects the device.
    ///
    /// A `ConnectFailed` carrying status 133 is retried until `max_attempts` tries are spent;
    /// every other failure returns at once. `timeout`, when set, races the whole connect. A
    /// `cancel_connect` resolves the call with `ERR_BLUETOOTH_CANCELLED`.
    pub async fn connect(&self, options: ConnectOptions) -> Result<(), BluetoothError> {
        if options.timeout == Some(Duration::ZERO) {
            return Err(BluetoothError::new(ErrorKind::Timeout));
        }
        let (cancel_sender, mut cancel_receiver) = oneshot::channel::<()>();
        let _slot = CancelSlot::register(&self.shared, &self.id, cancel_sender);
        let deadline = options
            .timeout
            .map(|timeout| tokio::time::Instant::now() + timeout);
        let mut remaining = options.max_attempts.max(1);
        loop {
            let attempt = self.shared.backend.connect(&self.id);
            let outcome = match deadline {
                Some(deadline) => tokio::select! {
                    outcome = attempt => outcome,
                    _ = &mut cancel_receiver => {
                        return Err(BluetoothError::new(ErrorKind::Cancelled))
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        return Err(BluetoothError::new(ErrorKind::Timeout))
                    }
                },
                None => tokio::select! {
                    outcome = attempt => outcome,
                    _ = &mut cancel_receiver => {
                        return Err(BluetoothError::new(ErrorKind::Cancelled))
                    }
                },
            };
            match outcome {
                Ok(()) => return Ok(()),
                Err(error) => {
                    let retry = remaining > 1
                        && error.kind() == ErrorKind::ConnectFailed
                        && error.status() == Some(133);
                    if !retry {
                        return Err(error);
                    }
                    remaining -= 1;
                }
            }
        }
    }

    /// Cancels a pending connect of this device; it resolves with `ERR_BLUETOOTH_CANCELLED`.
    /// With nothing pending, this is still `Ok`.
    pub fn cancel_connect(&self) -> Result<(), BluetoothError> {
        let mut connects = self
            .shared
            .connects
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(waiting) = connects.remove(&self.id) {
            for (_, sender) in waiting {
                let _ = sender.send(());
            }
        }
        Ok(())
    }

    /// Disconnects the device. Ok when the device is already disconnected.
    pub async fn disconnect(&self) -> Result<(), BluetoothError> {
        self.shared.backend.disconnect(&self.id).await
    }

    /// Whether the device is connected.
    pub async fn is_connected(&self) -> Result<bool, BluetoothError> {
        self.shared.backend.is_connected(&self.id).await
    }

    /// The device's current signal strength.
    pub async fn read_rssi(&self) -> Result<i8, BluetoothError> {
        self.shared.backend.read_rssi(&self.id).await
    }

    /// Asks for a larger MTU and returns the negotiated value.
    pub async fn request_mtu(&self, mtu: u16) -> Result<u16, BluetoothError> {
        self.shared.backend.request_mtu(&self.id, mtu).await
    }

    /// Discovers the device's services and caches them; a second call returns the cache without
    /// touching the backend.
    pub async fn discover_services(&self) -> Result<Vec<Service>, BluetoothError> {
        if let Some(services) = self.state.services.get() {
            return Ok(services.clone());
        }
        let services = self.shared.backend.discover_services(&self.id).await?;
        let _ = self.state.services.set(services.clone());
        Ok(services)
    }

    /// The discovered services. Empty before `discover_services`; never an error.
    pub fn services(&self) -> &[Service] {
        self.state
            .services
            .get()
            .map_or(&[], |services| services.as_slice())
    }

    /// The discovered service with this UUID.
    pub fn service(&self, uuid: Uuid) -> Result<&Service, BluetoothError> {
        let services = self.discovered()?;
        services
            .iter()
            .find(|service| service.uuid == uuid)
            .ok_or_else(|| BluetoothError::new(ErrorKind::ServiceNotFound))
    }

    /// The discovered characteristic with this UUID, across all services.
    pub fn characteristic(&self, uuid: Uuid) -> Result<&Characteristic, BluetoothError> {
        let services = self.discovered()?;
        services
            .iter()
            .flat_map(|service| &service.characteristics)
            .find(|characteristic| characteristic.uuid == uuid)
            .ok_or_else(|| BluetoothError::new(ErrorKind::CharacteristicNotFound))
    }

    fn discovered(&self) -> Result<&[Service], BluetoothError> {
        self.state
            .services
            .get()
            .map(|services| services.as_slice())
            .ok_or_else(|| BluetoothError::new(ErrorKind::ServicesNotDiscovered))
    }

    /// A discovered service-characteristic pair, or the code for what is missing.
    fn known_characteristic(
        &self,
        service_uuid: &Uuid,
        characteristic_uuid: &Uuid,
    ) -> Result<(), BluetoothError> {
        let known = self.discovered()?.iter().any(|service| {
            service.uuid == *service_uuid
                && service
                    .characteristics
                    .iter()
                    .any(|characteristic| characteristic.uuid == *characteristic_uuid)
        });
        if known {
            Ok(())
        } else {
            Err(BluetoothError::new(ErrorKind::CharacteristicNotFound))
        }
    }

    /// A discovered descriptor, or the code for what is missing.
    fn known_descriptor(&self, descriptor: &Descriptor) -> Result<(), BluetoothError> {
        let known = self.discovered()?.iter().any(|service| {
            service.uuid == descriptor.service_uuid
                && service.characteristics.iter().any(|characteristic| {
                    characteristic.uuid == descriptor.characteristic_uuid
                        && characteristic
                            .descriptors
                            .iter()
                            .any(|candidate| candidate.uuid == descriptor.uuid)
                })
        });
        if known {
            Ok(())
        } else {
            Err(BluetoothError::new(ErrorKind::DescriptorNotFound))
        }
    }

    /// A GATT op needs a device that is still connected; a dropped one reports `Disconnected`.
    async fn require_connected(&self) -> Result<(), BluetoothError> {
        match self.shared.backend.is_connected(&self.id).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(BluetoothError::new(ErrorKind::Disconnected)),
            Err(error) => Err(error),
        }
    }

    /// Reads the characteristic's value.
    pub async fn read_characteristic(
        &self,
        characteristic: &Characteristic,
    ) -> Result<Vec<u8>, BluetoothError> {
        self.known_characteristic(&characteristic.service_uuid, &characteristic.uuid)?;
        self.require_connected().await?;
        self.shared
            .backend
            .read_characteristic(&self.id, characteristic)
            .await
    }

    /// Writes the characteristic's value.
    pub async fn write_characteristic(
        &self,
        characteristic: &Characteristic,
        data: &[u8],
        write_type: crate::backend::WriteType,
    ) -> Result<(), BluetoothError> {
        self.known_characteristic(&characteristic.service_uuid, &characteristic.uuid)?;
        self.require_connected().await?;
        self.shared
            .backend
            .write_characteristic(&self.id, characteristic, data, write_type)
            .await
    }

    /// Subscribes to the characteristic's values and returns the stream. Subscribing again keeps
    /// one logical subscription and returns a fresh receiver.
    pub async fn subscribe_characteristic(
        &self,
        characteristic: &Characteristic,
    ) -> Result<broadcast::Receiver<ValueNotification>, BluetoothError> {
        self.known_characteristic(&characteristic.service_uuid, &characteristic.uuid)?;
        self.require_connected().await?;
        self.shared
            .backend
            .subscribe_characteristic(&self.id, characteristic)
            .await
    }

    /// Unsubscribes from the characteristic. Ok when not subscribed.
    pub async fn unsubscribe_characteristic(
        &self,
        characteristic: &Characteristic,
    ) -> Result<(), BluetoothError> {
        self.known_characteristic(&characteristic.service_uuid, &characteristic.uuid)?;
        self.require_connected().await?;
        self.shared
            .backend
            .unsubscribe_characteristic(&self.id, characteristic)
            .await
    }

    /// Reads the descriptor's value.
    pub async fn read_descriptor(
        &self,
        descriptor: &Descriptor,
    ) -> Result<Vec<u8>, BluetoothError> {
        self.known_descriptor(descriptor)?;
        self.require_connected().await?;
        self.shared
            .backend
            .read_descriptor(&self.id, descriptor)
            .await
    }

    /// Writes the descriptor's value.
    pub async fn write_descriptor(
        &self,
        descriptor: &Descriptor,
        data: &[u8],
    ) -> Result<(), BluetoothError> {
        self.known_descriptor(descriptor)?;
        self.require_connected().await?;
        self.shared
            .backend
            .write_descriptor(&self.id, descriptor, data)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        AdvertisementData, BackendFuture, CharacteristicProperties, ScanResult as ScanResultType,
        WriteType,
    };
    use crate::fake::{FakeBackend, FakeOp};
    use crate::unsupported::UnsupportedBackend;
    use std::future::{pending, ready};

    fn watch_result(id: &str) -> ScanResultType {
        ScanResultType {
            id: DeviceId::new(id),
            name: Some(id.to_owned()),
            rssi: Some(-42),
            advertisement: AdvertisementData {
                service_uuids: vec![Uuid::parse("180D").unwrap()],
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
                properties: CharacteristicProperties::from_bits(0x12),
                descriptors: Vec::new(),
                value: Some(vec![6, 66]),
            }],
            included_services: Vec::new(),
        }]
    }

    /// A connected device with its services discovered, over the fake.
    async fn connected_device() -> (Arc<FakeBackend>, Central, Device) {
        let fake = FakeBackend::new();
        let central = Central::new(fake.clone());
        fake.add_scanned_device(watch_result("watch"));
        fake.set_services(&DeviceId::new("watch"), heart_rate_services());
        let device = central.device(&DeviceId::new("watch"));
        device.connect(ConnectOptions::default()).await.unwrap();
        device.discover_services().await.unwrap();
        (fake, central, device)
    }

    #[tokio::test]
    async fn is_supported_reads_the_stack() {
        let fake = FakeBackend::new();
        let central = Central::new(fake.clone());
        assert!(central.is_supported().await);
        fake.set_adapter_state(AdapterState::Unsupported);
        assert!(!central.is_supported().await);
        let fallback = Central::new(Arc::new(UnsupportedBackend));
        assert!(!fallback.is_supported().await);
    }

    #[tokio::test]
    async fn platform_central_resolves_state_exactly_once() {
        let central = Central::platform_central();
        // Any platform, bridged or not: the call resolves instead of hanging.
        let _ = central.state().await;
    }

    #[tokio::test]
    async fn state_and_state_changes_pass_through() {
        let fake = FakeBackend::new();
        let central = Central::new(fake.clone());
        assert_eq!(central.state().await.unwrap(), AdapterState::On);
        let mut changes = central.state_changes().await.unwrap();
        fake.set_adapter_state(AdapterState::PoweredOff);
        assert_eq!(changes.recv().await.unwrap(), AdapterState::PoweredOff);
    }

    #[tokio::test]
    async fn one_scan_at_a_time_and_results_flow_until_stopped() {
        let fake = FakeBackend::new();
        let central = Central::new(fake.clone());
        let mut stream = central.scan(ScanFilter::default()).await.unwrap();
        let error = central.scan(ScanFilter::default()).await.unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_IN_PROGRESS");
        fake.add_scanned_device(watch_result("watch"));
        assert_eq!(stream.recv().await.unwrap().id, DeviceId::new("watch"));
        stream.stop().await;
        assert_eq!(stream.recv().await, None);
        // The gate is free again, and the fake's scan stopped before it was released.
        let mut second = central.scan(ScanFilter::default()).await.unwrap();
        fake.add_scanned_device(watch_result("ring"));
        assert_eq!(second.recv().await.unwrap().id, DeviceId::new("ring"));
    }

    #[tokio::test]
    async fn dropping_a_stream_releases_the_scan_gate() {
        let fake = FakeBackend::new();
        let central = Central::new(fake);
        {
            let _stream = central.scan(ScanFilter::default()).await.unwrap();
        }
        // The next scan pays the stop the drop owed, in line, before it starts.
        assert!(central.scan(ScanFilter::default()).await.is_ok());
    }

    #[tokio::test]
    async fn a_lagged_stream_resynchronises_instead_of_failing() {
        let fake = FakeBackend::new();
        let central = Central::new(fake.clone());
        let mut stream = central.scan(ScanFilter::default()).await.unwrap();
        for index in 0..40 {
            let mut result = watch_result("watch");
            result.rssi = Some(-40 - (index as i8));
            fake.add_scanned_device(result);
        }
        // The receiver lagged past the 32-slot buffer; it still delivers newer results.
        assert!(stream.recv().await.is_some());
        stream.stop().await;
        assert_eq!(stream.recv().await, None);
    }

    #[tokio::test]
    async fn a_scan_start_failure_frees_the_gate() {
        let fake = FakeBackend::new();
        let central = Central::new(fake.clone());
        fake.fail_next(
            FakeOp::StartScan,
            BluetoothError::new(ErrorKind::ScanStartFailed),
        );
        let error = central.scan(ScanFilter::default()).await.unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_SCAN_START_FAILED");
        assert!(central.scan(ScanFilter::default()).await.is_ok());
    }

    #[tokio::test]
    async fn a_platform_ended_scan_frees_the_gate() {
        let fake = FakeBackend::new();
        let central = Central::new(fake.clone());
        let mut stream = central.scan(ScanFilter::default()).await.unwrap();
        // The platform ends the scan itself: its sender drops and the stream's channel closes.
        fake.stop_scan().await.unwrap();
        assert_eq!(stream.recv().await, None);
        assert!(
            central.scan(ScanFilter::default()).await.is_ok(),
            "a platform-ended scan wedged the gate"
        );
    }

    #[tokio::test]
    async fn a_dropped_scan_start_frees_the_gate() {
        let central = Central::new(Arc::new(PendingBackend));
        {
            let mut call = std::pin::pin!(central.scan(ScanFilter::default()));
            // The gate is claimed, the backend never answers, and the call is abandoned.
            let _ = tokio::time::timeout(Duration::from_millis(25), call.as_mut()).await;
        }
        match tokio::time::timeout(
            Duration::from_millis(25),
            central.scan(ScanFilter::default()),
        )
        .await
        {
            // Still starting: the second call passed the gate and waits on the backend.
            Err(_still_starting) => {}
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                assert_ne!(error.code(), "ERR_BLUETOOTH_IN_PROGRESS", "the gate leaked")
            }
        }
    }

    #[tokio::test]
    async fn connect_and_disconnect_are_idempotent() {
        let fake = FakeBackend::new();
        let central = Central::new(fake.clone());
        fake.add_scanned_device(watch_result("watch"));
        let device = central.device(&DeviceId::new("watch"));
        device.connect(ConnectOptions::default()).await.unwrap();
        let error = device.connect(ConnectOptions::default()).await.unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_ALREADY_CONNECTED");
        device.disconnect().await.unwrap();
        device.disconnect().await.unwrap();
        assert!(!device.is_connected().await.unwrap());
    }

    #[tokio::test]
    async fn a_status_133_is_retried_up_to_max_attempts() {
        let fake = FakeBackend::new();
        let central = Central::new(fake.clone());
        fake.add_scanned_device(watch_result("watch"));
        let status_133 = || BluetoothError::new(ErrorKind::ConnectFailed).with_status(133);
        let device = central.device(&DeviceId::new("watch"));

        fake.fail_next(FakeOp::Connect, status_133());
        fake.fail_next(FakeOp::Connect, status_133());
        device
            .connect(ConnectOptions {
                timeout: None,
                max_attempts: 3,
            })
            .await
            .unwrap();

        fake.fail_next(FakeOp::Connect, status_133());
        let error = device
            .connect(ConnectOptions {
                timeout: None,
                max_attempts: 1,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_CONNECT_FAILED");
        assert_eq!(error.status(), Some(133));

        // Another failure is not retried at all.
        fake.fail_next(
            FakeOp::Connect,
            BluetoothError::new(ErrorKind::DeviceNotFound),
        );
        let error = device
            .connect(ConnectOptions {
                timeout: None,
                max_attempts: 5,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_DEVICE_NOT_FOUND");
    }

    /// A backend whose connect and scan start never resolve, to race the timeout, the cancel
    /// and the dropped start against.
    struct PendingBackend;

    impl crate::backend::CentralBackend for PendingBackend {
        fn adapter_state(&self) -> BackendFuture<'_, Result<AdapterState, BluetoothError>> {
            Box::pin(ready(Ok(AdapterState::On)))
        }
        fn state_changes(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<AdapterState>, BluetoothError>> {
            Box::pin(ready(Ok(broadcast::channel(32).1)))
        }
        fn start_scan(
            &self,
            _filter: ScanFilter,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ScanResultType>, BluetoothError>>
        {
            Box::pin(pending())
        }
        fn stop_scan(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(ready(Ok(())))
        }
        fn system_devices(
            &self,
            _service_uuids: &[Uuid],
        ) -> BackendFuture<'_, Result<Vec<DeviceInfo>, BluetoothError>> {
            Box::pin(ready(Ok(Vec::new())))
        }
        fn connect(&self, _id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(pending())
        }
        fn disconnect(&self, _id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(ready(Ok(())))
        }
        fn is_connected(&self, _id: &DeviceId) -> BackendFuture<'_, Result<bool, BluetoothError>> {
            Box::pin(ready(Ok(true)))
        }
        fn read_rssi(&self, _id: &DeviceId) -> BackendFuture<'_, Result<i8, BluetoothError>> {
            Box::pin(ready(Ok(-60)))
        }
        fn request_mtu(
            &self,
            _id: &DeviceId,
            _mtu: u16,
        ) -> BackendFuture<'_, Result<u16, BluetoothError>> {
            Box::pin(ready(Ok(185)))
        }
        fn discover_services(
            &self,
            _id: &DeviceId,
        ) -> BackendFuture<'_, Result<Vec<Service>, BluetoothError>> {
            Box::pin(ready(Ok(Vec::new())))
        }
        fn read_characteristic(
            &self,
            _id: &DeviceId,
            _characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            Box::pin(ready(Ok(Vec::new())))
        }
        fn write_characteristic(
            &self,
            _id: &DeviceId,
            _characteristic: &Characteristic,
            _data: &[u8],
            _write_type: WriteType,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(ready(Ok(())))
        }
        fn subscribe_characteristic(
            &self,
            _id: &DeviceId,
            _characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ValueNotification>, BluetoothError>>
        {
            Box::pin(ready(Ok(broadcast::channel(32).1)))
        }
        fn unsubscribe_characteristic(
            &self,
            _id: &DeviceId,
            _characteristic: &Characteristic,
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(ready(Ok(())))
        }
        fn read_descriptor(
            &self,
            _id: &DeviceId,
            _descriptor: &Descriptor,
        ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
            Box::pin(ready(Ok(Vec::new())))
        }
        fn write_descriptor(
            &self,
            _id: &DeviceId,
            _descriptor: &Descriptor,
            _data: &[u8],
        ) -> BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(ready(Ok(())))
        }
        fn connection_events(
            &self,
        ) -> BackendFuture<'_, Result<broadcast::Receiver<ConnectionEvent>, BluetoothError>>
        {
            Box::pin(ready(Ok(broadcast::channel(32).1)))
        }
    }

    #[tokio::test]
    async fn the_timeout_races_the_whole_connect() {
        let central = Central::new(Arc::new(PendingBackend));
        let device = central.device(&DeviceId::new("watch"));
        let error = device
            .connect(ConnectOptions {
                timeout: Some(Duration::from_millis(25)),
                max_attempts: 9,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_TIMEOUT");
        let error = device
            .connect(ConnectOptions {
                timeout: Some(Duration::ZERO),
                max_attempts: 1,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_TIMEOUT");
    }

    #[tokio::test]
    async fn cancelling_resolves_a_pending_connect() {
        let central = Central::new(Arc::new(PendingBackend));
        let device = central.device(&DeviceId::new("watch"));
        let task = tokio::spawn(async move {
            device
                .connect(ConnectOptions {
                    timeout: None,
                    max_attempts: 3,
                })
                .await
        });
        for _ in 0..1000 {
            if !task.is_finished() {
                tokio::task::yield_now().await;
            }
        }
        let pending_device = central.device(&DeviceId::new("watch"));
        pending_device.cancel_connect().unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_CANCELLED");
        // With nothing pending, the cancel is still Ok.
        pending_device.cancel_connect().unwrap();
    }

    #[tokio::test]
    async fn gatt_ops_guard_discovery_handles_and_connection() {
        let fake = FakeBackend::new();
        let central = Central::new(fake.clone());
        fake.add_scanned_device(watch_result("watch"));
        fake.set_services(&DeviceId::new("watch"), heart_rate_services());
        let device = central.device(&DeviceId::new("watch"));
        device.connect(ConnectOptions::default()).await.unwrap();

        let error = device
            .read_characteristic(&heart_rate_services()[0].characteristics[0])
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
        assert!(device.services().is_empty());
        let error = device.service(Uuid::parse("180D").unwrap()).unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");

        device.discover_services().await.unwrap();
        assert_eq!(device.services().len(), 1);
        let error = device.service(Uuid::parse("181A").unwrap()).unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICE_NOT_FOUND");
        let error = device
            .characteristic(Uuid::parse("FFFF").unwrap())
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
        assert!(device.characteristic(Uuid::parse("2A37").unwrap()).is_ok());

        let unknown = Characteristic {
            uuid: Uuid::parse("FFFF").unwrap(),
            service_uuid: Uuid::parse("180D").unwrap(),
            properties: CharacteristicProperties::default(),
            descriptors: Vec::new(),
            value: None,
        };
        let error = device.read_characteristic(&unknown).await.unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");

        fake.simulate_disconnect(&DeviceId::new("watch"));
        let error = device
            .read_characteristic(&heart_rate_services()[0].characteristics[0])
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_DISCONNECTED");
    }

    #[tokio::test]
    async fn discovery_is_cached_across_calls_and_handles() {
        let (fake, central, device) = connected_device().await;
        let again = central.device(&DeviceId::new("watch"));
        // The cache answers even with discovery scripted to fail.
        fake.fail_next(
            FakeOp::DiscoverServices,
            BluetoothError::new(ErrorKind::Backend),
        );
        let services = again.discover_services().await.unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(device.services().len(), 1);
    }

    #[tokio::test]
    async fn gatt_round_trip_and_subscriptions() {
        let (fake, _central, device) = connected_device().await;
        let characteristic = heart_rate_services()[0].characteristics[0].clone();
        assert_eq!(
            device.read_characteristic(&characteristic).await.unwrap(),
            vec![6, 66]
        );
        device
            .write_characteristic(&characteristic, &[7, 77], WriteType::WithResponse)
            .await
            .unwrap();
        assert_eq!(
            device.read_characteristic(&characteristic).await.unwrap(),
            vec![7, 77]
        );
        assert_eq!(device.read_rssi().await.unwrap(), -42);
        fake.set_mtu(247);
        assert_eq!(device.request_mtu(512).await.unwrap(), 247);

        let mut notifications = device
            .subscribe_characteristic(&characteristic)
            .await
            .unwrap();
        let mut fresh = device
            .subscribe_characteristic(&characteristic)
            .await
            .unwrap();
        fake.inject_notification(&DeviceId::new("watch"), characteristic.clone(), vec![9]);
        assert_eq!(notifications.recv().await.unwrap().value, vec![9]);
        assert_eq!(fresh.recv().await.unwrap().value, vec![9]);
        device
            .unsubscribe_characteristic(&characteristic)
            .await
            .unwrap();
        device
            .unsubscribe_characteristic(&characteristic)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn connection_events_flow_from_the_backend() {
        let (fake, central, _device) = connected_device().await;
        let mut events = central.connection_events().await.unwrap();
        fake.simulate_disconnect(&DeviceId::new("watch"));
        assert_eq!(
            events.recv().await.unwrap(),
            ConnectionEvent::Disconnected(DeviceId::new("watch"))
        );
    }

    #[tokio::test]
    async fn system_devices_pass_through() {
        let fake = FakeBackend::new();
        let central = Central::new(fake.clone());
        fake.add_scanned_device(watch_result("watch"));
        let devices = central.system_devices(&[]).await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, DeviceId::new("watch"));
    }
}
