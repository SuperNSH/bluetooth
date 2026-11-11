import { bluetoothError } from "./errors.js";
import { failingStream } from "./stream.js";
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

/** What a `Central` runs on. Every method rejects with `BluetoothError`, never a raw throw. */
export interface CentralBackend {
  adapterState(): Promise<AdapterState>;
  stateChanges(): AsyncIterable<AdapterState>;
  startScan(filter: ScanFilter): Promise<AsyncIterable<ScanResult>>;
  stopScan(): Promise<void>;
  systemDevices(uuids: Uuid[]): Promise<DeviceInfo[]>;
  connect(id: DeviceId): Promise<void>;
  disconnect(id: DeviceId): Promise<void>;
  isConnected(id: DeviceId): Promise<boolean>;
  readRssi(id: DeviceId): Promise<number>;
  requestMtu(id: DeviceId, mtu: number): Promise<number>;
  discoverServices(id: DeviceId): Promise<Service[]>;
  readCharacteristic(id: DeviceId, characteristic: Characteristic): Promise<Uint8Array>;
  writeCharacteristic(
    id: DeviceId,
    characteristic: Characteristic,
    value: Uint8Array,
    writeType: WriteType,
  ): Promise<void>;
  subscribeCharacteristic(id: DeviceId, characteristic: Characteristic): Promise<AsyncIterable<ValueNotification>>;
  unsubscribeCharacteristic(id: DeviceId, characteristic: Characteristic): Promise<void>;
  readDescriptor(id: DeviceId, descriptor: Descriptor): Promise<Uint8Array>;
  writeDescriptor(id: DeviceId, descriptor: Descriptor, value: Uint8Array): Promise<void>;
  connectionEvents(): AsyncIterable<ConnectionEvent>;
}

/** What a `Server` runs on: a platform GATT server bridge, or a memory backend in tests. */
export interface ServerBackend {
  addService(service: Service): Promise<void>;
  removeService(uuid: Uuid): Promise<void>;
  startAdvertising(options: AdvertiseOptions): Promise<void>;
  stopAdvertising(): Promise<void>;
  isAdvertising(): Promise<boolean>;
  setCharacteristicValue(serviceUuid: Uuid, characteristicUuid: Uuid, value: Uint8Array): Promise<void>;
  notify(serviceUuid: Uuid, characteristicUuid: Uuid, value: Uint8Array): Promise<void>;
  requests(): AsyncIterable<GattRequest>;
}

function unsupported(): Promise<never> {
  return Promise.reject(bluetoothError("ERR_BLUETOOTH_UNSUPPORTED"));
}

function unsupportedStream<T>(): AsyncIterable<T> {
  return failingStream<T>(bluetoothError("ERR_BLUETOOTH_UNSUPPORTED"));
}

/** The seam until the NSH host ships a bridge (docs/decisions.md #3): every call answers
 * `ERR_BLUETOOTH_UNSUPPORTED`, and nothing hangs. */
export const unsupportedBackend: CentralBackend & ServerBackend = {
  adapterState: unsupported,
  stateChanges: () => unsupportedStream<AdapterState>(),
  startScan: unsupported,
  stopScan: unsupported,
  systemDevices: unsupported,
  connect: unsupported,
  disconnect: unsupported,
  isConnected: unsupported,
  readRssi: unsupported,
  requestMtu: unsupported,
  discoverServices: unsupported,
  readCharacteristic: unsupported,
  writeCharacteristic: unsupported,
  subscribeCharacteristic: unsupported,
  unsubscribeCharacteristic: unsupported,
  readDescriptor: unsupported,
  writeDescriptor: unsupported,
  connectionEvents: () => unsupportedStream<ConnectionEvent>(),

  addService: unsupported,
  removeService: unsupported,
  startAdvertising: unsupported,
  stopAdvertising: unsupported,
  isAdvertising: unsupported,
  setCharacteristicValue: unsupported,
  notify: unsupported,
  requests: () => unsupportedStream<GattRequest>(),
};
