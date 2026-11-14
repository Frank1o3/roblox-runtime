# ADR-0001: host audio backends become one cpal implementation plus a libc OSS one

Date: 2026-11-14. Status: accepted.

## Context

The runtime's host audio layer is being rewritten in Rust to remove the C++
audio tree (`native/pipewire_backend.cpp`, `pulse_backend.cpp`,
`alsa_backend.cpp`, `oss_backend.cpp`, and their probes). The design decision
this ADR records was made before the rewrite and is now implemented:

- **PipeWire, PulseAudio and ALSA are one cpal-backed implementation**
  (`crates/audio/src/backend/cpal_output.rs`). cpal already wraps the three
  host sound servers, and maintaining three hand-written client-protocol
  implementations for one process's audio output was the dominant cost of the
  C++ layer.
- **OSS stays a direct libc implementation** (`backend/oss.rs`): OSS has no
  client library to wrap; it is `/dev/dsp`, ioctls and `write`.
- **Every backend hands the engine f32 frames.** Conversion to the device's
  native format happens on the way out (cpal's sample conversion), so the
  fill callback contract (`backend/output.rs::FillCallback`) is one shape for
  all backends.
- **Each cpal `Stream` is built, played and dropped on one owner thread.** The
  cpal `Stream` type is not `Send` in a useful way, and "drop the stream" is
  the operation that closes the device; making that happen on the thread that
  built it removes a whole class of teardown races and means `close` joining
  the owner thread is the only synchronization the teardown needs.
- **Availability probes prove a host constructs and has a default output
  device** (`backend/host.rs::probe`), rather than merely checking that a
  shared library can be `dlopen`-ed. The engine's `supportsAAudio()` is a
  one-way door — FMOD never falls back once told AAudio exists — so the probe
  has to prove a stream could actually open, not that a library is present.

## What is lost or weakened relative to the C++ backends

Recorded so nobody has to rediscover these from a behavior diff:

1. **Live default-sink following.** The C++ PipeWire/Pulse backends observed
   server events (or polled) so a *running* stream followed a change of the
   default sink. The cpal implementation binds one device at open time; a
   default-sink change applies the next time a stream opens. A device switch
   while playing keeps playing into the old (possibly now-silent) device until
   the stream is reopened.
2. **PipeWire `node.name` targeting.** The C++ PipeWire backend could steer a
   stream at a PipeWire node by `node.name`. cpal addresses devices by cpal
   `DeviceId`; `RBX_RUNTIME_AUDIO_SINK` matching is now exact-match on the
   cpal device id string, falling back to the default with a warning when the
   name is not found (see `backend/pcm.rs::choose_output_target`).
3. **`object.path`-based device types.** The C++ device enumeration keyed
   device *types* off PipeWire `object.path` values; the Rust enumeration
   (`backend/mod.rs::enumerate_devices`) classifies purely by direction and
   reports the cpal id as `node_name`.
4. **Single-binary-without-sound-libs.** The C++ layer dlopen'd
   `libpipewire-0.3.so.0`/`libpulse.so.0`/`libasound.so.2` at run time so the
   same binary ran on machines with none of them. cpal links (or feature-gates
   at build time) its host backends: a build without the `pipewire`/`pulseaudio`
   features cannot probe those hosts at all, rather than probing and refusing.
   The feature set (`pipewire`, `pulseaudio`, both default-on) is the new knob;
   ALSA comes in through cpal's default Linux host.

## Consequences

- Backend behavior parity tests are pure and device-free
  (`backend/pcm.rs`, `backend/capture.rs`, `backend/cpal_output.rs` unit
  tests). Live verification stays in examples/probes and the manual
  checklist.
- The backend selector lives in one place (`backend/host.rs`); the
  dlopen-based selector the C++ used is deleted rather than kept as a second
  opinion, because two selectors answering the same question differently is
  exactly the failure mode the old `RBX_RUNTIME_AUDIO_HOST` handling drifted
  into.
