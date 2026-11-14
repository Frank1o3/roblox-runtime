//! Injectable seams between the OpenSL ES surface and the host backends.
//!
//! The OpenSL layer is pure policy: it never talks to a device itself. The
//! default factories hand it the real backends — the capture opener funnels
//! through `CaptureStream::open`, the process's single microphone chokepoint —
//! and tests install fakes here to exercise the same policy code the engine
//! will run.
//!
//! The capture seam is **byte-level**, s16 native-endian interleaved at the
//! rate and channel count the *caller* asked for. The real adapter converts
//! from the capture ring's f32 and adjusts channel counts (averaging down,
//! duplicating up); a granted rate other than the requested one is logged as a
//! warning at open, because nothing here resamples and pretending otherwise
//! would pitch-shift every recording.

use std::sync::{Arc, OnceLock};

use crate::backend::capture::{CaptureOpenError, CaptureRequest, CaptureStream};
use crate::backend::output::OutputStream;

/// An open capture source as the OpenSL layer sees it: s16 native-endian
/// interleaved bytes at the rate and channels that were requested. `read` is
/// non-blocking and returns whatever is available.
pub trait OpenSlCapture: Send {
    fn read(&mut self, dst: &mut [u8]) -> usize;
    /// The capture device is currently held. False means nothing is open and
    /// no more data will ever come.
    fn is_open(&self) -> bool;
}

/// Opens capture sources at a caller-chosen rate and channel count. Refusing
/// an open here is a policy answer (another capture is live), never a device
/// error to retry around.
pub trait CaptureOpener: Send + Sync {
    fn open(&self, rate_hz: u32, channels: u16) -> Result<Box<dyn OpenSlCapture>, CaptureOpenError>;
}

/// Opens output streams. Real implementation: the host backend selector.
pub trait OutputFactory: Send + Sync {
    fn make_output(&self) -> Box<dyn OutputStream>;
}

/// The real capture, over the runtime's `CaptureStream`.
struct RealCapture {
    stream: CaptureStream,
    granted_channels: u16,
    requested_channels: u16,
}

impl OpenSlCapture for RealCapture {
    fn read(&mut self, dst: &mut [u8]) -> usize {
        if !self.stream.is_open() {
            return 0;
        }
        let want_samples = dst.len() / 2;
        let mut scratch = [0f32; 512];
        let mut written = 0usize;
        while written < want_samples {
            // Whole frames only, so a chunk boundary never splits a frame.
            let chunk = scratch.len() / usize::from(self.granted_channels)
                * usize::from(self.granted_channels);
            let got = self.stream.read(&mut scratch[..chunk]);
            if got == 0 {
                break;
            }
            let frames = got / usize::from(self.granted_channels);
            for frame in 0..frames {
                let base = frame * usize::from(self.granted_channels);
                let value = if self.granted_channels == self.requested_channels {
                    scratch[base]
                } else if self.granted_channels > self.requested_channels {
                    // Plain mono downmix per output channel group.
                    let group = usize::from(self.granted_channels)
                        / usize::from(self.requested_channels);
                    scratch[base..base + group].iter().sum::<f32>() / group as f32
                } else {
                    // Fewer granted channels than asked: repeat, rather than
                    // hand back a half-filled frame the caller reads as noise.
                    scratch[base..base + usize::from(self.granted_channels)]
                        .last()
                        .copied()
                        .unwrap_or(0.0)
                };
                let sample = (value * 32767.0).clamp(-32768.0, 32767.0) as i16;
                dst[written * 2..written * 2 + 2].copy_from_slice(&sample.to_le_bytes());
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
}

struct RealCaptureOpener;

impl CaptureOpener for RealCaptureOpener {
    fn open(&self, rate_hz: u32, channels: u16) -> Result<Box<dyn OpenSlCapture>, CaptureOpenError> {
        let stream = CaptureStream::open(
            "",
            CaptureRequest {
                sample_rate: Some(rate_hz),
                channels: Some(channels),
            },
        )?;
        if stream.rate_hz() != rate_hz {
            // Not silently wrong: a mismatch pitches every recording. Said
            // once, at open, where a reader is looking for exactly this.
            crate::logline::log(
                crate::logline::Priority::Warn,
                crate::logline::OPENSL_TAG,
                &format!(
                    "capture granted {} Hz instead of the requested {rate_hz}; recording will \
                     run at the wrong speed until the host offers it",
                    stream.rate_hz()
                ),
            );
        }
        Ok(Box::new(RealCapture {
            granted_channels: stream.channels(),
            requested_channels: channels,
            stream,
        }))
    }
}

struct RealOutputFactory;

impl OutputFactory for RealOutputFactory {
    fn make_output(&self) -> Box<dyn OutputStream> {
        crate::backend::host::make_output_stream()
    }
}

static CAPTURE_OPENER: OnceLock<Arc<dyn CaptureOpener>> = OnceLock::new();
static OUTPUT_FACTORY: OnceLock<Arc<dyn OutputFactory>> = OnceLock::new();

pub(crate) fn capture_opener() -> Arc<dyn CaptureOpener> {
    CAPTURE_OPENER
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(RealCaptureOpener))
}

pub(crate) fn output_factory() -> Arc<dyn OutputFactory> {
    OUTPUT_FACTORY
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(RealOutputFactory))
}

/// Install a capture factory (tests and future selection switches only; the
/// default is the real, chokepoint-carrying backend). Returns false if one
/// is already installed.
pub fn set_capture_opener(opener: Arc<dyn CaptureOpener>) -> bool {
    CAPTURE_OPENER.set(opener).is_ok()
}

pub fn set_output_factory(factory: Arc<dyn OutputFactory>) -> bool {
    OUTPUT_FACTORY.set(factory).is_ok()
}
