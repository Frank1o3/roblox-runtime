# Remaining work: remove project-owned C++ from Rusty Blox

Complete the migration across both repositories:

- **Runtime:** this repository (`roblox-runtime`), consumed by the client as a
  path/submodule dependency.
- **Client:** the parent `rusty-blox` repository.

The acceptance goal is that building either project does not compile or link
project-owned C or C++ source. Runtime functionality must be implemented in
Rust and preserve the public `roblox_runtime` APIs used by the client. System
libraries and Rust dependencies may still contain their own native code; record
those separately and do not describe them as project-owned C++.

Treat the checked-out source, build scripts, and manifests as authoritative.
`docs/porting-plan.md` and `docs/client-migration.md` are useful inventories,
but parts of them predate recent migrations and must be reconciled as work lands.
Do not assume an entry is live because a file exists: trace its build target,
symbol callers, runtime reachability, APK descriptors, and tests first.

## Current baseline to re-check

- The Rust VM lives in `crates/jnivm`; JNI and Roblox hooks live in
  `crates/jni-bridge` (the runtime facade is `roblox_runtime::jni`). The
  `roblox-framework` crate has been removed; shared Android system paths now
  live in `crates/android`.
- Accessibility hooks and battery JNI object/report helpers have Rust
  implementations. `native/accessibility.cpp` and `native/battery.cpp` have
  been removed. Battery readings are shared from `crates/android` and the
  reporting natives remain unobserved with no known caller; do not invent a
  caller or claim they were exercised.
- The Rust VM owns JavaVM creation, thread environments, and `JNI_OnLoad`, but
  the C++ compatibility shim and `third_party/libjnivm` remain. Other native
  Android/Roblox hooks and C ABI tables remain in `native/`.
- AAudio, its Rust callback path, and portions of audio policy are already in
  Rust. OpenSL symbols/backend and Java audio hooks still require inventory and
  migration. Preserve the existing CPAL host paths, OSS backend, and Android
  AAudio/OpenSL paths as distinct implementations.
- The runtime still has `crates/native-sys` with a CMake build, and the runtime
  tracks additional C++ under `third_party/` (`libbadcpu` and
  `mocktail-webview`); verify whether any remaining targets consume them before
  deleting or replacing them.
- The parent Rusty Blox client currently has no tracked C/C++ source or CMake
  target outside the runtime dependency. Verify its full Cargo/build and
  packaging graph before calling the client C++-free.

Update this baseline in the relevant docs whenever verified state changes.

## Required sequence

### 1. Build a verified ownership and dependency inventory

Inspect both repositories, including workspace manifests, `build.rs` files,
Cargo features, CMake targets, linker directives, submodules, packaging scripts,
and CI. Restrict source searches to project-owned paths; do not mistake
`references/`, generated output, or dependency source for live project code.

For every C/C++ file or generated/native build step, record:

- the build target and the exact symbols it contributes;
- all Rust and client callers, including dynamically resolved symbols;
- whether the code is reached by the tested APK/runtime;
- its Rust owner/replacement and the behavior/parity evidence required;
- whether it is unused, generated, vendored, or a system/dependency build.

Trace every `roblox-native-sys` dependency, CMake archive, C ABI export, JNI
registration function, JavaVM compatibility/fallback entry point, and native
symbol table before removing any of them. Keep the checklist in
`docs/porting-plan.md`; fix stale completion claims and mark each item
`verified`, `ported`, `unused and removed`, or `blocked` with evidence.

### 2. Finish the audio subsystem migration

Use `crates/audio` as the owner. Port and wire the remaining OpenSL ES ABI/data
symbols and Java audio hooks, then remove their C++ implementations. Verify
capture and microphone policy go through the existing Rust capture path. Port
the still-relevant native audio tests/probes to Rust or document why a case is
not applicable. Retain CPAL for enabled host backends, the dedicated OSS
backend, and Android AAudio/OpenSL implementations; do not collapse these paths
into one backend.

Acceptance: Rust tests cover stream lifecycle, callback/error behavior, symbol
tables, backend selection, and capture policy; runtime smoke checks cover audio
where the environment permits. Only then remove the corresponding CMake sources
and unused build dependencies.

### 3. Complete JNI VM parity for observed hooks

Compare `crates/jnivm` against the exact JNI operations required by the hooks
that remain live. Fill only those gaps: class/object/member lookup, descriptors,
strings, arrays, native registration/calls, local/global references, thread
attachment, exceptions, and direct buffers as evidence requires. Prefer safe VM
APIs; keep any ABI `unsafe` narrow, documented, and covered by tests.

Use APK dex metadata and runtime traces to verify class/method/field names and
descriptors. Preserve intentional compatibility behavior and record differences
from the former C++ implementation. Do not implement an entire JVM or JNI
surface without a demonstrated consumer.

### 4. Port and wire the remaining Java hooks

Register hooks directly against the single Rust VM and move Rust call sites off
the C++ bridge. Work in dependency order and only retain reachable behavior:

1. Android base/platform classes and initialization parameters from
   `android_classes.cpp`, `platform_classes.cpp`, `init_params.cpp`,
   `unanswered_classes.cpp`, and `insets.h`.
2. GameActivity Java classes, native registration lookup, lifecycle, input,
   display, IME, and surface behavior from `game_activity.cpp` and the remaining
   bridge drivers.
3. Deep links and service hooks: cookies, clipboard/message bus, permissions
   transport, and local storage. Keep service policy/state in the existing Rust
   owner crates and preserve the client-facing APIs.
4. Audio Java classes and any remaining battery or accessibility integration
   proven to be live.

For every port, add focused VM/bridge tests and descriptor checks. Preserve
unknown/unanswered methods where compatibility requires them, but do not add
speculative hooks for unreachable methods. Never log cookies or other secrets.

### 5. Remove the companion C++ JNI VM and bridge

After all live hooks use `jnivm`, verify the runtime creates and uses exactly one
VM from startup through APK `JNI_OnLoad`, class registration, callbacks, and
shutdown. Remove the second-VM initializer, Rust-to-C++ fallback/reference
mirrors, JNI shim exports, `libjnivm` link, and its submodule. Keep the public
`roblox_runtime::jni` facade stable; rename `crates/jni-bridge` to `crates/jni`
and package `roblox-jni` only when its C/C++ build script and links are gone.

Acceptance: no live reference to `init_compat_bridge`, `initialize_compat_vm`,
`roblox_jni_*` C++ bridge exports, C++ JNI environment/object types, or
`third_party/libjnivm`; integration tests demonstrate hook registration before
APK startup.

### 6. Port remaining native ABI and platform support

Move each still-live native implementation to its owning Rust subsystem: system
path/symbol resolution in `crates/android` or `crates/linker`; netdb, stdio,
thread tracing, and liblog compatibility in `crates/abi`; logger routing in
`crates/logging`; and any remaining linker/audio functionality in their Rust
crates. Compare exported symbol names, signatures, constants, errno behavior,
ownership, and edge cases against actual engine imports and the old behavior.

Rust stable cannot define C-variadic functions. Resolve any live variadic ABI
imports with a Rust-only, target-ABI-correct design (for example a narrowly
scoped assembly trampoline where appropriate, or a verified symbol-resolution
strategy). Document target architectures and test the real calling convention;
do not leave a C/C++ shim as a quiet exception.

Audit `third_party/libbadcpu` and `third_party/mocktail-webview` against build
files and runtime callers. Port any live behavior to Rust or remove the code if
it is genuinely unused. Do not delete merely because a previous note called it
unused; prove that no build or runtime path consumes it.

### 7. Remove native build infrastructure from the runtime

Only after every live target above has a Rust replacement or has been proven
unused, remove `native/`, runtime `third_party/` native dependencies,
`crates/native-sys`, CMake build scripts and dependencies, native link directives,
and obsolete submodule entries. Remove the `cmake` workspace dependency and any
unused `cc`/C++ toolchain requirements. Update README installation/build
requirements and CI so a clean runtime build needs no project C/C++ compiler or
CMake.

### 8. Audit and finish the Rusty Blox client

Inspect the parent repo independently after the runtime no longer exports or
requires native builds. Confirm its Cargo dependencies, build scripts,
packaging, CI, and runtime integration do not compile project-owned C/C++ code
or require CMake due to the runtime. Keep Rusty Blox on the stable
`roblox-runtime` public APIs. Update `docs/client-migration.md`, root README,
and client CI/package instructions. Distinguish a Rust crate's upstream native
build or host system dependency from C++ source owned by either repository.

### 9. Final clean-tree acceptance gate

Run from clean checkouts of both repositories. The task is complete only when:

- runtime and client build/test commands pass without CMake or project-owned
  C/C++ compilation;
- all native compatibility behavior still used by the APK is owned by Rust and
  has focused tests or recorded manual evidence;
- the runtime uses one Rust VM and preserves the public client facade;
- no runtime-owned C/C++ source, CMake target, native build crate, obsolete
  submodule, or dead native file remains;
- the client build graph contains no project-owned C/C++ target;
- scoped searches for C/C++ extensions, JNI C++ headers/types, native build
  tools, and old bridge symbols have no unexplained matches;
- `README.md`, `docs/porting-plan.md`, `docs/client-migration.md`, crate READMEs,
  and CI instructions agree with the checked-in implementation.

Do not claim game-load, audio, accessibility, or shutdown behavior is manually
verified unless it was actually exercised. Record environment limits and known
runtime issues (including non-graceful shutdown if still present) in the final
verification notes.
