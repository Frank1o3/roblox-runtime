# roblox-runtime

Standalone runtime for executing the Roblox Android client on Linux. This
repository owns Android compatibility and native loading; the `rusty-blox`
client supplies the base/split APKs, extracted libraries, data/cache
directories, settings, Fast Flags and host rendering surface.

The workspace is being extracted from the local `rbx-native-runtime` reference
implementation. See [the runtime audit](docs/runtime-audit.md) for source
boundaries and current migration status.

The runtime resolves engine imports against implemented ABI symbols, selected
host libraries and generated fallback stubs, then maps `libroblox.so` with its
constructors deferred. It does not yet run those constructors or call Roblox's
GameActivity bootstrap automatically. `LoadedEngine` exposes the ordered JNI
steps (`JNI_OnLoad`, then `initializeNativeCode`) once constructors have run.
The resolver uses host glibc for observed constructor-time libc calls whose
ABI matches bionic and keeps structure-sensitive APIs on runtime-owned
wrappers. With the explicitly ABI-unsafe diagnostic `host_libc` option, the
local APK run passed constructors, `JNI_OnLoad`, GameActivity initialisation
and flag setup, then segfaulted on an engine thread. Normal ABI mode segfaults
during `NativeSettingsInterface.nativeSetFilesDirectory`, the first
pre-constructor setter. Neither run establishes a playable client. The runtime
has EGL support for client-owned X11/Wayland surfaces and a Vulkan interposer in
`roblox-graphics-vulkan`. The Vulkan crate uses Ash to load the host
`libvulkan`, reports the host WSI extension to Roblox as Android surface
support, and translates Android surface creation to the supplied Xlib or
Wayland surface. Roblox's own renderer then submits to the host Vulkan driver
and hardware. Automatic selects Vulkan when the loader and matching WSI
extension are available, otherwise GLES3; explicit Vulkan requests fail if
that support is unavailable. This path compiles independently but has not yet
been observed rendering a Roblox frame. `RBX_RUNTIME_PRESENT_MODE` accepts
`auto`, `mailbox`, `uncapped`, `immediate`, `fifo`, `fifo-relaxed`, or `off`.
After startup, `LoadedEngine::resize_surface` updates the Android window
dimensions and delivers both app-bridge surface updates plus GameActivity's
surface-changed callback.

Build with a recent stable Rust toolchain, Clang/Clang++, CMake, GNU `patch`,
and initialized Git submodules:

```sh
git submodule update --init --recursive
cargo check --workspace
```
