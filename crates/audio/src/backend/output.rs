//! The seam every host output backend implements. Mirrors the C++
//! `OutputStream` (ADR-023), with the callback as a closure.

/// Fills `dst` (interleaved f32, `frames * channels` samples) with audio.
/// Return `false` to ask that the stream stop being pulled
/// (AAUDIO_CALLBACK_RESULT_STOP).
///
/// Realtime rule, inherited from the C++: everything reachable from this call
/// must not lock, allocate, free, log or make a syscall.
pub type FillCallback = Box<dyn FnMut(&mut [f32], u32) -> bool + Send>;

/// Requested host output configuration. `None` for rate or channels keeps the
/// backend's current default-device choice. A backend may reject a requested
/// configuration it cannot provide; it must not silently claim the requested
/// values when it opened a different stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRequest {
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub sample_bits: u32,
    pub sample_is_float: bool,
    pub target: String,
}

impl OutputRequest {
    pub fn default_device(sample_bits: u32, sample_is_float: bool, target: &str) -> Self {
        Self {
            sample_rate: None,
            channels: None,
            sample_bits,
            sample_is_float,
            target: target.to_owned(),
        }
    }
}

pub trait OutputStream: Send {
    /// `sample_bits`/`is_float` are the engine's format preference (0 = none).
    /// Every backend here hands the callback f32 frames, so they report
    /// 32-bit float. `target` is a device name, empty for the default.
    fn open(&mut self, request: OutputRequest, description: &str, fill: FillCallback) -> bool;
    fn close(&mut self);
    fn is_open(&self) -> bool;
    /// Must be safe from inside the fill callback: writes silence, never tears down.
    fn set_running(&self, running: bool);
    fn is_running(&self) -> bool;
    /// Rate, channels and burst are what the device granted, never what was asked for.
    fn rate_hz(&self) -> u32;
    fn channels(&self) -> u32;
    fn sample_bits(&self) -> u32;
    fn sample_is_float(&self) -> bool;
    fn burst_frames(&self) -> u32;
    fn silence_cycles(&self) -> u64;
}
