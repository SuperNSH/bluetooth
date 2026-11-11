export { asBluetoothError, BluetoothError, bluetoothError, type BluetoothErrorCode } from "./errors.js";
export type { CentralBackend, ServerBackend } from "./backend.js";
export { unsupportedBackend } from "./backend.js";
export {
  Central,
  Device,
  platformCentral,
  type ConnectOptions,
  type ScanStream,
} from "./central.js";
export { Server, platformServer } from "./peripheral.js";
export {
  createMemoryBackend,
  createMemoryServerBackend,
  type CapturedValue,
  type FakeOp,
  type MemoryCentralBackend,
  type MemoryServerBackend,
  type RecordedWrite,
  type ServerOp,
} from "./memory.js";
export type {
  AdapterState,
  AdvertiseOptions,
  AdvertisementData,
  Characteristic,
  CharacteristicProperties,
  ConnectionEvent,
  Descriptor,
  DeviceId,
  DeviceInfo,
  GattRequest,
  GattRequestKind,
  ScanFilter,
  ScanResult,
  Service,
  Uuid,
  ValueNotification,
  WriteType,
} from "./types.js";
export { parseUuid, scanFilterMatches } from "./types.js";
