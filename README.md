# roblox-runtime

Standalone runtime for executing the Roblox Android client on Linux.

`roblox-runtime` owns the Android compatibility, native loading, JNI bridge,
ABI handling and graphics integration required to run Roblox's Android client
inside a normal Linux process. The embedding client, currently `rusty-blox`,
owns the user-facing application layer: APK discovery/provisioning, extracted
client files, data/cache locations, settings, Fast Flags and the host window.

## Status

**Functional.**

The runtime is now capable of running the Roblox Android client natively on
Linux through `rusty-blox`. The runtime has working paths for native library
loading, Android compatibility, JNI/GameActivity integration, filesystem setup,
threading primitives, graphics integration and the host-side services Roblox
expects.

The runtime is designed as an embeddable component rather than a standalone
launcher. It does not download or discover Roblox builds itself; the embedding
application supplies the APKs and runtime directories through `RuntimeConfig`.

The current working JNI path uses the native compatibility core in the runtime's
native layer. A pure-Rust JNI VM (`jnivm`) is also present as an experimental
backend and is selected explicitly with `USE_EXPERIMENTAL_JNIVM=true`.

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
                ├── Android framework compatibility
                ├── JNI / GameActivity bridge
                ├── native linker + ELF loading
                ├── pthread / bionic compatibility
                ├── graphics integration
                └── Roblox libroblox.so

This keeps the runtime independent of the UI/application that embeds it while
allowing the client to control paths, configuration and the host window.

## Runtime components

### ABI and Android compatibility

The `abi` crate provides the Android/bionic ABI surface required by the client,
including bionic pthread primitives and generated fallback symbols.

The `android` crate supplies Android framework behavior that has to exist on the
Linux host, including assets, storage, loopers, native windows and system APIs.

### Native loading

The `linker` crate exposes the AOSP-derived bionic linker integration and ELF
inspection used to load `libroblox.so`.

Engine imports are inspected from the actual ELF rather than maintained as a
manually copied symbol list. Symbols are then supplied by runtime-owned
implementations, selected host libraries, or explicit compatibility stubs.

### JNI

The `jni` crate provides the JNI and GameActivity bridge used by the Roblox
client.

The `jnivm` crate contains an in-progress pure-Rust JNI VM. It is opt-in and
does not silently replace the working native compatibility path.

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

Requirements:

- recent stable Rust
- Clang/Clang++
- CMake
- GNU `patch`
- initialized Git submodules

Initialize the native dependencies and check the workspace:

    git submodule update --init --recursive
    cargo check --workspace

For a release build:

    cargo build --release

The release profile keeps debug information enabled because the runtime operates
at the native/ABI boundary and is substantially easier to diagnose with symbols.

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

## License

GPL-3.0-or-later.
