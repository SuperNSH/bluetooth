import type { ServerBackend } from "./backend.js";
import { unsupportedBackend } from "./backend.js";
import { bluetoothError } from "./errors.js";
import { cloneService, parseUuid } from "./types.js";
import type { AdvertiseOptions, GattRequest, Service } from "./types.js";

/** The peripheral role: a local GATT table plus advertising to nearby centrals. */
export class Server {
  private readonly backend: ServerBackend;
  private advertising = false;

  constructor(backend: ServerBackend) {
    this.backend = backend;
  }

  /** Every UUID in the tree must parse; a duplicate service rejects with `ERR_BLUETOOTH_SERVER_FAILED`. */
  async addService(service: Service): Promise<void> {
    parseUuid(service.uuid);
    for (const uuid of service.includedServices) parseUuid(uuid);
    for (const characteristic of service.characteristics) {
      parseUuid(characteristic.uuid);
      for (const descriptor of characteristic.descriptors) parseUuid(descriptor.uuid);
    }
    await this.backend.addService(cloneService(service));
  }

  /** Resolves whether the service existed; removing twice is fine. */
  async removeService(uuid: string): Promise<void> {
    await this.backend.removeService(parseUuid(uuid));
  }

  /** One advertisement at a time; a second rejects with `ERR_BLUETOOTH_IN_PROGRESS`. */
  async advertise(options: AdvertiseOptions): Promise<void> {
    if (this.advertising) throw bluetoothError("ERR_BLUETOOTH_IN_PROGRESS");
    // Claim the slot before the await, so two concurrent calls cannot both start.
    this.advertising = true;
    try {
      for (const uuid of options.serviceUuids) parseUuid(uuid);
      await this.backend.startAdvertising({ localName: options.localName, serviceUuids: [...options.serviceUuids] });
    } catch (error) {
      this.advertising = false;
      throw error;
    }
  }

  async stopAdvertising(): Promise<void> {
    // A failed stop keeps the slot claimed, as in Rust: only success releases it.
    await this.backend.stopAdvertising();
    this.advertising = false;
  }

  async isAdvertising(): Promise<boolean> {
    return this.advertising;
  }

  async setCharacteristicValue(serviceUuid: string, characteristicUuid: string, value: Uint8Array): Promise<void> {
    await this.backend.setCharacteristicValue(parseUuid(serviceUuid), parseUuid(characteristicUuid), value);
  }

  /** Nobody subscribed rejects with `ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED`. */
  async notify(serviceUuid: string, characteristicUuid: string, value: Uint8Array): Promise<void> {
    await this.backend.notify(parseUuid(serviceUuid), parseUuid(characteristicUuid), value);
  }

  requests(): AsyncIterable<GattRequest> {
    return this.backend.requests();
  }
}

/** Picks the platform's GATT server; until a bridge exists, every call answers Unsupported. */
export function platformServer(backend?: ServerBackend): Server {
  return new Server(backend ?? unsupportedBackend);
}
