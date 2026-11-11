//! Deterministic in-memory backends: the central and server roles with scripted one-shot
//! failures, an in-memory GATT db, and injectable events. Tests drive these; so can an app that
//! runs without hardware.

use std::{
    collections::{HashMap, VecDeque},
    future::ready,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use tokio::sync::broadcast;

use crate::backend::{
    AdapterState, AdvertiseOptions, BackendFuture, CentralBackend, Characteristic,
    CharacteristicProperties, ConnectionEvent, Descriptor, DeviceId, DeviceInfo, GattRequest,
    ScanFilter, ScanResult, ServerBackend, Service, Uuid, ValueNotification, WriteType,
};
use crate::error::{BluetoothError, ErrorKind};

/// Every stream the fakes hand out has this capacity, matching the platform streams.
const CHANNEL_CAPACITY: usize = 32;
/// The MTU `request_mtu` reports until `set_mtu` changes it.
const DEFAULT_MTU: u16 = 185;
/// The RSSI a device reports when it was registered without one.
const DEFAULT_RSSI: i8 = -60;

/// The central-role call a one-shot failure is armed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FakeOp {
    /// `adapter_state`.
    AdapterState,
    /// `state_changes`.
    StateChanges,
    /// `start_scan`.
    StartScan,
    /// `stop_scan`.
    StopScan,
    /// `system_devices`.
    SystemDevices,
    /// `connect`.
    Connect,
    /// `disconnect`.
    Disconnect,
    /// `is_connected`.
    IsConnected,
    /// `read_rssi`.
    ReadRssi,
    /// `request_mtu`.
    RequestMtu,
    /// `discover_services`.
    DiscoverServices,
    /// `read_characteristic`.
    ReadCharacteristic,
    /// `write_characteristic`.
    WriteCharacteristic,
    /// `subscribe_characteristic`.
    Subscribe,
    /// `unsubscribe_characteristic`.
    Unsubscribe,
    /// `read_descriptor`.
    ReadDescriptor,
    /// `write_descriptor`.
    WriteDescriptor,
    /// `connection_events`.
    ConnectionEvents,
}

/// The server-role call a one-shot failure is armed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerOp {
    /// `add_service`.
    AddService,
    /// `remove_service`.
    RemoveService,
    /// `start_advertising`.
    StartAdvertising,
    /// `stop_advertising`.
    StopAdvertising,
    /// `is_advertising`.
    IsAdvertising,
    /// `set_characteristic_value`.
    SetCharacteristicValue,
    /// `notify`.
    Notify,
    /// `requests`.
    Requests,
}

/// A central backend whose devices, values and failures the test scripts.
///
/// Devices appear through `add_scanned_device` or `set_services`; the GATT db lives in memory:
/// reads return the stored value, writes store it, and `request_mtu` reports the configured MTU.
pub struct FakeBackend {
    state: Mutex<FakeState>,
}

struct FakeState {
    adapter_state: AdapterState,
    state_tx: broadcast::Sender<AdapterState>,
    scanning: Option<ScanFilter>,
    scan_tx: broadcast::Sender<ScanResult>,
    devices: HashMap<DeviceId, FakeDevice>,
    connection_tx: broadcast::Sender<ConnectionEvent>,
    mtu: u16,
    fails: VecDeque<(FakeOp, BluetoothError)>,
}

struct FakeDevice {
    name: Option<String>,
    service_uuids: Vec<Uuid>,
    rssi: i8,
    connected: bool,
    services: Vec<Service>,
    subscriptions: HashMap<(Uuid, Uuid), broadcast::Sender<ValueNotification>>,
    descriptor_values: HashMap<(Uuid, Uuid, Uuid), Vec<u8>>,
}

impl Default for FakeDevice {
    fn default() -> Self {
        Self {
            name: None,
            service_uuids: Vec::new(),
            rssi: DEFAULT_RSSI,
            connected: false,
            services: Vec::new(),
            subscriptions: HashMap::new(),
            descriptor_values: HashMap::new(),
        }
    }
}

impl FakeDevice {
    fn from_scan(result: &ScanResult) -> Self {
        Self {
            name: result.name.clone(),
            service_uuids: result.advertisement.service_uuids.clone(),
            rssi: result.rssi.unwrap_or(DEFAULT_RSSI),
            ..Self::default()
        }
    }
}

impl FakeState {
    /// Pops the first armed failure for `op`, if one waits.
    fn take_fail(&mut self, op: FakeOp) -> Option<BluetoothError> {
        let index = self.fails.iter().position(|(what, _)| *what == op)?;
        self.fails.remove(index).map(|(_, error)| error)
    }

    fn connected_device(&self, id: &DeviceId) -> Result<&FakeDevice, BluetoothError> {
        match self.devices.get(id) {
            Some(device) if device.connected => Ok(device),
            Some(_) => Err(BluetoothError::new(ErrorKind::DeviceNotConnected)),
            None => Err(BluetoothError::new(ErrorKind::DeviceNotFound)),
        }
    }
}

fn find_characteristic<'a>(
    device: &'a FakeDevice,
    characteristic: &Characteristic,
) -> Option<&'a Characteristic> {
    device
        .services
        .iter()
        .find(|service| service.uuid == characteristic.service_uuid)?
        .characteristics
        .iter()
        .find(|known| known.uuid == characteristic.uuid)
}

fn has_descriptor(device: &FakeDevice, descriptor: &Descriptor) -> bool {
    device
        .services
        .iter()
        .find(|service| service.uuid == descriptor.service_uuid)
        .is_some_and(|service| {
            service
                .characteristics
                .iter()
                .find(|known| known.uuid == descriptor.characteristic_uuid)
                .is_some_and(|known| {
                    known
                        .descriptors
                        .iter()
                        .any(|candidate| candidate.uuid == descriptor.uuid)
                })
        })
}

fn characteristic_not_found() -> BluetoothError {
    BluetoothError::new(ErrorKind::CharacteristicNotFound)
}

fn descriptor_not_found() -> BluetoothError {
    BluetoothError::new(ErrorKind::DescriptorNotFound)
}

impl FakeBackend {
    /// A backend with the adapter on, nothing scanning, no devices and no services.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(FakeState {
                adapter_state: AdapterState::On,
                state_tx: broadcast::channel(CHANNEL_CAPACITY).0,
                scanning: None,
                scan_tx: broadcast::channel(CHANNEL_CAPACITY).0,
                devices: HashMap::new(),
                connection_tx: broadcast::channel(CHANNEL_CAPACITY).0,
                mtu: DEFAULT_MTU,
                fails: VecDeque::new(),
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets the adapter state and reports it through `state_changes`.
    pub fn set_adapter_state(&self, adapter_state: AdapterState) {
        let mut state = self.lock();
        state.adapter_state = adapter_state;
        let _ = state.state_tx.send(adapter_state);
    }

    /// Registers the device, and delivers the result to a running scan whose filter matches.
    pub fn add_scanned_device(&self, result: ScanResult) {
        let mut state = self.lock();
        let device = state
            .devices
            .entry(result.id.clone())
            .or_insert_with(|| FakeDevice::from_scan(&result));
        if result.name.is_some() {
            device.name = result.name.clone();
        }
        if let Some(rssi) = result.rssi {
            device.rssi = rssi;
        }
        if matches!(&state.scanning, Some(filter) if filter.matches(&result)) {
            let _ = state.scan_tx.send(result);
        }
    }

    /// Sets the device's GATT db, registering the device when needed.
    pub fn set_services(&self, id: &DeviceId, services: Vec<Service>) {
        self.lock().devices.entry(id.clone()).or_default().services = services;
    }

    /// Sets the MTU `request_mtu` reports.
    pub fn set_mtu(&self, mtu: u16) {
        self.lock().mtu = mtu;
    }

    /// Makes the next call of `op` fail with `error`, once. Arming the same op again stacks the
    /// failures; each fires in the order it was armed.
    pub fn fail_next(&self, op: FakeOp, error: BluetoothError) {
        self.lock().fails.push_back((op, error));
    }

    /// Marks the device disconnected and emits `ConnectionEvent::Disconnected` for it.
    pub fn simulate_disconnect(&self, id: &DeviceId) {
        let mut state = self.lock();
        if let Some(device) = state.devices.get_mut(id) {
            device.connected = false;
        }
        let _ = state
            .connection_tx
            .send(ConnectionEvent::Disconnected(id.clone()));
    }

    /// Sends a value to the current subscribers of the device's characteristic.
    pub fn inject_notification(
        &self,
        id: &DeviceId,
        characteristic: Characteristic,
        value: Vec<u8>,
    ) {
        let state = self.lock();
        if let Some(device) = state.devices.get(id) {
            let subscription = device.subscriptions.get(&(
                characteristic.service_uuid.clone(),
                characteristic.uuid.clone(),
            ));
            if let Some(sender) = subscription {
                let _ = sender.send(ValueNotification {
                    characteristic,
                    value,
                });
            }
        }
    }

    /// Emits a connection event as the platform would.
    pub fn inject_connection_event(&self, event: ConnectionEvent) {
        let _ = self.lock().connection_tx.send(event);
    }
}

impl CentralBackend for FakeBackend {
    fn adapter_state(&self) -> BackendFuture<'_, Result<AdapterState, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::AdapterState) {
            return Box::pin(ready(Err(error)));
        }
        Box::pin(ready(Ok(state.adapter_state)))
    }

    fn state_changes(
        &self,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<AdapterState>, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::StateChanges) {
            return Box::pin(ready(Err(error)));
        }
        Box::pin(ready(Ok(state.state_tx.subscribe())))
    }

    fn start_scan(
        &self,
        filter: ScanFilter,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<ScanResult>, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::StartScan) {
            return Box::pin(ready(Err(error)));
        }
        if state.scanning.is_some() {
            return Box::pin(ready(Err(BluetoothError::new(ErrorKind::InProgress))));
        }
        let (scan_tx, receiver) = broadcast::channel(CHANNEL_CAPACITY);
        state.scanning = Some(filter);
        state.scan_tx = scan_tx;
        Box::pin(ready(Ok(receiver)))
    }

    fn stop_scan(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::StopScan) {
            return Box::pin(ready(Err(error)));
        }
        state.scanning = None;
        // A fresh channel closes the old receivers, ending their streams.
        state.scan_tx = broadcast::channel(CHANNEL_CAPACITY).0;
        Box::pin(ready(Ok(())))
    }

    fn system_devices(
        &self,
        service_uuids: &[Uuid],
    ) -> BackendFuture<'_, Result<Vec<DeviceInfo>, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::SystemDevices) {
            return Box::pin(ready(Err(error)));
        }
        let devices = state
            .devices
            .iter()
            .filter(|(_, device)| {
                service_uuids.is_empty()
                    || device
                        .service_uuids
                        .iter()
                        .any(|uuid| service_uuids.contains(uuid))
            })
            .map(|(id, device)| DeviceInfo {
                id: id.clone(),
                name: device.name.clone(),
            })
            .collect();
        Box::pin(ready(Ok(devices)))
    }

    fn connect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::Connect) {
            return Box::pin(ready(Err(error)));
        }
        let device = match state.devices.get_mut(id) {
            Some(device) => device,
            None => return Box::pin(ready(Err(BluetoothError::new(ErrorKind::DeviceNotFound)))),
        };
        if device.connected {
            return Box::pin(ready(Err(BluetoothError::new(ErrorKind::AlreadyConnected))));
        }
        device.connected = true;
        Box::pin(ready(Ok(())))
    }

    fn disconnect(&self, id: &DeviceId) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::Disconnect) {
            return Box::pin(ready(Err(error)));
        }
        if let Some(device) = state.devices.get_mut(id) {
            device.connected = false;
        }
        Box::pin(ready(Ok(())))
    }

    fn is_connected(&self, id: &DeviceId) -> BackendFuture<'_, Result<bool, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::IsConnected) {
            return Box::pin(ready(Err(error)));
        }
        let connected = state.devices.get(id).is_some_and(|device| device.connected);
        Box::pin(ready(Ok(connected)))
    }

    fn read_rssi(&self, id: &DeviceId) -> BackendFuture<'_, Result<i8, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::ReadRssi) {
            return Box::pin(ready(Err(error)));
        }
        let answer = match state.connected_device(id) {
            Ok(device) => Ok(device.rssi),
            Err(error) => Err(error),
        };
        Box::pin(ready(answer))
    }

    fn request_mtu(
        &self,
        id: &DeviceId,
        _mtu: u16,
    ) -> BackendFuture<'_, Result<u16, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::RequestMtu) {
            return Box::pin(ready(Err(error)));
        }
        let answer = match state.connected_device(id) {
            Ok(_) => Ok(state.mtu),
            Err(error) => Err(error),
        };
        Box::pin(ready(answer))
    }

    fn discover_services(
        &self,
        id: &DeviceId,
    ) -> BackendFuture<'_, Result<Vec<Service>, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::DiscoverServices) {
            return Box::pin(ready(Err(error)));
        }
        let answer = match state.connected_device(id) {
            Ok(device) => Ok(device.services.clone()),
            Err(error) => Err(error),
        };
        Box::pin(ready(answer))
    }

    fn read_characteristic(
        &self,
        id: &DeviceId,
        characteristic: &Characteristic,
    ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::ReadCharacteristic) {
            return Box::pin(ready(Err(error)));
        }
        let answer = match state.connected_device(id) {
            Ok(device) => match find_characteristic(device, characteristic) {
                Some(known) => Ok(known.value.clone().unwrap_or_default()),
                None => Err(characteristic_not_found()),
            },
            Err(error) => Err(error),
        };
        Box::pin(ready(answer))
    }

    fn write_characteristic(
        &self,
        id: &DeviceId,
        characteristic: &Characteristic,
        data: &[u8],
        _write_type: WriteType,
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::WriteCharacteristic) {
            return Box::pin(ready(Err(error)));
        }
        let device = match state.devices.get_mut(id) {
            Some(device) => device,
            None => return Box::pin(ready(Err(BluetoothError::new(ErrorKind::DeviceNotFound)))),
        };
        if !device.connected {
            return Box::pin(ready(Err(BluetoothError::new(
                ErrorKind::DeviceNotConnected,
            ))));
        }
        let known = device
            .services
            .iter_mut()
            .find(|service| service.uuid == characteristic.service_uuid)
            .and_then(|service| {
                service
                    .characteristics
                    .iter_mut()
                    .find(|known| known.uuid == characteristic.uuid)
            });
        match known {
            Some(known) => {
                known.value = Some(data.to_vec());
                Box::pin(ready(Ok(())))
            }
            None => Box::pin(ready(Err(characteristic_not_found()))),
        }
    }

    fn subscribe_characteristic(
        &self,
        id: &DeviceId,
        characteristic: &Characteristic,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<ValueNotification>, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::Subscribe) {
            return Box::pin(ready(Err(error)));
        }
        let device = match state.devices.get_mut(id) {
            Some(device) => device,
            None => return Box::pin(ready(Err(BluetoothError::new(ErrorKind::DeviceNotFound)))),
        };
        if !device.connected {
            return Box::pin(ready(Err(BluetoothError::new(
                ErrorKind::DeviceNotConnected,
            ))));
        }
        if find_characteristic(device, characteristic).is_none() {
            return Box::pin(ready(Err(characteristic_not_found())));
        }
        let subscription = device
            .subscriptions
            .entry((
                characteristic.service_uuid.clone(),
                characteristic.uuid.clone(),
            ))
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0);
        Box::pin(ready(Ok(subscription.subscribe())))
    }

    fn unsubscribe_characteristic(
        &self,
        id: &DeviceId,
        characteristic: &Characteristic,
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::Unsubscribe) {
            return Box::pin(ready(Err(error)));
        }
        if let Some(device) = state.devices.get_mut(id) {
            let subscription = (
                characteristic.service_uuid.clone(),
                characteristic.uuid.clone(),
            );
            device.subscriptions.remove(&subscription);
        }
        Box::pin(ready(Ok(())))
    }

    fn read_descriptor(
        &self,
        id: &DeviceId,
        descriptor: &Descriptor,
    ) -> BackendFuture<'_, Result<Vec<u8>, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::ReadDescriptor) {
            return Box::pin(ready(Err(error)));
        }
        let answer = match state.connected_device(id) {
            Ok(device) if has_descriptor(device, descriptor) => {
                let key = (
                    descriptor.service_uuid.clone(),
                    descriptor.characteristic_uuid.clone(),
                    descriptor.uuid.clone(),
                );
                Ok(device
                    .descriptor_values
                    .get(&key)
                    .cloned()
                    .unwrap_or_default())
            }
            Ok(_) => Err(descriptor_not_found()),
            Err(error) => Err(error),
        };
        Box::pin(ready(answer))
    }

    fn write_descriptor(
        &self,
        id: &DeviceId,
        descriptor: &Descriptor,
        data: &[u8],
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::WriteDescriptor) {
            return Box::pin(ready(Err(error)));
        }
        let answer = match state.connected_device(id) {
            Ok(device) if has_descriptor(device, descriptor) => Ok(()),
            Ok(_) => Err(descriptor_not_found()),
            Err(error) => Err(error),
        };
        if answer.is_err() {
            return Box::pin(ready(answer));
        }
        let key = (
            descriptor.service_uuid.clone(),
            descriptor.characteristic_uuid.clone(),
            descriptor.uuid.clone(),
        );
        state
            .devices
            .get_mut(id)
            .map(|device| device.descriptor_values.insert(key, data.to_vec()));
        Box::pin(ready(Ok(())))
    }

    fn connection_events(
        &self,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<ConnectionEvent>, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(FakeOp::ConnectionEvents) {
            return Box::pin(ready(Err(error)));
        }
        Box::pin(ready(Ok(state.connection_tx.subscribe())))
    }
}

/// A server backend whose services, advertising and failures the test scripts.
///
/// A simulated central subscribes to every characteristic with notify or indicate properties;
/// `subscriber_count` reports it, and `notify` fails with `CharacteristicNotifyChangeFailed` for
/// characteristics it never subscribed to.
pub struct FakeServerBackend {
    state: Mutex<ServerState>,
}

struct ServerState {
    services: Vec<Service>,
    captured: Vec<(Uuid, Uuid, Vec<u8>)>,
    advertising: bool,
    request_tx: broadcast::Sender<GattRequest>,
    fails: VecDeque<(ServerOp, BluetoothError)>,
}

impl ServerState {
    fn take_fail(&mut self, op: ServerOp) -> Option<BluetoothError> {
        let index = self.fails.iter().position(|(what, _)| *what == op)?;
        self.fails.remove(index).map(|(_, error)| error)
    }

    fn characteristic(&self, service: &Uuid, characteristic: &Uuid) -> Result<(), BluetoothError> {
        let known = self
            .services
            .iter()
            .find(|known| &known.uuid == service)
            .ok_or_else(|| BluetoothError::new(ErrorKind::ServiceNotFound))?;
        if known
            .characteristics
            .iter()
            .any(|known| &known.uuid == characteristic)
        {
            Ok(())
        } else {
            Err(BluetoothError::new(ErrorKind::CharacteristicNotFound))
        }
    }

    /// The simulated central subscribes to notify- and indicate-capable characteristics.
    fn subscribed(&self, service: &Uuid, characteristic: &Uuid) -> bool {
        self.services
            .iter()
            .find(|known| &known.uuid == service)
            .and_then(|known| {
                known
                    .characteristics
                    .iter()
                    .find(|known| &known.uuid == characteristic)
            })
            .is_some_and(|known| {
                known.properties.contains(CharacteristicProperties::NOTIFY)
                    || known
                        .properties
                        .contains(CharacteristicProperties::INDICATE)
            })
    }
}

impl FakeServerBackend {
    /// A server with no services, not advertising, and no requests.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ServerState {
                services: Vec::new(),
                captured: Vec::new(),
                advertising: false,
                request_tx: broadcast::channel(CHANNEL_CAPACITY).0,
                fails: VecDeque::new(),
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, ServerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes the next call of `op` fail with `error`, once. Arming the same op again stacks the
    /// failures; each fires in the order it was armed.
    pub fn fail_next_server(&self, op: ServerOp, error: BluetoothError) {
        self.lock().fails.push_back((op, error));
    }

    /// Emits a request as a central would send it.
    pub fn inject_request(&self, request: GattRequest) {
        let _ = self.lock().request_tx.send(request);
    }

    /// The values set through `set_characteristic_value`, in call order.
    pub fn captured_values(&self) -> Vec<(Uuid, Uuid, Vec<u8>)> {
        self.lock().captured.clone()
    }

    /// How many centrals are subscribed to the characteristic: the simulated central for
    /// notify- or indicate-capable characteristics, none otherwise.
    pub fn subscriber_count(&self, service: Uuid, characteristic: Uuid) -> usize {
        let state = self.lock();
        usize::from(state.subscribed(&service, &characteristic))
    }
}

impl ServerBackend for FakeServerBackend {
    fn add_service(&self, service: Service) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(ServerOp::AddService) {
            return Box::pin(ready(Err(error)));
        }
        if state
            .services
            .iter()
            .any(|known| known.uuid == service.uuid)
        {
            return Box::pin(ready(Err(BluetoothError::new(ErrorKind::ServerFailed))));
        }
        state.services.push(service);
        Box::pin(ready(Ok(())))
    }

    fn remove_service(&self, uuid: Uuid) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(ServerOp::RemoveService) {
            return Box::pin(ready(Err(error)));
        }
        state.services.retain(|known| known.uuid != uuid);
        Box::pin(ready(Ok(())))
    }

    fn start_advertising(
        &self,
        _options: AdvertiseOptions,
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(ServerOp::StartAdvertising) {
            return Box::pin(ready(Err(error)));
        }
        if state.advertising {
            return Box::pin(ready(Err(BluetoothError::new(ErrorKind::InProgress))));
        }
        state.advertising = true;
        Box::pin(ready(Ok(())))
    }

    fn stop_advertising(&self) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(ServerOp::StopAdvertising) {
            return Box::pin(ready(Err(error)));
        }
        state.advertising = false;
        Box::pin(ready(Ok(())))
    }

    fn is_advertising(&self) -> BackendFuture<'_, Result<bool, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(ServerOp::IsAdvertising) {
            return Box::pin(ready(Err(error)));
        }
        Box::pin(ready(Ok(state.advertising)))
    }

    fn set_characteristic_value(
        &self,
        service: Uuid,
        characteristic: Uuid,
        value: Vec<u8>,
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(ServerOp::SetCharacteristicValue) {
            return Box::pin(ready(Err(error)));
        }
        if let Err(error) = state.characteristic(&service, &characteristic) {
            return Box::pin(ready(Err(error)));
        }
        state.captured.push((service, characteristic, value));
        Box::pin(ready(Ok(())))
    }

    fn notify(
        &self,
        service: Uuid,
        characteristic: Uuid,
        _data: &[u8],
    ) -> BackendFuture<'_, Result<(), BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(ServerOp::Notify) {
            return Box::pin(ready(Err(error)));
        }
        if let Err(error) = state.characteristic(&service, &characteristic) {
            return Box::pin(ready(Err(error)));
        }
        if !state.subscribed(&service, &characteristic) {
            return Box::pin(ready(Err(BluetoothError::new(
                ErrorKind::CharacteristicNotifyChangeFailed,
            ))));
        }
        Box::pin(ready(Ok(())))
    }

    fn requests(
        &self,
    ) -> BackendFuture<'_, Result<broadcast::Receiver<GattRequest>, BluetoothError>> {
        let mut state = self.lock();
        if let Some(error) = state.take_fail(ServerOp::Requests) {
            return Box::pin(ready(Err(error)));
        }
        Box::pin(ready(Ok(state.request_tx.subscribe())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{AdvertisementData, GattRequestKind};

    fn heart_rate_services() -> Vec<Service> {
        vec![Service {
            uuid: Uuid::parse("180D").unwrap(),
            primary: true,
            characteristics: vec![Characteristic {
                uuid: Uuid::parse("2A37").unwrap(),
                service_uuid: Uuid::parse("180D").unwrap(),
                properties: CharacteristicProperties::NOTIFY,
                descriptors: vec![Descriptor {
                    uuid: Uuid::parse("2902").unwrap(),
                    characteristic_uuid: Uuid::parse("2A37").unwrap(),
                    service_uuid: Uuid::parse("180D").unwrap(),
                }],
                value: Some(vec![6, 66]),
            }],
            included_services: Vec::new(),
        }]
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
    async fn the_adapter_state_is_on_and_changes_are_reported() {
        let fake = FakeBackend::new();
        assert_eq!(fake.adapter_state().await.unwrap(), AdapterState::On);
        let mut changes = fake.state_changes().await.unwrap();
        fake.set_adapter_state(AdapterState::PoweredOff);
        assert_eq!(changes.recv().await.unwrap(), AdapterState::PoweredOff);
    }

    #[tokio::test]
    async fn scan_results_arrive_only_while_scanning_and_only_on_a_match() {
        let fake = FakeBackend::new();
        let filter = ScanFilter {
            service_uuids: vec![Uuid::parse("180D").unwrap()],
            device_ids: Vec::new(),
        };
        fake.add_scanned_device(scan_result("before", "180D"));
        let mut results = fake.start_scan(filter).await.unwrap();
        fake.add_scanned_device(scan_result("watch", "180D"));
        fake.add_scanned_device(scan_result("other", "181A"));
        assert_eq!(results.recv().await.unwrap().id, DeviceId::new("watch"));
        fake.stop_scan().await.unwrap();
        fake.add_scanned_device(scan_result("after", "180D"));
        assert!(results.recv().await.is_err());
    }

    #[tokio::test]
    async fn a_second_scan_is_rejected_until_stopped() {
        let fake = FakeBackend::new();
        fake.start_scan(ScanFilter::default()).await.unwrap();
        let error = fake.start_scan(ScanFilter::default()).await.unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_IN_PROGRESS");
        fake.stop_scan().await.unwrap();
        assert!(fake.start_scan(ScanFilter::default()).await.is_ok());
    }

    #[tokio::test]
    async fn connect_disconnect_and_gatt_round_trip() {
        let fake = FakeBackend::new();
        let id = DeviceId::new("watch");
        fake.set_services(&id, heart_rate_services());
        let characteristic = Characteristic {
            uuid: Uuid::parse("2A37").unwrap(),
            service_uuid: Uuid::parse("180D").unwrap(),
            properties: CharacteristicProperties::READ,
            descriptors: Vec::new(),
            value: None,
        };

        let error = fake
            .read_characteristic(&id, &characteristic)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_DEVICE_NOT_CONNECTED");
        fake.connect(&id).await.unwrap();
        assert_eq!(
            fake.read_characteristic(&id, &characteristic)
                .await
                .unwrap(),
            vec![6, 66]
        );
        fake.write_characteristic(&id, &characteristic, &[7, 77], WriteType::WithResponse)
            .await
            .unwrap();
        assert_eq!(
            fake.read_characteristic(&id, &characteristic)
                .await
                .unwrap(),
            vec![7, 77]
        );
        let error = fake.connect(&id).await.unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_ALREADY_CONNECTED");
        fake.disconnect(&id).await.unwrap();
        fake.disconnect(&id).await.unwrap();
        assert!(!fake.is_connected(&id).await.unwrap());
    }

    #[tokio::test]
    async fn unknown_devices_and_handles_carry_their_codes() {
        let fake = FakeBackend::new();
        let stranger = DeviceId::new("stranger");
        assert_eq!(
            fake.connect(&stranger).await.unwrap_err().code(),
            "ERR_BLUETOOTH_DEVICE_NOT_FOUND"
        );
        let id = DeviceId::new("watch");
        fake.set_services(&id, heart_rate_services());
        fake.connect(&id).await.unwrap();
        let unknown = Characteristic {
            uuid: Uuid::parse("FFFF").unwrap(),
            service_uuid: Uuid::parse("180D").unwrap(),
            properties: CharacteristicProperties::default(),
            descriptors: Vec::new(),
            value: None,
        };
        assert_eq!(
            fake.read_characteristic(&id, &unknown)
                .await
                .unwrap_err()
                .code(),
            "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND"
        );
        let descriptor = Descriptor {
            uuid: Uuid::parse("2902").unwrap(),
            characteristic_uuid: Uuid::parse("2A37").unwrap(),
            service_uuid: Uuid::parse("180D").unwrap(),
        };
        fake.write_descriptor(&id, &descriptor, &[1, 0])
            .await
            .unwrap();
        assert_eq!(
            fake.read_descriptor(&id, &descriptor).await.unwrap(),
            vec![1, 0]
        );
        fake.disconnect(&id).await.unwrap();
        assert_eq!(
            fake.read_descriptor(&id, &descriptor)
                .await
                .unwrap_err()
                .code(),
            "ERR_BLUETOOTH_DEVICE_NOT_CONNECTED"
        );
        let missing = Descriptor {
            uuid: Uuid::parse("2901").unwrap(),
            characteristic_uuid: Uuid::parse("2A37").unwrap(),
            service_uuid: Uuid::parse("180D").unwrap(),
        };
        fake.connect(&id).await.unwrap();
        assert_eq!(
            fake.read_descriptor(&id, &missing)
                .await
                .unwrap_err()
                .code(),
            "ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND"
        );
    }

    #[tokio::test]
    async fn rssi_and_mtu_come_from_the_script() {
        let fake = FakeBackend::new();
        let id = DeviceId::new("watch");
        fake.add_scanned_device(scan_result("watch", "180D"));
        assert_eq!(
            fake.read_rssi(&id).await.unwrap_err().code(),
            "ERR_BLUETOOTH_DEVICE_NOT_CONNECTED"
        );
        fake.connect(&id).await.unwrap();
        assert_eq!(fake.read_rssi(&id).await.unwrap(), -42);
        assert_eq!(fake.request_mtu(&id, 512).await.unwrap(), 185);
        fake.set_mtu(247);
        assert_eq!(fake.request_mtu(&id, 512).await.unwrap(), 247);
    }

    #[tokio::test]
    async fn subscriptions_receive_injected_notifications_and_unsubscribe_is_ok() {
        let fake = FakeBackend::new();
        let id = DeviceId::new("watch");
        let services = heart_rate_services();
        let characteristic = services[0].characteristics[0].clone();
        fake.set_services(&id, services);
        fake.connect(&id).await.unwrap();
        let mut first = fake
            .subscribe_characteristic(&id, &characteristic)
            .await
            .unwrap();
        let mut second = fake
            .subscribe_characteristic(&id, &characteristic)
            .await
            .unwrap();
        fake.inject_notification(&id, characteristic.clone(), vec![1, 2, 3]);
        assert_eq!(first.recv().await.unwrap().value, vec![1, 2, 3]);
        assert_eq!(second.recv().await.unwrap().value, vec![1, 2, 3]);
        fake.unsubscribe_characteristic(&id, &characteristic)
            .await
            .unwrap();
        fake.unsubscribe_characteristic(&id, &characteristic)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_simulated_disconnect_marks_and_emits() {
        let fake = FakeBackend::new();
        let id = DeviceId::new("watch");
        fake.set_services(&id, Vec::new());
        fake.connect(&id).await.unwrap();
        let mut events = fake.connection_events().await.unwrap();
        fake.simulate_disconnect(&id);
        assert!(!fake.is_connected(&id).await.unwrap());
        assert_eq!(
            events.recv().await.unwrap(),
            ConnectionEvent::Disconnected(id.clone())
        );
    }

    #[tokio::test]
    async fn injected_connection_events_reach_the_stream() {
        let fake = FakeBackend::new();
        let mut events = fake.connection_events().await.unwrap();
        let connected = ConnectionEvent::Connected(DeviceId::new("watch"));
        fake.inject_connection_event(connected.clone());
        assert_eq!(events.recv().await.unwrap(), connected);
    }

    #[tokio::test]
    async fn armed_failures_fire_once_each_in_order() {
        let fake = FakeBackend::new();
        fake.fail_next(
            FakeOp::AdapterState,
            BluetoothError::new(ErrorKind::Backend),
        );
        fake.fail_next(
            FakeOp::AdapterState,
            BluetoothError::new(ErrorKind::PoweredOff),
        );
        assert_eq!(
            fake.adapter_state().await.unwrap_err().code(),
            "ERR_BLUETOOTH_BACKEND"
        );
        assert_eq!(
            fake.adapter_state().await.unwrap_err().code(),
            "ERR_BLUETOOTH_POWERED_OFF"
        );
        assert_eq!(fake.adapter_state().await.unwrap(), AdapterState::On);
    }

    #[tokio::test]
    async fn system_devices_filter_by_service() {
        let fake = FakeBackend::new();
        fake.add_scanned_device(scan_result("watch", "180D"));
        fake.add_scanned_device(scan_result("sensor", "181A"));
        let all = fake.system_devices(&[]).await.unwrap();
        assert_eq!(all.len(), 2);
        let heart = fake
            .system_devices(&[Uuid::parse("180D").unwrap()])
            .await
            .unwrap();
        assert_eq!(heart.len(), 1);
        assert_eq!(heart[0].id, DeviceId::new("watch"));
        assert_eq!(heart[0].name, Some("watch".to_owned()));
    }

    #[tokio::test]
    async fn the_server_serves_and_advertises() {
        let server = FakeServerBackend::new();
        let heart = heart_rate_services().remove(0);
        server.add_service(heart.clone()).await.unwrap();
        let error = server.add_service(heart).await.unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_SERVER_FAILED");
        let battery = Service {
            uuid: Uuid::parse("180F").unwrap(),
            primary: true,
            characteristics: Vec::new(),
            included_services: Vec::new(),
        };
        server.add_service(battery).await.unwrap();
        server
            .remove_service(Uuid::parse("180F").unwrap())
            .await
            .unwrap();
        server
            .remove_service(Uuid::parse("180F").unwrap())
            .await
            .unwrap();

        assert!(!server.is_advertising().await.unwrap());
        server
            .start_advertising(AdvertiseOptions::default())
            .await
            .unwrap();
        let error = server
            .start_advertising(AdvertiseOptions::default())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_IN_PROGRESS");
        assert!(server.is_advertising().await.unwrap());
        server.stop_advertising().await.unwrap();
        server.stop_advertising().await.unwrap();
        assert!(!server.is_advertising().await.unwrap());
    }

    #[tokio::test]
    async fn notify_needs_a_known_handle_and_a_subscriber() {
        let server = FakeServerBackend::new();
        let service = Uuid::parse("180D").unwrap();
        let measurement = Uuid::parse("2A37").unwrap();
        server
            .add_service(heart_rate_services().remove(0))
            .await
            .unwrap();

        assert_eq!(
            server
                .notify(Uuid::parse("181A").unwrap(), measurement.clone(), &[1])
                .await
                .unwrap_err()
                .code(),
            "ERR_BLUETOOTH_SERVICE_NOT_FOUND"
        );
        assert_eq!(
            server
                .notify(service.clone(), Uuid::parse("FFFF").unwrap(), &[1])
                .await
                .unwrap_err()
                .code(),
            "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND"
        );
        // 2A37 is notify-capable, so the simulated central is subscribed and the notify lands.
        server.notify(service, measurement, &[1]).await.unwrap();
        // A plain read characteristic has no subscriber.
        let battery = Service {
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
        };
        server.add_service(battery).await.unwrap();
        assert_eq!(
            server
                .notify(
                    Uuid::parse("180F").unwrap(),
                    Uuid::parse("2A19").unwrap(),
                    &[1]
                )
                .await
                .unwrap_err()
                .code(),
            "ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED"
        );
    }

    #[tokio::test]
    async fn set_characteristic_value_captures_and_finds_handles() {
        let server = FakeServerBackend::new();
        let service = Uuid::parse("180D").unwrap();
        let measurement = Uuid::parse("2A37").unwrap();
        server
            .add_service(heart_rate_services().remove(0))
            .await
            .unwrap();
        let error = server
            .set_characteristic_value(Uuid::parse("181A").unwrap(), measurement.clone(), vec![1])
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICE_NOT_FOUND");
        let error = server
            .set_characteristic_value(service.clone(), Uuid::parse("FFFF").unwrap(), vec![1])
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
        server
            .set_characteristic_value(service.clone(), measurement.clone(), vec![1, 2])
            .await
            .unwrap();
        assert_eq!(
            server.captured_values(),
            vec![(service, measurement, vec![1, 2])]
        );
    }

    #[tokio::test]
    async fn subscriber_count_follows_the_notify_property() {
        let server = FakeServerBackend::new();
        let service = Uuid::parse("180D").unwrap();
        let measurement = Uuid::parse("2A37").unwrap();
        assert_eq!(
            server.subscriber_count(service.clone(), measurement.clone()),
            0
        );
        server
            .add_service(heart_rate_services().remove(0))
            .await
            .unwrap();
        assert_eq!(server.subscriber_count(service.clone(), measurement), 1);
        assert_eq!(
            server.subscriber_count(service, Uuid::parse("FFFF").unwrap()),
            0
        );
    }

    #[tokio::test]
    async fn requests_reach_every_receiver() {
        let server = FakeServerBackend::new();
        let mut requests = server.requests().await.unwrap();
        let request = GattRequest {
            service_uuid: Uuid::parse("180D").unwrap(),
            characteristic_uuid: Uuid::parse("2A37").unwrap(),
            kind: GattRequestKind::Write {
                offset: 0,
                data: vec![9],
                with_response: true,
            },
        };
        server.inject_request(request);
        assert!(requests.recv().await.is_ok());
    }

    #[tokio::test]
    async fn armed_server_failures_fire_once() {
        let server = FakeServerBackend::new();
        server.fail_next_server(
            ServerOp::IsAdvertising,
            BluetoothError::new(ErrorKind::Backend),
        );
        assert_eq!(
            server.is_advertising().await.unwrap_err().code(),
            "ERR_BLUETOOTH_BACKEND"
        );
        assert!(!server.is_advertising().await.unwrap());
    }
}
