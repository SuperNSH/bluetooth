// Dynamic z-workflow for /Users/hmziq/fo/nsh-bluetooth — fresh script for this one task.
//
// Brief (verbatim constraints): build BOTH a Rust crate and a TypeScript package laid out like
// SuperNSH/biometrics (.refs/biometrics), same API names and same behavior in both, a test in both
// for every behavior; GLM-5.3 Max on EVERY area and every auditor; audit rounds per area: loop
// until zero issues, capped at 3, report honestly if an area never converges; no commits, no
// pushes, no git machinery; gates: cargo fmt --all -- --check, cargo clippy --all-targets --
// -D warnings, cargo test --all-targets, plus the TypeScript typecheck and tests; ci.yml updated
// to run both (per AGENTS.md CI compiles and tests — the workflow itself runs NO emulators and
// NO simulators and touches no other repo).
//
// Nine areas. The brief demands one area per platform bridge, so nine is the minimum faithful
// split (docs, rust-core, five bridges, ts-package, rust-tests+ci); each owns a disjoint file set.
// Per-area chain inside its own callback: implement -> audit loop (gates + read-only auditor +
// fix ask) -> done. The only global joins: the final whole-tree gates and the report. Bridge and
// rust-test areas wait on a real dependency (the core API compile unit), never on a phase.
//
// STEP-0 (done by the launcher before this run): .refs/ clones (5/5 OK, .gitignore updated),
// docs/decisions.md written, workspace restructured to crates/nsh-bluetooth + packages/..., empty
// platform stub files created, all named gates green on the scaffold, cross-target clippy verified
// for wasm32-unknown-unknown / x86_64-pc-windows-gnu / aarch64-linux-android. The Linux target
// cannot be checked on this Mac (bluer -> libdbus-sys needs dbus C headers); its bridge is
// verified by CI's ubuntu job — the linux area's gates are host-only and say so.

const MODELS = {
  // The user explicitly asked for GLM-5.3 Max on every area and every audit. No vision phases
  // exist in this run, so there is deliberately no flash pin at all.
  pro: model("account:zai-individual-coding-plan/GLM-5.3$high"),
};

const KIT = "/Users/hmziq/ZCodeProject/ai/harness/zcode/agents";

async function specialist(name: string, fallback: string): Promise<{ persona: string; source: string }> {
  const adapter =
    `You are the ${name} specialist for one area of a parallel workflow. ` +
    `Your full specialist contract follows verbatim — apply it to this area's work; ` +
    `where it conflicts with your ask, the ask wins.` + "\n\n";
  for (const p of [`.zcode/agents/${name}.md`, `${KIT}/${name}.md`]) {
    try {
      const raw = await files.read(p);
      return { persona: adapter + raw.replace(/^---\n[\s\S]*?\n---\n/, "").trim(), source: p };
    } catch { /* try next source */ }
  }
  return { persona: fallback, source: "fallback" };
}

// ---------------------------------------------------------------- shared text --------------

const RULES = `
Repo rules (AGENTS.md, binding): comments are 1-2 lines and state a non-obvious constraint; docs
read like a competent human wrote them, direct and concrete, no marketing filler; dead code is
deleted, no speculative abstractions; every behavior change has a test; an operation resolves
exactly once — success, a coded failure, or a cancel — it never hangs; the main branch is master.
No commits, no pushes, no branches, no worktrees, no git commands of any kind: edit the current
checkout directly. Never build, test or run an emulator or simulator. Do not touch any other repo
and do not edit files outside your ownership list.

Return format: your final message must be ONLY a JSON object (no markdown fences, no other text):
{"done": boolean, "summary": string, "blockers": string[] (omit if none), "contractLine": string}
"contractLine" = one sentence copied VERBATIM from your specialist contract above (or from this
ask's Repo rules if your contract has no sentence) that most shaped your work.
`;

const CODES = `
ERROR CODES — one stable code per failure mode. Rust: #[non_exhaustive] enum ErrorKind, one
variant per code below, struct BluetoothError { kind: ErrorKind, status: Option<i64> } with
new(kind), with_status(i64), kind(), status(), code() -> &'static str; ErrorKind::code() and
ErrorKind::message() (one-sentence "what to do", biometrics style); thiserror derive, Display =
message. TypeScript: union type BluetoothErrorCode of the exact same 35 strings, class
BluetoothError extends Error { readonly code }, factory bluetoothError(code), asBluetoothError()
mapping ERR_NSH_HOST_ERROR rejections whose message starts with "ERR_BLUETOOTH_...: " (biometrics
errors.ts pattern). Code strings (variant -> string):
Unsupported -> ERR_BLUETOOTH_UNSUPPORTED (no usable stack or no bridge on this platform)
Unauthorized -> ERR_BLUETOOTH_UNAUTHORIZED (iOS CoreBluetooth .unauthorized; Android 12+ scan/connect permission denied)
PoweredOff -> ERR_BLUETOOTH_POWERED_OFF
UnknownState -> ERR_BLUETOOTH_UNKNOWN_STATE (adapter state not yet known)
Resetting -> ERR_BLUETOOTH_RESETTING (stack momentarily resetting, iOS .resetting)
StateChangeFailed -> ERR_BLUETOOTH_STATE_CHANGE_FAILED (turn on/off failed)
LocationServicesDisabled -> ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED (Android <12 scan with location off)
InvalidIdentifiers -> ERR_BLUETOOTH_INVALID_IDENTIFIERS (malformed UUID or device id)
InProgress -> ERR_BLUETOOTH_IN_PROGRESS (one scan / one advertisement at a time)
ScanStartFailed -> ERR_BLUETOOTH_SCAN_START_FAILED
ConnectFailed -> ERR_BLUETOOTH_CONNECT_FAILED (Android status 133 lands here with status Some(133))
AlreadyConnected -> ERR_BLUETOOTH_ALREADY_CONNECTED
Disconnected -> ERR_BLUETOOTH_DISCONNECTED (device dropped mid-operation)
DeviceNotFound -> ERR_BLUETOOTH_DEVICE_NOT_FOUND
DeviceNotConnected -> ERR_BLUETOOTH_DEVICE_NOT_CONNECTED
Cancelled -> ERR_BLUETOOTH_CANCELLED (explicit cancel of an in-flight operation)
Timeout -> ERR_BLUETOOTH_TIMEOUT (ConnectOptions.timeout or platform timeout)
MtuChangeFailed -> ERR_BLUETOOTH_MTU_CHANGE_FAILED
ServicesDiscoveryFailed -> ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED
ServicesNotDiscovered -> ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED (GATT op before discover_services)
ServiceNotFound -> ERR_BLUETOOTH_SERVICE_NOT_FOUND
CharacteristicWriteFailed -> ERR_BLUETOOTH_CHARACTERISTIC_WRITE_FAILED
CharacteristicReadFailed -> ERR_BLUETOOTH_CHARACTERISTIC_READ_FAILED
CharacteristicNotifyChangeFailed -> ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED
CharacteristicNotFound -> ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND
CharacteristicsNotDiscovered -> ERR_BLUETOOTH_CHARACTERISTICS_NOT_DISCOVERED
CharacteristicInvalidDataFormat -> ERR_BLUETOOTH_CHARACTERISTIC_INVALID_DATA_FORMAT
DescriptorWriteFailed -> ERR_BLUETOOTH_DESCRIPTOR_WRITE_FAILED
DescriptorReadFailed -> ERR_BLUETOOTH_DESCRIPTOR_READ_FAILED
DescriptorNotFound -> ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND
DescriptorsNotDiscovered -> ERR_BLUETOOTH_DESCRIPTORS_NOT_DISCOVERED
DescriptorWriteNotAllowed -> ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED
AdvertiseFailed -> ERR_BLUETOOTH_ADVERTISE_FAILED
ServerFailed -> ERR_BLUETOOTH_SERVER_FAILED (GATT server platform failure, incl. duplicate add_service)
Backend -> ERR_BLUETOOTH_BACKEND (platform service failed with no better code)
`;

const CONTRACT = `
API CONTRACT — both languages, same names (Rust snake_case, TS camelCase), same behavior.

Types (Rust in crates/nsh-bluetooth/src/backend.rs; TS in packages/nsh-bluetooth/src/types.ts):
- AdapterState: enum { Unknown, Unsupported, Unauthorized, PoweredOff, Resetting, On } / TS string
  union "unknown" | "unsupported" | "unauthorized" | "poweredOff" | "resetting" | "on".
- Uuid: newtype over the canonical lowercase hyphenated string; parse accepts the full form,
  "XXXX" (16-bit) and "XXXXXXXX" (32-bit) short forms (0000XXXX-0000-1000-8000-00805f9b34fb);
  Display/toString = full lowercase; invalid input parses to InvalidIdentifiers. TS: parseUuid()
  returning a branded string type + error.
- DeviceId: newtype String / TS branded string. DeviceInfo { id, name: Option<string> }.
- CharacteristicProperties: u16 bit-consts READ, WRITE, WRITE_NO_RESPONSE, NOTIFY, INDICATE,
  BROADCAST, EXTENDED_PROPERTIES, RELIABLE_WRITE, WRITABLE_AUXILIARY, AUTHENTICATED_SIGNED_WRITES
  with contains()/is_empty() (no bitflags crate) / TS object of optional booleans, same words.
- AdvertisementData { local_name: Option<String>, manufacturer_data: Option<Vec<u8>>,
  service_uuids: Vec<Uuid>, tx_power: Option<i8> }.
- ScanResult { id: DeviceId, name: Option<String>, rssi: Option<i8>, advertisement: AdvertisementData }.
- ScanFilter { service_uuids: Vec<Uuid>, device_ids: Vec<DeviceId> } with matches(&ScanResult) -> bool
  (empty vecs match everything).
- Service { uuid, primary: bool, characteristics: Vec<Characteristic>, included_services: Vec<Uuid> }.
- Characteristic { uuid, service_uuid, properties, descriptors: Vec<Descriptor>, value: Option<Vec<u8>> }.
- Descriptor { uuid, characteristic_uuid, service_uuid }.
- WriteType: enum { WithResponse, WithoutResponse } / TS "withResponse" | "withoutResponse".
- ValueNotification { characteristic: Characteristic, value: Vec<u8> }.
- ConnectionEvent: enum { Connected(DeviceId), Disconnected(DeviceId) } / TS discriminated union.
- GattRequest { service_uuid, characteristic_uuid, kind: GattRequestKind } where GattRequestKind =
  Read { offset: usize } | Write { offset: usize, data: Vec<u8>, with_response: bool } (server role
  requests coming from centrals).
- AdvertiseOptions { local_name: Option<String>, service_uuids: Vec<Uuid> }.
- Streams: Rust tokio::sync::broadcast::Receiver (capacity 32; a Lagged receiver resynchronises —
  it never fails the stream); TS AsyncIterable with the same resync-on-lag semantics.

Platform seams (Rust traits; TS one interface file each):
trait CentralBackend: Send + Sync — every method returns BackendFuture<'_, Result<T, BluetoothError>>
where BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>:
  adapter_state() -> AdapterState; state_changes() -> broadcast::Receiver<AdapterState>;
  start_scan(ScanFilter) -> broadcast::Receiver<ScanResult>; stop_scan() -> ();
  system_devices(&[Uuid]) -> Vec<DeviceInfo>;
  connect(&DeviceId); disconnect(&DeviceId) (idempotent Ok when already disconnected);
  is_connected(&DeviceId) -> bool; read_rssi(&DeviceId) -> i8; request_mtu(&DeviceId, u16) -> u16;
  discover_services(&DeviceId) -> Vec<Service>;
  read_characteristic(&DeviceId, &Characteristic) -> Vec<u8>;
  write_characteristic(&DeviceId, &Characteristic, &[u8], WriteType);
  subscribe_characteristic(&DeviceId, &Characteristic) -> broadcast::Receiver<ValueNotification>;
  unsubscribe_characteristic(&DeviceId, &Characteristic) (Ok when not subscribed);
  read_descriptor(&DeviceId, &Descriptor) -> Vec<u8>; write_descriptor(&DeviceId, &Descriptor, &[u8]);
  connection_events() -> broadcast::Receiver<ConnectionEvent>.
trait ServerBackend: Send + Sync — same future shape:
  add_service(Service) (duplicate uuid -> ServerFailed); remove_service(Uuid) (idempotent Ok);
  start_advertising(AdvertiseOptions); stop_advertising() (Ok when not advertising);
  is_advertising() -> bool; set_characteristic_value(Uuid service, Uuid characteristic, Vec<u8>)
  (unknown characteristic -> CharacteristicNotFound); notify(Uuid, Uuid, &[u8]) (no subscriber ->
  CharacteristicNotifyChangeFailed); requests() -> broadcast::Receiver<GattRequest>.

High-level wrappers (Rust src/central.rs and src/peripheral.rs; TS src/central.ts, peripheral.ts):
- Central::new(Arc<dyn CentralBackend>) / new Central(backend); platform_central() (TS:
  platformCentral()) picks crate::platform::central_backend() — UnsupportedBackend fallback.
  is_supported() -> bool (never errors); state() -> Result<AdapterState>; state_changes();
  scan(ScanFilter) -> Result<ScanStream, BluetoothError> — one scan per Central at a time
  (InProgress); ScanStream::recv() -> Option<ScanResult> (None once stopped; lag resyncs) and
  stop(); Drop stops the scan best-effort. system_devices(&[Uuid]); device(&DeviceId) -> Device.
- Device: connect(ConnectOptions) where ConnectOptions { timeout: Option<Duration>, max_attempts:
  u8 } with Default { None, 1 }; a ConnectFailed carrying status Some(133) retries up to
  max_attempts total tries; the timeout races the whole attempt (Timeout). cancel_connect() ->
  a pending connect resolves Err(Cancelled), none pending -> Ok. disconnect() idempotent Ok.
  is_connected(); read_rssi(); request_mtu(u16); discover_services() (caches);
  services() -> &[Service] (a plain getter: empty before discovery, never an error);
  service(Uuid) and characteristic(Uuid) and every GATT op (read/write/subscribe/unsubscribe/
  read_descriptor/write_descriptor) guard: before discovery -> ServicesNotDiscovered, after
  discovery unknown handle -> the *_NOT_FOUND codes, dropped connection mid-op -> Disconnected
  (with the platform status where one exists). subscribe is idempotent (same logical
  subscription, fresh receiver).
- Server::new(Arc<dyn ServerBackend>); platform_server() -> Result<Server> (Err(Unsupported) when
  no server backend). add_service (invalid uuid -> InvalidIdentifiers; duplicate -> ServerFailed);
  remove_service; advertise(AdvertiseOptions) (already advertising -> InProgress); stop_advertising
  (Ok when not advertising); is_advertising(); set_characteristic_value; notify (unknown ->
  CharacteristicNotFound / ServiceNotFound; no subscriber -> CharacteristicNotifyChangeFailed);
  requests() -> broadcast::Receiver<GattRequest>.
- unsupported.rs: UnsupportedBackend (CentralBackend) and UnsupportedServer (ServerBackend) return
  Err(Unsupported) for every call, never panic, never hang.
- fake.rs (Rust) / memoryBackend (TS) — the deterministic mock everything tests drive:
  FakeBackend::new() -> Arc<FakeBackend> (state On, nothing scanning, no devices, no services).
  &self methods with interior Mutex: set_adapter_state(AdapterState); add_scanned_device(ScanResult)
  (delivered only while scanning AND only when the stored filter matches);
  set_services(DeviceId, Vec<Service>); fail_next(FakeOp, BluetoothError) — ONE-SHOT: the next call
  of that op fails with exactly this error then clears (multiple fail_next stack in order);
  simulate_disconnect(DeviceId) (marks disconnected, emits ConnectionEvent::Disconnected);
  inject_notification(DeviceId, Characteristic, Vec<u8>) (to current subscribers);
  inject_connection_event(ConnectionEvent). FakeOp = { AdapterState, StateChanges, StartScan,
  StopScan, SystemDevices, Connect, Disconnect, IsConnected, ReadRssi, RequestMtu,
  DiscoverServices, ReadCharacteristic, WriteCharacteristic, Subscribe, Unsubscribe,
  ReadDescriptor, WriteDescriptor, ConnectionEvents }. FakeBackend implements CentralBackend with
  an in-memory GATT db (reads return the stored value, writes store it, MTU negotiates to the
  configured value — set_mtu(u16), default 185). FakeServerBackend::new() -> Arc<...> implements
  ServerBackend in-memory: fail_next_server(ServerOp, BluetoothError) with ServerOp = { AddService,
  RemoveService, StartAdvertising, StopAdvertising, IsAdvertising, SetCharacteristicValue, Notify,
  Requests }; inject_request(GattRequest); captured_values() -> Vec<(Uuid, Uuid, Vec<u8>)>;
  subscriber_count(Uuid service, Uuid characteristic) -> usize. TS memoryBackend(config) mirrors
  all of it in one factory (createMemoryBackend) with the same method names in camelCase.

Every public call resolves exactly once — success, a coded error, or a cancel — and never hangs:
no unbounded await on an empty channel, every platform async op raced with a timeout, no
unwrap/panic/expect on a failure path, coded errors only.
`;

const EDGE = `
EDGE CASES — every case maps to the named test below; the test names are canonical across
languages (Rust integration test fn in crates/nsh-bluetooth/tests/, TS it("...") title in
packages/nsh-bluetooth/tests/, listed verbatim in docs/goals.md):
1  bluetooth_off_reports_powered_off — state PoweredOff; scan refused with PoweredOff
2  scan_refuses_when_unauthorized — Unauthorized blocks scan and connect
3  location_disabled_blocks_scan — LocationServicesDisabled on scan
4  unsupported_platform_codes_every_call — UnsupportedBackend: every central and server call -> Unsupported
5  second_scan_fails_in_progress — concurrent scan -> InProgress; first stream still works
6  scan_filter_matches_service_and_device — filter by service uuid / device id delivers only matches
7  scan_start_failed_carries_status — ScanStartFailed carries the platform status
8  invalid_identifier_rejected — malformed uuid -> InvalidIdentifiers (parse + lookups)
9  connect_timeout_resolves_once — Timeout, exactly one result, no hang
10 connect_retries_android_133 — fail_next(Connect, ConnectFailed status 133); max_attempts 2 succeeds on the 2nd try; max_attempts 1 surfaces the 133
11 connect_twice_already_connected — AlreadyConnected
12 cancel_connect_returns_cancelled — pending connect resolves Cancelled once
13 disconnect_is_idempotent — disconnect when not connected -> Ok
14 lost_connection_fails_operation — simulate_disconnect then a GATT op -> Disconnected; connection_events yields Disconnected
15 gatt_before_discovery_fails — read/write/subscribe before discover_services -> ServicesNotDiscovered
16 unknown_uuids_not_found — service/characteristic/descriptor lookups -> *_NOT_FOUND
17 characteristic_read_write_failures — scripted CharacteristicReadFailed / CharacteristicWriteFailed with status
18 subscribe_receives_notifications — inject_notification reaches the subscriber
19 subscribe_unsubscribe_idempotent — double subscribe ok; unsubscribe of nothing ok
20 write_without_response_records_type — WriteType::WithoutResponse recorded by the fake
21 mtu_change_failure — scripted RequestMtu failure -> MtuChangeFailed; negotiation returns the configured mtu
22 descriptor_failures_coded — DescriptorReadFailed / DescriptorWriteFailed / DescriptorWriteNotAllowed
23 advertise_twice_in_progress — second advertise -> InProgress
24 advertise_unsupported_on_platform — server backend absent -> Unsupported
25 advertise_failure_coded — scripted AdvertiseFailed
26 notify_without_subscriber_fails — CharacteristicNotifyChangeFailed; with subscriber -> delivered
27 duplicate_service_fails — add_service twice -> ServerFailed
28 server_unknown_characteristic — set_characteristic_value / notify on unknown uuid -> CharacteristicNotFound
29 state_stream_reports_resetting_and_unknown — fake state changes flow through state_changes
30 every_error_code_reachable — table-driven: all 35 codes construct with a non-empty message and the exact ERR_BLUETOOTH_* string (Rust) / exact union member (TS)
`;

const BRIDGE_COMMON = `
Bridge shape (docs/decisions.md #9): your file compiles on every host. Pure mapping helpers
(platform status/error code -> ErrorKind, with #[cfg(test)] unit tests in the same file) sit
outside any cfg; the native implementation sits in an inner "#[cfg(<your target>)] mod native"
and re-exports the concrete backend type. The entry points (already seeded by the core area) keep
their exact signatures — central_backend()/server_backend() return Option<Arc<dyn ...>>: None
off-target, Some(...) on-target once implemented. If your target has no peripheral role
capability, server_backend() stays None forever with a comment saying so.
Study the real native patterns first — all cloned read-only under .refs/:
btleplug/src/corebluetooth (Apple delegate wiring), btleplug/src/winrtble, btleplug/src/bluez,
btleplug/src/droidplug, and SuperNSH/biometrics crates/nsh-biometrics/src/platform/ (objc2, JNI,
windows, zbus patterns its crates use) plus its error.rs for the coded-error style.
Never panic and never hang: check every Result/Option at the boundary (JNI: check exceptions;
objc2: nil returns are coded errors; WinRT: race every IAsyncOperation with a timeout that maps
to Timeout; D-Bus: timeouts on replies). No unwrap/expect on failure paths. The current negotiated
MTU on platforms without a request-MTU API (Apple, web) is returned by request_mtu from the
platform's fixed value and never fails.
Scoped Cargo.toml exception: if a compile proves you need one more feature in your own platform's
[target.'cfg(...)'.dependencies] entry in crates/nsh-bluetooth/Cargo.toml, you may edit ONLY that
entry's features list — never versions, never another platform's section, never [dependencies].
`;

// ---------------------------------------------------------------- gates --------------------

interface GateSpec { name: string; cmd: string; args: string[]; timeoutMs?: number; }
interface GateOut { name: string; ok: boolean; output: string; }

async function gate(spec: GateSpec): Promise<GateOut> {
  let r: any;
  try {
    r = await world.run(spec.cmd, spec.args, { timeoutMs: spec.timeoutMs ?? 420000 });
  } catch (e) {
    return { name: spec.name, ok: false, output: `world.run threw: ${String(e)}` };
  }
  if (!r) return { name: spec.name, ok: false, output: "world.run returned nothing" };
  const code = r.exitCode ?? r.code ?? r.status;
  const out =
    typeof r.stdout === "string"
      ? `${r.stdout}${r.stderr ? "\n" + r.stderr : ""}`
      : String(r.output !== undefined ? r.output : JSON.stringify(r));
  return { name: spec.name, ok: Number(code) === 0, output: String(out).slice(-6000) };
}

const sleep = (ms: number) => new Promise<void>((res) => setTimeout(res, ms));

// A failed gate gets two quiet retries 30s apart: the crate is one compile unit and a sibling
// area may be mid-edit. Three consecutive failures are the area's real problem.
async function gateRetry(spec: GateSpec): Promise<GateOut> {
  let last = await gate(spec);
  for (let i = 0; i < 2 && !last.ok; i++) {
    await sleep(30000);
    last = await gate(spec);
  }
  return last;
}

// ---------------------------------------------------------------- ask plumbing -------------

interface AreaAskResult {
  done: boolean; summary: string; blockers?: string[];
  contractLine?: string; foreignOnly?: boolean; findings?: Array<{ file: string; issue: string; severity: string }>;
}

function asResult(raw: any): AreaAskResult {
  if (raw && typeof raw === "object" && !Array.isArray(raw) && (raw.done !== undefined || raw.summary !== undefined || raw.findings !== undefined)) {
    return raw as AreaAskResult;
  }
  const text = typeof raw === "string" ? raw : JSON.stringify(raw ?? null);
  const m = /\{[\s\S]*\}/.exec(text);
  if (m) {
    try {
      const parsed = JSON.parse(m[0]);
      if (parsed && typeof parsed === "object") return parsed as AreaAskResult;
    } catch { /* fall through */ }
  }
  return { done: true, summary: text.slice(0, 800) };
}

async function askVerified(agentObj: any, prompt: string, persona: string, label: string): Promise<AreaAskResult> {
  let res = asResult(await ask(agentObj, prompt));
  const line = (res.contractLine ?? "").trim();
  if (line.length > 12 && persona.includes(line)) return res;
  const retry = asResult(await ask(
    agentObj,
    `Your previous message did not carry a verifiable contractLine. Send the JSON object again, ` +
    `same content, with "contractLine" set to one sentence copied EXACTLY (verbatim, unedited) ` +
    `from your specialist contract above.`
  ));
  const line2 = (retry.contractLine ?? "").trim();
  if (line2.length > 12 && persona.includes(line2)) return retry;
  (res as any).unverifiedPersona = true;
  return res;
}

// ---------------------------------------------------------------- area definitions ---------

const PLAIN_IMPL =
  `You are a careful engineer implementing one area of a parallel workflow in an existing ` +
  `checkout. Read the whole ask before editing; follow the repo's conventions; do exactly what ` +
  `the ask says and nothing more.`;
const PLAIN_AUDIT =
  `You are a meticulous reviewer for one area of a parallel workflow. You judge the code against ` +
  `the ask and the repo rules and report only what you can point at.`;

interface AreaResult {
  name: string; status: "done" | "failed"; summary: string; rounds: number; converged: boolean;
}

interface AreaDef {
  name: string;
  specialist: string;          // kit specialist file name, or "" for plain persona
  auditor: string;             // auditor persona source ("" -> plain read-only reviewer)
  auditorsAreFfi: boolean;     // use the ffi auditor's read-only contract verbatim
  implAsk: () => string;
  auditFocus: string;
  gates: GateSpec[];
  waitsCore?: boolean;         // real dependency: the core compile unit must exist first
  resolvesCore?: boolean;
  result?: AreaResult;
}

const RUST_GATES: GateSpec[] = [
  { name: "cargo-fmt", cmd: "cargo", args: ["fmt", "--all", "--", "--check"] },
  { name: "cargo-clippy", cmd: "cargo", args: ["clippy", "--all-targets", "--", "-D", "warnings"], timeoutMs: 600000 },
  { name: "cargo-test", cmd: "cargo", args: ["test", "--all-targets"], timeoutMs: 600000 },
];

const AREAS: AreaDef[] = [
  {
    name: "docs",
    specialist: "z-nsh-engineer",
    auditor: "z-nsh-engineer",
    auditorsAreFfi: false,
    gates: [],
    auditFocus:
      `goals/parity/README tell one coherent story: the error table matches the 35-code contract ` +
      `exactly; every edge case maps to a named test that the rust-tests and ts-package areas own; ` +
      `parity rows cover every public API of flutter_blue_plus and react-native-ble-plx (or say ` +
      `"not applicable" with a reason); README status claims match what the updated ci.yml runs ` +
      `(rust fmt/clippy/test matrix, typescript typecheck+test job, cross-check matrix); no ` +
      `marketing filler; no dead links.`,
    implAsk: () => `You own exactly three files: docs/goals.md, docs/parity.md, README.md.

Write docs/goals.md: the crate's goals (BLE central and peripheral roles for NSH apps, in a Rust
crate and a TypeScript package with identical API names, codes and behavior); acceptance criteria
(the four named gates plus "every edge case below has the named test in BOTH languages" plus
"every call resolves exactly once: success, coded error, or cancel, never hanging"); the complete
error-code table from the contract below (35 rows: code string, Rust variant, when it fires,
typical platform trigger); the edge-case list below as a table mapping each case to its canonical
test name and the platform that triggers it; a short non-goals section (no device tests on
developer machines — GitHub CI only; web has no background scan and no peripheral role).
${CODES}
${EDGE}

Write docs/parity.md: two API-parity tables plus notes. Table 1 — flutter_blue_plus (read the
public surface in .refs/flutter_blue_plus/packages/flutter_blue_plus/lib/src/flutter_blue_plus.dart
and bluetooth_device/characteristic/service): every public static and method mapped to our
Central/Device/Server method (crate snake_case, package camelCase) with a same-behavior or
different-behavior note. Table 2 — react-native-ble-plx (read .refs/react-native-ble-plx/src/index.d.ts
and src/BleManager.js and src/BleError.js): every public BleManager method, Device/Characteristic
method, AND every BleErrorCode member mapped to ours; codes we fold together or don't model get
"not applicable" with a one-line reason (example: BluetoothManagerDestroyed has no counterpart in
a trait-based library; OperationStartFailed splits into ScanStartFailed/ConnectFailed/
AdvertiseFailed). A short note covers btleplug (its Central/Peripheral traits shaped
CentralBackend/ServerBackend; see docs/decisions.md #4) and NSH (no Bluetooth host feature in NSH
0.4.0 — decisions.md #3).

Update README.md in place: the Status section says what now exists (a Rust crate under
crates/nsh-bluetooth and a TypeScript package under packages/nsh-bluetooth mirroring it) and what
CI runs — cargo fmt --all -- --check, cargo clippy --all-targets -- -D warnings,
cargo test --all-targets on ubuntu/macos/windows, the TypeScript typecheck and tests, and the
cross-check matrix (iOS, Android, wasm targets) — matching the ci.yml the rust-tests area is
writing right now. Update the Layout list (crates/, packages/, docs/) and point at
docs/goals.md and docs/decisions.md. Keep the existing voice; no marketing filler; do not claim
anything CI does not run.

Read .refs/biometrics/docs/*.md for the org's doc voice. Do not edit any other file.
${RULES}`,
  },
  {
    name: "rust-core",
    specialist: "z-rust-engineer",
    auditor: "z-rust-engineer",
    auditorsAreFfi: false,
    resolvesCore: true,
    gates: RUST_GATES,
    auditFocus:
      `the contract is implemented exactly (35 codes, trait signatures, wrapper guards and their ` +
      `exact coded errors); every public call resolves exactly once and never hangs (no unbounded ` +
      `await on an empty channel; ConnectOptions.timeout actually races; broadcast Lagged ` +
      `resynchronises instead of failing streams); no unwrap/expect/panic on failure paths; no ` +
      `dead code (the scaffold status() fn and its test are gone); comments 1-2 lines, ` +
      `non-obvious constraints only; the five platform stub files carry exactly the seeded ` +
      `entry-point signatures; clippy-clean under -D warnings.`,
    implAsk: () => `You are the Rust core of nsh-bluetooth. You own exactly these files:
crates/nsh-bluetooth/src/lib.rs, error.rs, backend.rs, central.rs, peripheral.rs, fake.rs,
unsupported.rs, platform/mod.rs — plus crates/nsh-bluetooth/Cargo.toml (already written with
verified pins; keep versions, adjust only if a gate proves a pin wrong, with a one-line comment
saying why) — plus the five platform stub files crates/nsh-bluetooth/src/platform/{apple,android,
windows,linux,web}.rs which you SEED and the bridge areas REPLACE (you may write them now; the
five bridge areas own them from the moment they start).

Do NOT touch: crates/nsh-bluetooth/tests/, packages/, docs/, README.md, .github/, the platform
bridge implementations, .refs/.

Implement the contract below verbatim: error.rs (35 ErrorKind variants + BluetoothError), backend.rs
(types + CentralBackend + ServerBackend + BackendFuture), central.rs (Central, Device, ScanStream,
ConnectOptions, DeviceInfo and the exact guards/codes), peripheral.rs (Server and its guards),
unsupported.rs, fake.rs (FakeBackend, FakeServerBackend, FakeOp, ServerOp — the test areas drive
this exact surface), platform/mod.rs (module decls + central_backend()/server_backend() selectors
that chain the five bridges and fall back to UnsupportedBackend). lib.rs re-exports the public API,
declares the modules, DELETES the scaffold status() fn and its test (dead code).
${CONTRACT}
Seed each platform stub file with exactly this shape (name the platform correctly; the bridge area
will replace the None returns with real backends and add the cfg-gated native module below):
//! <Platform> bridge (<stack>). The <platform> bridge area owns this file; the native part lands there.
use std::sync::Arc;
use crate::backend::{CentralBackend, ServerBackend};
/// The platform's central backend, if this platform has one and it is implemented.
pub fn central_backend() -> Option<Arc<dyn CentralBackend>> {
    #[cfg(not(<your target cfg>))]
    return None;
    #[cfg(<your target cfg>)]
    {
        // Implemented by the bridge area; until then the platform falls back to UnsupportedBackend.
        return None;
    }
}
/// The platform's server (peripheral role) backend, if this platform has one and it is implemented.
pub fn server_backend() -> Option<Arc<dyn ServerBackend>> {
    #[cfg(not(<your target cfg>))]
    return None;
    #[cfg(<your target cfg>)]
    {
        // Implemented by the bridge area.
        return None;
    }
}
Target cfgs: apple -> target_vendor = "apple"; android -> target_os = "android"; windows ->
target_os = "windows"; linux -> target_os = "linux"; web -> target_family = "wasm".
Cargo.toml already carries verified target-gated dependencies for all five platforms — do not add
crates; the bridge areas extend only their own features list.

Reference material (read-only): .refs/biometrics/crates/nsh-biometrics/src/ (error.rs, backend.rs,
fake.rs show the org's exact style for the same architecture).
${RULES}`,
  },
  {
    name: "bridge-apple",
    specialist: "z-rust-engineer",
    auditor: "z-unsafe-ffi-auditor",
    auditorsAreFfi: true,
    waitsCore: true,
    gates: RUST_GATES,
    auditFocus:
      `unsafe and delegate/FFI soundness across the objc2 boundary: every nil/Option return at the ` +
      `CoreBluetooth boundary maps to a coded error (never unwrapped); delegate callbacks complete ` +
      `each one-shot continuation exactly once (didDiscoverPeripheral, didConnect/disconnect, ` +
      `didUpdateValueForCharacteristic, didReceiveRead/Write requests); the main queue/dispatch ` +
      `wiring cannot deadlock or leak continuations; state mapping ` +
      `(CBManagerState poweredOff/unsupported/unauthorized/resetting/unknown -> AdapterState; ` +
      `CBError codes -> ErrorKind) is total; no panic path; no hang path (every wait raced).`,
    implAsk: () => `You own exactly one file: crates/nsh-bluetooth/src/platform/apple.rs
(you may extend ONLY the [target.'cfg(target_vendor = "apple")'.dependencies] features lists in
crates/nsh-bluetooth/Cargo.toml if a compile proves a feature missing — never versions, never
another platform's section). Wait for the core API to exist (the run starts you after the core
area lands its files).

Implement the CoreBluetooth bridge behind #[cfg(target_vendor = "apple")] using objc2 +
objc2-core-bluetooth (versions pinned in Cargo.toml): CBCentralManager (delegate on a dedicated
dispatch queue) mapping CBManagerState to AdapterState (poweredOn -> On, poweredOff -> PoweredOff,
unauthorized -> Unauthorized, unsupported -> Unsupported, resetting -> Resetting, unknown ->
UnknownState); scanning via scanForPeripheralsWithServices + didDiscoverPeripheral into the
broadcast ScanResult stream (respect ScanFilter); connect/disconnect with one-shot continuations;
discoverServices mapping CBService/CBCharacteristic/CBDescriptor (properties from
characteristic.properties); write via writeValue:forCharacteristic:type: (WriteType = with/without
response), read via readValueForCharacteristic, notify via setNotifyValue +
didUpdateValueForCharacteristic; descriptors read/write; read_rssi from peripheral RSSI;
request_mtu returns peripheral.maximumWriteValueLengthForType + 3 (iOS has no request-MTU — never
fails); timeout
races on every async op (Timeout). Server role: CBPeripheralManager — addService (duplicate ->
ServerFailed), startAdvertising (CBPeripheralManager state/authorization errors -> AdvertiseFailed
/ Unauthorized), didReceiveReadRequest/didReceiveWriteRequest -> GattRequest into requests(),
updateValue -> notify (no subscriber -> CharacteristicNotifyChangeFailed), isAdvertising.
Pure mapping fns outside the cfg (tested in #[cfg(test)] in the same file, and callable on every
host): cb_manager_state_to_adapter_state, cb_error_to_kind, and gatt_server authorisation mapping.
The seeded central_backend()/server_backend() entry points return Some(...) on apple targets once
the backends exist. If a capability genuinely cannot be modeled, return the coded error rather
than approximating.
${BRIDGE_COMMON}
${RULES}`,
  },
  {
    name: "bridge-android",
    specialist: "z-rust-engineer",
    auditor: "z-unsafe-ffi-auditor",
    auditorsAreFfi: true,
    waitsCore: true,
    gates: [
      ...RUST_GATES,
      { name: "clippy-android-target", cmd: "cargo", args: ["clippy", "--target", "aarch64-linux-android", "--all-targets", "--", "-D", "warnings"], timeoutMs: 600000 },
    ],
    auditFocus:
      `JNI soundness: every env call's Result checked (no unwrap on JNIEnv results); exceptions ` +
      `checked and cleared after every upstream Java call (exception_occurred/exception_clear), ` +
      `mapped to coded errors; the VM from ndk-context absent or the host bridge class/method ` +
      `missing resolves immediately with Unsupported (never hangs, never panics); local refs ` +
      `deleted; no global ref leak; status mapping total (133 -> ConnectFailed with status); ` +
      `pure mapping fns unit-tested in-file.`,
    implAsk: () => `You own exactly one file: crates/nsh-bluetooth/src/platform/android.rs
(scoped Cargo.toml exception: only the [target.'cfg(target_os = "android")'.dependencies] features
lists). Wait for the core API to exist.

Implement the Android bridge behind #[cfg(target_os = "android")] using jni 0.22.4 + ndk-context
0.1.1 (biometrics' platform/android.rs in .refs/biometrics is the org's JNI pattern — read it
first). Android BLE lives in the app host, so the bridge talks to a host service over JNI: class
"dev/supernsh/nsh/BluetoothBridge" with methods the JNI side calls (startScan/stopScan/connect/
disconnect/discoverServices/readCharacteristic/writeCharacteristic/setCharacteristicNotification/
readDescriptor/writeDescriptor/requestMtu/addService/startAdvertising/stopAdvertising/notify —
JSON or byte-array arguments, and callbacks the backend registers). Every missing piece resolves
coded: ndk-context::get() None -> Unsupported; class or method not found -> Unsupported; host
callback errors -> the mapped code. Pure mapping fns live OUTSIDE the cfg and are unit-tested in
#[cfg(test)] in this file (callable on every host): android_status_to_kind (int from the host —
-1 unknown, 133 -> ConnectFailed with status Some(133), 10000+ BleErrorCode-ish inputs map like
the react-native-ble-plx table: 101 -> Unauthorized, 102 -> PoweredOff, 200/201 -> ConnectFailed
/ Disconnected, 205 -> DeviceNotConnected, etc.), android_state_to_adapter_state, permission
denials -> Unauthorized, location disabled -> LocationServicesDisabled.
The host service does not exist in NSH 0.4.0 (docs/decisions.md #3) — that is expected: on-device
behavior today is Unsupported, coded and immediate. The bridge still models the full contract so
the day the host ships BLE, the mapping is ready. Do NOT invent a Java-side implementation; the
Java class is a documented contract comment in the file.
${BRIDGE_COMMON}
${RULES}`,
  },
  {
    name: "bridge-windows",
    specialist: "z-rust-engineer",
    auditor: "z-unsafe-ffi-auditor",
    auditorsAreFfi: true,
    waitsCore: true,
    gates: [
      ...RUST_GATES,
      { name: "clippy-windows-target", cmd: "cargo", args: ["clippy", "--target", "x86_64-pc-windows-gnu", "--all-targets", "--", "-D", "warnings"], timeoutMs: 600000 },
    ],
    auditFocus:
      `COM/WinRT soundness: apartment initialised once (CoInitializeEx MTA via the windows crate) ` +
      `without double-init panics; every IAsyncOperation raced with a timeout (Timeout) and its ` +
      `error result mapped to a coded error (never unwrap); event tokens revoked on drop (no ` +
      `use-after-free via handlers); Radio state mapping total (Disabled/Off -> PoweredOff, ` +
      `Unsupported -> Unsupported, Denied -> Unauthorized); GATT status/Win32 error mapping total; ` +
      `pure mapping fns unit-tested in-file.`,
    implAsk: () => `You own exactly one file: crates/nsh-bluetooth/src/platform/windows.rs
(scoped Cargo.toml exception: only the [target.'cfg(target_os = "windows")'.dependencies] features
lists — add e.g. "Win32_System_ComThreading" or "Win32_System_WinRT" if the apartment init needs
it). Wait for the core API to exist.

Implement the Windows bridge behind #[cfg(target_os = "windows")] with the windows 0.62.2 crate
(WinRT; features pinned in Cargo.toml; btleplug/src/winrtble in .refs/btleplug is your reference —
read it first): BluetoothRadio/BluetoothAdapter state -> AdapterState (Disabled/Off -> PoweredOff,
Unsupported -> Unsupported, DeniedBySystem/User -> Unauthorized); scan via
BluetoothLEAdvertisementWatcher (Received -> ScanResult with advertisement data, StopScan ->
stop watcher; ScanningMode + acceptable service uuids honor ScanFilter); connect via
BluetoothLEDevice.FromBluetoothAddressAsync (NotFound -> DeviceNotFound); GattSession for MTU
(MaxPduSizeChanged -> request_mtu; failure -> MtuChangeFailed); services via GetGattServicesAsync
(Unreachable -> Disconnected); characteristics/descriptors via GattCharacteristicsResult mapping;
read via ReadValueAsync (IBuffer -> Vec<u8>), write via WriteValueAsync (WriteFlag distinguish
WithResponse/WithoutResponse), notify via RegisterValueChangedAsync/ValueChanged event ->
broadcast ValueNotification, descriptors likewise; connection state via ConnectionStatusChanged
-> ConnectionEvent. Server role: GattServiceProvider + GattLocalCharacteristic (subscription
tracking -> notify/SubscribedClients; ReadRequested/WriteRequested -> GattRequest;
StartAdvertising with BluetoothLEAdvertisementPublisher; errors -> AdvertiseFailed). Race EVERY
IAsyncOperation with tokio::time (Timeout). Pure mapping fns outside the cfg, unit-tested in
#[cfg(test)]: radio_state_to_adapter_state, gatt_status_to_kind (GattStatus/EBUSY/UNAUTHORIZED
values), winrt_error_to_kind (HRESULT).
${BRIDGE_COMMON}
${RULES}`,
  },
  {
    name: "bridge-linux",
    specialist: "z-rust-engineer",
    auditor: "z-rust-engineer",
    auditorsAreFfi: false,
    waitsCore: true,
    gates: RUST_GATES,
    auditFocus:
      `bluer usage correct against the vendored bluer 0.17.4 source in ~/.cargo/registry (verify ` +
      `every API name against the resolved source, not docs memory); every async call resolves ` +
      `(D-Bus replies raced with a timeout -> Timeout); adapter power/health mapping total ` +
      `(powered off -> PoweredOff, missing adapter -> Unsupported); BlueZ error mapping total; ` +
      `no panic path; GATT server registrations cleaned up on stop; NOTE the native part cannot ` +
      `compile on this Mac (bluer needs dbus C headers) — local gates check the pure parts, CI's ` +
      `ubuntu job compiles the native part; never claim local verification of the cfg-gated code.`,
    implAsk: () => `You own exactly one file: crates/nsh-bluetooth/src/platform/linux.rs
(scoped Cargo.toml exception: only the [target.'cfg(target_os = "linux")'.dependencies] features
lists). Wait for the core API to exist.

Implement the Linux bridge behind #[cfg(target_os = "linux")] with bluer 0.17.4 (BlueZ over D-Bus;
features pinned in Cargo.toml). Verify EVERY bluer API name against the vendored source: run
"cargo metadata --format-version 1" (or check ~/.cargo/registry/src/**/bluer-0.17.4/src) and read
the real signatures — never invent API names. Shape: bluer::Session; Adapter selection
(adapter() -> None -> Unsupported); power state -> AdapterState (is_powered false -> PoweredOff);
discovery via the adapter's device events/discovery stream honoring ScanFilter into ScanResult
(name, rssi, uuids, manufacturer data from advertising data); connect/disconnect/connect with
timeout; GATT client: discover_services (get_services -> Service/Characteristic/Descriptor
mapping incl properties flags), read (read_value), write (write_value with WriteType flags),
notify (notify/stop_notify + characteristic changes stream -> ValueNotification), descriptors
read/write, read_rssi, request_mtu (acquire mtu where BlueZ exposes it; MTU failures ->
MtuChangeFailed). Server role: bluer's gatt registry (register a GattApplication with
Service/Characteristic — local value storage, read/write callbacks -> GattRequest, notify to
subscribers) + adapter advertising (Advertisement registration; start/stop; errors ->
AdvertiseFailed). D-Bus timeouts: race every reply (Timeout). Pure mapping fns outside the cfg,
unit-tested in #[cfg(test)]: bluez_error_to_kind, bluez_powered_to_adapter_state.
IMPORTANT honesty note: this Mac cannot compile your cfg-gated native part (bluer needs dbus C
headers — docs/decisions.md #10). Your local gates therefore check fmt + host clippy + host tests
(the pure parts, which your in-file #[cfg(test)] tests cover on every host). CI's ubuntu job
compiles the native part — do not claim local verification of it, and keep the native code
straightforward enough to be checked there.
${BRIDGE_COMMON}
${RULES}`,
  },
  {
    name: "bridge-web",
    specialist: "z-rust-engineer",
    auditor: "z-rust-engineer",
    auditorsAreFfi: false,
    waitsCore: true,
    gates: [
      ...RUST_GATES,
      { name: "clippy-wasm-target", cmd: "cargo", args: ["clippy", "--target", "wasm32-unknown-unknown", "--all-targets", "--", "-D", "warnings"], timeoutMs: 600000 },
    ],
    auditFocus:
      `wasm-bindgen soundness: every js_sys Result handled (never unwrap); availability mapping ` +
      `total; the Web Bluetooth model stated honestly in the code (requestDevice needs a user ` +
      `gesture — no background scan; navigator.bluetooth.getAvailability/` +
      `requestDevice/getAvailability paths raced or error-mapped, never awaited unbounded); no ` +
      `peripheral role (server_backend stays None with the explaining comment); pure mapping fns ` +
      `unit-tested in-file; clippy-clean for wasm32-unknown-unknown AND the host target.`,
    implAsk: () => `You own exactly one file: crates/nsh-bluetooth/src/platform/web.rs
(scoped Cargo.toml exception: only the [target.'cfg(target_family = "wasm")'.dependencies] features
lists). Wait for the core API to exist.

Implement the Web Bluetooth bridge behind #[cfg(target_family = "wasm")] with wasm-bindgen 0.2 +
js-sys 0.3 (hand-written extern bindings for navigator.bluetooth — do NOT depend on web-sys
Bluetooth features; declare the minimal extern types yourself: Bluetooth, BluetoothDevice,
BluetoothRemoteGATTServer, BluetoothRemoteGATTService, BluetoothRemoteGATTCharacteristic,
BluetoothRemoteGATTDescriptor, RequestDeviceOptions with optionalServices/filters).
Honest Web Bluetooth model (this is what the platform gives us — say it in the file's top doc
comment): there is NO background scan and NO peripheral role. adapter_state maps
navigator.bluetooth.getAvailability() (false -> Unsupported; the API missing entirely ->
Unsupported). start_scan returns Err(Unsupported) with a message pointing at the file's own
pub async fn request_device(filter) -> Result<DeviceId, BluetoothError>, which wraps
navigator.bluetooth.requestDevice() (the user-gesture flow) and maps Web errors:
NotFoundError -> DeviceNotFound, SecurityError -> Unauthorized, invalid filter ->
InvalidIdentifiers. From a DeviceId: connect via gatt.connect(); disconnect; discover_services
(getPrimaryServices -> Service/Characteristic/Descriptor mapping, properties -> flags);
read_characteristic (readValue -> DataView bytes), write_characteristic
(writeValueWithResponse/writeValueWithoutResponse per WriteType), subscribe
(startNotifications + characteristicvaluechanged event -> broadcast ValueNotification),
descriptors read/write; request_mtu returns the fixed web value 23 and never fails; read_rssi ->
Err(Unsupported) (web has no RSSI reads — a capability the platform lacks). Server role does not
exist: server_backend() returns None forever with the explaining comment (the seeded signature
stays). Pure mapping fns
outside the cfg, unit-tested in #[cfg(test)]: web_error_name_to_kind
("NotFoundError" -> DeviceNotFound, "SecurityError" -> Unauthorized, "InvalidStateError" ->
DeviceNotConnected, unknown -> Backend).
${BRIDGE_COMMON}
${RULES}`,
  },
  {
    name: "ts-package",
    specialist: "",
    auditor: "",
    auditorsAreFfi: false,
    gates: [
      { name: "ts-typecheck", cmd: "bash", args: ["-lc", "cd packages/nsh-bluetooth && npm run typecheck"] },
      { name: "ts-test", cmd: "bash", args: ["-lc", "cd packages/nsh-bluetooth && npm test"], timeoutMs: 420000 },
    ],
    auditFocus:
      `the TS package mirrors the Rust contract exactly (same 35 code strings, same type names in ` +
      `camelCase, same wrapper guards and coded errors, memoryBackend mirrors FakeBackend/FakeOp ` +
      `one-for-one); every exported call resolves exactly once (no floating promises, no ` +
      `never-settling awaits; async iteration resyncs on lag instead of failing); strict tsc ` +
      `passes; vitest tests cover the edge-case list below and assert real behavior; no dead ` +
      `code; comments 1-2 lines.`,
    implAsk: () => `You own the whole directory packages/nsh-bluetooth/ (nothing else). Wait for
nothing — you are independent.

Create the TypeScript package that mirrors the Rust nsh-bluetooth crate API for Native Shell TSX
apps, laid out like the org's packages (read .refs/biometrics/packages/nsh-biometrics for the
layout conventions: package.json shape with "type": "module", main/types/exports pointing at
./src/index.ts, src/ + tests/):
- package.json: name "nsh-bluetooth", version "0.1.0", description "BLE central and peripheral
  roles for Native Shell TSX apps, mirroring the Rust crate", license MIT, type module,
  exports { ".": "./src/index.ts" }, scripts: "typecheck": "tsc --noEmit", "test": "vitest run".
  devDependencies: pin CURRENT stable typescript and vitest (get them at implement time via
  "npm view typescript version" and "npm view vitest version") and run npm install so a
  package-lock.json exists.
- tsconfig.json: standalone strict config (strict, noUncheckedIndexedAccess, target ES2022,
  module NodeNext, moduleResolution NodeNext, verbatimModuleSyntax, include src and tests).
  Note in a comment: the org's repos extend .nsh/tsconfig.json when built under the NSH
  toolchain; this package is toolchain-free (docs/decisions.md #7).
- src/errors.ts: the 35-code union + BluetoothError + bluetoothError + asBluetoothError exactly
  as the contract says (biometrics errors.ts is the style reference).
- src/types.ts, src/backend.ts (CentralBackend/ServerBackend interfaces — promise-based, streams
  as AsyncIterable with lag-resync), src/central.ts (Central + Device + ConnectOptions +
  platformCentral()), src/peripheral.ts (Server + platformServer()), src/memory.ts
  (createMemoryBackend mirroring FakeBackend/FakeServerBackend/FakeOp one-for-one), src/index.ts
  re-exports. The platform seam in TS: platformCentral()/platformServer() accept an injected
  backend or default to an unsupportedBackend — the browser/native wiring lands with the NSH host
  later (decisions.md #3); the unsupportedBackend returns coded Unsupported for every call.
- tests/ mirroring the edge-case list below with the EXACT canonical test names as it("...")
  titles, one test per case, driven by createMemoryBackend — plus the parity mapping tests
  (asBluetoothError ERR_NSH_HOST_ERROR forms).
${CONTRACT}
${EDGE}
Every exported call resolves exactly once and never hangs. No dead code, comments 1-2 lines.
npm install first (registry is reachable), then implement, then make typecheck + tests pass.
${RULES}`,
  },
  {
    name: "rust-tests",
    specialist: "z-rust-engineer",
    auditor: "z-rust-engineer",
    auditorsAreFfi: false,
    waitsCore: true,
    gates: [
      ...RUST_GATES,
      { name: "ci-yaml-valid", cmd: "bash", args: ["-lc", "npx --yes yaml-lint .github/workflows/ci.yml"], timeoutMs: 240000 },
      { name: "ci-covers-both", cmd: "bash", args: ["-lc", "grep -q 'cargo test' .github/workflows/ci.yml && grep -q 'npm run typecheck' .github/workflows/ci.yml && grep -q 'npm test' .github/workflows/ci.yml && grep -q 'aarch64-linux-android' .github/workflows/ci.yml && grep -q 'wasm32-unknown-unknown' .github/workflows/ci.yml"] },
    ],
    auditFocus:
      `the integration tests drive FakeBackend (never a real adapter, never a device, never an ` +
      `emulator) and cover EVERY edge case in the list with the EXACT canonical test names; each ` +
      `test asserts real behavior (a stub implementation would fail it); resolve-once is asserted ` +
      `(tokio::time::timeout wrapping the call under test where a hang would show); no dead code; ` +
      `ci.yml runs the named gates on the three OSes plus the typescript job plus the cross-check ` +
      `matrix, keeps the pinned action SHAs style, installs libdbus-1-dev pkg-config on linux, ` +
      `and claims nothing it does not run.`,
    implAsk: () => `You own exactly: crates/nsh-bluetooth/tests/*.rs (new files) and
.github/workflows/ci.yml. Wait for the core API to exist (the run starts you after the core area
lands its files) — you drive FakeBackend/FakeServerBackend, whose exact surface the contract
below pins; if the fake's surface differs from the contract, treat that as a finding and report
it in your result (do NOT edit src/ files — the core area fixes its own).

Write the Rust integration tests covering EVERY edge case in the list below, named EXACTLY as the
canonical names (snake_case fn names in tests/), split across test files by domain
(tests/adapter.rs, tests/scan.rs, tests/gatt.rs, tests/server.rs, tests/errors.rs — "every_error_
code_reachable" lives in errors.rs as a table-driven test over all 35 ErrorKind variants: exact
code string, non-empty message, Display equals the code). Every test asserts real behavior
through the public Central/Device/Server API over the fake; where a hang would be the bug, wrap
the call in tokio::time::timeout with a generous bound and assert it completed (resolve-once).
tokio dev-deps (rt, macros, time) are already in Cargo.toml.
${CONTRACT}
${EDGE}

Rewrite .github/workflows/ci.yml (keep the file's existing header comment style, the pinned-SHA
action references, CARGO_TERM_COLOR/CARGO_INCREMENTAL env, concurrency group and permissions
block): keep the format job; make the rust job a matrix [ubuntu-latest, macos-latest,
windows-latest] running cargo clippy -p nsh-bluetooth --all-targets -- -D warnings and
cargo test -p nsh-bluetooth --all-targets, with ubuntu installing libdbus-1-dev pkg-config first
(bluer's dbus dependency, see docs/decisions.md #10); add a cross-check job (matrix: macos-latest
-> aarch64-apple-ios, ubuntu-latest -> aarch64-linux-android, ubuntu-latest ->
wasm32-unknown-unknown) running cargo clippy -p nsh-bluetooth --target <t> --all-targets --
-D warnings; add a typescript job (ubuntu-latest: checkout, actions/setup-node pinned by SHA
like the other actions — verify the SHA exists via the GitHub API before pinning, node 24, then
"cd packages/nsh-bluetooth && npm install && npm run typecheck && npm test"). The workflow may
claim ONLY what it runs.
Do NOT touch: crates/nsh-bluetooth/src/**, packages/, docs/, README.md.
${RULES}`,
  },
];

// ---------------------------------------------------------------- per-area chains ----------

let resolveCoreApi!: () => void;
const coreApiReady = new Promise<void>((res) => { resolveCoreApi = res; });

function auditAsk(def: AreaDef): string {
  return (
    `READ-ONLY audit of area "${def.name}". You do not edit anything and run no mutating ` +
    `command — Read/Grep/Glob only. You audit ONLY the files this area owns; findings in any ` +
    `other file are not yours to report (the run owns sibling areas). Do not report style nits, ` +
    `speculative improvements, or anything you cannot point at in a file.\n` +
    `Area ownership: ${def.name === "docs" ? "docs/goals.md, docs/parity.md, README.md" :
      def.name === "rust-core" ? "crates/nsh-bluetooth/src/{lib,error,backend,central,peripheral,fake,unsupported,platform/mod}.rs + crates/nsh-bluetooth/Cargo.toml + the five seeded platform stub files" :
      def.name === "ts-package" ? "packages/nsh-bluetooth/** (whole directory)" :
      def.name === "rust-tests" ? "crates/nsh-bluetooth/tests/*.rs + .github/workflows/ci.yml" :
      "crates/nsh-bluetooth/src/platform/" + def.name.replace("bridge-", "") + ".rs (+ that platform's features lines in crates/nsh-bluetooth/Cargo.toml)"}.\n` +
    `Focus for this audit:\n${def.auditFocus}\n` +
    `The gates (run by the run itself, not by you) cover compile/fmt/tests; you judge design and ` +
    `contract fidelity. The API contract and error codes every area implements:\n` +
    `${CODES}\n${CONTRACT}\n` +
    `Return ONLY a JSON object (no fences): {"findings": [{"file": string, "issue": string, ` +
    `"severity": "high"|"medium"|"low"}], "contractLine": string}. findings is EMPTY when clean. ` +
    `contractLine = one sentence copied VERBATIM from your specialist contract above.`
  );
}

function fixAsk(def: AreaDef, failed: GateOut[], findings: Array<{ file: string; issue: string }>): string {
  const gateText = failed.length
    ? failed.map((g) => `GATE ${g.name} FAILED:\n${g.output}`).join("\n\n")
    : "All gates passed.";
  const findText = findings.length
    ? findings.map((f) => `- ${f.file}: ${f.issue}`).join("\n")
    : "No auditor findings.";
  return (
    `Fix round for area "${def.name}". Fix EXACTLY these problems, editing only the files you own:\n\n` +
    `${gateText}\n\nAuditor findings (fix what is real; if a finding points at a file you do not ` +
    `own, do NOT edit it — list it under blockers):\n${findText}\n\n` +
    `Rules unchanged: resolve exactly once, coded errors only, no dead code, comments 1-2 lines, ` +
    `no git commands, no other repos.\n${RULES}`
  );
}

async function runArea(def: AreaDef): Promise<AreaResult> {
  report({ name: def.name, phase: "implementing", summary: "started" }, "areas");
  try {
    if (def.waitsCore) {
      // Real dependency, not a barrier: the bridge/test areas compile against the core's files.
      await Promise.race([coreApiReady, sleep(1500000)]);
    }
    const implSpec = def.specialist
      ? await specialist(def.specialist, PLAIN_IMPL)
      : { persona: PLAIN_IMPL, source: "fallback" };
    const impl = agent(`Implementer·${def.name}`, { system: implSpec.persona, model: MODELS.pro });
    const implRes = await askVerified(impl, def.implAsk(), implSpec.persona, def.name);
    if (def.resolvesCore) resolveCoreApi();
    report({
      name: def.name, phase: "auditing",
      summary: implRes.summary.slice(0, 160) + (implRes.blockers?.length ? ` [blockers: ${implRes.blockers.length}]` : ""),
    }, "areas");

    const auditSpec = def.auditor
      ? await specialist(def.auditor, PLAIN_AUDIT)
      : { persona: PLAIN_AUDIT, source: "fallback" };
    const readonlyLine = def.auditorsAreFfi
      ? ""
      : "You are read-only in this run: never edit a file, run no mutating command.\n\n";

    let rounds = 0;
    let converged = false;
    let lastSummary = implRes.summary;
    let unverified = Boolean((implRes as any).unverifiedPersona);

    while (rounds < 3 && !converged) {
      rounds += 1;
      const gates: GateOut[] = [];
      for (const g of def.gates) gates.push(await gateRetry(g));
      const failed = gates.filter((g) => !g.ok);
      const aud = agent(`Auditor·${def.name}·r${rounds}`, {
        system: auditSpec.persona + "\n\n" + readonlyLine,
        model: MODELS.pro,
      });
      const audRes = await askVerified(aud, auditAsk(def), auditSpec.persona, def.name);
      if ((audRes as any).unverifiedPersona) unverified = true;
      const findings = Array.isArray(audRes.findings) ? audRes.findings : [];
      lastSummary = implRes.summary;

      if (failed.length === 0 && findings.length === 0) {
        converged = true;
        break;
      }
      report({
        name: def.name, phase: "fixing",
        summary: `round ${rounds}: ${failed.length} gate failure(s), ${findings.length} finding(s)`,
      }, "areas");
      // A fix after the third round's check would never be verified — stop honestly instead.
      if (rounds >= 3) break;
      const fixRes = await askVerified(impl, fixAsk(def, failed, findings), implSpec.persona, def.name);
      lastSummary = fixRes.summary || lastSummary;
      if (fixRes.blockers?.length) lastSummary += ` [blockers: ${fixRes.blockers.join("; ").slice(0, 200)}]`;
    }

    const status: "done" | "failed" = converged ? "done" : "failed";
    const summary =
      (converged
        ? `converged after ${rounds} audit round(s). ${lastSummary}`
        : `DID NOT CONVERGE within the 3-round cap (last round: gates+findings still open). ${lastSummary}`) +
      (unverified ? " [persona delivery unverified]" : "");
    report({ name: def.name, phase: status, summary: summary.slice(0, 240) }, "areas");
    return { name: def.name, status, summary: summary.slice(0, 900), rounds, converged };
  } catch (e) {
    const summary = `failed: ${String(e).slice(0, 400)}`;
    report({ name: def.name, phase: "failed", summary: summary.slice(0, 240) }, "areas");
    return { name: def.name, status: "failed", summary, rounds: 0, converged: false };
  }
}

// ---------------------------------------------------------------- the run ------------------

artifact.board("areas", {
  key: "name",
  status: "phase",
  columns: ["implementing", "auditing", "fixing", "done", "failed"],
});

phase("Implement every area in parallel; each runs its own audit chain — until zero issues, capped at 3 rounds");
const results = await Promise.all(AREAS.map(runArea));

phase("Whole-tree gates once at the join");
const joinGates: GateOut[] = [];
for (const g of [
  { name: "join-cargo-fmt", cmd: "cargo", args: ["fmt", "--all", "--", "--check"] },
  { name: "join-cargo-clippy", cmd: "cargo", args: ["clippy", "--all-targets", "--", "-D", "warnings"], timeoutMs: 600000 },
  { name: "join-cargo-test", cmd: "cargo", args: ["test", "--all-targets"], timeoutMs: 600000 },
  { name: "join-ts-typecheck", cmd: "bash", args: ["-lc", "cd packages/nsh-bluetooth && npm run typecheck"] },
  { name: "join-ts-test", cmd: "bash", args: ["-lc", "cd packages/nsh-bluetooth && npm test"], timeoutMs: 420000 },
]) {
  joinGates.push(await gate(g));
}
const failedJoin = joinGates.filter((g) => !g.ok);

const done = results.filter((r) => r.status === "done");
const failed = results.filter((r) => r.status === "failed");
const notConverged = results.filter((r) => !r.converged);
const conclusion =
  `${done.length}/${results.length} areas converged with all their gates green. ` +
  (failed.length ? `Failed or not converged: ${failed.map((f) => f.name).join(", ")}. ` : "") +
  (failedJoin.length
    ? `Whole-tree join gates FAILED: ${failedJoin.map((f) => f.name).join(", ")}.`
    : "Whole-tree join gates all green: cargo fmt/clippy/test, TS typecheck, TS tests.") +
  (notConverged.length ? ` Honest note: ${notConverged.length} area(s) never reached zero issues within the 3-round cap.` : "");

return {
  conclusion,
  areas: results,
  notes: [
    "No commits and no pushes were made anywhere (AGENTS.md + brief).",
    "Gates run locally by the workflow are the brief's named gates only; per AGENTS.md, GitHub CI (updated ci.yml) is where compile+test claims live on push. No emulator or simulator ran.",
    "The Linux bridge's cfg-gated native part cannot compile on this macOS host (bluer -> libdbus-sys needs dbus C headers, docs/decisions.md #10); CI's ubuntu job verifies it. The linux area's local gates cover the pure mapping parts.",
    failedJoin.length ? "Join gate failures are real and reported above." : "Join gates green.",
  ],
};
