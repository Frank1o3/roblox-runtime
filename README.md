# roblox-runtime

Standalone runtime for executing the Roblox Android client on Linux.

`roblox-runtime` owns the Android compatibility, native loading, JNI bridge,
ABI handling and graphics integration required to run Roblox's Android client
inside a normal Linux process. The embedding client, currently `rusty-blox`,
owns the user-facing application layer: APK discovery/provisioning, extracted
client files, data/cache locations, settings, Fast Flags and the host window.

## Status

**Runs, with native migration and shutdown work remaining.**

The release build has been manually observed loading and running an experience
through `rusty-blox` (details and limits below). The runtime still compiles
project-owned C++ compatibility code for ABI symbols, Java hooks, audio, and a
companion JNI VM. It is not yet a Rust-only implementation.

The runtime is designed as an embeddable component rather than a standalone
launcher. It does not download or discover Roblox builds itself; the embedding
application supplies the APKs and runtime directories through `RuntimeConfig`.

This is currently an **x86-64-only** runtime. Some AArch64-specific bionic
pthread wrappers exist as ABI groundwork, but `crates/linker/src/map.rs`
intentionally rejects non-x86-64 hosts at compile time; they do not constitute
end-to-end AArch64 support.

### Manual runtime observations

On 2026-11-15, the embedding `rusty-blox` client was run with `cargo run
--release`. The build completed, the user signed in, and an experience loaded
and ran. This is a manual smoke test of the release build; the equivalent debug
build is still reported to fail during game loading, and the cause of that
configuration difference is unknown.

On 2026-11-21, `cargo run --release -- --launch` was verified with a saved
cookie-only profile. The client loaded the managed x86-64 APK and
`libroblox.so`, recovered the saved account identity through Roblox's
authenticated-user endpoint, and reached the signed-in Home shell. This run
did not join an experience; its evidence covers APK loading and automatic
session restoration through Home.

A second 2026-11-21 run log supplied by the client shows an experience loading
and audio streams opening. A subsequent run also loaded the experience and audio,
then reported an unresolved `AAsset_seek` lookup before Wayland
`wl_shm.create_pool` failed with `Bad file descriptor`. Rust now implements
`AAsset_seek` and `AAsset_seek64` and supplies FMOD's `AssetManager` object.

After the reference and asset-manager fixes, `cargo run --release -- --launch`
again loaded the managed x86-64 APK, restored the saved identity, reached Home,
and started the saved experience. That run did not log the earlier
`NewGlobalRef` failure, the FMOD asset-manager lookup, the `AAsset_seek` lookup,
or the Wayland bad-file-descriptor message. I stopped it with Ctrl-C, so this
run does not verify graceful window shutdown.

The earlier log ended in a `SIGSEGV` on process exit. The
systemd-coredump entry has no core attached and reports the instruction pointer
at zero, so it does not identify the crashing call. The client now sends the
observed GameActivity focus, pause, surface-destroyed, stop and
`terminateNativeCode` callbacks before releasing the host surface. That
shutdown sequence still needs an interactive close-and-relaunch check.

When the client was stopped, the process exited with signal 11 (`SIGSEGV`)
instead of shutting down gracefully. The log also showed Wayland
`wl_shm.create_pool` requests failing with `Bad file descriptor` before exit;
the available log does not establish whether those messages are related to the
shutdown crash. Clean shutdown remains unverified and should be treated as a
known runtime limitation.

The active engine loader is the Rust ELF mapper in `crates/linker`. The primary
JavaVM, Rust hooks, and `JNI_OnLoad` path use `crates/jnivm`, while the native
compatibility bridge still creates a companion VM and owns many Android,
GameActivity, service, and audio hooks. The two VM paths have not yet been
consolidated.

## Architecture

    rusty-blox
        │
        │ application / integration layer
        │
        ├── APKs and extracted libraries
        ├── data + cache directories
        ├── settings / Fast Flags
        └── host render surface
                │
                ▼
          roblox-runtime
                │
                ├── ABI compatibility
                ├── Android platform compatibility
                ├── JNI / GameActivity bridge
                ├── Rust ELF loading
                ├── pthread / bionic compatibility
                ├── graphics integration
                └── Roblox libroblox.so

This keeps the runtime independent of the UI/application that embeds it while
allowing the client to control paths, configuration and the host window.

## Runtime components

### ABI and Android compatibility

The `abi` crate provides the Android/bionic ABI surface required by the client,
including bionic pthread primitives and generated fallback symbols.

The `android` crate supplies Android platform behavior that has to exist on the
Linux host, including assets, storage, loopers, native windows and system APIs.
It answers the observed NDK thermal queries with a neutral desktop status;
Android thermal listeners and device-specific thresholds are not emulated.

### Native loading

The `linker` crate maps ELF objects, resolves imports, and delays constructors
until Android services are ready. The former AOSP bionic linker integration
has been removed; the remaining native build support is for compatibility code
outside the linker.

Engine imports are inspected from the actual ELF rather than maintained as a
manually copied symbol list. Symbols are then supplied by runtime-owned
implementations, selected host libraries, or explicit compatibility stubs.

### JNI

The `jni` crate provides the JNI and GameActivity bridge used by the Roblox
client.

The `jnivm` crate owns the JavaVM used by the runtime startup and `JNI_OnLoad`
path. The native bridge still builds and runs a companion libjnivm VM for
unported hooks and callback drivers; hook registration and object state are
therefore not yet single-VM end to end.

### Graphics

The runtime supports host-backed EGL/GLES and Vulkan integration.

For Vulkan, `graphics-vulkan` loads the host Vulkan implementation and translates
the Android surface requests made by Roblox into the host window-system surface.
The Roblox renderer therefore submits through the host Vulkan driver rather than
running inside an Android emulator or virtual machine.

Vulkan presentation can be configured with:

    auto
    mailbox
    uncapped
    immediate
    fifo
    fifo-relaxed
    off

The `present_mode` option is supplied by the embedding application through
`RuntimeOptions`. With VSync enabled, `mailbox`, `immediate`, `fifo`, and
`fifo-relaxed` request those Vulkan modes directly; `fifo-relaxed` is adaptive
VSync. With VSync disabled, Vulkan requests immediate presentation, and `off`
does the same. A requested mode is used when the host surface supports it;
otherwise the engine's requested mode is retained. For OpenGL ES, disabling
`vsync` forces swap interval `0`; with it enabled, `opengl_swap_interval` selects
`1` on, `0` off, or `-1` adaptive where the EGL driver supports adaptive swap
control.

## Public integration surface

The main runtime API is exposed through `RuntimeConfig` and `RuntimeOptions`.

The embedding application supplies:

- base and split APK paths
- the extracted native-library directory
- writable data and cache directories
- Fast Flags
- graphics and presentation settings
- an optional login/session profile

The runtime then prepares the Android environment, extracts required assets,
sets up the engine working directory, prepares graphics, resolves native imports,
loads `libroblox.so`, initializes the compatibility environment and exposes the
loaded engine to the caller.

The runtime deliberately does not own APK discovery or application UI.

## Building

Requirements for the current build (the Rust-only target is not complete):

- recent stable Rust
- Clang/Clang++
- CMake
- initialized Git submodules, including `third_party/libjnivm`

Initialize the native dependencies and check the workspace:

    git submodule update --init --recursive
    cargo check --workspace

For a release build:

    cargo build --release

The release profile keeps debug information enabled because the runtime operates
at the crates/native-sys/native/ABI boundary and is substantially easier to diagnose with symbols.

## Runtime options

The embedding application can configure:

- `graphics_backend` — automatic, Vulkan or OpenGL ES
- `present_mode` — Vulkan presentation preference
- `vsync` — enable/disable synchronized presentation
- `opengl_swap_interval` — EGL swap interval (`-1`, `0`, or `1`)
- `host_libc` — explicitly enable the ABI-unsafe diagnostic host-libc path

`host_libc` is intended for diagnostics and compatibility investigation, not as
the normal execution mode. Structure-sensitive Android/bionic interfaces remain
runtime-owned where the host ABI is not compatible.

## Performance and diagnostics

The runtime keeps the hot path small where possible and exposes diagnostic
logging through `RUSTY_BLOX_LOG_LEVEL`.

Before startup, the runtime keeps the newest ten Roblox log files in its known
`appData/logs` directories and the newest ten `memProfStorage*.json` files in
`appData/LocalStorage`. It also removes empty directories under its data and
cache roots, without traversing `rbx-storage` directories or deleting content
blobs or database files. Cleanup failures are logged and do not block startup.

Because Roblox and the runtime execute in the same process, process-level memory
and CPU measurements include both the compatibility runtime and the Roblox
engine itself.

For example:

    ps aux | grep '[r]usty-blox'

reports the combined process rather than a separate runtime-only figure.

## Project goals

`roblox-runtime` is intended to be a reusable runtime component, not a launcher
that happens to contain a runtime.

The long-term direction is:

1. Keep the runtime boundary small and explicit.
2. Move Android-specific behavior into dedicated compatibility crates.
3. Keep ABI-sensitive native compatibility code isolated from higher-level Rust
   code.
4. Continue replacing compatibility dependencies with Rust implementations where
   doing so preserves the required ABI and behavior.
5. Keep `rusty-blox` responsible for application integration rather than making
   the runtime depend on one particular frontend.

## Roblox compatibility and account risk

This is an unofficial compatibility project. It is not affiliated with,
endorsed by, or sponsored by Roblox Corporation. It runs the Android client in
an unsupported host environment.

Roblox's terms, client-integrity checks, anti-cheat systems, or service-side
enforcement may refuse the client or result in account action, including
suspension or termination. No account-safety guarantee is made. Review
Roblox's current terms before use, and do not use an account you cannot afford
to lose.

## License

Project-owned runtime code is licensed under **GPL-3.0-or-later**. Certain
adapted files retain their upstream licenses and attribution; see
[docs/licensing.md](docs/licensing.md),
[crates/linker/ECLIPSE-NOTICE.md](crates/linker/ECLIPSE-NOTICE.md), and
[crates/jnivm/LIBJNIVM-NOTICE.md](crates/jnivm/LIBJNIVM-NOTICE.md).

## Community

Contributions should follow [CONTRIBUTING.md](CONTRIBUTING.md) and the
[Code of Conduct](CODE_OF_CONDUCT.md). Please read [SECURITY.md](SECURITY.md)
before reporting a vulnerability. Bug reports and feature proposals can be
submitted through the GitHub issue templates.
