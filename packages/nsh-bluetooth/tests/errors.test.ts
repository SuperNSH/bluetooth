import { expect, it } from "vitest";
import {
  asBluetoothError,
  bluetoothError,
  BluetoothError,
  type BluetoothErrorCode,
} from "../src/index.js";

const CODES: BluetoothErrorCode[] = [
  "ERR_BLUETOOTH_UNSUPPORTED",
  "ERR_BLUETOOTH_UNAUTHORIZED",
  "ERR_BLUETOOTH_POWERED_OFF",
  "ERR_BLUETOOTH_UNKNOWN_STATE",
  "ERR_BLUETOOTH_RESETTING",
  "ERR_BLUETOOTH_STATE_CHANGE_FAILED",
  "ERR_BLUETOOTH_LOCATION_SERVICES_DISABLED",
  "ERR_BLUETOOTH_INVALID_IDENTIFIERS",
  "ERR_BLUETOOTH_IN_PROGRESS",
  "ERR_BLUETOOTH_SCAN_START_FAILED",
  "ERR_BLUETOOTH_CONNECT_FAILED",
  "ERR_BLUETOOTH_ALREADY_CONNECTED",
  "ERR_BLUETOOTH_DISCONNECTED",
  "ERR_BLUETOOTH_DEVICE_NOT_FOUND",
  "ERR_BLUETOOTH_DEVICE_NOT_CONNECTED",
  "ERR_BLUETOOTH_CANCELLED",
  "ERR_BLUETOOTH_TIMEOUT",
  "ERR_BLUETOOTH_MTU_CHANGE_FAILED",
  "ERR_BLUETOOTH_SERVICES_DISCOVERY_FAILED",
  "ERR_BLUETOOTH_SERVICES_NOT_DISCOVERED",
  "ERR_BLUETOOTH_SERVICE_NOT_FOUND",
  "ERR_BLUETOOTH_CHARACTERISTIC_WRITE_FAILED",
  "ERR_BLUETOOTH_CHARACTERISTIC_READ_FAILED",
  "ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED",
  "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND",
  "ERR_BLUETOOTH_CHARACTERISTICS_NOT_DISCOVERED",
  "ERR_BLUETOOTH_CHARACTERISTIC_INVALID_DATA_FORMAT",
  "ERR_BLUETOOTH_DESCRIPTOR_WRITE_FAILED",
  "ERR_BLUETOOTH_DESCRIPTOR_READ_FAILED",
  "ERR_BLUETOOTH_DESCRIPTOR_NOT_FOUND",
  "ERR_BLUETOOTH_DESCRIPTORS_NOT_DISCOVERED",
  "ERR_BLUETOOTH_DESCRIPTOR_WRITE_NOT_ALLOWED",
  "ERR_BLUETOOTH_ADVERTISE_FAILED",
  "ERR_BLUETOOTH_SERVER_FAILED",
  "ERR_BLUETOOTH_BACKEND",
];

it("every_error_code_reachable", () => {
  expect(CODES).toHaveLength(35);
  expect(new Set(CODES).size).toBe(35);
  for (const code of CODES) {
    const error = bluetoothError(code);
    expect(error).toBeInstanceOf(BluetoothError);
    expect(error.code).toBe(code);
    expect(error.message.length).toBeGreaterThan(0);
  }
});

it("asBluetoothError maps an ERR_NSH_HOST_ERROR message to its code", () => {
  const host = { code: "ERR_NSH_HOST_ERROR", message: "ERR_BLUETOOTH_CANCELLED: user closed the dialog" };
  expect(asBluetoothError(host).code).toBe("ERR_BLUETOOTH_CANCELLED");
});

it("asBluetoothError carries a platform status through a host error", () => {
  const host = { code: "ERR_NSH_HOST_ERROR", message: "ERR_BLUETOOTH_CONNECT_FAILED: status 133", status: 133 };
  const error = asBluetoothError(host);
  expect(error.code).toBe("ERR_BLUETOOTH_CONNECT_FAILED");
  expect(error.status).toBe(133);
});

it("asBluetoothError maps anything unknown to ERR_BLUETOOTH_BACKEND", () => {
  expect(asBluetoothError({ code: "ERR_NSH_HOST_ERROR", message: "ERR_BLUETOOTH_NOPE: x" }).code).toBe("ERR_BLUETOOTH_BACKEND");
  expect(asBluetoothError({ code: "ERR_NSH_HOST_ERROR", message: "no code in here" }).code).toBe("ERR_BLUETOOTH_BACKEND");
  expect(asBluetoothError(new Error("boom")).code).toBe("ERR_BLUETOOTH_BACKEND");
  expect(asBluetoothError(undefined).code).toBe("ERR_BLUETOOTH_BACKEND");
});

it("asBluetoothError passes a coded error through and reads plain code fields", () => {
  const error = bluetoothError("ERR_BLUETOOTH_TIMEOUT");
  expect(asBluetoothError(error)).toBe(error);
  expect(asBluetoothError({ code: "ERR_BLUETOOTH_POWERED_OFF" }).code).toBe("ERR_BLUETOOTH_POWERED_OFF");
});
