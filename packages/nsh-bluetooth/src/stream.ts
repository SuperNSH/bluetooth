import type { BluetoothError } from "./errors.js";

const CAPACITY = 32;

function done<T>(): IteratorResult<T> {
  return { value: undefined, done: true };
}

/** Fan-out stream with tokio broadcast semantics: a 32-item ring; a reader that fell behind
 * skips to the newest 32 items, and buffered items stay readable after the close. */
export class Broadcast<T> {
  private readonly ring: Array<T | undefined> = new Array<T | undefined>(CAPACITY);
  private sent = 0;
  private closed = false;
  private waiters: Array<() => void> = [];

  send(value: T): void {
    if (this.closed) return;
    this.ring[this.sent % CAPACITY] = value;
    this.sent += 1;
    this.wake();
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.wake();
  }

  private wake(): void {
    const waiters = this.waiters;
    this.waiters = [];
    for (const wake of waiters) wake();
  }

  subscribe(): AsyncIterable<T> {
    const feed = this;
    let cursor = feed.sent;
    let finished = false;
    let parked: (() => void) | null = null;
    return {
      [Symbol.asyncIterator]: () => ({
        async next(): Promise<IteratorResult<T>> {
          for (;;) {
            if (finished) return done();
            if (cursor < feed.sent) {
              if (cursor < feed.sent - CAPACITY) cursor = feed.sent - CAPACITY;
              const value = feed.ring[cursor % CAPACITY] as T;
              cursor += 1;
              return { value, done: false };
            }
            if (feed.closed) {
              finished = true;
              return done();
            }
            await new Promise<void>((resolve) => {
              parked = resolve;
              feed.waiters.push(resolve);
            });
            parked = null;
          }
        },
        // return() must wake a parked next(), or the consumer's loop would never settle.
        async return(): Promise<IteratorResult<T>> {
          finished = true;
          parked?.();
          return done();
        },
      }),
    };
  }
}

/** A stream whose every iteration rejects with the coded error. */
export function failingStream<T>(error: BluetoothError): AsyncIterable<T> {
  return {
    [Symbol.asyncIterator]: () => ({
      next: (): Promise<IteratorResult<T>> => Promise.reject(error),
    }),
  };
}
