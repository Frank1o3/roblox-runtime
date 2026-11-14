//! Injectable seams between the AAudio surface and the host backends.
//!
//! The AAudio layer is pure policy: it never talks to a device itself. The
//! default factories hand it the real backend — [`RealCaptureBackend`]
//! funnels through `CaptureStream::open`, which is the process's single
//! microphone chokepoint — and tests install fakes here to exercise the same
//! policy code the engine will run.
//!
//! The capture seam is deliberately **byte-level**: an AAudio input stream
//! reports a declared 48 kHz mono s16 format (it cannot negotiate, because
//! measuring a granted format would mean opening a microphone on a stream
//! that has only been *opened*), so what crosses this seam is exactly the
//! byte stream that declaration promises. The real adapter converts from the
//! capture ring's f32; a fake can produce bytes directly.

use std::sync::{Arc, OnceLock};

use crate::backend::capture::{CaptureOpenError, CaptureRequest, CaptureStream};

/// An open capture source as the AAudio layer sees it: the declared
/// 48 kHz-mono-s16 byte stream. Every method is safe to call from an ordinary
/// thread; `read` is non-blocking and returns whatever is available.
pub trait Capture: Send {
    /// Pull raw little-endian s16 mono bytes into `dst`; returns the byte
    /// count (may be 0 — never blocks).
    fn read(&mut self, dst: &mut [u8]) -> usize;
    /// The capture device is currently held. False means nothing is open and
    /// no more data will ever come.
    fn is_open(&self) -> bool;
    /// The host backend gave up on this device (disconnected or dead).
    fn failed(&self) -> bool;
    /// Samples lost to a full ring, in bytes — the honest input xrun source.
    fn dropped_bytes(&self) -> u64;
}

/// Opens capture sources. Refusing an open here is a policy answer (another
/// capture is live), never a device error to retry around.
pub trait CaptureBackend: Send + Sync {
    fn open(&self) -> Result<Box<dyn Capture>, CaptureOpenError>;
}

/// The declared input format the AAudio surface promises the engine.
/// Declared, not negotiated: the engine reads rate/channels/format off a
/// stream that has been opened but not started, and no microphone may be
/// opened in that state. The real capture is asked for exactly these.
pub const DECLARED_CAPTURE_RATE: u32 = 48_000;
pub const DECLARED_CAPTURE_CHANNELS: u32 = 1;
pub const DECLARED_BYTES_PER_FRAME: u32 = DECLARED_CAPTURE_CHANNELS * 2;
/// 10 ms — the smallest burst that stays honest without a device-reported
/// quantum (which would itself require an open stream).
pub const DECLARED_BURST_FRAMES: u32 = DECLARED_CAPTURE_RATE / 100;
/// How far behind a reader may fall before it loses samples: the real ring's
/// size, in frames.
pub const DECLARED_RING_FRAMES: u32 = 1 << 15;

/// The real capture, over the runtime's `CaptureStream`.
///
/// The ring delivers interleaved f32 at whatever the device granted; this
/// adapter converts to the declared mono s16 byte stream. If the device
/// granted more than one channel the channels are averaged (a plain mono
/// downmix); a granted rate other than 48 kHz is logged at open as a warning,
/// because nothing here resamples and pretending otherwise would pitch-shift
/// every recording.
struct RealCapture {
    stream: CaptureStream,
    channels: u16,
}

impl Capture for RealCapture {
    fn read(&mut self, dst: &mut [u8]) -> usize {
        if !self.stream.is_open() {
            return 0;
        }
        let want_samples = dst.len() / 2;
        let mut scratch = [0f32; 512];
        let mut written = 0usize;
        while written < want_samples {
            // Whole frames only, so a chunk boundary never splits a frame.
            let chunk = scratch.len() / usize::from(self.channels) * usize::from(self.channels);
            let got = self.stream.read(&mut scratch[..chunk]);
            if got == 0 {
                break;
            }
            let frames = got / usize::from(self.channels);
            for frame in 0..frames {
                let base = frame * usize::from(self.channels);
                let mix = scratch[base..base + usize::from(self.channels)]
                    .iter()
                    .sum::<f32>()
                    / self.channels as f32;
                let value = (mix * 32767.0).clamp(-32768.0, 32767.0) as i16;
                dst[written * 2..written * 2 + 2].copy_from_slice(&value.to_le_bytes());
                written += 1;
            }
            if got < chunk {
                break; // the device has caught up
            }
        }
        written * 2
    }

    fn is_open(&self) -> bool {
        self.stream.is_open()
    }

    fn failed(&self) -> bool {
        self.stream.is_disconnected()
    }

    fn dropped_bytes(&self) -> u64 {
        // Ring overruns are counted in samples; frames lost is samples /
        // channels, and the declared byte stream is 2 bytes per frame.
        self.stream.overruns() / u64::from(self.channels) * 2
    }
}

struct RealCaptureBackend;

impl CaptureBackend for RealCaptureBackend {
    fn open(&self) -> Result<Box<dyn Capture>, CaptureOpenError> {
        let stream = CaptureStream::open(
            "",
            CaptureRequest {
                sample_rate: Some(DECLARED_CAPTURE_RATE),
                channels: Some(DECLARED_CAPTURE_CHANNELS as u16),
            },
        )?;
        let (rate, channels) = (stream.rate_hz(), stream.channels());
        if rate != DECLARED_CAPTURE_RATE {
            // Not silently wrong: a mismatch pitches every recording. Said
            // once, at open, where a reader is looking for exactly this.
            crate::logline::log(
                crate::logline::Priority::Warn,
                crate::logline::AUDIO_TAG,
                &format!(
                    "capture granted {rate} Hz instead of the declared {DECLARED_CAPTURE_RATE}; \
                     recording will run at the wrong speed until the host offers 48 kHz"
                ),
            );
        }
        Ok(Box::new(RealCapture { stream, channels }))
    }
}

/// Opens output streams. Real implementation: the host backend selector.
pub trait OutputBackend: Send + Sync {
    fn make_output(&self) -> Box<dyn crate::backend::output::OutputStream>;
}

struct RealOutputBackend;

impl OutputBackend for RealOutputBackend {
    fn make_output(&self) -> Box<dyn crate::backend::output::OutputStream> {
        crate::backend::host::make_output_stream()
    }
}

static CAPTURE_BACKEND: OnceLock<Arc<dyn CaptureBackend>> = OnceLock::new();
static OUTPUT_BACKEND: OnceLock<Arc<dyn OutputBackend>> = OnceLock::new();

pub(crate) fn capture_backend() -> Arc<dyn CaptureBackend> {
    CAPTURE_BACKEND
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(RealCaptureBackend))
}

pub(crate) fn output_backend() -> Arc<dyn OutputBackend> {
    OUTPUT_BACKEND
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(RealOutputBackend))
}

/// Install a capture factory (tests and future selection switches only; the
/// default is the real, chokepoint-carrying backend). Returns false if one
/// is already installed.
pub fn set_capture_backend(backend: Arc<dyn CaptureBackend>) -> bool {
    CAPTURE_BACKEND.set(backend).is_ok()
}

pub fn set_output_backend(backend: Arc<dyn OutputBackend>) -> bool {
    OUTPUT_BACKEND.set(backend).is_ok()
}
