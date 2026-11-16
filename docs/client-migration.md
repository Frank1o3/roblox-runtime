# Client integration status

`rusty-blox` consumes the runtime through `roblox-runtime` and
`roblox-logging`. It does not depend directly on the runtime's internal linker
or JNI implementation crates.

## Current state

### Re-verified client build ownership (2026-11-15)

The parent repository's current `Cargo.toml` depends on the runtime by path and
declares no build script, CMake target, or C/C++ source outside the runtime
submodule. A scoped source/build-file search excluding `references/`, generated
output, and `roblox-runtime/` found no client-owned `.c/.cc/.cpp/.h/.hpp` files,
`CMakeLists.txt`, or `build.rs`. `.gitmodules` contains only the
`roblox-runtime` submodule. This establishes the client's direct project-owned
source/build status; it does not establish a C/C++-free transitive dependency
graph. Rust dependencies and host system libraries may use native build steps
and are tracked separately from code owned by this repository.

The client is not independently C++-free to build today because its path
dependency pulls in `roblox-abi` and `roblox-jni-bridge`, which depend on
`roblox-native-sys`; that crate configures CMake/Clang and builds the runtime's
native targets. No separate client CI/package workflow files were found in the
current checkout, so there are no such instructions to update yet.

- Engine ELF inspection and loading use the Rust implementation in
  `crates/linker`. The C++ bionic linker, its shim, patch, binding crate, and
  submodule have been removed.
- The primary JavaVM and `JNI_OnLoad` path use the pure-Rust `jnivm` crate, but
  the native compatibility layer still creates a companion VM for many Android
  hooks and GameActivity drivers. The runtime is not yet single-VM end to end.
- Native build support remains for those compatibility libraries, so Clang,
  CMake, and the libjnivm submodule are still needed by the current runtime
  build.
- OpenSL symbol registration and seven IID data exports are now Rust-owned in
  `roblox-abi`; `slCreateEngine`, object vtables, audio backends, and Java audio
  hooks remain native.
- The bionic `getaddrinfo` and `freeaddrinfo` compatibility exports are now
  Rust-owned and tested in `roblox-abi`; `crates/native-sys/native/netdb_compat.cpp` has been
  removed from the CMake target.
- The client settings UI no longer offers a C++ linker/JNI backend selection
  or a C++ fallback toggle. `RuntimeOptions.host_libc` remains an independent,
  ABI-unsafe diagnostic option.

## Client API surface

The client uses `RuntimeConfig`, `RuntimeOptions`, `LoadedEngine`, session,
graphics, permissions, WebView, deeplink, logging, and
`roblox_runtime::jni::game_activity` APIs. The facade paths are unchanged by
the linker removal. The engine import types are now re-exported from
`crates/linker::imports` as `roblox_runtime::EngineImports` and
`ImportBinding`.

The `roblox_runtime::jni` path remains stable and is backed by the
`roblox-jni-bridge` crate. JNI hook modules such as accessibility and
GameActivity live in that bridge; the pure Rust `jnivm` crate owns VM state and
thread attachment. C++ compatibility bindings remain temporarily while hooks
are ported.

## Follow-up

1. Port the remaining Android class and GameActivity hooks into Rust and
   register them with `jnivm`.
2. Remove the native JNI shim and libjnivm C++ dependency once those hooks no
   longer need the companion VM.
3. Remove the remaining native build support and update packaging requirements
   after the other native compatibility code has Rust replacements.
4. Complete the manual runtime checks listed in `porting-plan.md`.
