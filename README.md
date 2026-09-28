# roblox-runtime

Standalone runtime for executing the Roblox Android client on Linux. This
repository owns Android compatibility and native loading; the `rusty-blox`
client supplies the base/split APKs, extracted libraries, data/cache
directories, settings, Fast Flags and host rendering surface.

The workspace is being extracted from the local `rbx-native-runtime` reference
implementation. See [the runtime audit](docs/runtime-audit.md) for source
boundaries and current migration status.

The Android compatibility crate currently serves assets from client-provided
base/split APKs and provides the first combined native-symbol override table.
Game startup, the host surface adapter and most Android APIs are still pending.

Build with a recent stable Rust toolchain, Clang/Clang++, CMake, and initialized
Git submodules:

```sh
git submodule update --init --recursive
cargo check --workspace
```
