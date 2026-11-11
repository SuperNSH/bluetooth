import { expect } from "vitest";
import {
  BluetoothError,
  parseUuid,
  type DeviceId,
  type ScanResult,
  type Service,
} from "../src/index.js";

export const SERVICE_UUID = "180d";
export const CHARACTERISTIC_UUID = "2a37";
export const DESCRIPTOR_UUID = "2902";

export function deviceId(id: string): DeviceId {
  return id as DeviceId;
}

export function bytes(...values: number[]): Uint8Array {
  return new Uint8Array(values);
}

export function mkService(): Service {
  const serviceUuid = parseUuid(SERVICE_UUID);
  const characteristicUuid = parseUuid(CHARACTERISTIC_UUID);
  return {
    uuid: serviceUuid,
    primary: true,
    includedServices: [],
    characteristics: [
      {
        uuid: characteristicUuid,
        serviceUuid,
        properties: { read: true, write: true, writeNoResponse: true, notify: true },
        descriptors: [
          { uuid: parseUuid(DESCRIPTOR_UUID), characteristicUuid, serviceUuid },
        ],
        value: bytes(1),
      },
    ],
  };
}

export function mkScan(id: string, serviceUuids: string[] = [SERVICE_UUID], name?: string): ScanResult {
  return {
    id: deviceId(id),
    name,
    rssi: -55,
    advertisement: { localName: name, serviceUuids: serviceUuids.map(parseUuid) },
  };
}

/** Awaits the rejection and asserts its exact code (and status when given). */
export async function expectReject(promise: Promise<unknown>, code: string, status?: number): Promise<void> {
  const error = await promise.then(
    () => {
      throw new Error(`expected a rejection with ${code}`);
    },
    (reason: unknown) => reason,
  );
  expect(error).toBeInstanceOf(BluetoothError);
  const coded = error as BluetoothError;
  expect(coded.code).toBe(code);
  if (status !== undefined) expect(coded.status).toBe(status);
}

/** Same assertion for the sync lookups, which throw instead of rejecting. */
export function expectCoded(run: () => unknown, code: string): void {
  try {
    run();
  } catch (error) {
    expect(error).toBeInstanceOf(BluetoothError);
    expect((error as BluetoothError).code).toBe(code);
    return;
  }
  throw new Error(`expected a throw with ${code}`);
}

export interface Collector<T> {
  items: T[];
  finished: Promise<void>;
  stop(): Promise<void>;
}

/** Pumps a stream into `items` until it ends or `stop()` is called. */
export function collect<T>(stream: AsyncIterable<T>): Collector<T> {
  const items: T[] = [];
  const iterator = stream[Symbol.asyncIterator]();
  let end: () => void = () => {};
  const finished = new Promise<void>((resolve) => {
    end = resolve;
  });
  const pump = (async () => {
    try {
      for (;;) {
        const result = await iterator.next();
        if (result.done) break;
        items.push(result.value);
      }
    } finally {
      end();
    }
  })();
  // A mid-stream rejection ends the collector here; the test asserts what it needs itself.
  void pump.catch(() => end());
  const stop = async (): Promise<void> => {
    await iterator.return?.({ value: undefined, done: true });
    await finished;
  };
  return { items, finished, stop };
}

/** Lets queued sends walk their microtask chains before the test asserts. */
export function flush(): Promise<void> {
  return new Promise<void>((resolve) => setTimeout(resolve, 0));
}
