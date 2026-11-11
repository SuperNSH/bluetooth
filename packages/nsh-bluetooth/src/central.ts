import type { CentralBackend } from "./backend.js";
import { unsupportedBackend } from "./backend.js";
import { asBluetoothError, bluetoothError } from "./errors.js";
import type {
  AdapterState,
  Characteristic,
  ConnectionEvent,
  Descriptor,
  DeviceId,
  DeviceInfo,
  ScanFilter,
  ScanResult,
  Service,
  Uuid,
  ValueNotification,
  WriteType,
} from "./types.js";
import { cloneCharacteristic, cloneService, cloneServices, parseUuid } from "./types.js";

// Stands in for "no timeout armed" in the connect race; it never settles.
const NEVER_SETTLES: Promise<never> = new Promise<never>(() => {});

/** A scan in flight: iterate the results, or `stop()` to end it. */
export type ScanStream = AsyncIterable<ScanResult> & { stop(): Promise<void> };

export interface ConnectOptions {
  /** Milliseconds the whole connect (all attempts together) may take; 0 is a passed deadline. */
  timeout?: number;
  /** Total tries, including the first. Android status 133 is retried up to this. */
  maxAttempts?: number;
}

/** One scan per `Central`. `active` names the generation that owns the gate, so an old stream's
 * end cannot release a newer scan. */
interface ScanGate {
  next: number;
  active: number | null;
}

/** The discovery cache every handle for one device id shares, set once, like Rust's OnceLock. */
interface DeviceState {
  services: Service[] | undefined;
}

/** The cancel signals of the connects waiting on one device, keyed by registration. */
interface CancelSlot {
  cancel: () => void;
}

interface CentralShared {
  backend: CentralBackend;
  scan: ScanGate;
  devices: Map<string, DeviceState>;
  connects: Map<string, Set<CancelSlot>>;
}

/** The central role: adapter state, scanning, and device handles. */
export class Central {
  private readonly shared: CentralShared;

  constructor(backend: CentralBackend) {
    this.shared = {
      backend,
      scan: { next: 0, active: null },
      devices: new Map(),
      connects: new Map(),
    };
  }

  // False only for a reported or thrown Unsupported; any other failure still means a stack exists.
  async isSupported(): Promise<boolean> {
    try {
      return (await this.shared.backend.adapterState()) !== "unsupported";
    } catch (error) {
      return asBluetoothError(error).code !== "ERR_BLUETOOTH_UNSUPPORTED";
    }
  }

  state(): Promise<AdapterState> {
    return this.shared.backend.adapterState();
  }

  stateChanges(): AsyncIterable<AdapterState> {
    return this.shared.backend.stateChanges();
  }

  /** Connection events for the devices this central uses, from the next one on. */
  connectionEvents(): AsyncIterable<ConnectionEvent> {
    return this.shared.backend.connectionEvents();
  }

  /** One scan per `Central` at a time; a second one rejects with `ERR_BLUETOOTH_IN_PROGRESS`.
   * The gate is claimed synchronously, so concurrent calls cannot both start. */
  async scan(filter: ScanFilter = {}): Promise<ScanStream> {
    const gate = this.shared.scan;
    if (gate.active !== null) throw bluetoothError("ERR_BLUETOOTH_IN_PROGRESS");
    const generation = gate.next;
    gate.next += 1;
    gate.active = generation;
    let results: AsyncIterable<ScanResult>;
    try {
      results = await this.shared.backend.startScan(filter);
    } catch (error) {
      this.endScan(generation);
      throw error;
    }
    return this.relay(results, generation);
  }

  /** Ends the running scan, whichever generation owns the gate. Resolves when none was running. */
  async stopScan(): Promise<void> {
    const active = this.shared.scan.active;
    if (active === null) return;
    // The stop completes before the gate is released; a failed stop still frees the gate.
    await this.shared.backend.stopScan().catch(() => {});
    this.endScan(active);
  }

  // Releases the gate when `generation` still owns it; an old stream cannot end a newer scan.
  private endScan(generation: number): void {
    if (this.shared.scan.active === generation) this.shared.scan.active = null;
  }

  private relay(results: AsyncIterable<ScanResult>, generation: number): ScanStream {
    const shared = this.shared;
    const iterator = results[Symbol.asyncIterator]();
    let done = false;
    // The backend stop is paid in line, before the gate is reusable (Rust's stop, not Drop);
    // a stale stream that lost the gate to Central.stopScan stops nothing.
    const end = async (): Promise<void> => {
      done = true;
      if (shared.scan.active !== generation) return;
      await shared.backend.stopScan().catch(() => {});
      if (shared.scan.active === generation) shared.scan.active = null;
    };
    const stop = async (): Promise<void> => {
      if (done) return;
      await end();
    };
    return {
      stop,
      [Symbol.asyncIterator]: () => ({
        async next(): Promise<IteratorResult<ScanResult>> {
          if (done) return { value: undefined, done: true };
          const result = await iterator.next();
          if (result.done) {
            // The platform ended the scan; free the gate as recv()'s Closed arm does in Rust,
            // or every later scan on this Central would meet InProgress forever.
            await end();
            return { value: undefined, done: true };
          }
          return result;
        },
        // A broken-out loop pays the stop inline; TS can await where Rust's Drop owes it.
        return: (): Promise<IteratorResult<ScanResult>> =>
          stop().then(() => ({ value: undefined, done: true })),
      }),
    };
  }

  systemDevices(uuids: Uuid[]): Promise<DeviceInfo[]> {
    return this.shared.backend.systemDevices(uuids);
  }

  /** A handle for the device. Handles for one id share their discovery cache. */
  device(id: DeviceId | string): Device {
    const key = id as string;
    let state = this.shared.devices.get(key);
    if (state === undefined) {
      state = { services: undefined };
      this.shared.devices.set(key, state);
    }
    return new Device(this.shared, key as DeviceId, state);
  }
}

/** One remote device: the connection, its services, and the GATT operations on them. */
export class Device {
  readonly id: DeviceId;
  private readonly shared: CentralShared;
  private readonly state: DeviceState;

  constructor(shared: CentralShared, id: DeviceId, state: DeviceState) {
    this.shared = shared;
    this.id = id;
    this.state = state;
  }

  /** Connects, retrying only Android's status 133. The timeout, when set, races the whole
   * connect (every attempt); `cancelConnect` rejects it with `ERR_BLUETOOTH_CANCELLED`. */
  async connect(options: ConnectOptions = {}): Promise<void> {
    if (options.timeout === 0) throw bluetoothError("ERR_BLUETOOTH_TIMEOUT");
    let attemptsLeft = Math.max(1, options.maxAttempts ?? 1);
    const deadline = options.timeout === undefined ? undefined : Date.now() + options.timeout;
    const cancelled = bluetoothError("ERR_BLUETOOTH_CANCELLED");
    const timedOut = bluetoothError("ERR_BLUETOOTH_TIMEOUT");
    let rejectCancel: ((reason: unknown) => void) | undefined;
    const cancelSignal = new Promise<never>((_, reject) => {
      rejectCancel = reject;
    });
    cancelSignal.catch(() => {});
    // Registered in the shared map, so any handle for this id can cancel this connect.
    const slot: CancelSlot = { cancel: () => rejectCancel?.(cancelled) };
    const waiting = this.shared.connects.get(this.id) ?? new Set<CancelSlot>();
    this.shared.connects.set(this.id, waiting);
    waiting.add(slot);
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      for (;;) {
        const remaining = deadline === undefined ? Number.POSITIVE_INFINITY : deadline - Date.now();
        if (remaining <= 0) throw timedOut;
        // Promise.resolve().then turns a backend that throws synchronously into a rejection.
        const attempt = Promise.resolve().then(() => this.shared.backend.connect(this.id));
        const timeoutSignal: Promise<never> =
          deadline === undefined
            ? NEVER_SETTLES
            : new Promise<never>((_, reject) => {
                // No timer without a deadline: setTimeout clamps Infinity down to 1ms.
                timer = setTimeout(() => reject(timedOut), remaining);
              });
        // Losers of the race must settle silently, never as unhandled rejections.
        attempt.catch(() => {});
        timeoutSignal.catch(() => {});
        try {
          await Promise.race([attempt, timeoutSignal, cancelSignal]);
          return;
        } catch (error) {
          if (error === timedOut || error === cancelled) throw error;
          const coded = asBluetoothError(error);
          if (coded.code === "ERR_BLUETOOTH_CONNECT_FAILED" && coded.status === 133 && attemptsLeft > 1) {
            attemptsLeft -= 1;
            continue;
          }
          throw coded;
        } finally {
          if (timer !== undefined) clearTimeout(timer);
        }
      }
    } finally {
      waiting.delete(slot);
      if (waiting.size === 0) this.shared.connects.delete(this.id);
    }
  }

  /** Cancels every pending connect of this device; each resolves `ERR_BLUETOOTH_CANCELLED`.
   * With nothing pending, this still resolves. */
  async cancelConnect(): Promise<void> {
    const waiting = this.shared.connects.get(this.id);
    if (waiting === undefined) return;
    for (const slot of [...waiting]) slot.cancel();
  }

  async disconnect(): Promise<void> {
    return this.shared.backend.disconnect(this.id);
  }

  isConnected(): Promise<boolean> {
    return this.shared.backend.isConnected(this.id);
  }

  readRssi(): Promise<number> {
    return this.shared.backend.readRssi(this.id);
  }

  requestMtu(mtu: number): Promise<number> {
    return this.shared.backend.requestMtu(this.id, mtu);
  }

  /** Discovers once; later calls, from any handle, return the shared cache. */
  async discoverServices(): Promise<Service[]> {
    if (this.state.services !== undefined) return cloneServices(this.state.services);
    const services = await this.shared.backend.discoverServices(this.id);
    this.state.services = cloneServices(services);
    return cloneServices(this.state.services);
  }

  /** Plain getter: empty before discovery, never an error. */
  get services(): Service[] {
    return this.state.services === undefined ? [] : cloneServices(this.state.services);
  }

  /** The lookup results are deep copies: the shared cache is Central-owned state. */
  service(uuid: string): Service {
    const parsed = parseUuid(uuid);
    const services = this.discovered();
    const service = services.find((candidate) => candidate.uuid === parsed);
    if (service === undefined) throw bluetoothError("ERR_BLUETOOTH_SERVICE_NOT_FOUND");
    return cloneService(service);
  }

  characteristic(uuid: string): Characteristic {
    return this.knownCharacteristic(uuid);
  }

  async read(uuid: string): Promise<Uint8Array> {
    const characteristic = this.knownCharacteristic(uuid);
    await this.requireConnected();
    return this.shared.backend.readCharacteristic(this.id, characteristic);
  }

  async write(uuid: string, value: Uint8Array, writeType: WriteType = "withResponse"): Promise<void> {
    const characteristic = this.knownCharacteristic(uuid);
    await this.requireConnected();
    await this.shared.backend.writeCharacteristic(this.id, characteristic, value, writeType);
  }

  /** Subscribing again keeps one logical subscription and returns a fresh stream. */
  async subscribe(uuid: string): Promise<AsyncIterable<ValueNotification>> {
    const characteristic = this.knownCharacteristic(uuid);
    await this.requireConnected();
    return this.shared.backend.subscribeCharacteristic(this.id, characteristic);
  }

  /** Unsubscribes from the characteristic. Ok when not subscribed. */
  async unsubscribe(uuid: string): Promise<void> {
    const characteristic = this.knownCharacteristic(uuid);
    await this.requireConnected();
    await this.shared.backend.unsubscribeCharacteristic(this.id, characteristic);
  }

  async readDescriptor(uuid: string): Promise<Uint8Array> {
    const descriptor = this.knownDescriptor(uuid);
    await this.requireConnected();
    return this.shared.backend.readDescriptor(this.id, descriptor);
  }

  async writeDescriptor(uuid: string, value: Uint8Array): Promise<void> {
    const descriptor = this.knownDescriptor(uuid);
    await this.requireConnected();
    await this.shared.backend.writeDescriptor(this.id, descriptor, value);
  }

  private discovered(): Service[] {
    if (this.state.services === undefined) {
      throw bluetoothError("ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED");
    }
    return this.state.services;
  }

  private knownCharacteristic(uuid: string): Characteristic {
    const parsed = parseUuid(uuid);
    const services = this.discovered();
    for (const service of services) {
      const characteristic = service.characteristics.find((candidate) => candidate.uuid === parsed);
      if (characteristic !== undefined) return cloneCharacteristic(characteristic);
    }
    throw bluetoothError("ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
  }

  private knownDescriptor(uuid: string): Descriptor {
    const parsed = parseUuid(uuid);
    const services = this.discovered();
    for (const service of services) {
      for (const characteristic of service.characteristics) {
        const descriptor = characteristic.descriptors.find((candidate) => candidate.uuid === parsed);
        if (descriptor !== undefined) return { ...descriptor };
      }
    }
    throw bluetoothError("ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND");
  }

  // A GATT op needs a device that is still connected; a dropped one reports Disconnected.
  private async requireConnected(): Promise<void> {
    if (!(await this.shared.backend.isConnected(this.id))) {
      throw bluetoothError("ERR_BLUETOOTH_DISCONNECTED");
    }
  }
}

/** Picks the platform's central; until the NSH host ships one, every call answers Unsupported. */
export function platformCentral(backend?: CentralBackend): Central {
  return new Central(backend ?? unsupportedBackend);
}
