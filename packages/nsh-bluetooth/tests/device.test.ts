import { beforeEach, expect, it } from "vitest";
import {
  bluetoothError,
  BluetoothError,
  Central,
  createMemoryBackend,
  parseUuid,
  type Device,
  type MemoryCentralBackend,
} from "../src/index.js";
import {
  bytes,
  CHARACTERISTIC_UUID,
  collect,
  DESCRIPTOR_UUID,
  deviceId,
  expectCoded,
  expectReject,
  flush,
  mkScan,
  mkService,
  SERVICE_UUID,
} from "./helpers.js";

let memory: MemoryCentralBackend;
let central: Central;
let device: Device;
const id = deviceId("dev-1");

beforeEach(() => {
  memory = createMemoryBackend();
  central = new Central(memory);
  device = central.device(id);
});

async function connectedAndDiscovered(): Promise<void> {
  memory.setServices(id, [mkService()]);
  await device.connect();
  await device.discoverServices();
}

it("invalid_identifier_rejected", async () => {
  for (const malformed of ["", "180", "180FF", "xyz", " 180d", "0000180d00001000800000805f9b34fb"]) {
    expectCoded(() => parseUuid(malformed), "ERR_BLUETOOTH_INVALID_IDENTIFIERS");
  }
  // 16-bit and 32-bit short forms expand to the same canonical UUID, case-insensitively.
  expect(parseUuid("2a37")).toBe(parseUuid("00002A37"));
  // An id nothing delivered is refused by the registry, not used.
  await expectReject(central.device(deviceId("ghost")).connect(), "ERR_BLUETOOTH_DEVICE_NOT_FOUND");
  await connectedAndDiscovered();
  expectCoded(() => device.service("zz"), "ERR_BLUETOOTH_INVALID_IDENTIFIERS");
  expectCoded(() => device.characteristic("zz"), "ERR_BLUETOOTH_INVALID_IDENTIFIERS");
  await expectReject(device.read("zz"), "ERR_BLUETOOTH_INVALID_IDENTIFIERS");
});

it("connect_timeout_resolves_once", async () => {
  memory.setServices(id, [mkService()]);
  memory.holdConnect(id);
  const attempt = device.connect({ timeout: 20, maxAttempts: 9 });
  const outcomes: unknown[] = [];
  attempt.catch((error) => outcomes.push(error));
  await expectReject(attempt, "ERR_BLUETOOTH_TIMEOUT");
  await expectReject(attempt, "ERR_BLUETOOTH_TIMEOUT");
  expect(outcomes).toHaveLength(1);
  // A zero timeout is a deadline already passed.
  await expectReject(device.connect({ timeout: 0 }), "ERR_BLUETOOTH_TIMEOUT");
  memory.finishConnect(id, false);
  await flush();
});

it("readRssi reports the scanned signal strength", async () => {
  memory.addScannedDevice(mkScan("dev-1"));
  // Not connected yet: the backend refuses the read.
  await expectReject(device.readRssi(), "ERR_BLUETOOTH_DEVICE_NOT_CONNECTED");
  await device.connect();
  await expect(device.readRssi()).resolves.toBe(-55);
});

it("backend GATT ops carry the registry codes", async () => {
  const characteristic = {
    uuid: parseUuid(CHARACTERISTIC_UUID),
    serviceUuid: parseUuid(SERVICE_UUID),
    properties: { read: true },
    descriptors: [],
  };
  await expectReject(memory.readCharacteristic(deviceId("ghost"), characteristic), "ERR_BLUETOOTH_DEVICE_NOT_FOUND");
  memory.addScannedDevice(mkScan("dev-1"));
  await expectReject(memory.readCharacteristic(id, characteristic), "ERR_BLUETOOTH_DEVICE_NOT_CONNECTED");
  memory.setServices(id, [mkService()]);
  await memory.connect(id);
  await expect(memory.readCharacteristic(id, characteristic)).resolves.toEqual(bytes(1));
});

it("lookups hand out copies of the shared cache", async () => {
  await connectedAndDiscovered();
  const characteristic = device.characteristic(CHARACTERISTIC_UUID);
  characteristic.value = bytes(99);
  characteristic.descriptors.push({
    uuid: parseUuid("2903"),
    characteristicUuid: parseUuid(CHARACTERISTIC_UUID),
    serviceUuid: parseUuid(SERVICE_UUID),
  });
  const service = device.service(SERVICE_UUID);
  service.characteristics.push(service.characteristics[0]!);
  // The mutations landed on copies: cache, backend tree and later reads are unaffected.
  expect(device.services[0]?.characteristics).toHaveLength(1);
  expect(device.services[0]?.characteristics[0]?.value).toEqual(bytes(1));
  await expect(device.read(CHARACTERISTIC_UUID)).resolves.toEqual(bytes(1));
  await expect(device.readDescriptor(DESCRIPTOR_UUID)).resolves.toEqual(new Uint8Array(0));
  const rediscovered = await memory.discoverServices(id);
  expect(rediscovered[0]?.characteristics).toHaveLength(1);
  expect(rediscovered[0]?.characteristics[0]?.descriptors).toHaveLength(1);
  expect(rediscovered[0]?.characteristics[0]?.value).toEqual(bytes(1));
});

it("a held connect that succeeds marks the device connected", async () => {
  memory.setServices(id, [mkService()]);
  memory.holdConnect(id);
  const attempt = device.connect();
  const settled = expect(attempt).resolves.toBeUndefined();
  // Give the backend call a turn to register its held promise, as the Rust test yields.
  await flush();
  memory.finishConnect(id, true);
  await settled;
  await expect(device.isConnected()).resolves.toBe(true);
});

it("connect_retries_android_133", async () => {
  memory.addScannedDevice(mkScan("dev-1"));
  memory.addScannedDevice(mkScan("dev-2"));
  memory.failNext("connect", new BluetoothError("ERR_BLUETOOTH_CONNECT_FAILED", 133));
  await device.connect({ maxAttempts: 2 });
  await expect(device.isConnected()).resolves.toBe(true);
  await device.disconnect();

  const other = central.device(deviceId("dev-2"));
  memory.failNext("connect", new BluetoothError("ERR_BLUETOOTH_CONNECT_FAILED", 133));
  await expectReject(other.connect({ maxAttempts: 1 }), "ERR_BLUETOOTH_CONNECT_FAILED", 133);
});

it("connect_twice_already_connected", async () => {
  memory.addScannedDevice(mkScan("dev-1"));
  await device.connect();
  await expectReject(device.connect(), "ERR_BLUETOOTH_ALREADY_CONNECTED");
  await expect(device.isConnected()).resolves.toBe(true);
});

it("cancel_connect_returns_cancelled", async () => {
  memory.setServices(id, [mkService()]);
  memory.holdConnect(id);
  const attempt = device.connect({ maxAttempts: 3 });
  const cancelled = expectReject(attempt, "ERR_BLUETOOTH_CANCELLED");
  // Any handle for the device can cancel: the pending map is shared per id.
  await central.device(id).cancelConnect();
  await cancelled;
  await device.cancelConnect();
  memory.finishConnect(id, false);
  await flush();
});

it("disconnect_is_idempotent", async () => {
  memory.addScannedDevice(mkScan("dev-1"));
  await device.disconnect();
  await device.disconnect();
  await expect(device.isConnected()).resolves.toBe(false);
  await device.connect();
  await device.disconnect();
  await device.disconnect();
  await expect(device.isConnected()).resolves.toBe(false);
});

it("lost_connection_fails_operation", async () => {
  await connectedAndDiscovered();
  const events = collect(central.connectionEvents());
  memory.simulateDisconnect(id);
  await expectReject(device.read(CHARACTERISTIC_UUID), "ERR_BLUETOOTH_DISCONNECTED");
  await flush();
  expect(events.items).toEqual([{ type: "disconnected", id }]);
  memory.injectConnectionEvent({ type: "connected", id });
  await flush();
  expect(events.items).toEqual([{ type: "disconnected", id }, { type: "connected", id }]);
  await events.stop();
});

it("gatt_before_discovery_fails", async () => {
  memory.setServices(id, [mkService()]);
  await device.connect();
  await expectReject(device.read(CHARACTERISTIC_UUID), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
  await expectReject(device.write(CHARACTERISTIC_UUID, bytes(1)), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
  await expectReject(device.subscribe(CHARACTERISTIC_UUID), "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
  expect(device.services).toEqual([]);
});

it("unknown_uuids_not_found", async () => {
  await connectedAndDiscovered();
  expectCoded(() => device.service("180f"), "ERR_BLUETOOTH_SERVICE_NOT_FOUND");
  expectCoded(() => device.characteristic("2a39"), "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
  await expectReject(device.read("2a39"), "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
  await expectReject(device.readDescriptor("2903"), "ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND");
  // The cache is shared per device id: another handle answers without the backend.
  memory.failNext("discoverServices", bluetoothError("ERR_BLUETOOTH_BACKEND"));
  const again = central.device(id);
  await expect(again.discoverServices()).resolves.toHaveLength(1);
});

it("characteristic_read_write_failures", async () => {
  await connectedAndDiscovered();
  memory.failNext("readCharacteristic", new BluetoothError("ERR_BLUETOOTH_CHARACTERISTIC_READ_FAILED", 5));
  await expectReject(device.read(CHARACTERISTIC_UUID), "ERR_BLUETOOTH_CHARACTERISTIC_READ_FAILED", 5);
  memory.failNext("writeCharacteristic", new BluetoothError("ERR_BLUETOOTH_CHARACTERISTIC_WRITE_FAILED", 6));
  await expectReject(device.write(CHARACTERISTIC_UUID, bytes(1, 2)), "ERR_BLUETOOTH_CHARACTERISTIC_WRITE_FAILED", 6);
});

it("subscribe_receives_notifications", async () => {
  await connectedAndDiscovered();
  const stream = await device.subscribe(CHARACTERISTIC_UUID);
  const collector = collect(stream);
  const characteristic = device.characteristic(CHARACTERISTIC_UUID);
  memory.injectNotification(id, characteristic, bytes(9, 9));
  await flush();
  expect(collector.items).toHaveLength(1);
  expect(Array.from(collector.items[0]?.value ?? [])).toEqual([9, 9]);
  expect(collector.items[0]?.characteristic.uuid).toBe(parseUuid(CHARACTERISTIC_UUID));
  await device.unsubscribe(CHARACTERISTIC_UUID);
  await collector.finished;
});

it("subscribe_unsubscribe_idempotent", async () => {
  await connectedAndDiscovered();
  const first = await device.subscribe(CHARACTERISTIC_UUID);
  const second = await device.subscribe(CHARACTERISTIC_UUID);
  const a = collect(first);
  const b = collect(second);
  memory.injectNotification(id, device.characteristic(CHARACTERISTIC_UUID), bytes(1));
  await flush();
  expect(a.items).toHaveLength(1);
  expect(b.items).toHaveLength(1);
  await device.unsubscribe(CHARACTERISTIC_UUID);
  await a.finished;
  await b.finished;
  await device.unsubscribe(CHARACTERISTIC_UUID);
});

it("write_without_response_records_type", async () => {
  await connectedAndDiscovered();
  await device.write(CHARACTERISTIC_UUID, bytes(7), "withoutResponse");
  const writes = memory.writes();
  expect(writes).toHaveLength(1);
  expect(writes[0]?.writeType).toBe("withoutResponse");
  expect(writes[0]?.characteristic).toBe(parseUuid(CHARACTERISTIC_UUID));
  expect(Array.from(writes[0]?.value ?? [])).toEqual([7]);
  // The write landed in the GATT tree: reads and a later backend discovery serve the new value.
  await expect(device.read(CHARACTERISTIC_UUID)).resolves.toEqual(bytes(7));
  const rediscovered = await memory.discoverServices(id);
  expect(Array.from(rediscovered[0]?.characteristics[0]?.value ?? [])).toEqual([7]);
});

it("mtu_change_failure", async () => {
  await connectedAndDiscovered();
  memory.failNext("requestMtu", bluetoothError("ERR_BLUETOOTH_MTU_CHANGE_FAILED"));
  await expectReject(device.requestMtu(512), "ERR_BLUETOOTH_MTU_CHANGE_FAILED");
  await expect(device.requestMtu(512)).resolves.toBe(185);
  memory.setMtu(240);
  await expect(device.requestMtu(100)).resolves.toBe(240);
});

it("descriptor_failures_coded", async () => {
  await connectedAndDiscovered();
  memory.failNext("readDescriptor", bluetoothError("ERR_BLUETOOTH_DESCRIPTOR_READ_FAILED"));
  await expectReject(device.readDescriptor(DESCRIPTOR_UUID), "ERR_BLUETOOTH_DESCRIPTOR_READ_FAILED");
  memory.failNext("writeDescriptor", bluetoothError("ERR_BLUETOOTH_DESCRIPTOR_WRITE_FAILED"));
  await expectReject(device.writeDescriptor(DESCRIPTOR_UUID, bytes(1)), "ERR_BLUETOOTH_DESCRIPTOR_WRITE_FAILED");
  memory.failNext("writeDescriptor", bluetoothError("ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED"));
  await expectReject(device.writeDescriptor(DESCRIPTOR_UUID, bytes(1)), "ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED");
  await device.writeDescriptor(DESCRIPTOR_UUID, bytes(3, 1));
  await expect(device.readDescriptor(DESCRIPTOR_UUID)).resolves.toEqual(bytes(3, 1));
  // The backend itself refuses a descriptor the device does not serve (fake.rs has_descriptor).
  const missing = { uuid: parseUuid("2903"), characteristicUuid: parseUuid(CHARACTERISTIC_UUID), serviceUuid: parseUuid(SERVICE_UUID) };
  await expectReject(memory.writeDescriptor(id, missing, bytes(1)), "ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND");
  await expectReject(memory.readDescriptor(id, missing), "ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND");
});
