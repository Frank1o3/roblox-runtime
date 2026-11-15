# Client integration status

`rusty-blox` consumes the runtime through `roblox-runtime` and
`roblox-logging`. It does not depend directly on the runtime's internal linker
or JNI implementation crates.

## Current state

- Engine ELF inspection and loading use the Rust implementation in
  `crates/linker`. The C++ bionic linker, its shim, patch, binding crate, and
  submodule have been removed.
- The runtime creates its JavaVM with the pure-Rust `jnivm` crate. Android
  framework hooks and GameActivity drivers still use the native compatibility
  layer and remain to be ported.
- Native build support remains for those compatibility libraries, so Clang,
  CMake, and the libjnivm submodule are still needed by the current runtime
  build.
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

The `roblox_runtime::jni` path remains stable while its implementation moves
from C++ compatibility shims to Rust VM calls and registered Rust hooks. The
crate has not yet been renamed from `roblox-jni-sys`; that rename belongs with
removal of its remaining C++ bindings.

## Follow-up

1. Port the remaining Android class and GameActivity hooks into Rust and
   register them with `jnivm`.
2. Remove the native JNI shim and libjnivm C++ dependency once those hooks no
   longer need the companion VM.
3. Remove the remaining native build support and update packaging requirements
   after the other native compatibility code has Rust replacements.
4. Complete the manual runtime checks listed in `porting-plan.md`.
