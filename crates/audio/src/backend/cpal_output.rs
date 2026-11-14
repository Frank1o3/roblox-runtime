//! The cpal-backed output stream: one implementation for PipeWire, PulseAudio
//! and ALSA, replacing `CallbackStream`, `PulseStream` and `AlsaStream`.
//!
//! The cpal `Stream` is built, played and dropped on one owner thread, so it
//! never needs to be `Send` and dropping it is what actually closes the device.
//! Rate and channels are the config the device was opened with, and burst
//! frames are measured from the first callback, never remembered from a request.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, ErrorKind, FromSample, SizedSample, Stream, StreamConfig};

use super::host::host_by_name;
use super::output::{FillCallback, OutputStream};

/// Everything the audio callback touches. No locks except one `try_lock` on a
/// mutex nobody else ever holds, which cannot block or deadlock.
#[derive(Clone)]
struct Ctx {
    fill: Arc<Mutex<FillCallback>>,
    running: Arc<AtomicBool>,
    silence: Arc<AtomicU64>,
    burst: Arc<AtomicU32>,
    channels: usize,
}

/// Realtime: no allocation, no logging, no blocking.
fn render(ctx: &Ctx, buf: &mut [f32]) {
    let frames = (buf.len() / ctx.channels.max(1)) as u32;
    ctx.burst.store(frames, Ordering::Relaxed);
    if ctx.running.load(Ordering::Relaxed) {
        let keep = match ctx.fill.try_lock() {
            Ok(mut fill) => fill(buf, frames),
            Err(_) => {
                buf.fill(0.0);
                true
            }
        };
        if !keep {
            // The engine asked to stop being pulled: silence, not a teardown.
            ctx.running.store(false, Ordering::Relaxed);
            buf.fill(0.0);
        }
    } else {
        buf.fill(0.0);
        ctx.silence.fetch_add(1, Ordering::Relaxed);
    }
}

fn on_stream_error(err: cpal::Error) {
    match err.kind() {
        ErrorKind::Xrun => {} // counted by the server; logging here would flood
        ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied => {
            eprintln!("W/RobloxRuntime-Audio           stream: {err}");
        }
        _ => eprintln!("E/RobloxRuntime-Audio           stream error: {err}"),
    }
}

fn build_f32(device: &cpal::Device, cfg: &StreamConfig, ctx: Ctx) -> Result<Stream, String> {
    device
        .build_output_stream(
            cfg,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| render(&ctx, data),
            on_stream_error,
            None,
        )
        .map_err(|e| e.to_string())
}

/// For devices that will not take f32: render f32, convert on the way out.
fn build_conv<T>(device: &cpal::Device, cfg: &StreamConfig, ctx: Ctx) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    // Reserved up front so `resize` below stays within capacity.
    let mut scratch: Vec<f32> = Vec::with_capacity(16_384);
    device
        .build_output_stream(
            cfg,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                scratch.resize(data.len(), 0.0);
                render(&ctx, &mut scratch);
                for (out, s) in data.iter_mut().zip(scratch.iter()) {
                    *out = T::from_sample(*s);
                }
            },
            on_stream_error,
            None,
        )
        .map_err(|e| e.to_string())
}

type Builder = fn(&cpal::Device, &StreamConfig, Ctx) -> Result<Stream, String>;

struct Granted {
    rate: u32,
    channels: u32,
}

fn pick_device(host: &cpal::Host, target: &str) -> Option<cpal::Device> {
    if !target.is_empty() {
        if let Some(d) = target.parse().ok().and_then(|id| host.device_by_id(&id)) {
            return Some(d);
        }
        eprintln!(
            "W/RobloxRuntime-Audio           output device '{target}' is not on this host; \
             falling back to the default so audio still plays somewhere."
        );
    }
    host.default_output_device()
}

fn build_stream(host_name: &str, target: &str, base: &Ctx) -> Result<(Stream, Granted), String> {
    let host = host_by_name(host_name).ok_or_else(|| format!("host '{host_name}' is unavailable"))?;
    let device = pick_device(&host, target).ok_or("no output device")?;
    let default = device.default_output_config().map_err(|e| e.to_string())?;
    let mut cfg: StreamConfig = default.into();
    let ctx = Ctx { channels: cfg.channels as usize, ..base.clone() };

    // Prefer a fixed 1024-frame buffer (ALSA's default can be absurd), then
    // the device default; prefer f32, then the integer formats.
    let formats: [(&str, Builder); 4] = [
        ("f32", build_f32),
        ("i16", build_conv::<i16>),
        ("i32", build_conv::<i32>),
        ("u16", build_conv::<u16>),
    ];
    let mut last_err = String::from("no format accepted");
    for fixed in [true, false] {
        cfg.buffer_size = if fixed { BufferSize::Fixed(1024) } else { BufferSize::Default };
        for (name, build) in formats {
            match build(&device, &cfg, ctx.clone()) {
                Ok(stream) => {
                    stream.play().map_err(|e| e.to_string())?;
                    // Wait for the first cycle so burst is measured.
                    let deadline = Instant::now() + Duration::from_secs(3);
                    while ctx.burst.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    if ctx.burst.load(Ordering::Relaxed) == 0 {
                        return Err("the host did not turn a cycle within 3s".into());
                    }
                    let granted = Granted { rate: cfg.sample_rate, channels: cfg.channels as u32 };
                    eprintln!(
                        "I/RobloxRuntime-Audio           {host_name}: {} Hz, {} channel(s), {name}, \
                         {} frames per burst (rate and channels are what the device granted).",
                        granted.rate, granted.channels, ctx.burst.load(Ordering::Relaxed)
                    );
                    return Ok((stream, granted));
                }
                Err(e) => last_err = format!("{name}/{}: {e}", if fixed { "fixed" } else { "default" }),
            }
        }
    }
    Err(last_err)
}

pub struct CpalOutputStream {
    host: String,
    running: Arc<AtomicBool>,
    silence: Arc<AtomicU64>,
    burst: Arc<AtomicU32>,
    thread: Option<JoinHandle<()>>,
    stop: Option<mpsc::Sender<()>>,
    rate: u32,
    channels: u32,
}

impl CpalOutputStream {
    pub fn new(host: &str) -> Self {
        Self {
            host: host.to_owned(),
            running: Arc::new(AtomicBool::new(false)),
            silence: Arc::new(AtomicU64::new(0)),
            burst: Arc::new(AtomicU32::new(0)),
            thread: None,
            stop: None,
            rate: 0,
            channels: 0,
        }
    }
}

impl OutputStream for CpalOutputStream {
    fn open(&mut self, _bits: u32, _float: bool, _desc: &str, target: &str, fill: FillCallback) -> bool {
        self.close();
        self.burst.store(0, Ordering::Relaxed);
        self.running.store(false, Ordering::Relaxed);
        let base = Ctx {
            fill: Arc::new(Mutex::new(fill)),
            running: self.running.clone(),
            silence: self.silence.clone(),
            burst: self.burst.clone(),
            channels: 0,
        };
        let (ready_tx, ready_rx) = mpsc::channel::<Result<Granted, String>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (host, target) = (self.host.clone(), target.to_owned());

        let spawned = std::thread::Builder::new().name("roblox-cpal-output".into()).spawn(move || {
            match build_stream(&host, &target, &base) {
                Ok((stream, granted)) => {
                    let _ = ready_tx.send(Ok(granted));
                    // Parked until `close` drops the sender; then the stream drops here.
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        let Ok(handle) = spawned else { return false };

        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(g)) => {
                self.rate = g.rate;
                self.channels = g.channels;
                self.thread = Some(handle);
                self.stop = Some(stop_tx);
                true
            }
            Ok(Err(e)) => {
                eprintln!("E/RobloxRuntime-Audio           {} output failed to open: {e}", self.host);
                let _ = handle.join();
                false
            }
            // Detached: dropping `stop_tx` lets the owner thread tear itself down.
            Err(_) => false,
        }
    }

    fn close(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        self.stop.take(); // dropping the sender wakes the owner thread
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }

    fn is_open(&self) -> bool {
        self.thread.is_some()
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
    /// What the engine is handed, whatever the device takes.
    fn sample_bits(&self) -> u32 {
        32
    }
    fn sample_is_float(&self) -> bool {
        true
    }
    fn burst_frames(&self) -> u32 {
        self.burst.load(Ordering::Relaxed)
    }
    fn silence_cycles(&self) -> u64 {
        self.silence.load(Ordering::Relaxed)
    }
}

impl Drop for CpalOutputStream {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(fill: FillCallback, channels: usize) -> Ctx {
        Ctx {
            fill: Arc::new(Mutex::new(fill)),
            running: Arc::new(AtomicBool::new(false)),
            silence: Arc::new(AtomicU64::new(0)),
            burst: Arc::new(AtomicU32::new(0)),
            channels,
        }
    }

    #[test]
    fn not_running_writes_silence_not_stale_bytes_and_counts_it() {
        let c = ctx(Box::new(|_, _| true), 2);
        let mut buf = [0.7f32; 8];
        render(&c, &mut buf);
        assert!(buf.iter().all(|&s| s == 0.0));
        assert_eq!(c.silence.load(Ordering::Relaxed), 1);
        assert_eq!(c.burst.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn running_pulls_the_engine_with_the_measured_frame_count() {
        let c = ctx(Box::new(|dst, frames| {
            assert_eq!(frames as usize * 2, dst.len());
            dst.fill(0.5);
            true
        }), 2);
        c.running.store(true, Ordering::Relaxed);
        let mut buf = [0.0f32; 8];
        render(&c, &mut buf);
        assert!(buf.iter().all(|&s| s == 0.5));
        assert_eq!(c.silence.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_stop_request_silences_and_stops_pulling() {
        let c = ctx(Box::new(|dst, _| {
            dst.fill(0.9);
            false
        }), 1);
        c.running.store(true, Ordering::Relaxed);
        let mut buf = [0.0f32; 4];
        render(&c, &mut buf);
        assert!(buf.iter().all(|&s| s == 0.0));
        assert!(!c.running.load(Ordering::Relaxed));
    }

    #[test]
    fn closing_a_never_opened_stream_is_harmless() {
        let mut s = CpalOutputStream::new("alsa");
        assert!(!s.is_open());
        s.close();
        s.close();
    }
}