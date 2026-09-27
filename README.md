# roblox-runtime

Standalone runtime for executing the Roblox Android client on Linux. This
repository owns Android compatibility and native loading; a separate client
supplies the APK, extracted libraries, data/cache directories, settings and
Fast Flags.

The workspace is being extracted from the local `rbx-native-runtime` reference
implementation. See [the runtime audit](docs/runtime-audit.md) for source
boundaries and current migration status.

Build with a recent stable Rust toolchain, Clang/Clang++, CMake, and initialized
Git submodules:

```sh
git submodule update --init --recursive
cargo check --workspace
```
