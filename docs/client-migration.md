# Client migration handoff: C++ removal from roblox-runtime

For the maintainer of the embedding client (`rusty-blox`, this repo's parent).
Everything below is observable from the runtime repo; nothing about the client
was guessed beyond what its `src/` and `Cargo.toml` actually reference.

The client depends on exactly two paths: `roblox-runtime = {path="roblox-runtime"}`
and `roblox-logging = {path="roblox-runtime/crates/logging"}`. It never touches
`roblox-jni-sys`, `roblox-linker-sys`, `jnivm`, or `roblox-linker` directly —
all access goes through the `roblox_runtime::` facade. That is why this
migration is small.

## What changes, itemized

### 1. Build requirements (delete from your CI)

After the final phase the runtime needs **only a Rust toolchain**. These go
away and any CI/package manifest relying on them must drop them:

- `git submodule update --init --recursive` (`.gitmodules` is deleted;
  `third_party/` is deleted)
- `clang`/`clang++`, `cmake`, `patch`
- CMake/Clang version gates in the runtime's README

Until the final phase, the C++ path still builds if it was building before; the
removal is a single set of commits and your side needs no intermediate change.

### 2. Runtime options and selection switches

| item | today | after | migration |
|---|---|---|---|
| `USE_EXPERIMENTAL_JNIVM` env | selects the Rust JNI VM; default is the C++ VM | becomes the only behavior; the variable is accepted but ignored (shim), later removed | remove it from any launch scripts; do not build logic on it |
| `USE_EXPERIMENTAL_RUST_LINKER` env | selects `crates/linker`; default is the bionic C++ linker | becomes the only behavior; accepted-but-ignored shim, later removed | same |
| `RuntimeOptions.host_libc` | diagnostic glibc fallback | unchanged in meaning | none |
| `set_jnivm_cpp_fallback(bool)` (client calls it with `user_settings.jnivm_cpp_fallback`) | toggles the Rust-VM → C++-classes compat bridge | the bridge disappears; the function stays as a **deprecated no-op** (marked for removal) so your build keeps working | delete the `jnivm_cpp_fallback` setting from your settings UI and stop calling the function; the shim will be removed in a later release |
| `LoadError::ConstructorDeferralUnavailable` | possible when the C++ linker lacks the deferral patch | **removed** (constructor deferral is inherent in the Rust loader) | if you match on it, drop the arm |

### 3. Public API the client uses — status

All of these keep their exact paths and signatures unless noted:

- `roblox_runtime::{RuntimeConfig, RuntimeOptions, LoadedEngine, JniError,
  LoadError, ConfigError}` — unchanged, except:
  - `LoadedEngine::base/code_region/symbol/run_constructors/
    prepare_before_constructors/prepare_startup_directories/
    install_startup_bootstrap/initialize_jni/initialize_game_activity/
    start_game_activity/resize_surface` — unchanged;
  - `JniError` gains no new variants; `LoadError` loses
    `ConstructorDeferralUnavailable`.
- `roblox_runtime::jni` (re-export of the crate in `crates/jni-sys`) — **the
  package is renamed** `roblox-jni-sys` → `roblox-jni` during removal, but the
  `roblox_runtime::jni` re-export keeps the module path, so
  `roblox_runtime::jni::game_activity::*` (the ~30 functions and
  `RawTextBoxInfo`/`TouchContact` types you call from `src/client.rs`,
  `src/text_overlay.rs`, `src/gamepad.rs`, `src/startup.rs`) **does not
  change**. Error/return types of those functions are unchanged in shape; two
  functions currently return `Result<_, String>` from the C++ shim and keep
  doing so.
- `roblox_runtime::jni::accessibility::RawNode` — unchanged; note the
  accessibility surface is measured-dead on Android (the C++ header records
  it); the port preserves that.
- `pub use roblox_linker_sys::elf::{Binding as ImportBinding, Imports as
  EngineImports}` — the underlying `elf` module moves out of the dying
  `-sys` crate, but the re-export path through `roblox_runtime` stays:
  `roblox_runtime::EngineImports` / `ImportBinding` keep working.
- `roblox_runtime::session::{Session, …}`, `graphics::install_surface`,
  `permissions::arm`, `webview::{take_event, arm, WebViewEvent,
  WebViewRequest}`, `deeplink`, `supplied_path_exists`, `gc`/`gc_if_due`,
  `abi` re-export — unchanged.
- `roblox-logging` (`crates/logging`) — unchanged; it is already pure Rust.

### 4. Behavior changes you must expect (not API)

- **Engine startup order is preserved** (deferred constructors, settings before
  flags), but the executing code is the Rust loader/VM. Any workaround you
  built around a C++-path crash signature (e.g. ordering workarounds around
  `mcpelauncher` quirks) is now dead code — review comments in your startup
  path.
- **Audio**: default backend selection moves to cpal probes (construct + default
  device). `RBX_RUNTIME_AUDIO_HOST` values (`auto`, `pipewire`, `pulse`,
  `alsa`, `oss`, `none`) and `RBX_RUNTIME_AUDIO`, `RBX_RUNTIME_AUDIO_SINK`,
  `RBX_RUNTIME_AUDIO_DEVICE` keep their meanings. Known degradations (recorded
  in the new runtime ADR): no live default-sink following while playing (audio
  switches apply on next stream open), PipeWire `node.name` targeting and
  object.path-based device types are gone. If your UI surfaces a device list,
  the list now comes from cpal enumeration (`roblox_audio_sinks`'s C++ shape
  survives through the Rust port).
- **Log lines**: format and destinations are unchanged (`__android_log_*`
  surface kept, `RUSTY_BLOX_LOG_LEVEL` respected). The C++ JNI-trace
  diagnostics (`RBX_RUNTIME_JNI_TRACE`) lose their libjnivm-specific output;
  the Rust VM's unresolved-class report replaces the "Constructed Unresolved
  symbol" capture. If you grep logs for that string, update the grep to the
  Rust report's line.

### 5. Ordered migration checklist

1. Wait for the runtime's "removal complete" release; nothing is required
   before it (the whole transition keeps both paths building).
2. Drop submodule init and Clang/CMake/patch from CI and packaging manifests.
3. Remove the `jnivm_cpp_fallback` setting and the `set_jnivm_cpp_fallback`
   call.
4. Remove `USE_EXPERIMENTAL_JNIVM` / `USE_EXPERIMENTAL_RUST_LINKER` handling
   from launch scripts/CI matrices.
5. Drop `LoadError::ConstructorDeferralUnavailable` matching if present.
6. Re-check any log-greps for `Constructed Unresolved symbol`.
7. Run the manual verification checklist with the new runtime (Landing screen,
   experience entry, voice chat with mic indicator, leave-game, audio).
8. File issues against the runtime for anything that regressed; the runtime's
   `docs/porting-plan.md` lists the parity gates each area passed.

## Compatibility shims kept in the runtime, marked for removal

| shim | purpose | removal trigger |
|---|---|---|
| `set_jnivm_cpp_fallback` no-op | keeps client builds compiling | first release after you confirm the setting is gone |
| `USE_EXPERIMENTAL_*` env vars (accepted, ignored) | keeps launch scripts working | one release later |
| `roblox_runtime::jni` re-export to the renamed package | keeps your imports untouched | never (stable path) |
