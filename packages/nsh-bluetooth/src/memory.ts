import type { CentralBackend, ServerBackend } from "./backend.js";
import { bluetoothError, type BluetoothError } from "./errors.js";
import { Broadcast, failingStream } from "./stream.js";
import type {
  AdapterState,
  AdvertiseOptions,
  Characteristic,
  ConnectionEvent,
  Descriptor,
  DeviceId,
  DeviceInfo,
  GattRequest,
  ScanFilter,
  ScanResult,
  Service,
  Uuid,
  ValueNotification,
  WriteType,
} from "./types.js";
import { cloneCharacteristic, cloneService, cloneServices, parseUuid, scanFilterMatches } from "./types.js";

/** The central ops `failNext` can script, one entry per backend method. */
export type FakeOp =
  | "adapterState"
  | "stateChanges"
  | "startScan"
  | "stopScan"
  | "systemDevices"
  | "connect"
  | "disconnect"
  | "isConnected"
  | "readRssi"
  | "requestMtu"
  | "discoverServices"
  | "readCharacteristic"
  | "writeCharacteristic"
  | "subscribe"
  | "unsubscribe"
  | "readDescriptor"
  | "writeDescriptor"
  | "connectionEvents";

/** The server ops `failNextServer` can script. */
export type ServerOp =
  | "addService"
  | "removeService"
  | "startAdvertising"
  | "stopAdvertising"
  | "isAdvertising"
  | "setCharacteristicValue"
  | "notify"
  | "requests";

/** One recorded characteristic write: what the device received, and how it was sent. */
export interface RecordedWrite {
  device: DeviceId;
  characteristic: Uuid;
  value: Uint8Array;
  writeType: WriteType;
}

/** A value set through `setCharacteristicValue`, in call order. */
export interface CapturedValue {
  service: Uuid;
  characteristic: Uuid;
  value: Uint8Array;
}

/** The memory central backend plus the handles a test uses to steer it. */
export interface MemoryCentralBackend extends CentralBackend {
  setAdapterState(state: AdapterState): void;
  /** Registers the device, and delivers the result to a running scan whose filter matches. */
  addScannedDevice(result: ScanResult): void;
  /** Sets the device's GATT db, registering the device when needed. */
  setServices(deviceId: string, services: Service[]): void;
  /** The MTU every requestMtu negotiates to; default 185. */
  setMtu(mtu: number): void;
  /** One-shot: the next call of that op rejects with exactly this error, then clears. */
  failNext(op: FakeOp, error: BluetoothError): void;
  /** Marks the device disconnected and emits `disconnected` for it. */
  simulateDisconnect(deviceId: string): void;
  injectNotification(deviceId: string, characteristic: Characteristic, value: Uint8Array): void;
  injectConnectionEvent(event: ConnectionEvent): void;
  /** Holds connects to this device until `finishConnect` answers them. */
  holdConnect(deviceId: string): void;
  /** Answers held connects: `true` connects, `false` rejects `ERR_BLUETOOTH_CONNECT_FAILED`. */
  finishConnect(deviceId: string, connected: boolean): boolean;
  /** Every characteristic write the backend stored, in order. */
  writes(): RecordedWrite[];
}

/** The memory server backend plus the handles a test uses to steer it. */
export interface MemoryServerBackend extends ServerBackend {
  failNextServer(op: ServerOp, error: BluetoothError): void;
  injectRequest(request: GattRequest): void;
  /** The values set through `setCharacteristicValue`, in call order. */
  capturedValues(): CapturedValue[];
  /** How many centrals are subscribed: the simulated one for notify- or indicate-capable
   * characteristics, none otherwise. */
  subscriberCount(serviceUuid: string, characteristicUuid: string): number;
}

interface FakeDevice {
  name?: string;
  serviceUuids: Uuid[];
  rssi: number;
  connected: boolean;
  services: Service[];
  subscriptions: Map<string, Broadcast<ValueNotification>>;
  descriptorValues: Map<string, Uint8Array>;
}

interface HeldConnect {
  deviceId: string;
  resolve: () => void;
  reject: (error: BluetoothError) => void;
}

function copyBytes(value: Uint8Array): Uint8Array {
  return new Uint8Array(value);
}

function newDevice(result?: ScanResult): FakeDevice {
  return {
    name: result?.name,
    serviceUuids: result === undefined ? [] : [...result.advertisement.serviceUuids],
    rssi: result?.rssi ?? -60,
    connected: false,
    services: [],
    subscriptions: new Map(),
    descriptorValues: new Map(),
  };
}

/** A deterministic central: an in-memory GATT db, one scan, and scripted one-shot failures. */
export function createMemoryBackend(): MemoryCentralBackend {
  let adapterState: AdapterState = "on";
  const stateFeed = new Broadcast<AdapterState>();
  const connectionFeed = new Broadcast<ConnectionEvent>();
  const devices = new Map<string, FakeDevice>();
  let scanFilter: ScanFilter | null = null;
  let scanFeed: Broadcast<ScanResult> | null = null;
  const failures = new Map<FakeOp, BluetoothError[]>();
  const writeLog: RecordedWrite[] = [];
  const heldConnects = new Set<string>();
  const pendingConnects: HeldConnect[] = [];
  let negotiatedMtu = 185;

  const scripted = (op: FakeOp): BluetoothError | undefined => failures.get(op)?.shift();
  const subscriptionKey = (serviceUuid: string, characteristicUuid: string): string =>
    `${serviceUuid}|${characteristicUuid}`;
  const descriptorKey = (descriptor: Descriptor): string =>
    `${descriptor.serviceUuid}|${descriptor.characteristicUuid}|${descriptor.uuid}`;

  // Mirrors fake.rs: an unknown device is not found, a known one must be connected.
  const connectedDevice = (deviceId: string): FakeDevice => {
    const device = devices.get(deviceId);
    if (device === undefined) throw bluetoothError("ERR_BLUETOOTH_DEVICE_NOT_FOUND");
    if (!device.connected) throw bluetoothError("ERR_BLUETOOTH_DEVICE_NOT_CONNECTED");
    return device;
  };

  const findCharacteristic = (device: FakeDevice, characteristic: Characteristic): Characteristic | undefined => {
    for (const service of device.services) {
      if (service.uuid !== characteristic.serviceUuid) continue;
      const known = service.characteristics.find((candidate) => candidate.uuid === characteristic.uuid);
      if (known !== undefined) return known;
    }
    return undefined;
  };

  const hasDescriptor = (device: FakeDevice, descriptor: Descriptor): boolean => {
    for (const service of device.services) {
      if (service.uuid !== descriptor.serviceUuid) continue;
      for (const characteristic of service.characteristics) {
        if (characteristic.uuid !== descriptor.characteristicUuid) continue;
        return characteristic.descriptors.some((candidate) => candidate.uuid === descriptor.uuid);
      }
    }
    return false;
  };

  const register = (deviceId: string, result?: ScanResult): FakeDevice => {
    let device = devices.get(deviceId);
    if (device === undefined) {
      device = newDevice(result);
      devices.set(deviceId, device);
    }
    return device;
  };

  return {
    adapterState: async () => {
      const error = scripted("adapterState");
      if (error !== undefined) throw error;
      return adapterState;
    },

    stateChanges: () => {
      const error = scripted("stateChanges");
      if (error !== undefined) return failingStream<AdapterState>(error);
      return stateFeed.subscribe();
    },

    startScan: async (filter: ScanFilter) => {
      const error = scripted("startScan");
      if (error !== undefined) throw error;
      if (scanFilter !== null) throw bluetoothError("ERR_BLUETOOTH_IN_PROGRESS");
      scanFilter = filter;
      // A fresh channel closes the old receivers, ending their streams.
      scanFeed?.close();
      scanFeed = new Broadcast<ScanResult>();
      return scanFeed.subscribe();
    },

    stopScan: async () => {
      const error = scripted("stopScan");
      if (error !== undefined) throw error;
      scanFilter = null;
      scanFeed?.close();
      scanFeed = null;
    },

    systemDevices: async (uuids: Uuid[]) => {
      const error = scripted("systemDevices");
      if (error !== undefined) throw error;
      const wanted = uuids.map((uuid) => uuid as string);
      const infos: DeviceInfo[] = [];
      for (const [id, device] of devices) {
        const matches =
          wanted.length === 0 || device.serviceUuids.some((uuid) => wanted.includes(uuid as string));
        if (matches) infos.push({ id: id as DeviceId, name: device.name });
      }
      return infos;
    },

    connect: (deviceId: DeviceId) => {
      const error = scripted("connect");
      if (error !== undefined) return Promise.reject(error);
      const device = devices.get(deviceId);
      if (device === undefined) return Promise.reject(bluetoothError("ERR_BLUETOOTH_DEVICE_NOT_FOUND"));
      if (device.connected) return Promise.reject(bluetoothError("ERR_BLUETOOTH_ALREADY_CONNECTED"));
      if (heldConnects.has(deviceId)) {
        return new Promise<void>((resolve, reject) => {
          pendingConnects.push({
            deviceId,
            resolve: () => {
              device.connected = true;
              resolve();
            },
            reject,
          });
        });
      }
      device.connected = true;
      return Promise.resolve();
    },

    disconnect: async (deviceId: DeviceId) => {
      const error = scripted("disconnect");
      if (error !== undefined) throw error;
      const device = devices.get(deviceId);
      if (device !== undefined) device.connected = false;
    },

    isConnected: async (deviceId: DeviceId) => {
      const error = scripted("isConnected");
      if (error !== undefined) throw error;
      return devices.get(deviceId)?.connected ?? false;
    },

    readRssi: async (deviceId: DeviceId) => {
      const error = scripted("readRssi");
      if (error !== undefined) throw error;
      return connectedDevice(deviceId).rssi;
    },

    requestMtu: async (deviceId: DeviceId, _mtu: number) => {
      const error = scripted("requestMtu");
      if (error !== undefined) throw error;
      connectedDevice(deviceId);
      return negotiatedMtu;
    },

    discoverServices: async (deviceId: DeviceId) => {
      const error = scripted("discoverServices");
      if (error !== undefined) throw error;
      return cloneServices(connectedDevice(deviceId).services);
    },

    readCharacteristic: async (deviceId: DeviceId, characteristic: Characteristic) => {
      const error = scripted("readCharacteristic");
      if (error !== undefined) throw error;
      const device = connectedDevice(deviceId);
      const known = findCharacteristic(device, characteristic);
      if (known === undefined) throw bluetoothError("ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
      return copyBytes(known.value ?? new Uint8Array(0));
    },

    writeCharacteristic: async (deviceId: DeviceId, characteristic: Characteristic, value: Uint8Array, writeType: WriteType) => {
      const error = scripted("writeCharacteristic");
      if (error !== undefined) throw error;
      const device = connectedDevice(deviceId);
      const known = findCharacteristic(device, characteristic);
      if (known === undefined) throw bluetoothError("ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
      // The write lands in the tree, so a later discovery serves the post-write value.
      known.value = copyBytes(value);
      writeLog.push({ device: deviceId, characteristic: characteristic.uuid, value: copyBytes(value), writeType });
    },

    subscribeCharacteristic: async (deviceId: DeviceId, characteristic: Characteristic) => {
      const error = scripted("subscribe");
      if (error !== undefined) throw error;
      const device = connectedDevice(deviceId);
      if (findCharacteristic(device, characteristic) === undefined) {
        throw bluetoothError("ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
      }
      const key = subscriptionKey(characteristic.serviceUuid, characteristic.uuid);
      let subscription = device.subscriptions.get(key);
      if (subscription === undefined) {
        subscription = new Broadcast<ValueNotification>();
        device.subscriptions.set(key, subscription);
      }
      return subscription.subscribe();
    },

    unsubscribeCharacteristic: async (deviceId: DeviceId, characteristic: Characteristic) => {
      const error = scripted("unsubscribe");
      if (error !== undefined) throw error;
      const device = devices.get(deviceId);
      if (device === undefined) return;
      const key = subscriptionKey(characteristic.serviceUuid, characteristic.uuid);
      const subscription = device.subscriptions.get(key);
      if (subscription === undefined) return;
      device.subscriptions.delete(key);
      subscription.close();
    },

    readDescriptor: async (deviceId: DeviceId, descriptor: Descriptor) => {
      const error = scripted("readDescriptor");
      if (error !== undefined) throw error;
      const device = connectedDevice(deviceId);
      if (!hasDescriptor(device, descriptor)) throw bluetoothError("ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND");
      return copyBytes(device.descriptorValues.get(descriptorKey(descriptor)) ?? new Uint8Array(0));
    },

    writeDescriptor: async (deviceId: DeviceId, descriptor: Descriptor, value: Uint8Array) => {
      const error = scripted("writeDescriptor");
      if (error !== undefined) throw error;
      const device = connectedDevice(deviceId);
      if (!hasDescriptor(device, descriptor)) throw bluetoothError("ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND");
      device.descriptorValues.set(descriptorKey(descriptor), copyBytes(value));
    },

    connectionEvents: () => {
      const error = scripted("connectionEvents");
      if (error !== undefined) return failingStream<ConnectionEvent>(error);
      return connectionFeed.subscribe();
    },

    setAdapterState: (next: AdapterState) => {
      adapterState = next;
      stateFeed.send(next);
    },

    addScannedDevice: (result: ScanResult) => {
      const device = register(result.id, result);
      if (result.name !== undefined) device.name = result.name;
      if (result.rssi !== undefined) device.rssi = result.rssi;
      if (scanFeed === null || scanFilter === null) return;
      if (!scanFilterMatches(scanFilter, result)) return;
      scanFeed.send(result);
    },

    setServices: (deviceId: string, services: Service[]) => {
      register(deviceId).services = cloneServices(services);
    },

    setMtu: (mtu: number) => {
      negotiatedMtu = mtu;
    },

    failNext: (op: FakeOp, error: BluetoothError) => {
      const queue = failures.get(op) ?? [];
      queue.push(error);
      failures.set(op, queue);
    },

    simulateDisconnect: (deviceId: string) => {
      const device = devices.get(deviceId);
      if (device !== undefined) device.connected = false;
      connectionFeed.send({ type: "disconnected", id: deviceId as DeviceId });
    },

    injectNotification: (deviceId: string, characteristic: Characteristic, value: Uint8Array) => {
      const device = devices.get(deviceId);
      if (device === undefined) return;
      const subscription = device.subscriptions.get(subscriptionKey(characteristic.serviceUuid, characteristic.uuid));
      if (subscription === undefined) return;
      subscription.send({ characteristic: cloneCharacteristic(characteristic), value: copyBytes(value) });
    },

    injectConnectionEvent: (event: ConnectionEvent) => {
      connectionFeed.send(event);
    },

    holdConnect: (deviceId: string) => {
      heldConnects.add(deviceId);
    },

    finishConnect: (deviceId: string, didConnect: boolean) => {
      const mine = pendingConnects.filter((held) => held.deviceId === deviceId);
      for (const held of mine) {
        pendingConnects.splice(pendingConnects.indexOf(held), 1);
        if (didConnect) held.resolve();
        else held.reject(bluetoothError("ERR_BLUETOOTH_CONNECT_FAILED"));
      }
      return mine.length > 0;
    },

    writes: () => writeLog.map((write) => ({ ...write, value: copyBytes(write.value) })),
  };
}

/** A deterministic GATT server: a service table, an advertising flag, and scripted failures. */
export function createMemoryServerBackend(): MemoryServerBackend {
  const services: Service[] = [];
  let advertising = false;
  const failures = new Map<ServerOp, BluetoothError[]>();
  const captured: CapturedValue[] = [];
  const requestFeed = new Broadcast<GattRequest>();

  const scripted = (op: ServerOp): BluetoothError | undefined => failures.get(op)?.shift();

  const findService = (serviceUuid: string): Service | undefined =>
    services.find((service) => service.uuid === serviceUuid);

  const findCharacteristic = (serviceUuid: string, characteristicUuid: string): Characteristic | undefined =>
    findService(serviceUuid)?.characteristics.find((characteristic) => characteristic.uuid === characteristicUuid);

  // The simulated central subscribes to notify- and indicate-capable characteristics.
  const subscribed = (serviceUuid: string, characteristicUuid: string): boolean => {
    const characteristic = findCharacteristic(serviceUuid, characteristicUuid);
    return characteristic !== undefined && (characteristic.properties.notify === true || characteristic.properties.indicate === true);
  };

  return {
    addService: async (service: Service) => {
      const error = scripted("addService");
      if (error !== undefined) throw error;
      if (findService(service.uuid) !== undefined) throw bluetoothError("ERR_BLUETOOTH_SERVER_FAILED");
      services.push(cloneService(service));
    },

    removeService: async (uuid: Uuid) => {
      const error = scripted("removeService");
      if (error !== undefined) throw error;
      const index = services.findIndex((service) => service.uuid === uuid);
      if (index >= 0) services.splice(index, 1);
    },

    startAdvertising: async (_options: AdvertiseOptions) => {
      const error = scripted("startAdvertising");
      if (error !== undefined) throw error;
      if (advertising) throw bluetoothError("ERR_BLUETOOTH_IN_PROGRESS");
      advertising = true;
    },

    stopAdvertising: async () => {
      const error = scripted("stopAdvertising");
      if (error !== undefined) throw error;
      advertising = false;
    },

    isAdvertising: async () => {
      const error = scripted("isAdvertising");
      if (error !== undefined) throw error;
      return advertising;
    },

    setCharacteristicValue: async (serviceUuid: Uuid, characteristicUuid: Uuid, value: Uint8Array) => {
      const error = scripted("setCharacteristicValue");
      if (error !== undefined) throw error;
      if (findService(serviceUuid) === undefined) throw bluetoothError("ERR_BLUETOOTH_SERVICE_NOT_FOUND");
      if (findCharacteristic(serviceUuid, characteristicUuid) === undefined) {
        throw bluetoothError("ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
      }
      captured.push({ service: serviceUuid, characteristic: characteristicUuid, value: copyBytes(value) });
    },

    notify: async (serviceUuid: Uuid, characteristicUuid: Uuid, _value: Uint8Array) => {
      const error = scripted("notify");
      if (error !== undefined) throw error;
      if (findService(serviceUuid) === undefined) throw bluetoothError("ERR_BLUETOOTH_SERVICE_NOT_FOUND");
      if (findCharacteristic(serviceUuid, characteristicUuid) === undefined) {
        throw bluetoothError("ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
      }
      if (!subscribed(serviceUuid, characteristicUuid)) {
        throw bluetoothError("ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED");
      }
    },

    requests: () => {
      const error = scripted("requests");
      if (error !== undefined) return failingStream<GattRequest>(error);
      return requestFeed.subscribe();
    },

    failNextServer: (op: ServerOp, error: BluetoothError) => {
      const queue = failures.get(op) ?? [];
      queue.push(error);
      failures.set(op, queue);
    },

    injectRequest: (request: GattRequest) => {
      requestFeed.send(request);
    },

    capturedValues: () => captured.map((entry) => ({ ...entry, value: copyBytes(entry.value) })),

    subscriberCount: (serviceUuid: string, characteristicUuid: string) =>
      subscribed(parseUuid(serviceUuid), parseUuid(characteristicUuid)) ? 1 : 0,
  };
}
