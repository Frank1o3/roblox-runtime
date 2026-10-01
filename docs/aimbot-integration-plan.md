# Optional Aimbot Integration Plan

## Goal

Add the Rust detector from `crates/extra` to rusty-blox as an opt-in capability. The aimbot is compiled only when requested through `USE_AIMBOT=true ./dev.sh`, and remains inactive unless the sibling app config `aimbot.json` enables it. The detector receives internal rendered frames only if the Vulkan frame-tap can safely provide them; unsupported readback must leave normal Roblox startup unaffected.

## Implementation status

- Build opt-in, minimal JSON config model/loading, CPU frame validation/masking, contour selection, and Kalman-based movement suggestions are implemented.
- Vulkan swapchain eligibility and metadata-lifecycle groundwork is implemented: readiness remains false, metadata is recorded for successful create paths, and present snapshots are cleared on replacement/destruction.
- Eligibility means only that Vulkan transfer-source prerequisites appear satisfied; it is not pixel readback support. No GPU copy/staging backend exists, so the detector and aiming remain inactive.
- When Aimbot config is enabled, the app explicitly reports that internal pixel readback is not implemented. It does not start a detector, worker, or mouse-aiming path; normal input is unchanged.
- Validation run on 2026-10-01: `cargo test -p roblox-graphics-vulkan` passed (4 tests), `cargo test -p extra` passed (6 tests), and standalone runtime `cargo check` passed. App `cargo check` and `cargo check --features aimbot` both fail before reaching app code because the configured nested runtime has `FrameTapState.format: u32` assigned `vk::Format::as_raw(): i32`. The nested runtime was intentionally not edited.

## Tasks

### 1. Build and configuration boundary

- [x] Add an `aimbot` feature and optional `extra` dependency to rusty-blox.
- [x] Make `dev.sh` map `USE_AIMBOT=true` to Cargo's `--features aimbot`; keep default/false builds free of OpenCV and restore the existing JNI tracing defaults.
- [x] Add typed detector/runtime settings and store `aimbot.json` beside `settings.json` and `fast-flags.json`.
- [x] Keep only used detector, tracking, and aim parameters. Exclude window/capture source, portal/PipeWire settings, capture FPS/ROI, and unused `lead`; do not add triggerbot/autofire in the initial integration.
- [x] Document that changing the build-time opt-in requires a rebuild; runtime config is a separate enable gate.

### 2. CPU frame detector

- [x] Extend `crates/extra` to accept borrowed CPU frames with explicit dimensions, stride, and pixel format.
- [x] Validate buffer size/stride and supported formats; produce a color mask using configured color space/tolerance.
- [x] Extract and score contours, select the best valid candidate, and update the Kalman/tracking estimate.
- [x] Keep Vulkan and JNI out of the detector crate; return data-only detections.
- [x] Add tests for config defaults/round-trip, frame validation, masking, contour area/distance, tracking reset/loss, and Kalman state.

### 3. Vulkan eligibility and metadata groundwork

- [x] Keep operational internal frame-readback readiness false until an implementation can deliver pixels; format probing alone cannot enable it.
- [x] Expose typed eligibility status/reasons distinct from operational readback support.
- [x] Record effective swapchain metadata after every successful create path; eligibility requires format support, surface transfer-source support, effective `TRANSFER_SRC` image usage, and nonzero extent.
- [x] Track accepted success/suboptimal present indices and intercept swapchain destruction to remove metadata/clear stale snapshots.
- [ ] Implement GPU staging/readback, synchronization, layout transitions, non-coherent memory handling, and recreation/shutdown behavior.
- [ ] Keep any future readback non-blocking with a bounded staging ring and stale-frame dropping.
- [ ] Report future backend failures without failing Roblox startup; do not substitute desktop capture.

### 4. App and input orchestration

- [x] Load `aimbot.json` in rusty-blox and pass it only when the aimbot Cargo feature is compiled.
- [x] Report config-enabled/readback-unimplemented status without claiming detector availability.
- [ ] Start the detector only when both build feature and config allow it.
- [ ] Route only the latest detector result to the winit event loop; perform any mouse forwarding through the existing app/runtime input bridge on the appropriate thread.
- [ ] Use existing physical-key/event state rather than installing a second input hook.
- [ ] Ensure workers are stopped/joined during app shutdown and cannot call JNI or block rendering.

### 5. Validation

- [x] Run `cargo test -p roblox-graphics-vulkan` and `cargo test -p extra` in the standalone runtime (4 and 6 tests passed respectively on 2026-10-01).
- [x] Run standalone runtime `cargo check` (passed on 2026-10-01).
- [ ] User verifies default/false builds omit the feature and true builds include it, including rebuilding after toggling the env.
- [ ] User verifies config location, defaults, round-trip, and malformed-config handling.
- [x] Verify config-enabled app messaging is explicitly inactive; app feature-off/on checks were attempted on 2026-10-01 and both are blocked by the nested runtime format type mismatch described above.
- [ ] User runs detector tests and checks that config-disabled behavior leaves ordinary input unchanged.
- [ ] On supported Vulkan, verify readback ordering, synchronization/layout restoration, and no present-thread waits; profile GPU/CPU overhead.
- [ ] On unsupported transfer usage/formats, protected content, resize/recreation, minimized window, and Vulkan/EGL fallback, verify graceful aimbot unavailability and normal app startup.

## Constraints and notes

- `Cargo` resolves optional dependencies before executing `build.rs`; a build script cannot activate an optional dependency by reading an env var. The launcher script therefore maps the env var to a Cargo feature.
- The Rusty Blox app resolves its configured nested `roblox-runtime` checkout. App checks validate that dependency, not the standalone runtime edited for the groundwork in this phase. This phase intentionally leaves the nested checkout untouched; its current `FrameTapState.format` type mismatch blocks app compilation until separately corrected.
- There is no existing internal frame readback in the runtime. The proprietary engine currently owns command submission and swapchain use; a Vulkan tap is a high-risk, driver-dependent change.
- The Aimbot project's PipeWire/X11 capture is desktop/composited capture, not the internal Roblox render target, and is intentionally not treated as equivalent.
- A Vulkan transfer-source eligibility result is preliminary metadata only; it does not mean pixels can be captured. Aiming stays inactive until actual readback and app orchestration are implemented and validated.
