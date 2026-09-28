# Runtime extraction audit

This audit is based on the local `rbx-native-runtime` tree at the time of
extraction. It is the source of truth; no upstream Cordial copy was consulted.
The source tree was clean at audit time. This file precedes the code migration
so that launcher boundaries are reviewable before any source is omitted.

## Entry path and dependency shape

`crates/cordial-runtime/src/bin/load.rs` is the only full launch path. It parses
paths and run options, prepares Cordial profile state, opens a host window,
loads Android libraries through `symtab` and `linking`, constructs the JNI
environment, calls Roblox bootstrap exports, and pumps Android/host events.
The useful runtime contract is that flow, not the Cordial CLI, profile policy,
updater bookkeeping, or plugin event publication around it.

The current implementation is not yet separable by simple file copying. The
runtime crate calls `cordial-shell` for the GTK/libadwaita toplevel, profile
locking, secrets, and web views; it calls `cordial-update` for HTTP, version,
and asset cache helpers; and it calls `cordial-plugins` for Fast Flag document
parsing as well as plugin execution and presence state. Those edges are
recorded below and must be replaced with runtime-owned interfaces or removed
only after their call sites are migrated. In particular, Android Wayland's
surface is a subsurface of the shell's GTK toplevel on the same Wayland
connection, so the host-window implementation is currently a runtime
requirement even though its present implementation comes from the shell.

## Crates and workspace files

| Source | Purpose | Runtime-required | Used by | Destination | Dependencies | Action | Reason |
| --- | --- | ---: | --- | --- | --- | --- | --- |
| `Cargo.toml` workspace | Membership, shared package metadata, lint policy | Yes, adapted | All crates | Workspace root | Cargo | adapt | New workspace uses resolver 3 and edition 2024. |
| `crates/cordial-linker-sys` | Rust FFI and CMake build for the ported bionic linker and libjnivm/native shims | Yes | Runtime loader and JNI callbacks | `crates/linker` plus `crates/jni` boundary | CMake, Clang, C++, submodules | split | Linker and Java VM are distinct APIs despite sharing a native build today. |
| `crates/cordial-runtime` | Android ABI implementations, loading, bootstrap helpers, and many Cordial services | Yes, selectively | `cordial-run`, native shims | focused crates and root integration | GTK, linker-sys, plugins, update, zip, zbus, mimalloc, HTTP | split | This is the source implementation, not a suitable destination crate as-is. |
| `crates/cordial-plugins` | Plugin registry, process sandbox, protocol, capabilities, broker and settings | No as a plugin system; flag parsing and select data formats are shared | Runtime flag helpers and shell | Excluded; runtime config supplied as data | serde, crypto, process and protocol dependencies | remove / adapt | Plugin execution is application functionality. Flag parsing is retained or replaced without plugin loading. |
| `crates/cordial-shell` | GTK/libadwaita client, host window, profile, installation, settings and launch | No as a shell; host-window primitives currently required | Runtime Wayland, clipboard, webview, launcher | host-window slice to `platform`; remaining shell excluded | GTK4, libadwaita, WebKit optional, desktop integration | split / remove | Preserve the existing Wayland host-window behavior behind a platform API; omit UI and installation code. |
| `crates/cordial-update` | Roblox metadata, downloads, APK/cache/update orchestration | No acquisition/update client; minimal generic HTTP or engine version read may be useful | Runtime assets, settings, launch | runtime-owned path/cache API; acquisition excluded | ureq, crypto, archive | split / remove | APK acquisition and update policy belong to `rusty-blox`; runtime accepts supplied paths. |
| `Cargo.lock` | Resolved source dependency graph | Build reproducibility | Workspace | Regenerate for destination | Cargo | adapt | Must be generated from the destination workspace after extraction. |

## Runtime Rust modules

| Source | Purpose | Runtime-required | Used by | Destination | Dependencies | Action | Reason |
| --- | --- | ---: | --- | --- | --- | --- | --- |
| `src/bin/load.rs` | End-to-end library load, JNI setup, bootstrap and event loop, mixed with Cordial CLI and profile/plugin hooks | Yes, core flow | User-facing `cordial-run` | root `src/` orchestration, progressively decomposed | Most runtime modules; shell, plugins, update | split | Keep bootstrap order and behavior while replacing externally owned paths/options and removing launcher policy. |
| `src/android/*` | NDK API, assets, input, looper, display, graphics and Android system shims | Yes | Loader and native symbol table | `crates/android`; graphics/display edges to graphics/platform crates | linker ABI, zip, GTK host window | split | Coherent Android compatibility surface; GL, Vulkan and Wayland are dedicated boundaries. |
| `android/asset.rs` | APK asset access, overlays, extraction/indexing and cache invalidation | Yes | Engine file/asset calls and launch diagnostics | `crates/android` with externally provided APK/cache roots | zip, update cache helpers, overlay policy | adapt | Keep APK reads and non-destructive overlays; remove updater-owned location assumptions. |
| `android/input.rs`, `gamepad.rs` | Android touch, keyboard, text, pointer capture and gamepad ABI | Yes | Native input callbacks | `crates/input` | Android ABI, platform input | move | Host-independent input translation deserves its own subsystem. |
| `android/gl.rs`, `glcount.rs` | EGL/GLES exports and diagnostics | Yes | Dynamic symbol registration | `crates/graphics-gl` | graphics host libraries, linker | move | Android-facing GL compatibility is kept separate from host backend. |
| `android/vulkan.rs` | Android Vulkan loader/API compatibility | Yes | Dynamic symbol registration and engine Vulkan loads | `crates/graphics-vulkan` | Vulkan loader, Android ABI, linker | move | Large proven implementation stays intact and isolated. |
| `android/wayland.rs` | Wayland subsurface, frame scheduling, text input, window events and host integration | Yes | Display backend and input lifecycle | `crates/wayland` | wayland-client/protocols, platform host-window API | move / adapt | Large subsystem; remove direct shell types through a narrow host-window contract without redesigning protocol behavior. |
| `android/window.rs` | X11 display/window fallback | Yes while fallback remains supported | Android display selection | `crates/platform` or `crates/graphics` | x11rb, EGL | move | Host display backend, not Android API implementation. |
| `android/looper.rs`, `frame_pacing.rs` | ALooper API, event pump, frame pacing and census | Yes | Bootstrap event loop | `crates/android` initially | epoll, input, display lifecycle | move | Runtime lifecycle and event dispatch. |
| `android/accessibility.rs`, `clipboard.rs`, `capture.rs`, `editor_font.rs` | Android-facing accessibility, clipboard, capture and editor-font behavior | Yes where called by Roblox | JNI/native callbacks and host UI | `crates/android` with host interfaces | zbus, GTK host window, fontconfig | adapt | These are live runtime integrations; don't discard because they appear desktop-oriented. |
| `android/config.rs`, `system.rs`, `asset.rs`, `window.rs` | Android configuration, system property and window/asset APIs | Yes | Android ABI callbacks | `crates/android` | platform and filesystem | move | API behavior is required by the Android client. |
| `android/config.rs`, `system.rs`, `battery.rs` | Device configuration, caller-provided files/cache paths, host `/system` font tree, battery ABI values | Yes | Framework callbacks and launch setup | `crates/android` | native `roblox_liblog` bridge | move / adapt | Extracted into `roblox-android`; `/system` cache path now comes from `RuntimeConfig`, not Cordial's XDG lookup. |
| `bionic/*` | pthread, signals, sysconf tables, trace/fault handling and libc compatibility | Yes | ABI exports and linker | `crates/abi` | libc, linker callback surface | move | Core compatibility boundary. |
| `elf.rs`, `symtab.rs`, `linking.rs`, `stubs.rs`, `unimplemented.rs`, `ffi_util.rs` | ELF inspection, dynamic symbol resolution, loading glue, ABI stubs and FFI helpers | Yes | Bootstrap and linker callbacks | `crates/linker` / `crates/abi` | `cordial-linker-sys`, libc | split | Linker orchestration separated from Android/Bionic function implementations. |
| `graphics.rs`, `headless.rs`, `refresh.rs` | Graphics selection, headless probing, refresh-rate integration | Yes in runtime launch | Loader and host display | `crates/graphics` / `crates/platform` | Android graphics and shell display | split | Policy/orchestration stays small; concrete renderers remain separate. |
| `client_settings.rs`, `flags.rs` | Roblox client settings and Fast Flag merge/document logic | Settings loading yes; plugin resolution no | Bootstrap, assets, optional plugins | runtime config/API in root or `crates/android` | update HTTP, plugin parser/enablement, JSON | adapt | Client passes settings/flags as configuration; runtime must not load plugins or fetch its own config. |
| `profile.rs`, `storage.rs`, `identity.rs`, `secrets.rs`, `cookies.rs` | Cordial profile policy, Roblox filesystem layout, identity/cookie persistence | Select filesystem mapping required; profile and account policy are client-owned | Loader and engine shims | `crates/platform` plus supplied data paths; secret/account portions excluded or injected | shell profile/secrets | split / adapt | Runtime needs data/cache roots and virtual Android paths, not Cordial profile management. |
| `permissions.rs`, `deeplink.rs`, `webview.rs`, `browser_tracker.rs` | Android permission/deep-link/webview compatibility and browser state | Mixed: required when Roblox invokes these APIs | JNI callbacks and shell UI | runtime API plus platform adapter; browser/client policy excluded | GTK/WebKit, plugins URL-open, shell policy | split / adapt | Preserve Android-facing callbacks; remove Cordial browser and user-facing navigation policy from runtime core. |
| `battery.rs`, `game_log.rs`, `bloxstrap_rpc.rs`, `roblox_api.rs` | Device responses, log observation, external RPC and Roblox web metadata/presence | Mixed | Runtime startup and Cordial features | `crates/android` for device ABI; other application integration excluded | plugins, updater HTTP, shell | split / remove | Keep Android responses needed by engine; external client and presence behavior is not runtime execution. |
| `plugin_host.rs` | Plugin process lifecycle and event bridge | No | Launcher and plugin callbacks | Excluded | `cordial-plugins`, process APIs | remove | This is entirely plugin infrastructure. |
| `devctl.rs`, `mimalloc_lib.rs` | Development control surface and dynamically discoverable allocator | Runtime diagnostics optional; allocator discovery required for parity | MCP tooling / engine | `crates/platform` diagnostics and `crates/runtime` allocator | local socket; mimalloc | split | Keep developer control optional and isolate it; preserve allocator behavior. |

## Native implementation and third-party code

| Source | Purpose | Runtime-required | Used by | Destination | Dependencies | Action | Reason |
| --- | --- | ---: | --- | --- | --- | --- | --- |
| `native/shim.cpp` | C ABI bridge into mcpelauncher bionic linker | Yes | Rust linker wrapper | `crates/linker/native` | mcpelauncher-linker | move | Preserve native linker; do not rewrite in Rust. |
| `native/jni_shim.cpp`, `android_classes.cpp`, `game_activity.cpp`, `init_params.cpp`, `platform_classes.cpp`, `unanswered_classes.cpp` | JNI registrations and Android Java class/method compatibility | Yes | libjnivm and engine JNI calls | `crates/jni/native` | libjnivm | move | Core Android Java surface. |
| `native/accessibility.cpp`, `battery.cpp`, `clipboard.cpp`, `cookies.cpp`, `deeplink.cpp`, `local_storage.cpp`, `permissions_transport.cpp`, `legacy_stdio.cpp` | Native/JNI framework bridges and host/runtime state access | Yes where registered/called | JNI class registrations | `crates/jni/native` | JNI and runtime-owned interfaces | move / adapt | Preserve shims while replacing Cordial-specific callback wiring. |
| `native/liblog.cpp`, `system_paths.cpp`, `netdb_compat.cpp`, `thread_trace.cpp` | Android log and libc/system compatibility | Yes | Android native calls and linker | `crates/abi/native` | host libc, optional host audio | split | Separate generic ABI compatibility from JNI registration. |
| `native/opensles.cpp`, `aaudio*`, `*_backend.cpp`, audio probes/tests | OpenSL/AAudio bridge and optional PipeWire/Pulse/ALSA/OSS host output | Audio compatibility required for parity; probes are development-only | Android audio calls | `crates/android-audio/native` or `crates/abi/native` | optional system headers, dlopen libraries | split | Keep honest unsupported behavior and selected host adapters; probes do not ship in runtime API. |
| `native/CMakeLists.txt` | Builds linker, JNI, log/audio libraries and native checks | Yes, adapted | `cordial-linker-sys/build.rs` | per-crate native builds or shared `crates/native-compat` | CMake, Clang, optional audio headers | split | Avoid one monolithic native target after Rust boundaries are established. |
| `third_party/mcpelauncher-linker` | AOSP bionic linker host port | Yes | `native/shim.cpp` | destination submodule under `third_party/` | git submodule | preserve | Required native dependency; no rewrite. |
| `third_party/libjnivm` | JNI virtual machine and class/method registry | Yes | JNI shim | destination submodule under `third_party/` | git submodule | preserve | Required compatibility implementation. |
| `third_party/libbadcpu` | x86-64 CPU feature emulator source | No | No linked caller | Excluded from destination | C++ | remove | CMake built it as an unlinked archive; the linker build script links no `badcpu` archive and no runtime caller references it. Its dead build target is removed. |
| `third_party/mocktail-webview` | Web view support or test dependency | Unclear from runtime calls | Webview path/tests | inspect manifest and references before disposition | native/third-party | audit | No removal decision until its consumers are fully traced. |

## Build, submodules and validation notes

The source linker crate's `build.rs` requires Clang/Clang++, configures
`native/CMakeLists.txt`, and links static `cordial_linker_shim`,
`cordial_jni_shim`, `cordial_liblog`, `jnivm`, `logger`, and `linker`, followed
by host `stdc++`, `z`, `dl`, and `pthread`. It watches both the native tree and
the mcpelauncher linker sources. The CMake file also compiles audio backends
conditionally and runs native audio and permissions tests as build dependencies.
The destination must retain these requirements until the native build has been
split and observed to work.

The source `.gitmodules` identifies `third_party/mcpelauncher-linker` from
`minecraft-linux/mcpelauncher-linker` and `third_party/libjnivm` from
`ChristopherHX/libjnivm`. Both submodules are checked out locally at audit time.
Their local contents and pinned gitlinks must be carried to the destination;
they are not to be replaced with fetched alternatives. `libbadcpu` and
`mocktail-webview` are ordinary vendored directories, not submodules.

The source test surface includes unit/integration tests inside each Rust crate,
runtime probes, and native CMake checks. Migration validation is expected to
run `cargo check --workspace`, `cargo test --workspace`, and
`cargo clippy --workspace --all-targets -- -D warnings` in the destination.
Because separate builds must not share `target/`, destination validation must
use its own Cargo target directory. Runtime parity additionally requires a
client-supplied APK and native library directory; this audit does not make a
runtime claim based solely on compilation.

The destination exposes linker and JNI native code through separate Rust
crates. The existing native build is retained because the shims share CMake
setup and symbols. Bionic functions, their generated stubs, and the missing
symbol table are extracted into `roblox-abi`; the generated input remains the
source TSV. The old `badcpu` target is omitted because the linker build script
never linked that archive.

## Boundary decisions

* `rusty-blox` owns APK discovery/import (including Sober path lookup), the
  managed APK/native-library space, data roots and host-window creation. The
  runtime receives those paths, configuration and a host rendering surface; it
  contains no APK download/update client or `rusty-blox` dependency.
* Fast Flags are values/configuration supplied by the caller. The plugin host,
  plugin registry, plugin broker and plugin process lifecycle are not carried
  into the runtime.
* Renderer selection and Android input translation belong to the runtime.
  `rusty-blox` collects host input events and forwards them; the runtime maps
  them to Android input semantics. Linker, JNI, ABI, Android, graphics GL,
  graphics Vulkan, Wayland, input and platform responsibilities are independent
  crate boundaries. Root `src/` owns a small runtime configuration and
  orchestration API only.
* Existing GTK host-window use is a real dependency of the current Wayland
  implementation. It will be carried as a platform backend first, then hidden
  behind a stable API; removing GTK before replacing the toplevel would break
  the same-connection subsurface relationship.
* Existing source runtime defects are inherited. Migration changes will be
  limited to API adaptation and regressions introduced by extraction.

## Destination status

The destination workspace now contains `roblox-linker`, `roblox-jni`, and
`roblox-abi`. JNI, GameActivity, accessibility, and the large GameActivity
surface are split into focused Rust source files. `LoadedEngine` now wires
JavaVM creation, `JNI_OnLoad`, and the exported `GameActivity.initializeNativeCode`
wrapper behind the constructor/VM ordering checks. The linker and JNI C ABI
symbols and private Rust/C++ struct names have been renamed together to remove
the old project prefix; the Android system properties identify the emulated
host as Linux/Roblox Runtime. The native linker implementation remains the
same local implementation and both native dependencies are represented by
their source-pinned submodules.

The root API currently defines caller-supplied base/split APK, native-library, data,
cache, Fast Flag, and settings inputs and validates empty paths. It prepares the
initial Android configuration, files directory, `/system` font tree, and
caller-supplied APK asset manager from those paths. `roblox-android` now
provides the `AAssetManager` buffer, length, close, and file-descriptor calls
across base and split APKs, extracts a stamped filesystem asset tree from the
same APK set, and exposes a single symbol override registry for its current
Android surface. `roblox-linker` reads the supplied engine ELF's undefined
dynamic symbols and marks weak imports as optional. The root resolver combines implemented
ABI/Android addresses, selected host GLES/EGL/math/compression symbols and the
generated stubs, rejects strong imports with no answer, registers virtual
Android libraries, and maps `libroblox.so` through bionic with ELF constructors
deferred. The CMake build applies a small linker patch to a build-tree copy of
the pinned submodule, leaving the submodule checkout untouched.
The real Sober APK run reported 565 required and 8 weak imports and mapped the
112,661,808-byte executable segment successfully. Because this workspace has no
launch binary for the development MCP to drive, a temporary constructor probe
was used against the supplied local APK. It first reported zero-return stubs
for `syscall`, `memset`, `__cxa_atexit`, and
locale functions, then a zero-return stub for `__ctype_get_mb_cur_max`. The
resolver now maps these seven observed ABI-compatible libc calls from host
glibc. It resolves pthread mutex calls directly on x86-64, where the layouts
match, and keeps dedicated bionic wrappers on aarch64. Resolver tests verify
those registrations and the architecture-specific handling. The linker
initially also exposed unresolved references to four `roblox_local_storage_*`
callbacks; these are now implemented by a private per-user JSON store. The
probe then followed the source-observed order: create the JavaVM, call the four
native directory setters, and run deferred constructors. In the explicit,
ABI-unsafe `host_libc` diagnostic mode, the observed output ended with
`constructor probe: deferred ELF constructors returned` (with a `time` stub
still reported). This does not establish that normal ABI mode succeeds or that
Roblox launches; the mode can pass incompatible glibc structures and remains
diagnostic only.

The Android native-window shim now exposes a client-supplied surface token,
dimensions and EGL adaptations for host Xlib/Wayland windows. Installing a
surface also supplies its dimensions to the JNI display shim. Graphics
preparation selects Vulkan when Ash loads the host Vulkan library and the
matching Xlib/Wayland WSI extension is available; otherwise Automatic selects
GLES3. `roblox-graphics-vulkan` loads the host loader through Ash, exposes its
`vkGetInstanceProcAddr` through virtual Android Vulkan sonames, maps
`VK_KHR_android_surface` to the active host WSI, and forwards the engine's
remaining calls to the host driver. It also carries over Cordial's present-mode
selection and Wayland current-extent adaptation. The implementation has not
yet been observed creating a Vulkan surface, swapchain, or rendered frame, so
no playable launch claim is made.

`LoadedEngine::resize_surface` now updates the installed dimensions and sends
both app-bridge surface updates followed by GameActivity's surface-changed
callback. This path compiles but has not yet been observed against a running
client.

Before the Vulkan crate was added, `cargo fmt --all -- --check`,
`cargo check --workspace`, and `cargo test --workspace` passed, including ELF
parser and resolver tests. For this port, formatting passes and
`cargo check -p roblox-graphics-vulkan` passes. The full workspace check has not
completed in this environment because the linker build script requires CMake,
which is unavailable here; the runtime integration and live Vulkan path remain
unverified. Strict workspace Clippy previously failed in `roblox-abi` on
inherited undocumented unsafe blocks and existing style lints in the copied
bionic/stub code. These lint findings are not evidence of runtime parity.

## Client and renderer contract

`RuntimeConfig::apk_paths` carries the base APK and any split APKs as paths
selected/imported by the client. `RuntimeOptions::graphics_backend` records an
explicit runtime preference. `RuntimeConfig::prepare_graphics` verifies the
client surface and selects the host Vulkan path when Ash loads Vulkan and the
surface's WSI extension exists; Automatic falls back to GLES3, while an
explicit unavailable backend reports an error. Roblox owns Vulkan device,
swapchain and rendering creation; this path has not yet been observed against a
running client.

The client owns creation, visibility and destruction of the host window. The
runtime accepts Xlib handles or same-connection Wayland display/surface plus a
`wl_egl_window`; those objects must outlive the runtime surface. `rusty-blox`
now creates a winit window and hands these handles to the runtime, retaining
the Wayland EGL window for the surface lifetime. It runs the JNI/GameActivity
and app-bridge startup sequence, forwards resize events and sends mouse plus a
mapped keyboard subset through the engine's native input interface. It also
passes caller-provided Fast Flags and Client Settings paths/values. IME text
forwarding remains pending.
`rusty-blox` now discovers Sober's base and x86-64 split APK, imports both into
its managed data root and extracts native libraries from the split. The client
builds a `RuntimeConfig`, prepares Android filesystem paths and APK assets,
then attempts engine constructors, JNI/GameActivity initialization and the
initial surface handoff. `roblox-android` now implements Android `ALooper`
prepare/acquire/release, fd registration and callbacks, polling, removal and
wake entry points over epoll/eventfd. Negative-timeout polls are capped at
50 ms to bound recovery after a missed wake. This API implementation has not
been exercised against Roblox yet. The runtime still has no render context or
host event pump, so successful startup calls alone do not establish a playable
game.
