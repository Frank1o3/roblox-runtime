//! Open Sound System output, for machines with no sound daemon at all.
//!
//! `/dev/dsp` is a file: three ioctls to agree a format, then `write()`. The
//! blocking write is the clock, so the writer loop has no sleep. The device is
//! exclusive on most drivers, which is why this is only selected when
//! `RBX_RUNTIME_AUDIO_HOST=oss` names it. Linux and FreeBSD share these values.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;

use super::output::{FillCallback, OutputRequest, OutputStream};

const CHANNELS: u32 = 2;
const RATE: u32 = 48_000;
/// About 10 ms at 48 kHz.
const PERIOD: u32 = 480;

// _IOWR('P', n, int) on Linux and FreeBSD.
const SNDCTL_DSP_SPEED: u32 = 0xC004_5002;
const SNDCTL_DSP_SETFMT: u32 = 0xC004_5005;
const SNDCTL_DSP_CHANNELS: u32 = 0xC004_5006;
#[cfg(target_endian = "little")]
const AFMT_S16_NE: i32 = 0x10;
#[cfg(target_endian = "big")]
const AFMT_S16_NE: i32 = 0x20;

/// `RBX_RUNTIME_AUDIO_DEVICE`, or the conventional node.
fn device_path() -> String {
    match std::env::var("RBX_RUNTIME_AUDIO_DEVICE") {
        Ok(v) if !v.is_empty() => v,
        _ => "/dev/dsp".to_owned(),
    }
}

/// Clamp then scale. NaN becomes 0 via Rust's saturating float-to-int cast.
pub fn f32_to_s16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0) as i16
}

/// One OSS ioctl that reads and writes an int. OSS negotiates by writing the
/// granted value back through the argument.
fn dsp_ioctl(file: &File, request: u32, value: &mut i32) -> bool {
    // SAFETY: `file` keeps the fd open for the call and `value` is a valid,
    // exclusive pointer to one c_int, which is what every request above takes.
    unsafe { libc::ioctl(file.as_raw_fd(), request as _, value as *mut i32) >= 0 }
}

pub struct OssStream {
    device: String,
    thread: Option<JoinHandle<()>>,
    open: bool,
    quit: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    silence: Arc<AtomicU64>,
    rate: u32,
    channels: u32,
    period: u32,
}

impl OssStream {
    pub fn new() -> Self {
        Self::with_device(device_path())
    }

    pub fn with_device(device: String) -> Self {
        Self {
            device,
            thread: None,
            open: false,
            quit: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            silence: Arc::new(AtomicU64::new(0)),
            rate: RATE,
            channels: CHANNELS,
            period: PERIOD,
        }
    }
}

impl Default for OssStream {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputStream for OssStream {
    fn open(&mut self, request: OutputRequest, _desc: &str, mut fill: FillCallback) -> bool {
        self.close();
        let file = match OpenOptions::new().write(true).open(&self.device) {
            Ok(f) => f,
            Err(e) => {
                eprintln!(
                    "W/RobloxRuntime-OSS             cannot open {}: {e}",
                    self.device
                );
                return false;
            }
        };

        // Order matters: format, then channels, then rate. A driver may reset
        // the rate if the format is set after it.
        let mut fmt = AFMT_S16_NE;
        if !dsp_ioctl(&file, SNDCTL_DSP_SETFMT, &mut fmt) || fmt != AFMT_S16_NE {
            eprintln!(
                "W/RobloxRuntime-OSS             {} will not take 16-bit native-endian PCM.",
                self.device
            );
            return false;
        }
        let wanted_channels = request.channels.unwrap_or(CHANNELS);
        let wanted_rate = request.sample_rate.unwrap_or(RATE);
        let mut ch = wanted_channels as i32;
        if !dsp_ioctl(&file, SNDCTL_DSP_CHANNELS, &mut ch) || ch <= 0 {
            eprintln!(
                "W/RobloxRuntime-OSS             {} refused {wanted_channels} channels.",
                self.device
            );
            return false;
        }
        let mut rate = wanted_rate as i32;
        if !dsp_ioctl(&file, SNDCTL_DSP_SPEED, &mut rate) || rate <= 0 {
            eprintln!(
                "W/RobloxRuntime-OSS             {} refused {wanted_rate} Hz.",
                self.device
            );
            return false;
        }
        if request.channels.is_some_and(|wanted| ch as u32 != wanted)
            || request
                .sample_rate
                .is_some_and(|wanted| rate as u32 != wanted)
        {
            eprintln!(
                "W/RobloxRuntime-OSS             {} granted {rate} Hz/{ch} channels, not the requested output format.",
                self.device
            );
            return false;
        }

        // What the driver granted, not what was asked for.
        self.channels = ch as u32;
        self.rate = rate as u32;
        self.period = PERIOD;

        let samples = (self.period * self.channels) as usize;
        let period = self.period;
        self.quit.store(false, Ordering::Relaxed);
        self.running.store(false, Ordering::Relaxed);
        let (quit, running, silence) = (
            self.quit.clone(),
            self.running.clone(),
            self.silence.clone(),
        );
        let device = self.device.clone();

        eprintln!(
            "I/RobloxRuntime-OSS             {} at {} Hz, {} channels, {}-frame periods",
            self.device, self.rate, self.channels, self.period
        );

        let spawned = std::thread::Builder::new().name("roblox-oss-writer".into()).spawn(move || {
            let mut file = file; // owned here for the stream's whole life
            // Sized once; nothing below allocates.
            let mut buffer = vec![0f32; samples];
            let mut scratch = vec![0u8; samples * 2];
            while !quit.load(Ordering::Relaxed) {
                if running.load(Ordering::Relaxed) {
                    if !fill(&mut buffer, period) {
                        // Silence from here, not a teardown: this may be the
                        // engine's own callback asking.
                        running.store(false, Ordering::Relaxed);
                    }
                } else {
                    buffer.fill(0.0);
                    silence.fetch_add(1, Ordering::Relaxed);
                }
                for (i, src) in buffer.iter().enumerate() {
                    scratch[i * 2..i * 2 + 2].copy_from_slice(&f32_to_s16(*src).to_ne_bytes());
                }
                // write_all retries on EINTR. Any other failure means the
                // device is lost; retrying would spin.
                if let Err(e) = file.write_all(&scratch) {
                    eprintln!("E/RobloxRuntime-OSS             write to {device} failed: {e}; the stream is stopping.");
                    return;
                }
            }
        });
        match spawned {
            Ok(h) => {
                self.thread = Some(h);
                self.open = true;
                true
            }
            Err(_) => false,
        }
    }

    fn close(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
        self.running.store(false, Ordering::Relaxed);
        // Joining drops the thread's File, which closes the device.
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
        self.open = false;
    }

    fn is_open(&self) -> bool {
        self.open
    }
    fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::Relaxed);
    }
    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
    fn rate_hz(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> u32 {
        self.channels
    }
    /// What the engine is handed, not what the device takes.
    fn sample_bits(&self) -> u32 {
        32
    }
    fn sample_is_float(&self) -> bool {
        true
    }
    fn burst_frames(&self) -> u32 {
        self.period
    }
    fn silence_cycles(&self) -> u64 {
        self.silence.load(Ordering::Relaxed)
    }
}

impl Drop for OssStream {
    fn drop(&mut self) {
        self.close();
    }
}

/// Opening the device is the only proof it is there and free. Cached: the
/// device is usually exclusive, so the probe briefly takes sound from whatever
/// holds it, and `supportsAAudio()` is a one-way door.
pub fn oss_available() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| OpenOptions::new().write(true).open(device_path()).is_ok())
}

pub fn make_oss_stream() -> Box<dyn OutputStream> {
    Box::new(OssStream::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_clamps_and_scales() {
        assert_eq!(f32_to_s16(0.0), 0);
        assert_eq!(f32_to_s16(1.0), 32767);
        assert_eq!(f32_to_s16(-1.0), -32767);
        assert_eq!(f32_to_s16(5.0), 32767);
        assert_eq!(f32_to_s16(-5.0), -32767);
        assert_eq!(f32_to_s16(f32::NAN), 0);
    }

    #[test]
    fn a_missing_device_fails_to_open_and_leaves_nothing_behind() {
        let mut s = OssStream::with_device("/nonexistent/dsp".into());
        assert!(!s.open(
            OutputRequest::default_device(0, false, ""),
            "t",
            Box::new(|_, _| true)
        ));
        assert!(!s.is_open());
    }

    #[test]
    fn close_is_idempotent() {
        let mut s = OssStream::with_device("/nonexistent/dsp".into());
        s.close();
        s.close();
        assert!(!s.is_running());
    }
}
