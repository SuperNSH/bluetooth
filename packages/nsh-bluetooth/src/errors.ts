/** The codes a Bluetooth rejection carries. Apps match on `code`. */
export type BluetoothErrorCode =
  | "ERR_BLUETOOTH_UNSUPPORTED"
  | "ERR_BLUETOOTH_UNAUTHORIZED"
  | "ERR_BLUETOOTH_POWERED_OFF"
  | "ERR_BLUETOOTH_UNKNOWN_STATE"
  | "ERR_BLUETOOTH_RESETTING"
  | "ERR_BLUETOOTH_STATE_CHANGE_FAILED"
  | "ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED"
  | "ERR_BLUETOOTH_INVALID_IDENTIFIERS"
  | "ERR_BLUETOOTH_IN_PROGRESS"
  | "ERR_BLUETOOTH_SCAN_START_FAILED"
  | "ERR_BLUETOOTH_CONNECT_FAILED"
  | "ERR_BLUETOOTH_ALREADY_CONNECTED"
  | "ERR_BLUETOOTH_DISCONNECTED"
  | "ERR_BLUETOOTH_DEVICE_NOT_FOUND"
  | "ERR_BLUETOOTH_DEVICE_NOT_CONNECTED"
  | "ERR_BLUETOOTH_CANCELLED"
  | "ERR_BLUETOOTH_TIMEOUT"
  | "ERR_BLUETOOTH_MTU_CHANGE_FAILED"
  | "ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED"
  | "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED"
  | "ERR_BLUETOOTH_SERVICE_NOT_FOUND"
  | "ERR_BLUETOOTH_CHARACTERISTIC_WRITE_FAILED"
  | "ERR_BLUETOOTH_CHARACTERISTIC_READ_FAILED"
  | "ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED"
  | "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND"
  | "ERR_BLUETOOTH_CHARACTERISTICS_NOT_DISCOVERED"
  | "ERR_BLUETOOTH_CHARACTERISTIC_INVALID_DATA_FORMAT"
  | "ERR_BLUETOOTH_DESCRIPTOR_WRITE_FAILED"
  | "ERR_BLUETOOTH_DESCRIPTOR_READ_FAILED"
  | "ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND"
  | "ERR_BLUETOOTH_DESCRIPTORS_NOT_DISCOVERED"
  | "ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED"
  | "ERR_BLUETOOTH_ADVERTISE_FAILED"
  | "ERR_BLUETOOTH_SERVER_FAILED"
  | "ERR_BLUETOOTH_BACKEND";

/** Same wording as the Rust crate's ErrorKind::message strings. */
const MESSAGES: Record<BluetoothErrorCode, string> = {
  ERR_BLUETOOTH_UNSUPPORTED: "Bluetooth is not usable on this platform. Check `is_supported` first.",
  ERR_BLUETOOTH_UNAUTHORIZED: "Grant the app Bluetooth permission in the system settings and try again.",
  ERR_BLUETOOTH_POWERED_OFF: "Bluetooth is off. Turn it on in the system settings and try again.",
  ERR_BLUETOOTH_UNKNOWN_STATE: "The adapter state is not known yet. Wait for `state_changes` to report it.",
  ERR_BLUETOOTH_RESETTING: "The Bluetooth stack is restarting. Wait for `state_changes` and try again.",
  ERR_BLUETOOTH_STATE_CHANGE_FAILED: "Turning Bluetooth on or off failed. Try again.",
  ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED: "Turn location services on to scan, or scan on Android 12 or newer.",
  ERR_BLUETOOTH_INVALID_IDENTIFIERS: "A UUID or device id is malformed. Use one from a scan or `system_devices`.",
  ERR_BLUETOOTH_IN_PROGRESS: "One scan or advertisement runs at a time. Stop it before starting another.",
  ERR_BLUETOOTH_SCAN_START_FAILED: "The scan did not start. Try again.",
  ERR_BLUETOOTH_CONNECT_FAILED: "The connection failed. Move closer to the device and try again.",
  ERR_BLUETOOTH_ALREADY_CONNECTED: "The device is already connected. Use it or disconnect it first.",
  ERR_BLUETOOTH_DISCONNECTED: "The connection dropped. Reconnect and try again.",
  ERR_BLUETOOTH_DEVICE_NOT_FOUND: "No device with this id is known. Use one from a scan or `system_devices`.",
  ERR_BLUETOOTH_DEVICE_NOT_CONNECTED: "The device is not connected. Connect it before using it.",
  ERR_BLUETOOTH_CANCELLED: "The operation was cancelled.",
  ERR_BLUETOOTH_TIMEOUT: "The operation timed out. Try again.",
  ERR_BLUETOOTH_MTU_CHANGE_FAILED: "The MTU change was refused. Keep the current MTU.",
  ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED: "Discovering services failed. Reconnect and try again.",
  ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED: "Call `discover_services` before GATT operations.",
  ERR_BLUETOOTH_SERVICE_NOT_FOUND: "No discovered service has this UUID. Discover services again.",
  ERR_BLUETOOTH_CHARACTERISTIC_WRITE_FAILED: "Writing the characteristic failed. Check its properties and try again.",
  ERR_BLUETOOTH_CHARACTERISTIC_READ_FAILED: "Reading the characteristic failed. Check its properties and try again.",
  ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED: "The subscription change failed. Check the characteristic's properties.",
  ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND: "No discovered characteristic has this UUID.",
  ERR_BLUETOOTH_CHARACTERISTICS_NOT_DISCOVERED: "The service's characteristics are not discovered yet.",
  ERR_BLUETOOTH_CHARACTERISTIC_INVALID_DATA_FORMAT: "The characteristic's value does not fit the expected format.",
  ERR_BLUETOOTH_DESCRIPTOR_WRITE_FAILED: "Writing the descriptor failed. Check its permissions and try again.",
  ERR_BLUETOOTH_DESCRIPTOR_READ_FAILED: "Reading the descriptor failed. Check its permissions and try again.",
  ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND: "No discovered descriptor has this UUID.",
  ERR_BLUETOOTH_DESCRIPTORS_NOT_DISCOVERED: "The characteristic's descriptors are not discovered yet.",
  ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED: "This descriptor cannot be written.",
  ERR_BLUETOOTH_ADVERTISE_FAILED: "Advertising did not start. Wait a moment and try again.",
  ERR_BLUETOOTH_SERVER_FAILED: "The GATT server operation failed. Check the service and try again.",
  ERR_BLUETOOTH_BACKEND: "The platform's Bluetooth service failed. Try again.",
};

const KNOWN = new Set<string>(Object.keys(MESSAGES));

/** A rejection of the Bluetooth API. `code` is the stable name; `status` is the platform status. */
export class BluetoothError extends Error {
  readonly code: BluetoothErrorCode;
  readonly status?: number;

  constructor(code: BluetoothErrorCode, status?: number) {
    super(MESSAGES[code]);
    this.name = "BluetoothError";
    this.code = code;
    this.status = status;
  }
}

/** The error for a known code. */
export function bluetoothError(code: BluetoothErrorCode): BluetoothError {
  return new BluetoothError(code);
}

/** Turns whatever the native side or a host threw into a coded `BluetoothError`. A host's
 * rejection carries `ERR_NSH_HOST_ERROR` with the code leading the message; unknowns are backend. */
export function asBluetoothError(error: unknown): BluetoothError {
  if (error instanceof BluetoothError) return error;
  const raw = (error ?? {}) as { code?: unknown; message?: unknown; status?: unknown };
  const code = typeof raw.code === "string" ? raw.code : "";
  const message = typeof raw.message === "string" ? raw.message : "";
  const status = typeof raw.status === "number" ? raw.status : undefined;
  if (code === "ERR_NSH_HOST_ERROR") {
    const named = /^(ERR_BLUETOOTH_[A-Z_]+): /.exec(message)?.[1];
    return named !== undefined && KNOWN.has(named)
      ? new BluetoothError(named as BluetoothErrorCode, status)
      : bluetoothError("ERR_BLUETOOTH_BACKEND");
  }
  return KNOWN.has(code)
    ? new BluetoothError(code as BluetoothErrorCode, status)
    : bluetoothError("ERR_BLUETOOTH_BACKEND");
}
