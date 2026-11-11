import { beforeEach, expect, it } from "vitest";
import {
  bluetoothError,
  BluetoothError,
  Central,
  createMemoryBackend,
  parseUuid,
  type MemoryCentralBackend,
} from "../src/index.js";
import { collect, deviceId, expectReject, flush, mkScan } from "./helpers.js";

let memory: MemoryCentralBackend;
let central: Central;

beforeEach(() => {
  memory = createMemoryBackend();
  central = new Central(memory);
});

it("bluetooth_off_reports_powered_off", async () => {
  memory.setAdapterState("poweredOff");
  await expect(central.state()).resolves.toBe("poweredOff");
  // Off is still a usable stack; only Unsupported clears this.
  await expect(central.isSupported()).resolves.toBe(true);
  // The fake does not gate scan on the adapter state; the platform bridge's refusal is scripted.
  memory.failNext("startScan", bluetoothError("ERR_BLUETOOTH_POWERED_OFF"));
  await expectReject(central.scan({}), "ERR_BLUETOOTH_POWERED_OFF");
});

it("scan_refuses_when_unauthorized", async () => {
  memory.addScannedDevice(mkScan("dev-1"));
  memory.failNext("startScan", bluetoothError("ERR_BLUETOOTH_UNAUTHORIZED"));
  await expectReject(central.scan({}), "ERR_BLUETOOTH_UNAUTHORIZED");
  memory.failNext("connect", bluetoothError("ERR_BLUETOOTH_UNAUTHORIZED"));
  await expectReject(central.device(deviceId("dev-1")).connect(), "ERR_BLUETOOTH_UNAUTHORIZED");
});

it("location_disabled_blocks_scan", async () => {
  memory.failNext("startScan", bluetoothError("ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED"));
  await expectReject(central.scan({}), "ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED");
});

it("second_scan_fails_in_progress", async () => {
  const stream = await central.scan({});
  await expectReject(central.scan({}), "ERR_BLUETOOTH_IN_PROGRESS");
  const collector = collect(stream);
  memory.addScannedDevice(mkScan("dev-1"));
  await flush();
  expect(collector.items.map((result) => result.id)).toEqual([deviceId("dev-1")]);
  await stream.stop();
  await collector.finished;
  // The gate is claimed synchronously: two racing calls cannot both start.
  const raced = await Promise.allSettled([central.scan({}), central.scan({})]);
  expect(raced.filter((outcome) => outcome.status === "rejected")).toHaveLength(1);
  const winner = raced.find((outcome) => outcome.status === "fulfilled");
  expect(winner).toBeDefined();
  if (winner?.status === "fulfilled") await winner.value.stop();
});

it("a broken-out scan releases the gate", async () => {
  const stream = await central.scan({});
  memory.addScannedDevice(mkScan("dev-1"));
  const seen: string[] = [];
  for await (const result of stream) {
    seen.push(result.id as string);
    break;
  }
  expect(seen).toEqual(["dev-1"]);
  // The for-await break is the TS drop: the gate frees and the backend scan stops.
  await flush();
  const second = await central.scan({});
  memory.addScannedDevice(mkScan("dev-2"));
  const collector = collect(second);
  await flush();
  expect(collector.items.map((result) => result.id)).toEqual([deviceId("dev-2")]);
  await second.stop();
  await collector.finished;
});

it("scan_filter_matches_service_and_device", async () => {
  const byService = await central.scan({ serviceUuids: [parseUuid("180f")] });
  const services = collect(byService);
  memory.addScannedDevice(mkScan("dev-1", ["180f"]));
  memory.addScannedDevice(mkScan("dev-2", ["180d"]));
  await flush();
  expect(services.items.map((result) => result.id)).toEqual([deviceId("dev-1")]);
  await byService.stop();
  await services.finished;

  const byDevice = await central.scan({ deviceIds: [deviceId("dev-3")] });
  const devices = collect(byDevice);
  memory.addScannedDevice(mkScan("dev-3", []));
  memory.addScannedDevice(mkScan("dev-4", []));
  await flush();
  expect(devices.items.map((result) => result.id)).toEqual([deviceId("dev-3")]);
  await byDevice.stop();
  await devices.finished;
});

it("scan_start_failed_carries_status", async () => {
  memory.failNext("startScan", new BluetoothError("ERR_BLUETOOTH_SCAN_START_FAILED", 7));
  await expectReject(central.scan({}), "ERR_BLUETOOTH_SCAN_START_FAILED", 7);
  // The failed start freed the gate; the next scan runs.
  await expect(central.scan({})).resolves.toBeDefined();
  await central.stopScan();
});

it("state_stream_reports_resetting_and_unknown", async () => {
  const collector = collect(central.stateChanges());
  memory.setAdapterState("resetting");
  memory.setAdapterState("unknown");
  await flush();
  expect(collector.items).toEqual(["resetting", "unknown"]);
  await expect(central.state()).resolves.toBe("unknown");
  await collector.stop();
});

it("systemDevices lists known devices filtered by service uuid", async () => {
  memory.addScannedDevice(mkScan("dev-1", ["180f"], "band"));
  memory.addScannedDevice(mkScan("dev-2", ["180d"]));
  await expect(central.systemDevices([])).resolves.toHaveLength(2);
  const filtered = await central.systemDevices([parseUuid("180f")]);
  expect(filtered).toEqual([{ id: deviceId("dev-1"), name: "band" }]);
});

it("a backend-ended scan releases the gate", async () => {
  const stream = await central.scan({});
  const collector = collect(stream);
  memory.addScannedDevice(mkScan("dev-1"));
  // The platform ends the scan itself; the stream drains, finishes, and frees the gate.
  await memory.stopScan();
  await collector.finished;
  expect(collector.items.map((result) => result.id)).toEqual([deviceId("dev-1")]);
  const second = await central.scan({});
  memory.addScannedDevice(mkScan("dev-2"));
  const tail = collect(second);
  await flush();
  expect(tail.items.map((result) => result.id)).toEqual([deviceId("dev-2")]);
  await second.stop();
  await tail.finished;
});

it("a stale stream's stop leaves a newer scan alone", async () => {
  const first = await central.scan({});
  await central.stopScan();
  const second = await central.scan({});
  const collector = collect(second);
  // `first` lost the gate to Central.stopScan; its late stop must not touch the backend.
  await first.stop();
  memory.addScannedDevice(mkScan("dev-1"));
  await flush();
  expect(collector.items.map((result) => result.id)).toEqual([deviceId("dev-1")]);
  // The newer scan still owns the gate, so a third scan is refused.
  await expectReject(central.scan({}), "ERR_BLUETOOTH_IN_PROGRESS");
  await second.stop();
  await collector.finished;
});

it("a stopped scan still delivers its buffered results", async () => {
  const stream = await central.scan({});
  memory.addScannedDevice(mkScan("dev-1"));
  await central.stopScan();
  // The close ends the stream, but the buffered result drains first, as tokio broadcast does.
  const collector = collect(stream);
  await collector.finished;
  expect(collector.items.map((result) => result.id)).toEqual([deviceId("dev-1")]);
});

it("scan stream resyncs to the newest 32 items after a lag", async () => {
  const stream = await memory.startScan({});
  for (let index = 0; index < 40; index += 1) memory.addScannedDevice(mkScan(`dev-${index}`));
  const collector = collect(stream);
  await flush();
  expect(collector.items).toHaveLength(32);
  expect(collector.items[0]?.id).toBe(deviceId("dev-8"));
  expect(collector.items[31]?.id).toBe(deviceId("dev-39"));
  await memory.stopScan();
  await collector.finished;
});
