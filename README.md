# nsh-bluetooth

BLE central and peripheral roles for NSH apps.

Part of the Native Shell (NSH) capability set. Same layout as the other SuperNSH repos: a Rust crate, docs in `docs/`, CI on GitHub Actions, and a web status page on GitHub Pages at https://supernsh.github.io/bluetooth/.

## Status

**Building out.** A Rust crate under `crates/nsh-bluetooth` and a TypeScript package under `packages/nsh-bluetooth` share the same `ERR_BLUETOOTH_*` error codes and the same behavior; the names match except one split [docs/parity.md](docs/parity.md) documents (Rust `read_characteristic` / `write_characteristic` / `subscribe_characteristic` vs TypeScript `device.read` / `device.write` / `device.subscribe`). The contract both halves implement is [docs/goals.md](docs/goals.md); the choices behind it are in [docs/decisions.md](docs/decisions.md).

CI (`.github/workflows/ci.yml`) runs on every push:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --all-targets` on ubuntu, macos and windows
- the TypeScript typecheck and the TypeScript tests in `packages/nsh-bluetooth`
- a cross-check matrix for the iOS, Android and wasm targets, so every bridge typechecks on every host

No CI runner has a Bluetooth radio. Every test runs through the fake backend, in both languages. Nothing has been run on a real device.

## Layout

- `crates/nsh-bluetooth/`: the Rust crate.
- `packages/nsh-bluetooth/`: the TypeScript package that mirrors it.
- `docs/goals.md`: the contract — goals, acceptance criteria, the error-code table, and the edge cases with their canonical test names.
- `docs/decisions.md`: each choice and why.
- `docs/parity.md`: API parity with flutter_blue_plus and react-native-ble-plx.
- `docs/brief.md`: options and research notes from the bootstrap.
- `site/`: static status page, published by `.github/workflows/pages.yml`.
- `.zcode/workflow.ts`: the phases this area will run through.

## Checks

Every compile and test runs in GitHub Actions. Nothing is built on a developer machine.

## Licence

MIT. See [LICENSE](LICENSE).

Repository: https://github.com/SuperNSH/bluetooth
