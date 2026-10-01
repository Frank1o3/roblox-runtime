# Optional Aimbot Integration Plan

## Goal

Add the Rust detector from `crates/extra` to rusty-blox as an opt-in capability. The aimbot is compiled only when requested through `USE_AIMBOT=true ./dev.sh`, and remains inactive unless the sibling app config `aimbot.json` enables it. The detector receives internal rendered frames only if the Vulkan frame-tap can safely provide them; unsupported readback must leave normal Roblox startup unaffected.

## Tasks

### 1. Build and configuration boundary

- [ ] Add an `aimbot` feature and optional `extra` dependency to rusty-blox.
- [ ] Make `dev.sh` map `USE_AIMBOT=true` to Cargo's `--features aimbot`; keep default/false builds free of OpenCV and restore the existing JNI tracing defaults.
- [ ] Add typed detector/runtime settings and store `aimbot.json` beside `settings.json` and `fast-flags.json`.
- [ ] Keep only used detector, tracking, and aim parameters. Exclude window/capture source, portal/PipeWire settings, capture FPS/ROI, and unused `lead`; do not add triggerbot/autofire in the initial integration.
- [ ] Document that changing the build-time opt-in requires a rebuild; runtime config is a separate enable gate.

### 2. CPU frame detector

- [ ] Extend `crates/extra` to accept owned/borrowed CPU frames with explicit dimensions, stride, and pixel format.
- [ ] Validate buffer size/stride and supported formats; produce a color mask using configured color space/tolerance.
- [ ] Extract and score contours, select the best valid candidate, and update the Kalman/tracking estimate.
- [ ] Keep Vulkan and JNI out of the detector crate; return data-only detections.
- [ ] Add tests for config defaults/round-trip, frame validation, masking, contour area/distance, tracking reset/loss, and Kalman state.

### 3. Vulkan internal frame tap

- [ ] Add the tap behind the opt-in in `crates/graphics-vulkan`.
- [ ] Track per-device and per-swapchain image metadata/lifetimes and the image index passed to present.
- [ ] Check supported transfer-source usage and pixel formats before enabling capture.
- [ ] Use asynchronous GPU staging/readback with correct semaphore consumption, layout transitions, non-coherent memory invalidation, and recreation/shutdown handling.
- [ ] Keep the present hook non-blocking; use a small staging ring and drop stale frames under backpressure.
- [ ] If capture is unsupported or setup fails, report why and disable the aimbot without failing Roblox startup. Do not silently substitute desktop capture.

### 4. App and input orchestration

- [ ] Load `aimbot.json` in rusty-blox and pass it only when the aimbot Cargo feature is compiled.
- [ ] Start the detector only when both build feature and config allow it.
- [ ] Route only the latest detector result to the winit event loop; perform any mouse forwarding through the existing app/runtime input bridge on the appropriate thread.
- [ ] Use existing physical-key/event state rather than installing a second input hook.
- [ ] Ensure workers are stopped/joined during app shutdown and cannot call JNI or block rendering.

### 5. Validation

- [ ] User verifies default/false builds omit the feature and true builds include it, including rebuilding after toggling the env.
- [ ] User verifies config location, defaults, round-trip, and malformed-config handling.
- [ ] User runs detector tests and checks that config-disabled behavior leaves ordinary input unchanged.
- [ ] On supported Vulkan, verify readback ordering, synchronization/layout restoration, and no present-thread waits; profile GPU/CPU overhead.
- [ ] On unsupported transfer usage/formats, protected content, resize/recreation, minimized window, and Vulkan/EGL fallback, verify graceful aimbot unavailability and normal app startup.

## Constraints and notes

- `Cargo` resolves optional dependencies before executing `build.rs`; a build script cannot activate an optional dependency by reading an env var. The launcher script therefore maps the env var to a Cargo feature.
- The Rusty Blox app uses its nested `roblox-runtime` Git submodule. Runtime changes must be committed there and the parent gitlink advanced for the app to consume them.
- There is no existing internal frame readback in the runtime. The proprietary engine currently owns command submission and swapchain use; a Vulkan tap is a high-risk, driver-dependent change.
- The Aimbot project's PipeWire/X11 capture is desktop/composited capture, not the internal Roblox render target, and is intentionally not treated as equivalent.
- Functional tests are left for the user to run; compile checks may be used during implementation.
