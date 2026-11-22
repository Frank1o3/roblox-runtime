# Porting plan: removing every C++ dependency from roblox-runtime

Status: **Implementation in progress. The MCP launcher linker has been removed
and the Rust ELF mapper is the only engine loader. JNI hook migration remains.**

Goal: after this work the workspace is 100% Rust. No `crates/native-sys/native/`, no `third_party/`,
no CMake/C++ toolchain, no `*-sys` crate that links a C++ project. System libraries
loaded at run time (Vulkan, cpal's host audio stack) remain.

### User-directed scope adjustment

The Rust `crates/linker` and `crates/jnivm` implementations are the starting
points. Do not recreate the full libjnivm API or port every class in `crates/native-sys/native/`.
Implement only the JNI VM behavior and Android/Roblox hooks required by the
observed compatibility surface: dex descriptors, recorded runtime lookups,
current Rust call sites, and measured behavior. Record referenced-but-unreached
classes as intentionally unanswered instead of implementing them speculatively.

The crate boundary is explicit: `crates/jnivm` owns the VM model and VM
lifecycle; `crates/jni-bridge` (package `roblox-jni-bridge`) owns the regular
JNI-facing integration, Android/Roblox class hooks, and GameActivity callbacks.
The public `roblox_runtime::jni` facade remains stable. Platform services such
as audio, battery, storage, deep links, and system paths stay in their
appropriate subsystem crates. Finish the observed JNI VM and bridge hooks, then
remove the companion C++ VM and its fallback references before continuing with
the other native C++ subsystems.

---

## 0. What was read (Step 0 record)

Sections A-E record the original inventory and migration assumptions from
2026-11-14. The user-directed scope adjustment above supersedes their proposed
MCP linker/libjnivm porting work and the linker-related entries in those
sections.

- Repo layout: this workspace is a git submodule of the client repo `rusty-blox`
  (the parent directory). The client is visible at `..` and was read for §E.
- Documents the C++ comments cite that **do not exist in this repo**:
  `AGENTS.md`, `docs/adr/*`, `docs/NEXT.md` are absent. Only
  `docs/analysis/undefined-symbols.tsv` exists. The C++ comments reference
  `docs/adr/ADR-023…ADR-034` and several `docs/analysis/*.md` — those documents
  live in the reference projects (cordial), not here. **Consequence: the ADR the
  task asks for (audio behavior lost/weakened) will be the first ADR this repo
  has; I propose creating `docs/adr/` with it.**
- References: `references/Eclipse` (MIT — compatible with BSD-3-Clause, used as a
  structural guide) and `references/cordial` (GPL-3 — **see §License**).
  Also present in `references/` and not needed for this task: cork (AGPL-3),
  mocktail/nightcap (Apache-2.0), stacked (GPL-3), Stud (AGPL-3). Nothing will be
  copied from any of them.
- Every `Cargo.toml` and `build.rs` in the workspace, `crates/native-sys/native/CMakeLists.txt`,
  `.gitmodules`, `third_party/` contents, and the six relevant crates were read.

### Baseline (recorded 2026-11-14)

`cargo test --workspace` in `roblox-runtime/`: **build green (C++ native path
compiles via CMake/Clang), 81 unit tests pass, 0 failures**, including the
already-ported audio crate (`cargo test -p roblox-audio`: 19 passed). The audio
files listed as "already ported, uncompiled" in the task actually compile and
pass. See §Audio for what still has to happen there anyway.

### License and provenance finding — flagged for audit

The 2026-11-14 source comparison recorded `crates/native-sys/native/*.cpp` as a near-verbatim
derivative of cordial's GPL-3 `crates/native-sys/native/` tree. That inventory predates current
source removals and is historical evidence, not a current per-file audit.
Measured with `diff` (differing lines of both files combined):

| file | lines here | diff vs cordial |
| --- | --- | --- |
| opensles.cpp | 1655 | 45 |
| liblog.cpp | 231 | 36 |
| system_paths.cpp | 350 | 71 |
| aaudio.cpp | 1363 | 224 |
| shim.cpp | 156 | 100 |
| jni_shim.cpp | 697 | 539 |

The checked-in `LICENSE`, `Cargo.toml` files, and runtime README currently all
claim BSD-3-Clause. Reconcile provenance for each remaining native file and
ported replacement against that claim. Two further wrinkles recorded in the
prior comparison:

1. cordial's `NOTICE` also carries Apache-2.0-derived code from mocktail
   (per-file headers), which is one-way compatible into GPL but not back out.
2. `third_party/libjnivm` and `third_party/mcpelauncher-linker` are MIT — fine to
   port from with attribution — but libjnivm itself descends from fake-jni/baron,
   and the task's rule is explicit: *nothing derived from `third_party/` survives
   in Rust form without your explicit sign-off*.

The user has explicitly directed this Rust-only migration and authorized work
in phases. That resolves the earlier request for engineering-task sign-off; it
does not change the repository license or settle source provenance. Port
behavior from public specifications and observed contracts, keep provenance
notes, and do not copy implementation text from reference projects. The old
claim that README advertised GPL-3 is stale: its current License section says
BSD 3-Clause, consistent with `LICENSE` and the manifests.

MIT sources are not a problem: libjnivm and Eclipse are MIT. Their licenses
still require attribution when code is used; no reference-project source is to
be copied as part of this behavioral migration.

---

## A. Inventory of `crates/native-sys/native/` and `third_party/`, and target crates

### A.0 Re-verified build graph (2026-11-15)

The following is from the current manifests, `build.rs` files, and
`crates/native-sys/native/CMakeLists.txt`; it supersedes conflicting older rows in this document.
The runtime workspace still builds project-owned C++:

| target / edge | verified current effect | status |
| --- | --- | --- |
| `roblox-native-sys` (`crates/native-sys/build.rs`) | Runs CMake with Clang/Clang++ for `crates/native-sys/native/`; the crate is a transitive dependency of `roblox-abi` and `roblox-jni-bridge`. | live build infrastructure |
| `roblox_liblog` | Compiles `liblog.cpp`, `system_paths.cpp`, `opensles.cpp`, the PipeWire/Pulse/ALSA/OSS sources, and `thread_trace.cpp`. `roblox-abi/build.rs` links the archive; `crates/abi/src/bionic/mod.rs` calls `roblox_liblog_symbols` to obtain its symbols. `getaddrinfo/freeaddrinfo` are now registered directly from `crates/abi/src/bionic/netdb.rs`. | live; remaining per-symbol Rust ownership/parity incomplete |
| `roblox_jni_shim` | Compiles JNI shim, Android/GameActivity/init/service/audio/platform hook sources and links `jnivm`. `roblox-jni-bridge/build.rs` links it. | live; Rust VM is already used, but C++ hook callers and compatibility VM remain |
| CMake `jnivm` and `logger` | Built from `third_party/libjnivm` and `crates/native-sys/native/jnivm_logger/log.cpp`; `roblox_jni_shim` links `jnivm`, and the bridge links both archives. | live C++ VM/build dependency |
| `roblox_pipewire_backend_test` | Conditional on PipeWire headers; built as a dependency of `roblox_liblog` and run as a POST_BUILD command. It compiles PipeWire/Pulse/ALSA/OSS sources plus its test. | live build-time C++ test |
| `roblox_permissions_transport_test` | Built as a dependency of `roblox_jni_shim` and run POST_BUILD; compiles `permissions_transport.cpp` and its test with libjnivm headers. | live build-time C++ test |
| linker C++ shim | No linker shim target/source is in the current CMake file. Runtime uses `crates/linker`; old linker-shim rows elsewhere in this document are stale. | Rust linker is current; confirm no residual link caller |
| `third_party/libbadcpu` | No reference from current Cargo/build/CMake targets or runtime call sites found in scoped project paths; runtime maps host-architecture ELF directly. | unused and removed |
| `third_party/mocktail-webview` | No build target or runtime call site found. The live client webview is `src/webview.rs`; remaining native comment mentions describe provenance/context, not linkage. | unused and removed |

This is a build-ownership inventory, not a claim that every compiled hook is
reached by the currently exercised APK. Reachability and descriptor evidence
must be recorded per JNI hook before removal. System libraries and Rust
dependencies that may compile native code are outside this project-owned C++
inventory.

### A.1 What `crates/native-sys/native/` builds (CMakeLists.txt)

The current file defines four static libraries and two test executables; the
PipeWire test is conditional on headers:

| library | files | consumers |
| --- | --- | --- |
| `roblox_liblog` | liblog, system_paths, OpenSL and PipeWire/Pulse/ALSA/OSS backends, thread_trace | `crates/abi/build.rs` links it; its symbol table is read by `crates/abi/src/bionic/mod.rs` |
| `roblox_jni_shim` | jni_shim + Android, GameActivity, init params, service, audio, platform and unanswered-class hooks | `crates/jni-bridge/build.rs` links it, plus `jnivm` and `logger` |
| `logger` | `jnivm_logger/log.cpp` | passed into `libjnivm`'s `HAVE_LOGGER` seam |
| `jnivm` | `third_party/libjnivm` subdirectory target | linked by `roblox_jni_shim` and the Rust bridge |
| `roblox_pipewire_backend_test` | conditional PipeWire backend test executable | dependency of `roblox_liblog`; POST_BUILD runs it |
| `roblox_permissions_transport_test` | permissions response test executable | dependency of `roblox_jni_shim`; POST_BUILD runs it |

Current CMake POST_BUILD tests are `pipewire_backend_test.cpp` (conditional on
PipeWire headers) and `permissions_transport_test.cpp`. No other native test or
probe source named in the historical inventory is present in the current
`crates/native-sys/native/` tree. The Rust audio crate contains tests and probe examples; compare
their current scenarios rather than relying on historical port counts.
Patch: `patches/linker-constructor-deferral.patch` adds
`mcpelauncher_defer_next_ctors` / `mcpelauncher_run_deferred_ctors` /
`mcpelauncher_set_realpath` to the submodule's `linker.cpp`.

### A.2 Per-file inventory and target

Conventions: "ABI types/constants shared across the boundary" → `crates/abi`;
audio backend/stream logic → `crates/audio`; Android/Roblox JNI hooks →
`crates/jni-bridge`; platform service behavior → its existing subsystem crate; virtual-library
registration → `crates/linker`; log lines → `crates/logging`; bionic-libc
interception → `crates/abi/bionic/*`; Android filesystem/system → `crates/android`.

**Audio group → `crates/audio`** (Rust policy exists; native ABI ownership
still follows the checked-in CMake target, not the crate layout):

| C++ file | exports | target |
| --- | --- | --- |
| `pipewire_backend.cpp/.h` | C ABI sink helpers plus PipeWire playback/capture, queue, and enumeration implementation | Still compiled into `roblox_liblog`; `crates/audio` has Rust stream/backend modules, but no Rust export with the old C ABI names. Reconcile per-backend behavior and callers before removing this target. |
| `pulse_backend.cpp`, `alsa_backend.cpp` | PulseAudio and ALSA host stream implementations | Still compiled into `roblox_liblog`; Rust host backends exist. Keep host backend selection distinct and establish parity/call evidence before removal. |
| `oss_backend.cpp` | OSS host stream implementation | Still compiled into `roblox_liblog`; `crates/audio/src/backend/oss.rs` is the Rust owner candidate. |
| `aaudio.cpp/.h` | Android AAudio ABI entry points/backend selection | No such source is present in the current `crates/native-sys/native/` tree or CMake source list. Rust AAudio FFI is present in `crates/audio/src/aaudio/ffi.rs`; verify exported symbol resolution/callers before marking the migration complete. |
| `aaudio_input_callback.cpp/.h` | C++ AAudio input callback driver | No such source is present in the current `crates/native-sys/native/` tree or CMake source list. Rust callback code exists in `crates/audio/src/input_callback.rs`. |
| `opensles.cpp` | `slCreateEngine` and OpenSL vtable functions, reached through the runtime OpenSL symbol table | Still compiled into `roblox_liblog`; supplies the function and vtable behavior. Rust `crates/abi/src/bionic/mod.rs` now owns the eight-name registry and seven pointer-valued `SL_IID_*` definitions. Rust `crates/audio/src/opensles/` contains policy, player/recorder models, and tests. `slCreateEngine` and vtable behavior remain C++-owned. |
| `audio_classes.cpp` | Java audio class registration and device/capture behavior | Still compiled into `roblox_jni_shim`. Rust bridge now implements the measured FMOD `AudioDevice` init/write/close behavior using `roblox-audio`'s request-aware output and owned PCM queue. The C++ registrar/path remains live; broader AudioManager/AudioRecord/WebRTC classes remain C++-owned. Verify single-path runtime dispatch and audio smoke before removing duplicate C++ FMOD implementation. |
| `liblog.cpp` (231) | `__android_log_write/buf_write/print/vprint/assert`, `android_set_abort_message` via `roblox_liblog_symbols` | **TODO** → `crates/abi/bionic/liblog.rs` (variadic constraint: see §Variadic) |
| `system_paths.cpp` (350) | Native table now retains `stat`, `lstat`, `opendir`, `realpath`, `statvfs`, `open`, and `__open_2`, plus the optional trace setter. Rust owns and registers `access`, `readlink`, `fopen`, `mkdir`, `unlink`, and `rename` in `crates/abi/src/bionic/system_paths.rs`, using configured root state from `crates/android/src/system.rs`. C++ definitions/table entries for those six functions are removed. | **partially ported**; retained functions are structure-sensitive or variadic; parity tests cover Rust remapping and registration. |
| `netdb_compat.cpp` (206) | `roblox_getaddrinfo/freeaddrinfo` table | **ported and removed**; `crates/abi/src/bionic/netdb.rs` exports the bionic-layout resolver and symbol table. Focused tests cover numeric IPv4 layout/freeing, bionic error mapping, and registration. |
| `legacy_stdio.cpp` (103) | `roblox_legacy_{fflush,fclose,fseek,ftell,fputs,setvbuf,fread,fwrite,vfprintf,fprintf}` | **TODO** → `crates/abi/bionic/legacy_stdio.rs` (Rust already owns `__sF` and exports `roblox_legacy_sf_base/stride`) |
| `thread_trace.cpp` (173) | `roblox_pthread_create`, `roblox_set_thread_trace`, table (`pthread_create`) | **TODO** → `crates/abi/bionic/pthread.rs` (the aarch64 `roblox_pthread_attr_real` it resolves is already Rust) |
| `audio_probe.cpp`, `alsa/pulse_backend_probe.cpp` | none | Rust examples exist under `crates/audio/examples/`; current native tree has no probe source. Compare scenarios before declaring all cases ported. |
| `pipewire_backend_test.cpp` | PipeWire queue/backend behavior | Still a CMake POST_BUILD test target; compare its cases with Rust tests before removal. |
| `aaudio_input_callback_test.cpp`, `aaudio_input_test.cpp` | AAudio callback/input behavior | No matching native source is present in the current tree; Rust tests/examples are present. Historical test counts above are not a current source inventory. |

#### OpenSL ABI port boundary

The next safe OpenSL slice is not the three output-mix methods in isolation.
The C++ engine currently creates `OutputMixObject` instances, returns their
object-interface pointers, and resolves `SL_IID_OUTPUTMIX` in
`outputmixobj_GetInterface`. Porting those methods without object creation and
interface lookup would leave them unreachable. Implement the Rust vertical
slice in this dependency order:

1. Define `#[repr(C)]` vtables for the common `SLObjectItf`, engine interface,
   and output-mix interface, with slot order checked against the OpenSL header
   and current C++ definitions.
2. Add stable Rust-owned object wrappers with explicit lifetime/ownership,
   interface-pointer lookup, stale-handle rejection, and destruction. Do not
   expose `Arc` policy registry keys as ABI pointers.
3. Port `slCreateEngine`, engine `Realize/GetState/GetInterface/Destroy`,
   `CreateOutputMix`, and mix object `Realize/GetState/GetInterface/Destroy`
   together. Connect wrappers to `roblox-audio` policy handles.
4. Port the three output-mix vtable slots, then test calls through the actual
   function pointers, including null outputs, unsupported IIDs, repeated
   destruction, and invalid/stale interface pointers.
5. Keep the C++ definitions until Rust ABI tests and an audio probe demonstrate
   parity; only then remove that portion of the C++ vtables. Continue with
   player, queue, volume, recorder, and callback interfaces as separate
   vertical slices.

`references/Eclipse/src/loader/opensl.rs` is a useful MIT-licensed design
reference for C-layout vtables, boxed interface wrappers, pointer recovery, and
an end-to-end create/realize/interface/player probe. Its implementation is a
behavioral reference, not proof that this runtime's policy or audio paths are
equivalent. Eclipse itself is not C/C++-free: its build script compiles several
C shims and a C++ loader shim. Its OpenSL/AAudio adapters are Rust and use CPAL;
retain this runtime's dedicated OSS and distinct Android AAudio/OpenSL paths.

The Rust `org/fmod/AudioDevice` hooks now copy Java byte arrays, apply bounded
backpressure through `OwnedPcmQueue`, negotiate the requested rate/channel
format, and cancel writers during close. The native C++ registrar and
implementation remain compiled; do not remove them until VM dispatch and a
runtime audio smoke confirm the Rust path is the active implementation. Keep
the existing `CaptureStream` interlock for AudioRecord/WebRTC microphone paths
and verify start/stop/release transitions before removing those C++ hooks.

**JNI group → `crates/jni-bridge` and `crates/jnivm`:**

| C++ file | exports | target |
| --- | --- | --- |
| `jni_shim.cpp` | Rust now owns JavaVM creation, `JNIEnv` access, and `JNI_OnLoad`; removed the obsolete traced-VM wrappers and dormant method/field fallback exports | **Remaining:** compatibility VM class hooks, `RegisterNatives` mirroring for C++ GameActivity drivers, class-dump diagnostics, and target-ABI `va_list` decoding still use this shim/libjnivm |
| `android_classes.cpp` (2,037) | `roblox_register_android_classes` + 20 textbox/identity/app-ready exports; 10+ classes (StringBridge, DeviceStaticParams, NativeTextBoxInfo, NativeGL/Locale/UserJavaInterface, LoggingProtocol, SessionReporter, VideoCodecCapability, MediaCodecInfoUtils, BuildVersion…) | port reached Java hooks to `crates/jni-bridge`; state sinks remain in their subsystem APIs where applicable |
| `game_activity.cpp` (2,134) | 36 externs driving AGDK lifecycle/input/IME/gamepad; registers GameActivity/MotionEvent/KeyEvent/State/InputConnection/AssetManager/Configuration/ClassLoader; reads back RegisterNatives entries via `Class::natives` | keep Rust caller drivers in `crates/jni-bridge/game_activity`; move required class behavior and native lookup to the Rust VM/bridge based on observed calls |
| `init_params.cpp` (3,100) | 31 externs; ~19 classes (DisplayMetrics, Configuration, Point, DeviceUtils, Insets, Surface, Map/ArrayList/List, Locale, LocaleList, NativeFlagsInitResult, JSONObject, ClientLocalFlags, NativeHelper, LocalStorageManager, Resources, Activity, DeviceParams, PlatformParams, InitParams, StartAppParams) | port only reached hooks to `crates/jni-bridge`; verify descriptors against dex and runtime traces |
| `accessibility.cpp` (802) | 7 externs + AT-SPI bridge structs; measured dead on Android | **Ported to Rust** in `crates/jni-bridge/src/accessibility.rs`; its node/event state and JNI hooks are Rust-owned, and the C++ translation unit is removed. The client AT-SPI transport remains separate. |
| `cookies.cpp` (280) | 3 externs; JNICookieProtocol classes | port JNI registration to `crates/jni-bridge`; cookie handling remains in the owning session/service API and must never log cookie values |
| `deeplink.cpp` (269) | 5 externs; attacker-shape rules | port reached JNI hooks to `crates/jni-bridge`; URL validation stays in the existing Rust `src/deeplink.rs` |
| `clipboard.cpp` (641) | 5 messagebus externs (root `src/webview.rs` and `src/permissions.rs` call `roblox_messagebus_subscribe/set_request_handler_async`) | port JNI registrations to `crates/jni-bridge`; route payloads to existing Rust services |
| `permissions_transport.h/.cpp` | dual-response delivery logic used by the native clipboard hook; native transport contract tests | equivalent Rust ordering/error tests now run in `roblox-runtime/src/permissions.rs`; the C++ helper remains compiled and used by `clipboard.cpp` pending migration of that hook |
| `local_storage.cpp` (705) | `roblox_local_storage_set_platform_impl`, `roblox_update_screen_orientation`; consumes 4 Rust exports that already exist in `crates/android/src/local_storage.rs` | port JNI registrations to `crates/jni-bridge`; route payloads to existing Rust services (or `crates/android` — it is storage policy; Q1 covers) |
| `legacy_stdio.cpp`, `platform_classes.cpp` (205), `unanswered_classes.cpp` (343) | platform classes (ActivityThread/Application), unanswered classes (NetworkUtils with the engine's misspelled `getPublicIPv4Addresseses`, AppRtcDeviceWrapper, memstorage/Connection, BuildInfo) | port JNI registrations to `crates/jni-bridge`; route payloads to existing Rust services |
| `battery.cpp` (349) | BatteryStatus boxed fields + two native-call helpers | ported to `crates/jni-bridge/src/battery.rs`; reads are shared from `crates/android/src/battery.rs`; report natives remain unobserved and have no caller |
| `insets.h` (81) | shared Insets class so a hook can return it | becomes a plain Rust hook returning an object — the C++ descriptor-derivation reason for its existence disappears |
| `jnivm_logger/log.cpp` (112) | `Log::debug` seam capturing "Constructed Unresolved symbol" | replaced by `crates/logging` + a `jnivm` unresolved-report hook (jnivm already logs through roblox-logging; needs the same `crate::unimplemented` recording) |

**Linker group → `crates/linker`:**

| C++ file | exports | target |
| --- | --- | --- |
| historical `shim.cpp` | old `roblox_linker_*` C ABI over the launcher linker | unused and removed; no such file or CMake target exists in the current tree |
| historical `third_party/mcpelauncher-linker` | bionic linker and constructor-deferral patch | removed; current engine loader is `crates/linker` |
| `third_party/libjnivm` (632 KB, MIT, fake-jni ancestry) | the C++ JNI VM | replaced by `crates/jnivm` (§B) |
| `third_party/libbadcpu` | Scoped build and caller search found no use; host-architecture ELF mapping is Rust. | unused and removed |
| `third_party/mocktail-webview` | Scoped build and caller search found no use; client webview is `src/webview.rs`. | unused and removed |

---

## B. jnivm parity

The remaining C++ compatibility hooks use libjnivm's API; `crates/jnivm` owns
Roblox's JavaVM and implements only the observed JNI surface. Rust method/field
lookups no longer dispatch to C++ fallback hooks. Capability-by-capability:

| capability the C++ shims use | crates/jnivm status |
| --- | --- |
| class registration, hook registration with **explicit descriptors** | ✓ `register_class`, `register_method(class, name, descriptor, is_static)`, `register_method_handler`. Rust cannot derive descriptors from C++ types — every ported hook carries a descriptor string, verified against the dex by a new tool (§B.1). This converts the "silently dead hook" failure class into a build-time-checked one |
| `Class::natives` lookup of RegisterNatives-registered engine natives (game_activity.cpp reads back `initializeNativeCode` etc. this way; plain GetMethodID does *not* dispatch to RegisterNatives in libjnivm) | ✓ `register_native` / `find_native`; **verify**: that `Call*Method` on a RegisterNatives-bound name resolves through `find_native` with the same semantics (GetMethodID-not-dispatching is *parity*, not a bug to fix) |
| `<init>` static-factory rewrite | ✓ jnivm rewrites instance `<init>` lookups to a static factory signature (matches libjnivm's behavior; descriptor check must model it) |
| `shared_from_this` (hooks hand their own object to engine natives / detached threads) | ✓ equivalent: hooks receive `ObjectId` receiver and can hand objects back via `class_ref`/object values; no borrow-splitting hazard like the C++ one |
| direct ByteBuffers (`NewDirectByteBuffer`/`GetDirectBufferAddress`/capacity; AudioRecord.read feeds engine-recorded audio this way) | ✓ `ObjectValue::DirectByteBuffer` + ABI entries |
| process-wide attach for engine-created threads (`process_env` attaching on demand) | ~ `attach_current_thread` exists and `current_env()` is per-thread; the call and exception ABI entries auto-attach through `attached_thread_env` (mirroring the C++ shim's attach-on-demand), while array/string entries validate the passed env against the thread-local handle via `env_token` and do not attach. **gap**: unify the two entry styles or auto-attach at hook entry / `pthread_create` override (thread_trace port is the natural place, mirroring `process_env`) |
| strings | ✓ UTF-16 String objects, `Get/ReleaseStringChars`, `Get/ReleaseStringUTFChars` with buffer pools; `NewStringUTF(null)→""` parity |
| arrays | ✓ byte arrays complete (AudioRecord path); object arrays complete; all eight primitive element types (boolean, char, short, int, long, float, double) now have full JNI ABI entries — New/Get/Release/GetRegion/SetRegion — driven by a sealed `PrimitiveElement` trait and generic VM methods, covered by VM-level unit tests. Needed by init_params (`Map`/`List` paths, `getSupportedRefreshRates` int arrays) |
| Call*MethodV/A return variants | ✓ Object/Boolean/Int/Long/Void hand-written; Char/Short/Float/Double generated by a macro over the same invoke/invoke_a path (instance + static, V + A). init_params reads floats (`dpiScale`, `refreshRate`); battery reads Float. The variadic `...` slots stay unwired by design — hooks port onto the V/A entries |
| CharSequence | ✓ interface dispatch via descriptor compatibility at lookup time: `Ljava/lang/CharSequence;` and `Ljava/lang/String;` describe the same method to this VM (String is the only CharSequence implementation it hands out), in both directions, for returns and parameters. Hook ports register one handler instead of the C++ double-registration workaround; the first compat match logs once per process |
| exceptions | ✓ per-thread pending-throwable model: `Vm::throw`/`throw_new` (ThrowNew builds a `Throwable` object with optional detail message), `pending_exception`/`clear_pending_exception`, and truthful ABI `Throw`/`ThrowNew`/`ExceptionOccurred`/`ExceptionCheck`/`ExceptionClear`/`ExceptionDescribe` (describe prints class+message then clears, matching OpenJDK). Ported hooks still need to translate their failures into these calls |
| Call*MethodV/A return variants | ✓ Object/Boolean/Int/Long/Void plus Char/Short/Float/Double are implemented for instance and static V/A calls; init_params reads `dpiScale`/`refreshRate`, and battery reads Float |
| GetStatic*/SetStatic* fields, all types | ✓ fields.rs (all 8 types, instance+static) |
| `NewObjectV/A`, AllocObject | ~ `<init>` factory trick works; `AllocObject` falls to descriptor-correct zero values — acceptable unless the dex check shows use |
| JNI table completeness | ~ `env_table()` leaves unlisted entries null; enumerate engine's used entries from the dump-classes inventory and fill |
| GC (`gc()`, GcStats) | ✓ (Rust mark-and-sweep and string-buffer pruning) |
| unresolved reporting ("Constructed Unresolved symbol") | ~ jnivm logs unresolved via roblox-logging; **gap**: feed the same inventory the C++ `logger` fed `crate::unimplemented` (kind=JNI), so `unimplemented::report` stays populated |
| VM lifecycle | ~ `DestroyJavaVM` → JNI_ERR (parity with libjnivm's behavior here is fine); `JNI_VERSION_1_6` ✓ |



### Latest JNI migration progress (2026-11-15)

- Removed the disabled Rust-to-C++ method and field fallback dispatcher,
  companion object mirroring APIs, and their C++ exports. Unimplemented methods
  and fields now stay within the Rust VM and use its descriptor-correct default
  behavior.
- `roblox-jni-bridge::jni` now forwards JavaVM creation, `JNIEnv` access, and
  `JNI_OnLoad` directly to `jnivm`. Removed the obsolete C++ traced-VM wrappers.
- The C++ compatibility VM is **still required** by the native GameActivity and
  Android hook drivers. In particular, Rust `RegisterNatives` registrations are
  mirrored into that VM so its lifecycle/input callbacks can find engine native
  function pointers. This is a remaining migration task; `libjnivm`, CMake, and
  `roblox-native-sys` have not been removed.
- Manual release-build observation: the user reported a signed-in session and
  loaded experience; stopping the client ended in SIGSEGV. See `README.md` for
  the shutdown observation and its limits.

### JNI and Android follow-up (2026-11-21)

- The attached release log showed five Android NDK thermal lookups, the
  AAsset sequential-read lookup, and `AAudioStream_waitForStateChange` reaching
  the runtime's unimplemented `dlsym` path. Rust now supplies those symbols.
  Thermal queries return desktop-neutral status/headroom and no device
  thresholds; callbacks are not implemented because the observed path does not
  register listeners.
- Rust accessibility hooks already covered `Rect`, `AccessibilityNodeInfo`,
  `AccessibilityEvent`, `AccessibilityManager`, and `AccessibilityAction`.
  Their registration now declares hook classes directly, so the VM no longer
  logs them as fabricated unresolved classes before installing their methods.
- The client now sends GameActivity focus-lost, pause, surface-destroyed, stop,
  and terminate callbacks before releasing the host surface. The available
  coredump record has no core attached and only reports a zero instruction
  pointer; graceful shutdown remains unverified.
- The C++ companion VM and bridge remain live. The observed `NewGlobalRef`
  failure for an unknown reference is consistent with the still-unresolved
  cross-VM object boundary; these symbol and registration fixes do not remove
  that dependency.

### B.1 Descriptor verification

No `tools/` directory exists in this repo (cordial's `tools/hook_descriptors.py`
is GPL — not copied; a fresh checker will be written from the public dex
specification, in `tools/`):

- `tools/dex_methods.py` — parse `classes*.dex` from the APKs the client already
  supplies, dump class → (method, descriptor, ACC_STATIC).
- `tools/check_hook_descriptors.py` — compare the Rust hook registry (dumped via
  a `jnivm` debug API mirroring today's `--dump-classes`) against the dex:
  existence, descriptor equality, static-vs-instance match. Every ported hook
  must appear in the dex; every dex method Roblox reached (from the observed
  surface) must have a hook or a recorded "answered with nothing" decision
  (`unanswered_classes.cpp` semantics).
- The C++-dead-hook rule: where libjnivm's C++-derived descriptor mismatched the
  dex (known instances: `getAllocatableBytes` instance-vs-static,
  `getWindowInsets` Object-vs-Insets, `getProductName` String-vs-CharSequence),
  the Rust port binds what the **dex** says and records the C++ behavior change
  in the porting commit. Parity here means "the engine sees what the dex
  promises", not "the C++ bug is preserved" — flagged per hook for sign-off.

---

## C. Rust linker status

The current workspace contains `crates/linker` and no linker C++ shim target.
`roblox-runtime` uses the Rust mapper for engine loading and exposes import
inspection through `roblox_runtime::EngineImports` / `ImportBinding`. The
client consumes the runtime facade; it has no direct dependency on a linker
implementation crate. The 2026-11-15 manual release run loaded and ran an
experience, while graceful shutdown remains unverified and the process exited
with SIGSEGV. The old detailed parity checklist below this heading described a
removed MCP/bionic linker and is intentionally not retained as current work.

---

## D. Current native build dependency map

The current project-owned native build path is `roblox-abi` and
`roblox-jni-bridge` → `roblox-native-sys` → CMake targets documented in §A.0.
`roblox-abi/build.rs` links `roblox_liblog`; `roblox-jni-bridge/build.rs` links
`roblox_jni_shim`, `roblox_liblog`, `jnivm`, and `logger`. The native C++ VM is
also pulled from `third_party/libjnivm`. The `roblox_runtime::jni` facade
re-exports the in-repo package `roblox-jni-bridge`; it has not been renamed.
`jni-sys` is a separate Rust dependency used for JNI ABI types.

There is no current `roblox-linker-sys` crate, linker C++ shim, `cpp-compat`
Cargo feature, or `rust-compat` Cargo feature. Do not use those names as live
build switches or migration steps.

---

## E. Client impact

The client directly depends on `roblox-runtime` and `roblox-logging`, and uses
the stable `roblox_runtime::jni::game_activity` facade. The client has no
project-owned C/C++ source, CMake target, or build script outside the runtime
submodule, but its dependency graph currently builds the runtime's native
targets. See `docs/client-migration.md` for the verified client inventory.

---

## F. Phased migration status

No per-area `cpp-compat` / `rust-compat` Cargo features are present. The Rust
and C++ implementations do not share a general feature-based selector; migrate
only after the relevant Rust behavior and evidence are ready, then remove the
old path and its build edge.

- **P0 — audio migration (in progress; native implementation remains live).**
  1. Reconcile Rust `crates/audio` modules with the still-built native host
     backends and their actual consumers. Preserve CPAL host paths, the
     dedicated OSS implementation, and Android AAudio/OpenSL as distinct
     implementations; do not merge them to simplify the port.
  2. Wire OpenSL ABI data, functions, and vtables to the existing Rust OpenSL
     policy, then compare behavior against APK imports and the old source.
  3. Verify Rust AAudio exports against the actual symbol table and call sites;
     Rust source presence alone does not prove the native path is unused.
  4. Verify capture and microphone policy through the Rust capture path, then
     port descriptor-confirmed Java audio hooks from `audio_classes.cpp`.
  5. Port or explicitly disposition native audio tests and probes; retain Rust
     lifecycle, callback/error, symbol-table, backend-selection, and
     capture-policy coverage.
  Rust owns the OpenSL symbol registry and seven IID data exports, and has a
  Rust policy/test surface. C++ still owns `slCreateEngine`, the vtables, audio
  backends, and Java audio hooks.

- **P1 — JNI parity and hooks (pending).** Verify only operations and
  descriptors observed in the APK/runtime. Port Android/platform, GameActivity,
  and service hooks in dependency order; keep service state in its current Rust
  owner crates. Retain the public `roblox_runtime::jni` facade.

- **P2 — single Rust VM (pending).** Remove the companion VM, mirrored
  registrations, shim exports, and libjnivm dependency only after startup, APK
  `JNI_OnLoad`, hook registration, callbacks, and shutdown use one Rust VM.

- **P3 — ABI and platform ports (pending).** Move reached native symbols to
  `crates/abi`, `crates/android`, `crates/linker`, `crates/audio`, and
  `crates/logging`; verify names, signatures, constants, ownership, and errno
  behavior. Resolve live variadic imports under §Variadic.

- **P4 — native build removal and client gate (pending).** Remove CMake,
  `roblox-native-sys`, native sources, and obsolete submodules only after all
  live targets are ported or proven unused. Verify clean runtime and client
  builds and update their build requirements.

### Variadic ABI constraint

The details below are historical and have not been verified against the current
engine import table or target ABI. Do not treat them as an accepted design.
For each live variadic symbol, first record its exact import, signature, target
architectures, and calling convention; a Rust-only replacement must be tested
against that calling convention. No C/C++ shim may remain as an exception.

Rust (stable) cannot define variadic `extern "C"` functions. Today the C++
layer owns every variadic entry point the engine imports:
`open` (path redirect), `fprintf`/`vfprintf` (`__sF` remap), and
`__android_log_print`. `printf`/`asprintf`/`vasprintf`/`syslog` resolve to host
glibc or stubs today — add them to the host-compatible list rather than
intercepting (verify current resolution first; `printf` currently looks stubbed,
which is a "stub that lies" to fix while there).

Historical unvalidated proposal (retain only as research notes):

- `open`: a naked-assembly trampoline (stable `#[naked]`, or `global_asm!`) that
  redirects the path through a Rust helper and tail-calls host `open`
  preserving the optional mode argument. ~10 instructions per ISA (x86-64 +
  aarch64; this repo builds for both).
- `fprintf`: trampoline spills the SysV (or AArch64 PCS) register save area,
  builds a `va_list`, remaps the FILE* through the existing `__sF` logic, and
  tail-calls host `vfprintf`. glibc does the formatting; only the save-area
  layout is ours, and it is fully spec'd and testable.
- `__android_log_print`: trampoline builds a `va_list`, Rust helper formats via
  host `vsnprintf` and routes through `crates/logging`.
- `vfprintf`/`vsnprintf`/`vasprintf` (va_list-taking) need no asm — a small
  Rust `VaList` walker per ABI reads them.

Each trampoline gets a pinning test (golden: format a known set through the
trampoline and compare against the same call on host libc semantics).

---

## G. Parity gates (planned; tools not present yet)

1. **Symbol-set parity, per virtual library, compared by script**: inventory
   the runtime's actual registered exports and compare each against current
   engine imports. Some registries are already Rust (AAudio and the OpenSL
   names); others still come from C++ (`roblox_*_symbols`).
   `tools/check_virtual_libraries.py` still needs to be implemented.
2. **JNI hook descriptors vs dex** (`tools/check_hook_descriptors.py`, §B.1);
   the tools described in §B.1 are plans, not checked-in scripts yet.
3. All ported tests pass; each remaining C++ test is either retained while its
   implementation is live or has a Rust test covering the same contract.
4. Probes print their PASS lines against a real PipeWire session (examples, run
   manually).
5. Manual evidence and remaining checks: [README.md](../README.md) records a
   release run that signed in, loaded an experience, and ran it on 2026-11-15;
   it also records the unresolved debug-load difference and SIGSEGV on stop.
   Still unverified: live voice chat and microphone state transitions, graceful
   game/process shutdown, audio device changes while playing, clipboard copy
   out of Roblox, cookie persistence, and a Rust dump of the observed JNI
   surface. Do not report those as exercised until they are.

## H. Provenance notes (per ported area)

- **Rust host audio design** (`backend/cpal_output.rs`, `host.rs`, `oss.rs`):
  CPAL backs the Rust PipeWire/PulseAudio/ALSA host choices; OSS remains a
  separate direct implementation. These Rust paths are present, but the native
  host audio source and C++ Java audio classes still compile. Treat the ADR as
  Rust-path design, not evidence that C++ behavior has been replaced.
- **AAudio/OpenSL behavior**: the in-repo C++ is the source of truth for the
  FMOD contract (25 names, read-back getters, one-way `supportsAAudio()`); the
  measured contract is re-checked from `docs/analysis`-style comments in the
  C++ itself. Eclipse's `src/loader/aaudio.rs` (MIT) confirmed the shape
  (slab registry, error-streak→Disconnected mapping) as prior art, not copied.
- **libjnivm → crates/jnivm**: the Rust VM was written before this plan; §B
  gaps were found by comparing its public surface against every libjnivm API
  the C++ shims call (cordial's usage was the checklist; nothing copied —
  cordial is GPL and stays a behavioral reference only).
- **Linker**: `crates/linker` is the current Rust engine loader; no project C++
  linker target remains in the current build graph.
- **Descriptor tooling**: fresh implementation from the public dex spec;
  cordial's GPL `tools/hook_descriptors.py` is not read as a porting source.

## I. Remaining decisions

The intended hook boundary is `jnivm` for VM lifecycle,
`roblox-jni-bridge` for JNI integration, and subsystem crates for platform
behavior. The implementation still creates a companion C++ VM and many hooks
remain native, so that boundary is not yet realized end to end. No
`roblox-framework` crate is needed; its former path bridge has moved into
`roblox-android`.

The remaining linker and variadic-ABI implementation details are tracked in
§C and §Variadic. JNI descriptors must match the supplied APK dex where
available; behavior mismatches should be called out in hook migration notes.
