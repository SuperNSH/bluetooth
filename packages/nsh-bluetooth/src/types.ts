import { bluetoothError } from "./errors.js";

/** Adapter power state, mirroring CoreBluetooth's CBManagerState plus Android's off states. */
export type AdapterState = "unknown" | "unsupported" | "unauthorized" | "poweredOff" | "resetting" | "on";

declare const uuidBrand: unique symbol;
/** A canonical lowercase hyphenated UUID, produced by `parseUuid`. */
export type Uuid = string & { readonly [uuidBrand]: true };

declare const deviceBrand: unique symbol;
/** An opaque platform device id, as handed out by a scan or `systemDevices`. */
export type DeviceId = string & { readonly [deviceBrand]: true };

const FULL_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const BASE = "-0000-1000-8000-00805f9b34fb";

/** Parses a full, 32-bit or 16-bit UUID into its full lowercase form; anything else, padded
 * input included, throws `ERR_BLUETOOTH_INVALID_IDENTIFIERS`. */
export function parseUuid(value: string): Uuid {
  const lowered = value.toLowerCase();
  const expanded =
    lowered.length === 4
      ? `0000${lowered}${BASE}`
      : lowered.length === 8
        ? `${lowered}${BASE}`
        : lowered;
  if (!FULL_UUID.test(expanded)) throw bluetoothError("ERR_BLUETOOTH_INVALID_IDENTIFIERS");
  return expanded as Uuid;
}

export interface DeviceInfo {
  id: DeviceId;
  name?: string;
}

/** GATT property flags; `true` means the characteristic allows that use. */
export interface CharacteristicProperties {
  read?: boolean;
  write?: boolean;
  writeNoResponse?: boolean;
  notify?: boolean;
  indicate?: boolean;
  broadcast?: boolean;
  extendedProperties?: boolean;
  reliableWrite?: boolean;
  writableAuxiliary?: boolean;
  authenticatedSignedWrites?: boolean;
}

export interface AdvertisementData {
  localName?: string;
  manufacturerData?: Uint8Array;
  serviceUuids: Uuid[];
  txPower?: number;
}

export interface ScanResult {
  id: DeviceId;
  name?: string;
  rssi?: number;
  advertisement: AdvertisementData;
}

/** Empty (or missing) lists match everything; a non-empty list matches on any member. */
export interface ScanFilter {
  serviceUuids?: Uuid[];
  deviceIds?: DeviceId[];
}

/** `true` when the result satisfies the filter; empty filter lists match everything. */
export function scanFilterMatches(filter: ScanFilter, result: ScanResult): boolean {
  const services = filter.serviceUuids ?? [];
  const devices = filter.deviceIds ?? [];
  const byService = services.length === 0 || services.some((uuid) => result.advertisement.serviceUuids.includes(uuid));
  const byDevice = devices.length === 0 || devices.some((id) => id === result.id);
  return byService && byDevice;
}

export interface Descriptor {
  uuid: Uuid;
  characteristicUuid: Uuid;
  serviceUuid: Uuid;
}

export interface Characteristic {
  uuid: Uuid;
  serviceUuid: Uuid;
  properties: CharacteristicProperties;
  descriptors: Descriptor[];
  value?: Uint8Array;
}

export interface Service {
  uuid: Uuid;
  primary: boolean;
  characteristics: Characteristic[];
  includedServices: Uuid[];
}

export type WriteType = "withResponse" | "withoutResponse";

export interface ValueNotification {
  characteristic: Characteristic;
  value: Uint8Array;
}

export type ConnectionEvent = { type: "connected"; id: DeviceId } | { type: "disconnected"; id: DeviceId };

export type GattRequestKind =
  | { type: "read"; offset: number }
  | { type: "write"; offset: number; data: Uint8Array; withResponse: boolean };

/** A request a remote central made against our GATT server. */
export interface GattRequest {
  serviceUuid: Uuid;
  characteristicUuid: Uuid;
  kind: GattRequestKind;
}

export interface AdvertiseOptions {
  localName?: string;
  serviceUuids: Uuid[];
}

function copyBytes(value: Uint8Array): Uint8Array {
  return new Uint8Array(value);
}

/** Deep copy, so a consumer or a backend cannot mutate the other side's view of the GATT tree. */
export function cloneCharacteristic(characteristic: Characteristic): Characteristic {
  return {
    uuid: characteristic.uuid,
    serviceUuid: characteristic.serviceUuid,
    properties: { ...characteristic.properties },
    descriptors: characteristic.descriptors.map((descriptor) => ({ ...descriptor })),
    value: characteristic.value === undefined ? undefined : copyBytes(characteristic.value),
  };
}

export function cloneService(service: Service): Service {
  return {
    uuid: service.uuid,
    primary: service.primary,
    includedServices: [...service.includedServices],
    characteristics: service.characteristics.map(cloneCharacteristic),
  };
}

export function cloneServices(services: Service[]): Service[] {
  return services.map(cloneService);
}
