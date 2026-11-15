# Porting plan: removing every C++ dependency from roblox-runtime

Status: **Step 1 — inventory and gap analysis. Written for review; no porting has
started beyond the audio crate work that predates this document.**

Goal: after this work the workspace is 100% Rust. No `native/`, no `third_party/`,
no CMake/C++ toolchain, no `*-sys` crate that links a C++ project. System libraries
loaded at run time (Vulkan, cpal's host audio stack) remain.

---

## 0. What was read (Step 0 record)

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
- Every `Cargo.toml` and `build.rs` in the workspace, `native/CMakeLists.txt`,
  `.gitmodules`, `third_party/` contents, and the six relevant crates were read.

### Baseline (recorded 2026-11-14)

`cargo test --workspace` in `roblox-runtime/`: **build green (C++ native path
compiles via CMake/Clang), 81 unit tests pass, 0 failures**, including the
already-ported audio crate (`cargo test -p roblox-audio`: 19 passed). The audio
files listed as "already ported, uncompiled" in the task actually compile and
pass. See §Audio for what still has to happen there anyway.

### License finding — flagged, not resolved

`native/*.cpp` is a **near-verbatim derivative of cordial's GPL-3 `native/`
tree**. Measured with `diff` (differing lines of both files combined):

| file | lines here | diff vs cordial |
|---|---|---|
| opensles.cpp | 1655 | 45 |
| liblog.cpp | 231 | 36 |
| system_paths.cpp | 350 | 71 |
| aaudio.cpp | 1363 | 224 |
| shim.cpp | 156 | 100 |
| jni_shim.cpp | 697 | 539 |

This repo claims BSD-3-Clause (`LICENSE`, all `Cargo.toml`), while the source it
derives from is GPL-3 ("Copyright 2026 the Cordial contributors"). Two further
wrinkles:

1. cordial's `NOTICE` also carries Apache-2.0-derived code from mocktail
   (per-file headers), which is one-way compatible into GPL but not back out.
2. `third_party/libjnivm` and `third_party/mcpelauncher-linker` are MIT — fine to
   port from with attribution — but libjnivm itself descends from fake-jni/baron,
   and the task's rule is explicit: *nothing derived from `third_party/` survives
   in Rust form without your explicit sign-off*.

**Question for sign-off before any porting commit:** a Rust port of
`native/*.cpp` is a derivative work of GPL-3 code. Either (a) the Rust ports are
licensed GPL-3-or-later (incompatible with the repo's BSD-3 claim), (b) you
confirm you are the sole copyright holder of the relevant cordial code and
authorize relicensing your own contribution, or (c) the Rust ports are written
from the AOSP/public spec + measured behavior only, using the C++ strictly as a
behavioral oracle like cordial was used — slower, and honestly hard to argue for
files that are 97% identical today. Also note the README's License section says
"GPL-3.0-or-later" while `LICENSE` and every `Cargo.toml` say BSD-3-Clause; that
contradiction needs resolving regardless.

MIT sources are not a problem: libjnivm, mcpelauncher-linker, and Eclipse are
MIT. Porting from them with a provenance note is permissible under BSD-3 with
attribution. Only the cordial-derived `native/` body is the issue.

---

## A. Inventory of `native/` and `third_party/`, and target crates

### A.1 What `native/` builds (CMakeLists.txt)

Four static libraries, plus four `EXCLUDE_FROM_ALL` test binaries run as
POST_BUILD steps:

| library | files | consumers |
|---|---|---|
| `roblox_liblog` | liblog, system_paths, netdb_compat, opensles, pipewire/pulse/alsa/oss backends, aaudio, aaudio_input_callback, thread_trace | `crates/abi/build.rs` links it; its symbol tables are read by `crates/abi/src/bionic/mod.rs` |
| `roblox_linker_shim` | shim.cpp | `crates/linker-sys/build.rs`, `crates/android/build.rs` |
| `roblox_jni_shim` | jni_shim + all class-hook files | `crates/jni-sys/build.rs`, `crates/android/build.rs` |
| `logger` | jnivm_logger/log.cpp | linked into libjnivm via its `HAVE_LOGGER` target seam |

Tests: `pipewire_backend_test.cpp` (15), `aaudio_input_callback_test.cpp` (5),
`aaudio_input_test.cpp` (14), `permissions_transport_test.cpp` (4).
Probes (standalone mains, never tests): `audio_probe.cpp` (1,125),
`alsa_backend_probe.cpp`, `pulse_backend_probe.cpp`.
Patch: `patches/linker-constructor-deferral.patch` adds
`mcpelauncher_defer_next_ctors` / `mcpelauncher_run_deferred_ctors` /
`mcpelauncher_set_realpath` to the submodule's `linker.cpp`.

### A.2 Per-file inventory and target

Conventions: "ABI types/constants shared across the boundary" → `crates/abi`;
audio backend/stream logic → `crates/audio`; Java class hooks → **proposed new
dedicated crate `crates/framework` (see question Q1)**; virtual-library
registration → `crates/linker`; log lines → `crates/logging`; bionic-libc
interception → `crates/abi/bionic/*`; Android filesystem/system → `crates/android`.

**Audio group → `crates/audio`** (partly done; see §Audio status):

| C++ file | exports | target |
|---|---|---|
| `pipewire_backend.cpp/.h` (2,490) | `roblox_audio_sinks`, `roblox_audio_sinks_free`; C++ `OutputStream`/`PlaybackStream`/`CallbackStream`/`CaptureStream`, backend selector, `voice_muted()`, `testing::fill_pcm` | `backend/pcm.rs` (PlaybackStream queue logic, 10 tests ported ✓), `backend/cpal_output.rs` (CallbackStream → cpal, done), `backend/host.rs` (selector, done), capture stream **TODO**, enumeration done via cpal |
| `pulse_backend.cpp`, `alsa_backend.cpp` (807) | C++ `pulse_available/make_pulse_stream`, `alsa_available/make_alsa_stream` | merged into `cpal_output.rs` by design decision; losses recorded in new ADR |
| `oss_backend.cpp` (269) | C++ `oss_available/make_oss_stream` | `backend/oss.rs` ✓ (tests: conversion clamp test ported) |
| `aaudio.cpp/.h` (1,398) | 25 AAudio symbols via `roblox_aaudio_symbols` table + `roblox_audio_backend_is_aaudio/_available/_announce` | `aaudio/` module: `types.rs` ✓, `builder.rs` ✓; stream surface **TODO** (§Audio) |
| `aaudio_input_callback.cpp/.h` (173) | C++ `InputCallbackDriver` | `input_callback.rs` ✓ (5 tests ported ✓) |
| `opensles.cpp` (1,655) | `slCreateEngine` + 7 `SL_IID_*` **data** symbols via `roblox_opensles_symbols` | **TODO** → `crates/audio/opensles/` |
| `audio_classes.cpp` (2,195) | Java hooks: AudioManager/AudioDeviceInfo, AudioRecord, WebRtcAudioManager/Record/Track, org/fmod/AudioDevice; the microphone-rule chokepoint | **TODO** → `crates/audio` JNI hooks module (they *are* the audio policy; registered via `jnivm::register_method_handler`) — or `crates/framework` if Q1 answers "one hooks crate for everything" |
| `liblog.cpp` (231) | `__android_log_write/buf_write/print/vprint/assert`, `android_set_abort_message` via `roblox_liblog_symbols` | **TODO** → `crates/abi/bionic/liblog.rs` (variadic constraint: see §Variadic) |
| `system_paths.cpp` (350) | 13-symbol table (`stat lstat access opendir realpath readlink fopen statvfs open __open_2 mkdir unlink rename`), `roblox_set_system_root`, `roblox_set_path_trace` | **TODO** → `crates/android/src/system.rs` (it already owns `roblox_set_system_root`) |
| `netdb_compat.cpp` (206) | `roblox_getaddrinfo/freeaddrinfo` table | **TODO** → `crates/abi/bionic/netdb.rs` |
| `legacy_stdio.cpp` (103) | `roblox_legacy_{fflush,fclose,fseek,ftell,fputs,setvbuf,fread,fwrite,vfprintf,fprintf}` | **TODO** → `crates/abi/bionic/legacy_stdio.rs` (Rust already owns `__sF` and exports `roblox_legacy_sf_base/stride`) |
| `thread_trace.cpp` (173) | `roblox_pthread_create`, `roblox_set_thread_trace`, table (`pthread_create`) | **TODO** → `crates/abi/bionic/pthread.rs` (the aarch64 `roblox_pthread_attr_real` it resolves is already Rust) |
| `audio_probe.cpp`, `alsa/pulse_backend_probe.cpp` (1,287) | none | → `crates/audio/examples/` (probes never run under `cargo test`) |
| `*_test.cpp` (4 files, 38 tests) | — | → cargo tests of the same names; 15 ported ✓ (pcm 10 + input_callback 5), **23 remaining** |

**JNI group → `crates/framework` (proposed, Q1):**

| C++ file | exports | target |
|---|---|---|
| `jni_shim.cpp` (697) | 18 `roblox_jni_*` externs; owns process VM, `reserved0`, exception barriers, compat-bridge side | replaced by `crates/jni-sys`'s `jni.rs` calling `jnivm` directly; `init_compat_bridge` and the whole C++ fallback disappear |
| `android_classes.cpp` (2,037) | `roblox_register_android_classes` + 20 textbox/identity/app-ready exports; 10+ classes (StringBridge, DeviceStaticParams, NativeTextBoxInfo, NativeGL/Locale/UserJavaInterface, LoggingProtocol, SessionReporter, VideoCodecCapability, MediaCodecInfoUtils, BuildVersion…) | port → `crates/framework`; the textbox/identity *state* sinks stay where their Rust consumers already live (`crates/jni-sys` drivers + `jnivm::native_helper`) |
| `game_activity.cpp` (2,134) | 36 externs driving AGDK lifecycle/input/IME/gamepad; registers GameActivity/MotionEvent/KeyEvent/State/InputConnection/AssetManager/Configuration/ClassLoader; reads back RegisterNatives entries via `Class::natives` | drivers already in Rust (`crates/jni-sys/game_activity/*`) — only the C++ bodies they call move to Rust (`crates/framework`); `crates/jnivm` needs `find_native`-based registered-native lookup (exists) |
| `init_params.cpp` (3,100) | 31 externs; ~19 classes (DisplayMetrics, Configuration, Point, DeviceUtils, Insets, Surface, Map/ArrayList/List, Locale, LocaleList, NativeFlagsInitResult, JSONObject, ClientLocalFlags, NativeHelper, LocalStorageManager, Resources, Activity, DeviceParams, PlatformParams, InitParams, StartAppParams) | port → `crates/framework`; descriptor-checked per §B |
| `accessibility.cpp` (802) | 7 externs + AT-SPI bridge structs; measured dead on Android (header records it) | port → `crates/framework` (hooks stay; Rust AT-SPI bridge already client-side) |
| `cookies.cpp` (280) | 3 externs; JNICookieProtocol classes | port → `crates/framework`; privacy rule (never log a cookie) preserved |
| `deeplink.cpp` (269) | 5 externs; attacker-shape rules | port → `crates/framework` (deeplink.rs in root src already exists client-side) |
| `clipboard.cpp` (641) | 5 messagebus externs (root `src/webview.rs` and `src/permissions.rs` call `roblox_messagebus_subscribe/set_request_handler_async`) | port → `crates/framework` |
| `permissions_transport.h/.cpp` (54) | dual-response delivery logic + 4 tests | port → `crates/framework` with its tests |
| `local_storage.cpp` (705) | `roblox_local_storage_set_platform_impl`, `roblox_update_screen_orientation`; consumes 4 Rust exports that already exist in `crates/android/src/local_storage.rs` | port → `crates/framework` (or `crates/android` — it is storage policy; Q1 covers) |
| `legacy_stdio.cpp`, `platform_classes.cpp` (205), `battery.cpp` (349), `unanswered_classes.cpp` (343) | platform classes (ActivityThread/Application), BatteryStatus mirror + 2 externs, unanswered classes (NetworkUtils with the engine's misspelled `getPublicIPv4Addresseses`, AppRtcDeviceWrapper, memstorage/Connection, BuildInfo) | port → `crates/framework`; battery *data* already comes from `crates/android/src/battery.rs` |
| `insets.h` (81) | shared Insets class so a hook can return it | becomes a plain Rust hook returning an object — the C++ descriptor-derivation reason for its existence disappears |
| `jnivm_logger/log.cpp` (112) | `Log::debug` seam capturing "Constructed Unresolved symbol" | replaced by `crates/logging` + a `jnivm` unresolved-report hook (jnivm already logs through roblox-logging; needs the same `crate::unimplemented` recording) |

**Linker group → `crates/linker`:**

| C++ file | exports | target |
|---|---|---|
| `shim.cpp` (156) | 12 `roblox_linker_*` externs over the C++ linker API | replaced by `crates/linker-sys`'s Rust API → then by `crates/linker` (§C) |
| `third_party/mcpelauncher-linker` (257 MB, MIT) | the bionic linker: virtual libraries, dlopen/dlsym, TLS, constructors, deferral patch | replaced by `crates/linker` (already: elf/map/reloc/resolve/dl/tls/stub) |
| `third_party/libjnivm` (632 KB, MIT, fake-jni ancestry) | the C++ JNI VM | replaced by `crates/jnivm` (§B) |
| `third_party/libbadcpu` (68 KB, checked-in, **not referenced by any build file**) | CPU translator, unused | delete |
| `third_party/mocktail-webview` (180 KB, checked-in, **not referenced**) | webview, unused (root `src/webview.rs` is the live one) | delete |

---

## B. jnivm parity

The C++ shims use libjnivm's API; `crates/jnivm` must cover every capability
before the hooks can port. Capability-by-capability:

| capability the C++ shims use | crates/jnivm status |
|---|---|
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
| Call*MethodV/A return variants | ~ Object/Boolean/Int/Long/Void implemented; **gap**: Char/Short/Float/Double (init_params reads floats: `dpiScale`, `refreshRate`; battery reads Float) |
| GetStatic*/SetStatic* fields, all types | ✓ fields.rs (all 8 types, instance+static) |
| `NewObjectV/A`, AllocObject | ~ `<init>` factory trick works; `AllocObject` falls to descriptor-correct zero values — acceptable unless the dex check shows use |
| JNI table completeness | ~ `env_table()` leaves unlisted entries null; enumerate engine's used entries from the dump-classes inventory and fill |
| GC (`gc()`, GcStats) | ✓ (mark-and-sweep, C++ ref release, string-buffer pruning) |
| unresolved reporting ("Constructed Unresolved symbol") | ~ jnivm logs unresolved via roblox-logging; **gap**: feed the same inventory the C++ `logger` fed `crate::unimplemented` (kind=JNI), so `unimplemented::report` stays populated |
| VM lifecycle | ~ `DestroyJavaVM` → JNI_ERR (parity with libjnivm's behavior here is fine); `JNI_VERSION_1_6` ✓ |

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

## C. linker parity

What the APK needed from the launcher linker, and `crates/linker` status:

| capability | status in crates/linker |
|---|---|
| virtual libraries registered by name→symbol table (`roblox_linker_sys::register` ≡ `register_library`) | ✓ `dl::register_library` (+ `register_global_symbols`) |
| engine `DT_NEEDED` resolution against those tables, nothing on disk | ✓ `MappedLibrary::load` + search dir; **verify**: missing-dependency tolerance matches the C++ path's failure mode (bionic aborts the load; crates/linker tolerates if imports resolve — same net effect, keep it) |
| `dlopen`/`dlsym`/`dlclose`/`dlerror`/`dladdr`/`dlvsym`/`dl_iterate_phdr` for the guest | ✓ `dl::rust_dlopen`… exist; **verify** they are installed as the guest's `libdl.so` table (the bionic path got these from `linker::init()` loading `libdl.so`; the Rust path must register `dl::linker_symbols()` the same way) — FMOD's `dlopen("libaaudio.so")` must land in the registered table, not the host loader |
| symbol overrides (per-library tables; overrides beat stubs beat host) | ✓ resolution precedence in `symbols.rs`; cross-check per-library |
| constructor running + deferral (`defer_next_ctors`, `run_deferred_ctors`, idempotency) | ✓ inherent: `run_constructors` is caller-driven and collected constructors run once. The C++ patch also added `set_realpath` (Dl_info override for constructor-time code) — **gap**: no equivalent in crates/linker; add a `set_realpath`-style override on MappedObject |
| TLS (bionic layout, TLSDESC, `dlpi_tls_data`) | ✓ `tls.rs` static TLS block + `relocate_tls`; **verify** against a real engine load (the engine's emutls traffic is heavy); x86_64-only today — matches Eclipse's known gap, record in ADR |
| `PATH_MAX=256` upstream bug (the Flatpak overflow) | N/A if no hand-written path buffers — audit `map.rs`/`dl.rs` buffer sizes |
| mapped images visible to `dl_iterate_phdr`/unwinders (Eclipse's module-registry gotcha) | **gap**: rust_dl_iterate_phdr answers from mapped objects (present) — verify backtraces of engine threads resolve; otherwise engine abort reporting is line-noise |
| RTLD semantics (RTLD_NOW/LAZY, RTLD_DEFAULT handle=1) | ✓ present; verify `RTLD_DEFAULT` dlsym fallback to runtime libc matches `roblox_linker_sys::runtime_symbol` |
| guest/host one-address-space hazards | the Rust linker maps the arm64 or x86_64 engine natively (no translator; `libbadcpu` was never wired). bionic-vs-host libc differences are handled by `crates/abi` tables (already Rust), not by the linker |

**Conclusion:** no structural gap; the C/dlopen wiring, `set_realpath`, and a
round of "does the engine actually load" verification are the work.

---

## D. Dependency removal map

Name clash, stated first: the **crates.io** dependency `jni-sys = "0.4.1"`
(used by `crates/jnivm` for raw JNI C types) vs the **in-repo** package
`roblox-jni-sys` (directory `crates/jni-sys`). No literal collision today
(different package names), but the directory name `crates/jni-sys` is a
confusion trap. The removal plan renames the in-repo crate (below), keeping the
public re-export `roblox_runtime::jni` stable.

### `crates/linker-sys` (package roblox-linker-sys) — every call site

| consumer | use today | becomes |
|---|---|---|
| `crates/abi` | `roblox_linker_sys::host_symbol` (host dlsym w/ cache) + build.rs link lines | `host_symbol` moves to `crates/linker` (it is pure dlopen/dlsym logic); build.rs link lines deleted; `DEP_ROBLOX_LINKER_SYS_SHIM_ROOT` gone |
| `crates/android` | build.rs only (link line pulls the C++ archives) | build.rs deleted |
| `crates/jni-sys` | build.rs only | build.rs deleted |
| root `roblox-runtime` | `EngineLibrary::Bionic`, `elf::{Binding, Imports, undefined_symbols, exported_symbols}`, `init/register/set_library_path/dlopen/defer_next_ctors/run_deferred_ctors/constructor_deferral_available` | `EngineLibrary` enum collapses to the Rust variant; `elf.rs` (pure Rust, 551 lines) **moves to `crates/linker`** (or stays as a small pure-Rust `roblox-elf` module inside the root crate — Q2); `LoadError::ConstructorDeferralUnavailable` disappears; `USE_EXPERIMENTAL_RUST_LINKER` becomes the only path |
| client | only via facade re-exports `roblox_runtime::abi`… | unaffected (see client-migration.md) |

`crates/linker-sys` is deleted in the final phase. Nothing needs to "remain as a
pure-Rust type crate" if `elf.rs` finds a new home first.

### `crates/jni-sys` (package roblox-jni-sys) — every call site

| consumer | use today | becomes |
|---|---|---|
| root `roblox-runtime` | `pub use roblox_jni_sys as jni`; `src/lib.rs` (bootstrap, startup setters, initialize_game_activity…), `src/graphics.rs` | same API surface, now implemented over `jnivm` instead of the C++ shim; the `jni` re-export keeps its path (renamed package, re-export unchanged) |
| client (parent repo) | `roblox_runtime::jni::game_activity::*` (≈30 distinct fns incl. `RawTextBoxInfo`, `focused_textbox`, `appbridge_call_bare`, IME/gamepad/textbox setters), `roblox_runtime::jni` implicit via facade | path-identical re-export; zero client code changes (see client-migration.md) |
| internals | `game_activity::{bootstrap,display,surface,input,lifecycle_text,services,accessibility}.rs` call the 18+36+31+7 C++ externs | each extern call site re-points to the Rust port (same signatures, Rust-native errors); `jni::init_compat_bridge` deleted with the C++ fallback |

Crate disposition (Q2): rename the package `roblox-jni-sys` → `roblox-jni`
(directory `crates/jni`), since after this work it is a driver crate, not a
`-sys`. The workspace dependency `jnivm`'s crates.io `jni-sys` types stay
untouched.

### Also removed

- `cmake` build-dependency (workspace `Cargo.toml`), all four `build.rs` link
  scripts, `native/CMakeLists.txt`, `native/`, `third_party/`, `.gitmodules`
  (both submodule entries), README build requirements (Clang/CMake/patch/submodules).
- `jnivm::set_cpp_fallback_enabled` + the fallback machinery in `abi/calls.rs`
  (`roblox_jni_fallback_*` externs) — the compat bridge only ever existed to
  back Rust-VM users with C++ classes; once the C++ classes are ports, it goes.
- Root `Cargo.toml` of the *client* mentions nothing native; only the runtime
  README does (update it).

---

## E. Client impact

Delivered as `docs/client-migration.md` alongside this file. Summary: the client
(the parent repo, visible here) touches only the facade
(`RuntimeConfig/LoadedEngine/RuntimeOptions/session/graphics/webview/permissions/jni::game_activity`);
its migration is: drop submodule init + native toolchain from CI, delete the
`jnivm_cpp_fallback` setting (shim kept meanwhile), and expect the two
"experimental" env flags to become the only behavior. No public symbol it uses
disappears; everything gets a same-path re-export or an explicit note.

---

## F. Phase order and feature-flag strategy

The tree builds and tests green at every commit. The C++ path and the Rust path
coexist, selected the way they already are (env vars), extended with per-area
selection:

- **P0 — audio completion (crates/audio only; no interface change).**
  1. Consolidate backend selection: `backend/mod.rs` currently carries a stale
     dlopen-probe selector duplicating `backend/host.rs`'s cpal-probe selector
     (the design decision). Delete the stale one, re-export
     `host::{effective_backend_name, host_backend_available, host_backend_name,
     make_output_stream}` from `mod.rs` as the task's hypothesis expects.
     Replace `eprintln!` in cpal_output.rs with `roblox-logging`.
  2. Port `opensles.cpp` → `crates/audio/opensles/` (engine/player/recorder,
     IID pointer-identity rule; recorder is real — SL_IID_RECORD reversal kept).
  3. Port the AAudio stream surface (`aaudio.cpp`): builder, open/start/stop/
     pause/close, getters-report-what-was-granted, blocking read, input via
     `input_callback.rs`; the one-way `supportsAAudio()` gate.
  4. Port the capture stream (cpal input stream wrapper with the C++
     `CaptureStream` contract: no PipeWire/host resource before `open()` or
     after `close()`; single process-wide capture chokepoint).
  5. Port remaining tests: 14 from `aaudio_input_test.cpp`, plus
     `pipewire_backend_test.cpp`'s remaining sink-selection cases if any are
     still uncovered (pcm.rs covers 10 of 15 today).
  6. Port `audio_classes.cpp` (Java audio classes; microphone rule) — depends on
     P2 (jnivm hooks) for registration, but its logic lands here.
  7. Probes (`audio_probe`, alsa/pulse probes) → `examples/`.
  Feature: none needed — the audio crate is inert until wired into symbol
  registration in P3/P5; the C++ audio path keeps serving `aaudio_overrides()`
  until the switch flips.

- **P1 — libc-compat ports (`crates/abi`, `crates/android`), selectable.**
  liblog, system_paths, netdb, legacy_stdio, thread_trace → Rust, each with its
  parity test (constant tables diffed mechanically against the C++ tables).
  Selection: a cargo feature `cpp-compat` (default ON) on the root crate keeps
  the C++ tables registered; `rust-compat` registers the Rust ones. Both sets
  can even coexist under different symbol names during bring-up. Variadic
  constraint handling per §Variadic below.

- **P2 — jnivm gap closure** (§B gaps: attach-style unification). Each with unit
  tests; no behavior change for the C++ path. Progress: primitive-array ABI
  entries (commit `95131b1`), the exception model (commit `0cd6e9c`), the
  Char/Short/Float/Double Call\* return variants (commit `54d3a9a`), the
  unresolved-report hook (commit `cd1d650`), and CharSequence interface
  dispatch are complete; the remaining gap is unifying `env_token` vs
  `attached_thread_env` attach semantics.

- **P3 — JNI hook ports into `crates/framework`** (proposed name), group by
  file: platform/unanswered/battery → cookies/clipboard/permissions/deeplink →
  local_storage → accessibility → android_classes → game_activity drivers →
  init_params. Registered in the Rust VM only; the C++ VM keeps serving the
  legacy path via its own registration. `USE_EXPERIMENTAL_JNIVM=true` selects
  the Rust VM; a new env/feature flag (`RBX_RUNTIME_HOOKS=rust`) lets the C++
  VM also defer to Rust hooks through the existing fallback, so hooks can land
  before the VM flip. Descriptor check (§B.1) runs per file before its commit.

- **P4 — linker flip**: complete §C gaps (libdl registration of `rust_dlsym`
  etc., realpath override), run the engine under the Rust linker, parity-check
  the symbol table (§Gates). `USE_EXPERIMENTAL_RUST_LINKER` becomes default-on,
  then the only path.

- **P5 — removal commits, in this order:**
  1. flip every consumer to the Rust crates (features default to Rust paths);
  2. delete `crates/jni-sys`'s C++ links + `crates/linker-sys` (after `elf.rs`
     and `host_symbol` moved), rename package to `roblox-jni`;
  3. `git rm third_party/`, clean `.gitmodules`;
  4. delete `native/`, all `build.rs` C++ steps, `cmake` dependency,
     `CMakeLists.txt`; update README + docs.
  No C++ file is deleted whose behavior has no Rust test (gate below).

### Variadic constraint (P1 blocker, needs your sign-off: Q3)

Rust (stable) cannot define variadic `extern "C"` functions. Today the C++
layer owns every variadic entry point the engine imports:
`open` (path redirect), `fprintf`/`vfprintf` (`__sF` remap), and
`__android_log_print`. `printf`/`asprintf`/`vasprintf`/`syslog` resolve to host
glibc or stubs today — add them to the host-compatible list rather than
intercepting (verify current resolution first; `printf` currently looks stubbed,
which is a "stub that lies" to fix while there).

Plan for the three that must be intercepted:

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

## G. Parity gates (before any deletion)

1. **Symbol-set parity, per virtual library, compared by script**: dump the
   registered tables (Rust) vs the C++ tables (`roblox_*_symbols`) — e.g. the
   25 AAudio names, the 7 OpenSL IID data symbols, the 13 path symbols — and
   diff. Tool: `tools/check_virtual_libraries.py` (fresh).
2. **JNI hook descriptors vs dex** (`tools/check_hook_descriptors.py`, §B.1).
3. All ported tests pass; every `*_test.cpp` has a same-named cargo test.
4. Probes print their PASS lines against a real PipeWire session (examples, run
   manually).
5. Written manual-verification checklist (below) — **none of these are claimed
   verified by this work**:
   - signed-in launch to the Landing screen;
   - entering an experience;
   - voice chat in a live call (microphone indicator lights only while
     recording, dark the moment it stops, including pause);
   - leaving a game without a freeze;
   - audio device switch while playing (the lost live-default-sink-following
     behavior makes this a *known degraded* case — verify at least that audio
     survives until restart);
   - clipboard copy out of Roblox; cookies persist across restart;
   - `--dump-classes` equivalent still produces the observed-surface inventory.

## H. Provenance notes (per ported area)

- **Audio backend design** (`backend/cpal_output.rs`, `host.rs`): written
  against cpal 0.18.2's *examples*, per the task note. Reference check done
  against `~/.cargo/registry/.../cpal-0.18.2`: `SampleRate = u32` ✓;
  `host_by_name` matches `format!("{HostId:?}")` case-insensitively against
  cpal's `available_hosts()` ✓ (works for whichever host features are enabled);
  the build error type in 0.18.2 is `crate::Error` (not 0.15's
  `BuildStreamError`) and the code maps it to `String` ✓. Eclipse showed the
  "callback does bounded work, helper threads do the rest" shape; the decisions
  (one cpal impl for PW/Pulse/ALSA, owner-thread stream lifecycle, probe =
  construct + default device) were re-derived here.
- **AAudio/OpenSL behavior**: the in-repo C++ is the source of truth for the
  FMOD contract (25 names, read-back getters, one-way `supportsAAudio()`); the
  measured contract is re-checked from `docs/analysis`-style comments in the
  C++ itself. Eclipse's `src/loader/aaudio.rs` (MIT) confirmed the shape
  (slab registry, error-streak→Disconnected mapping) as prior art, not copied.
- **libjnivm → crates/jnivm**: the Rust VM was written before this plan; §B
  gaps were found by comparing its public surface against every libjnivm API
  the C++ shims call (cordial's usage was the checklist; nothing copied —
  cordial is GPL and stays a behavioral reference only).
- **Linker**: crates/linker is already Rust; §C lists what remains to verify.
  mcpelauncher-linker is MIT; its deferral patch semantics are reimplemented
  as inherent caller-driven constructor running.
- **Descriptor tooling**: fresh implementation from the public dex spec;
  cordial's GPL `tools/hook_descriptors.py` is not read as a porting source.

## I. Open questions (all blocking)

1. **Q1 — home of the JNI class hooks.** Proposal: one new dedicated crate
   `crates/framework` (`roblox-framework`) holding every Java class hook port
   (android/game-activity/init-params/cookies/clipboard/…), depending on
   `jnivm` + `roblox-android` + `roblox-audio`. Alternative: audio hooks in
   `crates/audio` and the rest split between `crates/android` and the renamed
   `crates/jni`. The dedicated crate keeps "the VM" (jnivm) free of Roblox
   policy and keeps policy in one place; it does add a crate.
2. **Q2 — `elf.rs` and `host_symbol` disposition** when linker-sys dies: move
   into `crates/linker`, or a small standalone `roblox-elf` crate? (Root crate
   re-export path stays `roblox_runtime::…` either way.)
3. **Q3 — variadic strategy** (§Variadic): naked-asm trampolines are the only
   stable-Rust way to receive `open`/`fprintf`/`__android_log_print`. OK to
   embed per-ISA assembly, or would you rather keep exactly those three
   variadic functions as tiny C (violating the 100%-Rust goal, but
   honest-by-test)?
4. **Q4 — license** (§License finding): how do you want to handle the
   cordial-derived `native/` body? This gates every port commit.
5. **Q5 — dead-hook policy**: C++ hooks whose descriptors mismatched the dex
   are silently dead today (§B.1). Confirm the Rust ports bind per the dex and
   *change* that behavior (engine gets the hook it asked for), with per-hook
   notes — rather than preserving the dead state.
