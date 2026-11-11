import { expect, it } from "vitest";
import {
  parseUuid,
  platformCentral,
  platformServer,
  unsupportedBackend,
  type Characteristic,
  type Descriptor,
} from "../src/index.js";
import { deviceId, expectReject, mkService } from "./helpers.js";

it("unsupported_platform_codes_every_call", async () => {
  const backend = unsupportedBackend;
  const id = deviceId("dev-1");
  const characteristic: Characteristic = {
    uuid: parseUuid("2a37"),
    serviceUuid: parseUuid("180d"),
    properties: { read: true },
    descriptors: [],
  };
  const descriptor: Descriptor = {
    uuid: parseUuid("2902"),
    characteristicUuid: characteristic.uuid,
    serviceUuid: characteristic.serviceUuid,
  };
  const calls: Array<() => Promise<unknown>> = [
    () => backend.adapterState(),
    () => backend.startScan({}),
    () => backend.stopScan(),
    () => backend.systemDevices([]),
    () => backend.connect(id),
    () => backend.disconnect(id),
    () => backend.isConnected(id),
    () => backend.readRssi(id),
    () => backend.requestMtu(id, 185),
    () => backend.discoverServices(id),
    () => backend.readCharacteristic(id, characteristic),
    () => backend.writeCharacteristic(id, characteristic, new Uint8Array(), "withResponse"),
    () => backend.subscribeCharacteristic(id, characteristic),
    () => backend.unsubscribeCharacteristic(id, characteristic),
    () => backend.readDescriptor(id, descriptor),
    () => backend.writeDescriptor(id, descriptor, new Uint8Array()),
    () => backend.addService(mkService()),
    () => backend.removeService(parseUuid("180d")),
    () => backend.startAdvertising({ serviceUuids: [] }),
    () => backend.stopAdvertising(),
    () => backend.isAdvertising(),
    () => backend.setCharacteristicValue(parseUuid("180d"), parseUuid("2a37"), new Uint8Array()),
    () => backend.notify(parseUuid("180d"), parseUuid("2a37"), new Uint8Array()),
  ];
  for (const call of calls) await expectReject(call(), "ERR_BLUETOOTH_UNSUPPORTED");
  for (const stream of [backend.stateChanges(), backend.connectionEvents(), backend.requests()]) {
    await expectReject(stream[Symbol.asyncIterator]().next(), "ERR_BLUETOOTH_UNSUPPORTED");
  }
  // The high-level seams answer through the same backend, and isSupported never rejects.
  await expectReject(platformCentral().scan({}), "ERR_BLUETOOTH_UNSUPPORTED");
  await expectReject(platformServer().advertise({ serviceUuids: [] }), "ERR_BLUETOOTH_UNSUPPORTED");
  await expect(platformCentral().isSupported()).resolves.toBe(false);
});
