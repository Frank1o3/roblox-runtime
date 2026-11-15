# Prompt: finish the Rust JNI compatibility migration

Work in `roblox-runtime` to remove the C++ JNI compatibility VM and move the
remaining Roblox Android Java hooks onto the existing Rust `jnivm` VM. Treat
`docs/porting-plan.md` as the detailed inventory, but treat this prompt and the
user's direction as authoritative where that plan is stale.

## Goal and boundaries

- Keep `crates/jnivm` as the Rust VM and extend it only where the hooks need
  missing JNI behavior. Do not port or keep the C++ `libjnivm` VM.
- Implement Roblox-specific class hooks in the existing in-repo JNI crate,
  currently `crates/jni-sys` / package `roblox-jni-sys`. Once it no longer
  exposes C bindings, rename it to `crates/jni` / `roblox-jni` and preserve the
  public `roblox_runtime::jni` facade.
- The end state is Rust source only: no C/C++ implementation, `native/`, CMake
  build, C++-linking `*-sys` crate, or `third_party/libjnivm` dependency.
- Use the C++ implementation to discover observable behavior and edge cases.
  Implement from the public Android/JNI contracts and measured runtime behavior;
  do not copy source text or leave a C++ fallback. Preserve required class names,
  method names, descriptors, return values, state changes, and error behavior.
- Keep existing Rust subsystem designs. In particular, host audio uses CPAL for
  its enabled host backends (including PipeWire, PulseAudio, and ALSA); dedicated
  OSS and Android AAudio/OpenSL paths remain separate Rust implementations.
  Do not restore a C++ audio backend or make CPAL exclusive of those paths.

## Current facts to verify before editing

- `roblox-runtime/src/lib.rs::create_java_vm` creates the Rust VM and then calls
  `roblox_jni_sys::jni::init_compat_bridge()`.
- `native/jni_shim.cpp::initialize_compat_vm` creates a second C++ VM and
  registers Android, GameActivity, and init-parameter classes. The runtime's
  JavaVM is Rust-owned; this bridge remains for hooks implemented against C++
  `jnivm::ENV`/objects and compatibility references.
- The `crates/jni-sys` build script links `roblox_jni_shim`, `jnivm`, logger,
  `roblox_liblog`, and the C++ runtime. Trace every such symbol and caller before
  removing a link or file.
- `crates/jni-sys/src/accessibility.rs` currently defines a C ABI mirror and
  calls `roblox_accessibility_*` functions. It is not yet the Rust
  `AccessibilityNodeInfo`/`AccessibilityManager` hook implementation. Port the
  hooks from `native/accessibility.cpp` into this JNI crate and replace the C ABI
  mirror calls with Rust-owned VM state/API where practical. Keep the Linux
  AT-SPI bridge in the existing Rust client-side accessibility code.
- `native/init_params.cpp` registers a large set of startup parameter, display,
  device, locale, collection, resource, and application classes. Port these
  registrations and methods into the JNI crate, including exact descriptors;
  don't mistake the Rust native-call drivers in `crates/jni-sys/src/game_activity`
  for implementations of these Java hooks.
- Other C++ hook sources include `android_classes.cpp`, `game_activity.cpp`,
  `platform_classes.cpp`, `unanswered_classes.cpp`, `accessibility.cpp`,
  `cookies.cpp`, `clipboard.cpp`, `permissions_transport.cpp`, `deeplink.cpp`,
  `local_storage.cpp`, `battery.cpp`, and `audio_classes.cpp`. Confirm the live
  set against CMake and call sites; port only behavior the APK/runtime needs,
  while retaining required compatibility responses.

## Implementation sequence

1. **Refresh the inventory.** Read all relevant Markdown documentation and
   crate manifests. Trace `init_compat_bridge`, every `roblox_jni_*` and
   `roblox_*_classes` export, all C++ class-registration functions, fallback
   reference APIs, and every `roblox-native-sys` dependency. Make a checklist
   mapping each source file/export/hook to its Rust replacement and caller.
2. **Close Rust VM gaps first.** For each needed hook, identify required JNI
   operations (class/object/string/array access, method dispatch, native
   registration, thread attachment, exceptions, direct buffers). Add safe Rust
   VM APIs where possible and a narrowly scoped, documented unsafe ABI only
   where JNI requires it. Remove C++ fallback reference calls as their Rust
   callers migrate; do not preserve a parallel VM to avoid implementing a gap.
3. **Port accessibility hooks.** Move the `AccessibilityNodeInfo`,
   `AccessibilityManager`, and event behavior to Rust registrations in the JNI
   crate. Preserve the existing bridge-connected gate, node state/actions,
   event queue, snapshot/generation behavior, and the client AT-SPI contract.
   Keep a clear distinction between Android JNI hook state and the AT-SPI
   transport. Remove the `roblox_accessibility_*` C++ exports after callers use
   Rust APIs.
4. **Port init parameters.** Register and implement the classes and methods in
   `native/init_params.cpp` using the Rust VM. Preserve descriptors and data
   flow for display metrics/configuration, device/platform/init/start parameters,
   locale/list/map/JSON helpers, native flags, resources, surfaces, and local
   storage. Share state with existing Rust Android/runtime APIs rather than
   maintaining duplicate C++ state.
5. **Port remaining required hooks by dependency.** Use
   `docs/porting-plan.md`'s inventory to migrate GameActivity and Android base
   classes, platform/unanswered classes, cookies, clipboard/message bus,
   permissions transport, deep links, local storage, battery, and audio Java
   classes. Preserve behavior relied on by the APK and client. Register hooks
   directly in the Rust VM and move runtime call sites to Rust APIs.
6. **Remove the compatibility bridge.** Once no hook or caller needs it, delete
   `init_compat_bridge`, `initialize_compat_vm`, C++ fallback machinery, C ABI
   wrappers, and the C++-specific JNI implementation. Keep the public Rust JNI
   facade stable; rename the in-repo `jni-sys` crate to `jni` as described above.
7. **Finish native removal.** Trace all remaining `native/` consumers, including
   audio, bionic compatibility, logging, and system paths. Complete their Rust
   replacements, then remove `native/`, `native-sys`, CMake/build scripts,
   C++-only dependencies, and `third_party/libjnivm`. Do not leave dead C++ files
   or unused CMake targets behind.
8. **Update documentation.** Reconcile `docs/porting-plan.md`, `README.md`,
   `docs/client-migration.md`, and crate READMEs with the implemented state.
   Remove outdated claims that the compatibility VM remains or that the plan is
   blocked on decisions already settled by the user.

## Correctness and completion checks

- Check hook descriptors against the APK dex where available. Record intentional
  descriptor differences or behavior changes; don't silently carry over dead
  hooks caused by mismatches.
- Preserve existing runtime behavior and public facade paths. Add focused Rust
  tests for ported stateful behavior and VM/JNI parity, and run the relevant
  workspace checks when implementing the migration.
- Search the finished tree for `jnivm.h`, `jnivm::`, `roblox_jni_`,
  `roblox_accessibility_`, `roblox-native-sys`, `CMakeLists.txt`, `.cpp`, `.cc`,
  `.cxx`, and `.h`. Resolve every remaining C++ build or source reference, not
  just references in the JNI crate.
- Confirm the runtime creates and uses one Rust VM, initializes all required
  hooks before APK `JNI_OnLoad`/startup calls them, and has no compatibility VM
  initialization or fallback reference path.
- Do not claim game-load, audio, or accessibility behavior is manually verified
  unless it was actually exercised. List any remaining verification limitation.
