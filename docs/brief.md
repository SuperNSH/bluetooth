# Bluetooth LE: brief

BLE central and peripheral roles for NSH apps.

## Options

- btleplug: Rust central on Linux, Windows and macOS
- blew or objc2-core-bluetooth: Apple peripheral and GATT server
- bluer: Linux BlueZ bindings
- Reference only: flutter_blue_plus, react-native-ble-plx

## Recommended path

btleplug for central. Native peripheral code for Apple and Android.

## Edge cases to design for

- Bluetooth off and unauthorized states must be explicit
- Android 12+ needs runtime scan and connect permissions
- MTU limits; request MTU explicitly on Android
- Disconnects (Android status 133) need retry logic
- iOS Simulator has no BLE; tests use a fake transport

## Research status

Research done 2026-11-09 by web search. Versions and licences are unverified until checked on the registry before pinning. Full catalogue: the research notes for the SuperNSH library set.
