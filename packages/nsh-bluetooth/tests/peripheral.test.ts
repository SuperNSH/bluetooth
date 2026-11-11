import { beforeEach, expect, it } from "vitest";
import {
  bluetoothError,
  createMemoryServerBackend,
  parseUuid,
  platformServer,
  Server,
  type MemoryServerBackend,
} from "../src/index.js";
import {
  bytes,
  CHARACTERISTIC_UUID,
  collect,
  expectReject,
  flush,
  mkService,
  SERVICE_UUID,
} from "./helpers.js";

let memory: MemoryServerBackend;
let server: Server;

beforeEach(() => {
  memory = createMemoryServerBackend();
  server = new Server(memory);
});

it("advertise_twice_in_progress", async () => {
  await server.advertise({ serviceUuids: [parseUuid(SERVICE_UUID)] });
  await expect(server.isAdvertising()).resolves.toBe(true);
  await expectReject(server.advertise({ serviceUuids: [] }), "ERR_BLUETOOTH_IN_PROGRESS");
  await server.stopAdvertising();
  await expect(server.isAdvertising()).resolves.toBe(false);
  await server.stopAdvertising();
});

it("advertise_unsupported_on_platform", async () => {
  const fallback = platformServer();
  await expectReject(fallback.advertise({ serviceUuids: [] }), "ERR_BLUETOOTH_UNSUPPORTED");
  // The failed start freed the wrapper's slot; the refusal repeats, not InProgress.
  await expect(fallback.isAdvertising()).resolves.toBe(false);
  await expectReject(fallback.advertise({ serviceUuids: [] }), "ERR_BLUETOOTH_UNSUPPORTED");
});

it("advertise_failure_coded", async () => {
  memory.failNextServer("startAdvertising", bluetoothError("ERR_BLUETOOTH_ADVERTISE_FAILED"));
  await expectReject(server.advertise({ serviceUuids: [] }), "ERR_BLUETOOTH_ADVERTISE_FAILED");
  // The failed start freed the slot; a retry runs.
  await expect(server.isAdvertising()).resolves.toBe(false);
  await server.advertise({ serviceUuids: [] });
  await server.stopAdvertising();
});

it("notify_without_subscriber_fails", async () => {
  await server.addService(mkService());
  // 2A37 is notify-capable, so the simulated central is subscribed and the notify lands.
  await server.notify(SERVICE_UUID, CHARACTERISTIC_UUID, bytes(1));
  // A plain read characteristic has no subscriber.
  await server.addService({
    uuid: parseUuid("180f"),
    primary: true,
    includedServices: [],
    characteristics: [
      {
        uuid: parseUuid("2a19"),
        serviceUuid: parseUuid("180f"),
        properties: { read: true },
        descriptors: [],
      },
    ],
  });
  await expectReject(server.notify("180f", "2a19", bytes(1)), "ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED");
  expect(memory.subscriberCount(SERVICE_UUID, CHARACTERISTIC_UUID)).toBe(1);
  expect(memory.subscriberCount("180f", "2a19")).toBe(0);
});

it("duplicate_service_fails", async () => {
  await server.addService(mkService());
  await expectReject(server.addService(mkService()), "ERR_BLUETOOTH_SERVER_FAILED");
  await server.removeService(SERVICE_UUID);
  await server.removeService(SERVICE_UUID);
  await server.addService(mkService());
});

it("server_unknown_characteristic", async () => {
  await server.addService(mkService());
  await expectReject(server.setCharacteristicValue("180f", CHARACTERISTIC_UUID, bytes(1)), "ERR_BLUETOOTH_SERVICE_NOT_FOUND");
  await expectReject(server.setCharacteristicValue(SERVICE_UUID, "2a39", bytes(1)), "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
  await expectReject(server.notify("180f", CHARACTERISTIC_UUID, bytes(1)), "ERR_BLUETOOTH_SERVICE_NOT_FOUND");
  await expectReject(server.notify(SERVICE_UUID, "2a39", bytes(1)), "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
  // A successful set lands in capturedValues, mirroring fake.rs.
  await server.setCharacteristicValue(SERVICE_UUID, CHARACTERISTIC_UUID, bytes(1, 2));
  expect(memory.capturedValues()).toEqual([
    { service: parseUuid(SERVICE_UUID), characteristic: parseUuid(CHARACTERISTIC_UUID), value: bytes(1, 2) },
  ]);
});

it("requests stream delivers remote reads and writes", async () => {
  await server.addService(mkService());
  const collector = collect(server.requests());
  memory.injectRequest({
    serviceUuid: parseUuid(SERVICE_UUID),
    characteristicUuid: parseUuid(CHARACTERISTIC_UUID),
    kind: { type: "read", offset: 0 },
  });
  memory.injectRequest({
    serviceUuid: parseUuid(SERVICE_UUID),
    characteristicUuid: parseUuid(CHARACTERISTIC_UUID),
    kind: { type: "write", offset: 0, data: bytes(4, 2), withResponse: true },
  });
  await flush();
  expect(collector.items).toHaveLength(2);
  expect(collector.items[1]?.kind.type).toBe("write");
  await collector.stop();
});
