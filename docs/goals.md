# Goals

BLE central and peripheral roles for NSH apps, in one Rust crate (`crates/nsh-bluetooth`) and one TypeScript package (`packages/nsh-bluetooth`) with identical error codes and identical behavior. Rust names are snake_case; TypeScript names are the same words in camelCase (`discover_services` / `discoverServices`), except one split docs/parity.md documents: the GATT calls are `read_characteristic` / `write_characteristic` / `subscribe_characteristic` in Rust and `device.read(uuid)` / `device.write(uuid, …)` / `device.subscribe(uuid)` in TypeScript. What one language does, the other does.

Three surfaces, behind the crate's own `CentralBackend` and `ServerBackend` traits (decisions.md #3 and #4):

- `Central`: adapter state, scanning, connecting, and the connection events of its devices (`Central::connection_events` / `central.connectionEvents()`).
- `Device`: the GATT client — discovery, reads, writes, subscriptions, MTU, RSSI.
- `Server`: the peripheral role — a GATT service tree, advertising, characteristic values and notifications.

Each platform bridge implements what its platform supports. A call a platform cannot serve resolves `ERR_BLUETOOTH_UNSUPPORTED`. No call panics and no call hangs.

## Acceptance criteria

Four gates, all on GitHub Actions, never on a developer machine (AGENTS.md):

1. `cargo fmt --all -- --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test --all-targets`, on ubuntu, macos and windows runners
4. the TypeScript typecheck and the TypeScript tests in `packages/nsh-bluetooth`

CI also cross-checks the iOS, Android and wasm targets, so every bridge typechecks on every host (decisions.md #9 and #10).

Two rules cut across the gates:

- Every edge case in the table below has its named test in BOTH languages: a Rust integration test `fn` in `crates/nsh-bluetooth/tests/` and a TypeScript `it("...")` in `packages/nsh-bluetooth/tests/`, under the same name.
- Every call resolves exactly once: success, a coded error, or a cancel. It never hangs, and a platform answer that arrives after the call resolved is dropped.

No CI runner has a Bluetooth radio. Every test runs through the fake backend (`fail_next`, `inject_notification`, `simulate_disconnect` and friends), on every runner, in both languages.

## Error model

One stable code per failure mode, the same strings in both languages.

Rust (the biometrics `error.rs` pattern):

- `#[non_exhaustive] enum ErrorKind`, one variant per code below.
- `struct BluetoothError { kind: ErrorKind, status: Option<i64> }` with `new(kind)`, `with_status(i64)`, `kind()`, `status()` and `code() -> &'static str`.
- `ErrorKind::code()` and `ErrorKind::message()`; the message is one sentence saying what to do. `thiserror` derive; `Display` prints the message.

TypeScript (the biometrics `errors.ts` pattern):

- `BluetoothErrorCode`, a union of the exact same 35 strings.
- `class BluetoothError extends Error { readonly code }`.
- `bluetoothError(code)` constructs one; `asBluetoothError(unknown)` turns anything thrown into one, mapping `ERR_NSH_HOST_ERROR` rejections whose message starts with `"ERR_BLUETOOTH_...: "` to the coded error. Anything unknown is `ERR_BLUETOOTH_BACKEND`.

## Error codes

| Code | Rust variant | Fires when | Typical platform trigger |
|---|---|---|---|
| `ERR_BLUETOOTH_UNSUPPORTED` | `Unsupported` | No usable Bluetooth stack, or no bridge for this platform | iOS Simulator (no BLE stack); a target whose bridge is the shared `UnsupportedBackend` |
| `ERR_BLUETOOTH_UNAUTHORIZED` | `Unauthorized` | The app is not allowed to use Bluetooth | CoreBluetooth `.unauthorized`; Android 12+ without `BLUETOOTH_SCAN`/`BLUETOOTH_CONNECT` |
| `ERR_BLUETOOTH_POWERED_OFF` | `PoweredOff` | The adapter is off | CoreBluetooth `.poweredOff`; Android `STATE_OFF` |
| `ERR_BLUETOOTH_UNKNOWN_STATE` | `UnknownState` | The adapter state is not known yet | CoreBluetooth `.unknown`, before the first state report |
| `ERR_BLUETOOTH_RESETTING` | `Resetting` | The stack is momentarily resetting | CoreBluetooth `.resetting` |
| `ERR_BLUETOOTH_STATE_CHANGE_FAILED` | `StateChangeFailed` | Reserved: an adapter power change failed. No adapter-power call is in the API, so nothing fires it today | would be Android's enable request refused or timed out |
| `ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED` | `LocationServicesDisabled` | A scan needs location services and they are off | Android before 12: scan results require location |
| `ERR_BLUETOOTH_INVALID_IDENTIFIERS` | `InvalidIdentifiers` | A UUID or device id is malformed | a service uuid that is not a well-formed 16/32/128-bit uuid; a device id that is neither MAC nor UUID |
| `ERR_BLUETOOTH_IN_PROGRESS` | `InProgress` | A second scan or a second advertisement while one runs | any; one scan and one advertisement per `Central`/`Server` (decisions.md #8) |
| `ERR_BLUETOOTH_SCAN_START_FAILED` | `ScanStartFailed` | The platform refused to start the scan | Android scan throttling; the platform status is kept in `status` |
| `ERR_BLUETOOTH_CONNECT_FAILED` | `ConnectFailed` | The connection attempt failed | Android status 133 (`GATT_ERROR`) lands here with `status` `Some(133)` |
| `ERR_BLUETOOTH_ALREADY_CONNECTED` | `AlreadyConnected` | Connecting to a device already connected | any |
| `ERR_BLUETOOTH_DISCONNECTED` | `Disconnected` | The device dropped mid-operation | link loss during a GATT operation (Android 133/8/19; Apple `peripheral disconnected`) |
| `ERR_BLUETOOTH_DEVICE_NOT_FOUND` | `DeviceNotFound` | No device with that id | connect or lookup with an unknown id |
| `ERR_BLUETOOTH_DEVICE_NOT_CONNECTED` | `DeviceNotConnected` | A GATT operation without a connection | read/write/subscribe after a disconnect |
| `ERR_BLUETOOTH_CANCELLED` | `Cancelled` | An in-flight operation was explicitly cancelled | the caller cancelled a pending connect (`cancel_connect_returns_cancelled`) |
| `ERR_BLUETOOTH_TIMEOUT` | `Timeout` | `ConnectOptions::timeout` passed, or the platform's own deadline did | Android's ~30 s connect timeout |
| `ERR_BLUETOOTH_MTU_CHANGE_FAILED` | `MtuChangeFailed` | The MTU request failed | Android `onMtuChanged` with a non-zero status |
| `ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED` | `ServicesDiscoveryFailed` | Service discovery failed | Android discovery status 133; a CoreBluetooth discovery error |
| `ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED` | `ServicesNotDiscovered` | A GATT operation before `discover_services` | any; the library enforces the order itself |
| `ERR_BLUETOOTH_SERVICE_NOT_FOUND` | `ServiceNotFound` | The service uuid is not in the discovered set | any |
| `ERR_BLUETOOTH_CHARACTERISTIC_WRITE_FAILED` | `CharacteristicWriteFailed` | The characteristic write failed | an ATT error status from the peripheral |
| `ERR_BLUETOOTH_CHARACTERISTIC_READ_FAILED` | `CharacteristicReadFailed` | The characteristic read failed | an ATT error status, e.g. read not permitted |
| `ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED` | `CharacteristicNotifyChangeFailed` | Subscribing or unsubscribing failed | a CCCD write failure; on the server side, `notify` with no subscriber |
| `ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND` | `CharacteristicNotFound` | The characteristic uuid is unknown | a lookup miss after discovery; a server call on an unregistered uuid |
| `ERR_BLUETOOTH_CHARACTERISTICS_NOT_DISCOVERED` | `CharacteristicsNotDiscovered` | A characteristic operation before characteristics are known | a characteristic operation before that level of discovery |
| `ERR_BLUETOOTH_CHARACTERISTIC_INVALID_DATA_FORMAT` | `CharacteristicInvalidDataFormat` | The value is not valid data for the call | a value that does not decode (in TS, a base64 string that is not one) |
| `ERR_BLUETOOTH_DESCRIPTOR_WRITE_FAILED` | `DescriptorWriteFailed` | The descriptor write failed | an ATT error status |
| `ERR_BLUETOOTH_DESCRIPTOR_READ_FAILED` | `DescriptorReadFailed` | The descriptor read failed | an ATT error status |
| `ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND` | `DescriptorNotFound` | The descriptor uuid is unknown | a lookup miss |
| `ERR_BLUETOOTH_DESCRIPTORS_NOT_DISCOVERED` | `DescriptorsNotDiscovered` | A descriptor operation before descriptors are known | a descriptor operation before that level of discovery |
| `ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED` | `DescriptorWriteNotAllowed` | The write targets a descriptor the OS owns | the CCCD (0x2902), which subscribe/unsubscribe maintain |
| `ERR_BLUETOOTH_ADVERTISE_FAILED` | `AdvertiseFailed` | Advertising failed to start or failed while running | Android `ADVERTISE_FAILED_*`; advertising with the peripheral manager not powered on |
| `ERR_BLUETOOTH_SERVER_FAILED` | `ServerFailed` | The GATT server platform call failed | `add_service` twice; an Android `BluetoothGattServer` callback error |
| `ERR_BLUETOOTH_BACKEND` | `Backend` | The platform failed with no better code | anything unmappable; `status` keeps the platform's own code |

## Edge cases

Each case maps to one canonical test name, identical across languages: a Rust integration test `fn` in `crates/nsh-bluetooth/tests/`, a TypeScript `it("...")` in `packages/nsh-bluetooth/tests/`. CI runs both on every push.

| # | Case | Test name | Triggered by |
|---|---|---|---|
| 1 | Adapter off | `bluetooth_off_reports_powered_off` | any: CoreBluetooth `.poweredOff`, Android `STATE_OFF`; the state is `PoweredOff` and a scan is refused with it |
| 2 | No permission | `scan_refuses_when_unauthorized` | iOS `.unauthorized`; Android 12+ scan/connect permission denied |
| 3 | Location off | `location_disabled_blocks_scan` | Android before 12, scanning with location services off |
| 4 | No stack or bridge | `unsupported_platform_codes_every_call` | the shared `UnsupportedBackend`: every central and server call resolves `Unsupported` |
| 5 | Second scan | `second_scan_fails_in_progress` | the library's own rule, any platform; the first scan's stream keeps working |
| 6 | Scan filter | `scan_filter_matches_service_and_device` | any; filters by service uuid and device id deliver only matches |
| 7 | Scan start failure | `scan_start_failed_carries_status` | Android scan throttling; the platform status rides the error |
| 8 | Bad identifier | `invalid_identifier_rejected` | any; a malformed uuid at parse and at lookups |
| 9 | Connect timeout | `connect_timeout_resolves_once` | `ConnectOptions::timeout`, any platform; one result, no hang |
| 10 | Android 133 retry | `connect_retries_android_133` | Android; `fail_next(Connect, ConnectFailed status 133)`: `max_attempts` 2 succeeds on the second try, 1 surfaces the 133 |
| 11 | Double connect | `connect_twice_already_connected` | any |
| 12 | Cancel connect | `cancel_connect_returns_cancelled` | any; a pending connect resolves `Cancelled` exactly once |
| 13 | Idempotent disconnect | `disconnect_is_idempotent` | any; disconnect when not connected is `Ok` |
| 14 | Link loss | `lost_connection_fails_operation` | any; `simulate_disconnect`, then a GATT op fails `Disconnected` and `connection_events` yields `Disconnected` |
| 15 | GATT before discovery | `gatt_before_discovery_fails` | any; read/write/subscribe before `discover_services` fails `ServicesNotDiscovered` |
| 16 | Unknown uuids | `unknown_uuids_not_found` | any; service, characteristic and descriptor lookups |
| 17 | Read/write failures | `characteristic_read_write_failures` | any; scripted `CharacteristicReadFailed` / `CharacteristicWriteFailed` with status |
| 18 | Notifications | `subscribe_receives_notifications` | any; `inject_notification` reaches the subscriber |
| 19 | Subscribe idempotent | `subscribe_unsubscribe_idempotent` | any; double subscribe is fine, unsubscribing nothing is fine |
| 20 | Write type recorded | `write_without_response_records_type` | any; `WriteType::WithoutResponse` is recorded by the fake |
| 21 | MTU failure | `mtu_change_failure` | Android; a scripted `RequestMtu` failure, and negotiation returns the configured mtu |
| 22 | Descriptor failures | `descriptor_failures_coded` | any; `DescriptorReadFailed`, `DescriptorWriteFailed`, `DescriptorWriteNotAllowed` |
| 23 | Second advertisement | `advertise_twice_in_progress` | the library's own rule, any platform |
| 24 | No server backend | `advertise_unsupported_on_platform` | the web bridge (no peripheral role); any backend without a server |
| 25 | Advertise failure | `advertise_failure_coded` | a scripted `AdvertiseFailed` |
| 26 | Notify without subscriber | `notify_without_subscriber_fails` | any server; `CharacteristicNotifyChangeFailed`, and with a subscriber it is delivered |
| 27 | Duplicate service | `duplicate_service_fails` | any GATT server; `add_service` twice fails `ServerFailed` |
| 28 | Unknown server characteristic | `server_unknown_characteristic` | any server; `set_characteristic_value` / `notify` on an unknown uuid fails `CharacteristicNotFound` |
| 29 | State stream | `state_stream_reports_resetting_and_unknown` | iOS `.resetting` and `.unknown` flow through `state_changes` |
| 30 | All codes reachable | `every_error_code_reachable` | table-driven over the code table: all 35 codes construct with a non-empty message and the exact `ERR_BLUETOOTH_*` string (Rust) / the exact union member (TS) |

## Non-goals

- No device tests on developer machines. GitHub CI compiles and tests everything; no emulator or simulator is started locally (AGENTS.md).
- The web has no background scan and no peripheral role. Web Bluetooth needs a user gesture to pick a device and offers no GATT server, so the web bridge is central-only and foreground-only.
- No NSH host glue yet. NSH (0.4.0 and 0.5.0) has no Bluetooth host feature to register with (decisions.md #3); the `native_module!` bridge lands when NSH ships one.
