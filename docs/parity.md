# Parity

How the two reference libraries map onto this crate. The mapping is the target both halves implement; a row is not a claim that it is implemented today. What exists is what the README's Status section says CI runs, and a behavior named in a row is proven only by the test that names it (docs/goals.md).

Names read on 2026-11-09 from the clones in `.refs/` (decisions.md #1): flutter_blue_plus `1e3bd09`, public surface in `packages/flutter_blue_plus/lib/src/flutter_blue_plus.dart`, `bluetooth_device.dart`, `bluetooth_characteristic.dart`, `bluetooth_service.dart`, `bluetooth_descriptor.dart` and `bluetooth_events.dart`; react-native-ble-plx `92a496d`, public types in `src/index.d.ts`, behavior in `src/BleManager.js`, error codes in `src/BleError.js`.

Ours is named once per row, Rust first, TypeScript second: `Central::scan` / `central.scan`. One naming split is shipped, not a typo: Rust's GATT calls are `read_characteristic` / `write_characteristic` / `subscribe_characteristic` because they take a `&Characteristic` from the discovery cache, where TypeScript ships `device.read(uuid)` / `device.write(uuid, …)` / `device.subscribe(uuid)`. Connection events are `Central::connection_events` / `central.connectionEvents()`, a thin wrapper over the backend's stream, from the next event on.

## Table 1: flutter_blue_plus

| flutter_blue_plus | Ours | Behavior |
|---|---|---|
| `FlutterBluePlus.isSupported` | `Central::is_supported` / `central.isSupported` | Same: false where there is no usable stack. |
| `FlutterBluePlus.adapterStateNow` | `Central::state` / `central.state` | Same: a one-shot read; `Unknown` until the stack reports. |
| `FlutterBluePlus.adapterState` (stream) | `Central::state_changes` / `central.stateChanges` | Different: ours starts at the next change, so `state()` reads the current one (`state_stream_reports_resetting_and_unknown`). A lagged receiver resynchronises instead of failing (decisions.md #8). |
| `FlutterBluePlus.adapterName` | not modeled | The adapter's own name is not needed to scan, connect or serve. |
| `FlutterBluePlus.isScanning` / `isScanningNow` | not modeled | The scan stream is the truth: while it lives, a scan runs, and a second `scan` gets `ERR_BLUETOOTH_IN_PROGRESS` (`second_scan_fails_in_progress`). |
| `FlutterBluePlus.lastScanResults` / `scanResults` / `onScanResults` | `Central::scan` / `central.scan` | Different: ours emits each matching advertisement as its own item, not a growing deduplicated list, and re-listening does not re-emit. |
| `FlutterBluePlus.logs` / `setLogLevel` | not modeled | No logging surface in the API. |
| `FlutterBluePlus.setOperationQueueMode` | not modeled | No operation queue; every call is its own future. The only global rules are one scan and one advertisement at a time. |
| `FlutterBluePlus.setOptions` (`showPowerAlert`, `restoreState`) | not modeled | iOS-only options; NSH owns the app's lifecycle and restoration. |
| `FlutterBluePlus.events` (`BluetoothEvents`) | `Central::state_changes`, `Central::connection_events` / `central.stateChanges`, `central.connectionEvents()` | Different: the two streams apps act on. Connection events are one stream for the whole central, each event tagged with the device id, from the next event on. FBP's other event getters surface in ours as per-call results, not as separate streams. |
| `FlutterBluePlus.startScan(withServices, withRemoteIds, …)` | `Central::scan(&ScanFilter)` / `central.scan(filter)` | Different: filters are service uuids and device ids (`scan_filter_matches_service_and_device`); FBP's name, keyword, manufacturer-data and service-data filters and its continuous and one-by-one modes are not modeled. A second scan fails `ERR_BLUETOOTH_IN_PROGRESS` instead of replacing the running one. |
| `FlutterBluePlus.stopScan` | `ScanStream::stop()` / `stream.stop()`, or `central.stopScan()` | Same: an explicit stop, fine when none runs. Dropping the Rust `ScanStream` stops the scan too; in TypeScript, ending iteration alone does not — the backend scans until `stop()` runs. |
| `FlutterBluePlus.cancelWhenScanComplete` | not applicable | The caller owns its own subscriptions. |
| `FlutterBluePlus.turnOn` | not modeled | No adapter-power call exists in `CentralBackend` or in either half; the app tells the user to turn Bluetooth on. `ERR_BLUETOOTH_STATE_CHANGE_FAILED` is reserved for the day one lands. |
| `FlutterBluePlus.turnOff` | not modeled | Same: no adapter-power call. FBP deprecated it after Android SDK 33 anyway. |
| `FlutterBluePlus.connectedDevices` | not modeled | The app tracks its own `Device` handles; `system_devices` lists the system's devices, not the app's connections. |
| `FlutterBluePlus.systemDevices(withServices)` | `Central::system_devices(&[Uuid])` / `central.systemDevices(uuids)` | Same: the devices the system already knows, filtered by service uuids when the list is not empty; returns `DeviceInfo` (id and name), and `central.device(id)` makes the handle to connect. |
| `FlutterBluePlus.bondedDevices` | not modeled | No Android bond management. |
| `FlutterBluePlus.getPhySupport` | not modeled | No PHY API. |
| FBP's deprecated statics (`isOn`, `name`, `state`, `instance`, `isAvailable`, `connectedSystemDevices`, `scan`) | none | Dropped, as FBP dropped them. |
| `BluetoothDevice(remoteId)` / `BluetoothDevice.fromId` | `Central::device(id)` / `central.device(id)` | Same: the handle is cheap and unconnected; `Device::connect(options)` does the connecting. Ids come from scan results or `system_devices`. |
| `BluetoothDevice.remoteId` | `Device::id` / `device.id` | Same. |
| `BluetoothDevice.platformName` / `advName` | fields on the scan advertisement | Different: no name cache on the handle; the advertisement carries the name the device sent. |
| `BluetoothDevice.servicesList` | `Device::services` / `device.services` | Same list when discovered, empty before. A GATT call before discovery is `ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED`, not an empty list (`gatt_before_discovery_fails`). |
| `BluetoothDevice.isConnected` / `isDisconnected` | `Device::is_connected` / `device.isConnected` | Same. |
| `BluetoothDevice.connect(license, timeout, mtu, autoConnect)` | `Device::connect(ConnectOptions)` / `device.connect(options)` | Different: no license argument. `timeout` is `ConnectOptions::timeout` (`connect_timeout_resolves_once`); Android 133 is retried up to `ConnectOptions::max_attempts` (`connect_retries_android_133`); a larger mtu is asked for with `request_mtu` after connecting; `autoConnect` is not modeled; `cancel_connect` cancels. |
| `BluetoothDevice.disconnect(queue, timeout, androidDelay)` | `Device::disconnect` / `device.disconnect` | Different: idempotent — `Ok` when not connected (`disconnect_is_idempotent`); a pending connect is cancelled by `cancel_connect`, not by `disconnect`; no queue or delay knobs. |
| `BluetoothDevice.discoverServices` | `Device::discover_services` / `device.discoverServices` | Same: re-call after every connection; failure `ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED`. FBP's automatic subscribe to Services Changed (0x2A05) is not modeled. |
| `BluetoothDevice.disconnectReason` | `Central::connection_events` / `central.connectionEvents()` | Different: the event says `Disconnected` and names the device; the raw reason is not surfaced, and a failed operation keeps the platform code in `status`. |
| `BluetoothDevice.connectionState` | `Central::connection_events` / `central.connectionEvents()` | Different: one stream for every device of the central, each event tagged with the id, from the next event on; FBP gives a per-device stream that starts with the current state (`lost_connection_fails_operation`). |
| `BluetoothDevice.mtuNow` / `mtu` (stream) | `Device::request_mtu` / `device.requestMtu` | Different: the negotiated value is the call's result; there is no mtu stream to watch. |
| `BluetoothDevice.readRssi` | `Device::read_rssi` / `device.readRssi` | Same; a failure has no code of its own and resolves `ERR_BLUETOOTH_BACKEND` with the platform status. |
| `BluetoothDevice.requestMtu(desiredMtu)` | `Device::request_mtu` / `device.requestMtu` | Same: returns the negotiated mtu (`mtu_change_failure`); no predelay argument. |
| `BluetoothDevice.requestConnectionPriority` | not modeled | No connection-parameter API. |
| `BluetoothDevice.setPreferredPhy` | not modeled | No PHY API. |
| `BluetoothDevice.createBond` / `removeBond` / `bondState` / `prevBondState` | not modeled | No Android bond management. |
| `BluetoothDevice.clearGattCache` | not modeled | Android's GATT cache refresh is not exposed. |
| `BluetoothDevice.cancelWhenDisconnected` | not applicable | The caller owns its own subscriptions. |
| `BluetoothDevice.isAutoConnectEnabled` | not modeled | No auto-connect. |
| `BluetoothService` (`uuid`, `isPrimary`, `characteristics`, `includedServices`) | the `Service` value `discover_services` returns | Same fields: uuid, `primary`, characteristics, and `included_services` as the uuids of the services this one includes; the included services are referenced, not expanded into `Service` values. |
| `BluetoothCharacteristic.properties` | flags on the discovered characteristic | Same: read/write/notify capabilities come with discovery. |
| `BluetoothCharacteristic.descriptors` | `characteristic.descriptors` on the discovered value | Same: descriptors hang off the discovered `Characteristic`; there is no separate lookup call. |
| `BluetoothCharacteristic.lastValue` / `lastValueStream` / `onValueReceived` | `Device::read_characteristic`, `Device::subscribe_characteristic` / `device.read`, `device.subscribe` | Different: no cached last value and no re-emit. Reads return the value; the subscribe stream yields notifications (`subscribe_receives_notifications`). |
| `BluetoothCharacteristic.isNotifying` | not a query | Different: the subscribe stream's existence is the state; double-subscribing is fine (`subscribe_unsubscribe_idempotent`). |
| `BluetoothCharacteristic.read()` | `Device::read_characteristic(&Characteristic)` / `device.read(uuid)` | Same: bytes in, bytes out (`Vec<u8>` / `Uint8Array`). |
| `BluetoothCharacteristic.write(value, withoutResponse, allowLongWrite)` | `Device::write_characteristic(&Characteristic, bytes, WriteType)` / `device.write(uuid, value, writeType)` | Same for with and without response; the backend records the type (`write_without_response_records_type`). `allowLongWrite` is not modeled. |
| `BluetoothCharacteristic.setNotifyValue(notify, forceIndications)` | `Device::subscribe_characteristic` / `Device::unsubscribe_characteristic`; TS `device.subscribe(uuid)` / `device.unsubscribe(uuid)` | Different: a stream, not a boolean; subscribing twice keeps one subscription and hands out a fresh receiver. Notifications are preferred over indications, as on iOS. `forceIndications` is not modeled. |
| `BluetoothDescriptor.lastValue` / `lastValueStream` / `onValueReceived` | the `Device::read_descriptor` result | Different: no cached value stream. |
| `BluetoothDescriptor.read()` | `Device::read_descriptor(&Descriptor)` / `device.readDescriptor(uuid)` | Same (`descriptor_failures_coded`). Rust takes the `&Descriptor` from the discovery cache; TypeScript looks it up by uuid. |
| `BluetoothDescriptor.write(value)` | `Device::write_descriptor(&Descriptor, bytes)` / `device.writeDescriptor(uuid, value)` | Same, except the CCCD (0x2902): the OS owns it, so writing it is `ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED`; `subscribe` maintains it. |

flutter_blue_plus has no peripheral role, so our `Server` (`add_service`, `remove_service`, `advertise`, `stop_advertising`, `set_characteristic_value`, `notify`, `requests`) has no FBP rows. Its tests are `advertise_twice_in_progress`, `advertise_unsupported_on_platform`, `advertise_failure_coded`, `notify_without_subscriber_fails`, `duplicate_service_fails` and `server_unknown_characteristic`.

## Table 2: react-native-ble-plx

| react-native-ble-plx | Ours | Behavior |
|---|---|---|
| `new BleManager(options)` | `Central::new()` / `new Central()` | Different: no singleton, no restore-state options; streams live on the handles. |
| `BleManager.destroy()` | not applicable | No manager to destroy in a trait-based library; dropping the handles ends the streams and in-flight calls still resolve once. |
| `BleManager.setLogLevel` / `logLevel` | not modeled | No logging surface. |
| `BleManager.cancelTransaction(transactionId)` | dropping the future / cancelling the handle | Different: no transaction ids. Each call is cancelled through its own handle and resolves `ERR_BLUETOOTH_CANCELLED` once (`cancel_connect_returns_cancelled`). |
| `BleManager.enable()` | not modeled | No adapter-power call in `CentralBackend` or in either half; `ERR_BLUETOOTH_STATE_CHANGE_FAILED` is reserved for the day one lands. |
| `BleManager.disable()` | not modeled | Same: no adapter-power call. |
| `BleManager.state()` | `Central::state` / `central.state` | Same. |
| `BleManager.onStateChange(listener, emitCurrentState)` | `Central::state_changes` / `central.stateChanges` | Same purpose, different start: ours begins at the next change; call `state()` for the current one, which is what plx's `emitCurrentState` option does. |
| `BleManager.startDeviceScan(uuids, options, listener)` | `Central::scan(&ScanFilter)` / `central.scan(filter)` | Different: a second scan resolves `ERR_BLUETOOTH_IN_PROGRESS` where plx silently stops the first; `scanMode`, `callbackType` and `allowDuplicates` are not modeled; filters are service uuids and device ids. |
| `BleManager.stopDeviceScan()` | `ScanStream::stop()` / `stream.stop()`, or `central.stopScan()` | Same: an explicit stop, fine when none runs. Dropping the Rust `ScanStream` stops the scan too; in TypeScript the backend scans until `stop()` runs. |
| `BleManager.requestConnectionPriorityForDevice` | not modeled | No connection-parameter API. |
| `BleManager.readRSSIForDevice` | `Device::read_rssi` / `device.readRssi` | Same. |
| `BleManager.requestMTUForDevice(id, mtu)` | `Device::request_mtu` / `device.requestMtu` | Same: returns the negotiated mtu; failure `ERR_BLUETOOTH_MTU_CHANGE_FAILED` (`mtu_change_failure`). |
| `BleManager.devices(deviceIdentifiers)` | not modeled | No lookup of known devices by id; ours filters the system's devices by service uuid instead. |
| `BleManager.connectedDevices(serviceUUIDs)` | `Central::system_devices(&[Uuid])` / `central.systemDevices(uuids)` | Different: ours lists the devices the system already knows, filtered by service uuids — plx narrows to currently connected ones with discovered services; ours does not split those apart. |
| `BleManager.connectToDevice(id, options)` | `central.device(id)` then `Device::connect(ConnectOptions)` / `device.connect(options)` | Different: `options.timeout` maps to `ConnectOptions::timeout`. plx on Android first cancels an existing connection; ours resolves `ERR_BLUETOOTH_ALREADY_CONNECTED` (`connect_twice_already_connected`). `requestMTU` and `refreshGatt` options are not modeled. |
| `BleManager.cancelDeviceConnection(id)` | `Device::cancel_connect` / `device.cancelConnect()` cancels a pending connect; `Device::disconnect` / `device.disconnect()` disconnects | Same outcome, split in two calls: the pending connect resolves `ERR_BLUETOOTH_CANCELLED` (`cancel_connect_returns_cancelled`), and the disconnect is idempotent (`disconnect_is_idempotent`). |
| `BleManager.onDeviceDisconnected(id, listener)` | `Central::connection_events` / `central.connectionEvents()` | Same events; one stream for every device of the central, each event tagged with the id, instead of a per-id callback. |
| `BleManager.isDeviceConnected(id)` | `Device::is_connected` / `device.isConnected` | Same. |
| `BleManager.discoverAllServicesAndCharacteristicsForDevice` | `Device::discover_services` / `device.discoverServices` | Same: one call discovers services, characteristics and descriptors; failure `ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED`. |
| `BleManager.servicesForDevice` | `Device::services` / `device.services` | Same. |
| `BleManager.characteristicsForDevice(id, serviceUUID)` | `Device::services()` / `device.services()`, then the service's `characteristics`; a single lookup is `Device::characteristic(uuid)` / `device.characteristic(uuid)` | Different: no per-service list call; the discovery cache holds them. Before discovery both halves report `ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED`, and a miss is `ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND`. `ERR_BLUETOOTH_CHARACTERISTICS_NOT_DISCOVERED` is in the code set but our one-pass discovery never fires it. |
| `BleManager.descriptorsForDevice(id, svc, chr)` | `characteristic.descriptors` on the cache values; TS single calls `device.readDescriptor(uuid)` / `device.writeDescriptor(uuid, value)` | Different: no list call; Rust's `read_descriptor` / `write_descriptor` take the `&Descriptor` from the cache, TypeScript looks it up by uuid. Before discovery, `ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED`; a miss is `ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND`. `ERR_BLUETOOTH_DESCRIPTORS_NOT_DISCOVERED` is in the code set but never fired by the one-pass discovery. |
| `BleManager.readCharacteristicForDevice` | `Device::read_characteristic(&Characteristic)` / `device.read(uuid)` | Same behavior; values are bytes (`Uint8Array`), not base64 strings. |
| `BleManager.writeCharacteristicWithResponseForDevice` | `Device::write_characteristic(.., WriteType::WithResponse)` / `device.write(uuid, value, "withResponse")` | Same. |
| `BleManager.writeCharacteristicWithoutResponseForDevice` | `Device::write_characteristic(.., WriteType::WithoutResponse)` / `device.write(uuid, value, "withoutResponse")` | Same. |
| `BleManager.monitorCharacteristicForDevice(.., subscriptionType)` | `Device::subscribe_characteristic` / `device.subscribe(uuid)` | Same delivery (`subscribe_receives_notifications`); a stream instead of a callback; notifications preferred over indications, as plx prefers; `subscriptionType` is not modeled. |
| `BleManager.readDescriptorForDevice` | `Device::read_descriptor` / `device.readDescriptor` | Same. |
| `BleManager.writeDescriptorForDevice` | `Device::write_descriptor` / `device.writeDescriptor` | Same; a CCCD write is `ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED`. |
| `Device.connect` / `cancelConnection` / `isConnected` / `onDisconnected` | `device.connect(options)` / `device.cancelConnect()` / `device.isConnected()`; disconnection events come from `Central::connection_events` / `central.connectionEvents()` | Same; plx's `Device` methods are the manager methods with the id filled in. |
| `Device.discoverAllServicesAndCharacteristics` / `services` / `characteristicsForService` / `descriptorsForService` | `Device::discover_services` / `Device::services` / the cache rows above | Same, as the manager rows above. |
| `Device.readCharacteristicForService` / `writeCharacteristicWithResponseForService` / `writeCharacteristicWithoutResponseForService` / `monitorCharacteristicForService` | `Device::read_characteristic` / `write_characteristic` with the `WriteType` / `subscribe_characteristic`; TS `device.read` / `device.write` / `device.subscribe` | Same, as the manager rows above. |
| `Device.readDescriptorForService` / `writeDescriptorForService` | `Device::read_descriptor` / `write_descriptor`; TS `device.readDescriptor` / `device.writeDescriptor` | Same. |
| `Device.requestConnectionPriority` | not modeled | No connection-parameter API. |
| `Device.readRSSI` / `requestMTU` | `Device::read_rssi` / `request_mtu` | Same, as the manager rows above. |
| `Service.characteristics` / `descriptorsForCharacteristic` | `service.characteristics` / `characteristic.descriptors` on the cache values | Same data, read off the discovered values instead of fetched per call. |
| `Service.readCharacteristic` / `writeCharacteristicWithResponse` / `writeCharacteristicWithoutResponse` / `monitorCharacteristic` | the characteristic rows above | Same. |
| `Service.readDescriptorForCharacteristic` / `writeDescriptorForCharacteristic` | `Device::read_descriptor` / `write_descriptor` | Same. |
| `Characteristic.descriptors` | `characteristic.descriptors` on the discovered value | Same. |
| `Characteristic.read` / `writeWithResponse` / `writeWithoutResponse` / `monitor` | the characteristic rows above | Same. |
| `Characteristic.readDescriptor` / `writeDescriptor` | `Device::read_descriptor` / `write_descriptor` | Same. |
| `Descriptor.read` / `write` | `Device::read_descriptor` / `write_descriptor` | Same. |
| plx's numeric `Identifier` addressing (`id`, `serviceID`, `characteristicID`) | not modeled | Everything is addressed by uuid path (device id, service, characteristic, descriptor); no per-discovery numeric handles. |

plx spreads one operation over four wrapper classes (manager, device, service, characteristic); ours has one `Device`, so every wrapper lands on the same method.

### `BleErrorCode` members

| `BleErrorCode` | Code here | Note |
|---|---|---|
| `UnknownError` (0) | `ERR_BLUETOOTH_BACKEND` | Folded: no better code; the platform status is kept in `status`. |
| `BluetoothManagerDestroyed` (1) | not applicable | No manager to destroy in a trait-based library. |
| `OperationCancelled` (2) | `ERR_BLUETOOTH_CANCELLED` | Same meaning: an explicit cancel of an in-flight call. |
| `OperationTimedOut` (3) | `ERR_BLUETOOTH_TIMEOUT` | Same: `ConnectOptions::timeout` or the platform's own deadline. |
| `OperationStartFailed` (4) | splits | `ERR_BLUETOOTH_SCAN_START_FAILED`, `ERR_BLUETOOTH_CONNECT_FAILED` or `ERR_BLUETOOTH_ADVERTISE_FAILED`, by the operation that could not start. |
| `InvalidIdentifiers` (5) | `ERR_BLUETOOTH_INVALID_IDENTIFIERS` | Same. |
| `BluetoothUnsupported` (100) | `ERR_BLUETOOTH_UNSUPPORTED` | Same. |
| `BluetoothUnauthorized` (101) | `ERR_BLUETOOTH_UNAUTHORIZED` | Same. |
| `BluetoothPoweredOff` (102) | `ERR_BLUETOOTH_POWERED_OFF` | Same. |
| `BluetoothInUnknownState` (103) | `ERR_BLUETOOTH_UNKNOWN_STATE` | Same. |
| `BluetoothResetting` (104) | `ERR_BLUETOOTH_RESETTING` | Same. |
| `BluetoothStateChangeFailed` (105) | `ERR_BLUETOOTH_STATE_CHANGE_FAILED` | Same code, but ours has no adapter-power call that fires it today. |
| `DeviceConnectionFailed` (200) | `ERR_BLUETOOTH_CONNECT_FAILED` | Same; Android 133 keeps `status` 133 (`connect_retries_android_133`). |
| `DeviceDisconnected` (201) | `ERR_BLUETOOTH_DISCONNECTED` | Same. |
| `DeviceRSSIReadFailed` (202) | `ERR_BLUETOOTH_BACKEND` | Folded: `read_rssi` has no code of its own. |
| `DeviceAlreadyConnected` (203) | `ERR_BLUETOOTH_ALREADY_CONNECTED` | Same. |
| `DeviceNotFound` (204) | `ERR_BLUETOOTH_DEVICE_NOT_FOUND` | Same. |
| `DeviceNotConnected` (205) | `ERR_BLUETOOTH_DEVICE_NOT_CONNECTED` | Same. |
| `DeviceMTUChangeFailed` (206) | `ERR_BLUETOOTH_MTU_CHANGE_FAILED` | Same. |
| `ServicesDiscoveryFailed` (300) | `ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED` | Same. |
| `IncludedServicesDiscoveryFailed` (301) | not applicable | `discover_services` returns the whole tree; there is no separate included-services call. |
| `ServiceNotFound` (302) | `ERR_BLUETOOTH_SERVICE_NOT_FOUND` | Same. |
| `ServicesNotDiscovered` (303) | `ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED` | Same. |
| `CharacteristicsDiscoveryFailed` (400) | `ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED` | Folded: characteristics come with `discover_services`, so its failure code covers this. |
| `CharacteristicWriteFailed` (401) | `ERR_BLUETOOTH_CHARACTERISTIC_WRITE_FAILED` | Same. |
| `CharacteristicReadFailed` (402) | `ERR_BLUETOOTH_CHARACTERISTIC_READ_FAILED` | Same. |
| `CharacteristicNotifyChangeFailed` (403) | `ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED` | Same. |
| `CharacteristicNotFound` (404) | `ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND` | Same. |
| `CharacteristicsNotDiscovered` (405) | `ERR_BLUETOOTH_CHARACTERISTICS_NOT_DISCOVERED` | In the code set, but our one-pass discovery reports `ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED` before discovery instead. |
| `CharacteristicInvalidDataFormat` (406) | `ERR_BLUETOOTH_CHARACTERISTIC_INVALID_DATA_FORMAT` | Same. |
| `DescriptorsDiscoveryFailed` (500) | `ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED` | Folded, like 400: descriptors come with `discover_services`. |
| `DescriptorWriteFailed` (501) | `ERR_BLUETOOTH_DESCRIPTOR_WRITE_FAILED` | Same. |
| `DescriptorReadFailed` (502) | `ERR_BLUETOOTH_DESCRIPTOR_READ_FAILED` | Same. |
| `DescriptorNotFound` (503) | `ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND` | Same. |
| `DescriptorsNotDiscovered` (504) | `ERR_BLUETOOTH_DESCRIPTORS_NOT_DISCOVERED` | In the code set, but the one-pass discovery reports `ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED` before discovery instead. |
| `DescriptorInvalidDataFormat` (505) | `ERR_BLUETOOTH_CHARACTERISTIC_INVALID_DATA_FORMAT` | Folded: one invalid-data code covers characteristic and descriptor payloads. |
| `DescriptorWriteNotAllowed` (506) | `ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED` | Same. |
| `ScanStartFailed` (600) | `ERR_BLUETOOTH_SCAN_START_FAILED` | Same; the platform status rides along (`scan_start_failed_carries_status`). |
| `LocationServicesDisabled` (601) | `ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED` | Same. |

plx's `attErrorCode`, `iosErrorCode` and `androidErrorCode` have no codes of their own here: the number is kept in `BluetoothError::status`. Three of our codes have no plx counterpart — `ERR_BLUETOOTH_IN_PROGRESS`, `ERR_BLUETOOTH_ADVERTISE_FAILED` and `ERR_BLUETOOTH_SERVER_FAILED` — because plx replaces a running scan silently and has no peripheral role.

## Notes

- **btleplug** (`.refs/btleplug`, `1af8299`): its `Central` and `Peripheral` traits shaped our `CentralBackend`/`ServerBackend` split — a backend trait the library drives, the fake implements and each platform bridge fills in. btleplug itself is not used (decisions.md #4): it has no peripheral role, and its adapter-state handling would leave the explicit states and coded errors uneven across platforms.
- **NSH**: NSH 0.4.0 has no Bluetooth host feature (decisions.md #3), so nothing registers with an NSH host yet. The crate is plain Rust behind its own traits, and the TypeScript package mirrors it directly; the `native_module!` glue lands when NSH ships a Bluetooth host feature.
