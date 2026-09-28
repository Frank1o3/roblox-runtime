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
The resolver uses host glibc for observed constructor-time libc calls whose
ABI matches bionic and keeps structure-sensitive APIs on runtime-owned
wrappers. With the explicitly ABI-unsafe diagnostic `host_libc` option, the
local APK probe returned from deferred constructors after pre-constructor JNI
setup; normal ABI mode still has an unresolved constructor failure. The runtime
has an initial EGL adapter for client-owned X11/Wayland surfaces; it does not
yet create the client window or renderer context. GLES3 is the only supported
backend; Vulkan requests fail explicitly until its Android loader and surface
adapter are ported. The runtime checks the installed surface and host EGL/GLES
libraries before engine constructors run.

Build with a recent stable Rust toolchain, Clang/Clang++, CMake, GNU `patch`,
and initialized Git submodules:

```sh
git submodule update --init --recursive
cargo check --workspace
```
