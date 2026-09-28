# roblox-runtime

Standalone runtime for executing the Roblox Android client on Linux. This
repository owns Android compatibility and native loading; the `rusty-blox`
client supplies the base/split APKs, extracted libraries, data/cache
directories, settings, Fast Flags and host rendering surface.

The workspace is being extracted from the local `rbx-native-runtime` reference
implementation. See [the runtime audit](docs/runtime-audit.md) for source
boundaries and current migration status.

The runtime resolves engine imports against implemented ABI symbols, selected
host libraries and generated fallback stubs, then maps `libroblox.so` with its
constructors deferred. It does not yet run those constructors or call Roblox's
GameActivity bootstrap automatically. `LoadedEngine` exposes the ordered JNI
steps (`JNI_OnLoad`, then `initializeNativeCode`) once constructors have run.
The host surface adapter, constructor-time compatibility gaps and most Android
APIs are still pending.

Build with a recent stable Rust toolchain, Clang/Clang++, CMake, GNU `patch`,
and initialized Git submodules:

```sh
git submodule update --init --recursive
cargo check --workspace
```
