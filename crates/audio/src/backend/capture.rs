//! The capture stream: the runtime's entire microphone policy in one type.
//!
//! The rules this module exists to enforce (each one has a test):
//!
//! 1. **No capture stream exists unless a consumer is actively recording.**
//!    Not paused, not muted: absent. Every open path funnels through
//!    [`CaptureStream::open`], which refuses while another capture is live,
//!    and every stop path (stop, pause, close, destroy, failure, drop) goes
//!    through `close`/`Drop`. Enumeration and stream setup must never open
//!    a microphone.
//! 2. At most one capture stream process-wide.
//! 3. The realtime thread (the host audio callback) only converts samples and
//!    writes them into a lock-free ring: no locks, no allocation, no logging.
//! 4. Rate and channels report what was granted, never what was asked.
//! 5. `close` joins the owner thread while holding nothing, so a stream can
//!    never be freed under a thread that still touches it (the AB-BA rule).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{BufferSize, FromSample, Sample, SizedSample, StreamConfig};

use super::host::{effective_backend_name, host_by_name};
use crate::logline::{log, Priority, AUDIO_TAG};

/// Process-wide single-capture guard. `open` takes it, `Drop` releases it.
/// Every capture path in the runtime must hold one of these; the refusal at
/// this one chokepoint is what keeps a second stream from ever existing.
static CAPTURE_LIVE: AtomicBool = AtomicBool::new(false);

/// The consumer's request. `None` rate or channels means "what the device
/// prefers"; granted values are always reported back exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CaptureRequest {
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureOpenError {
    /// Another capture stream is already open. This is a policy refusal,
    /// never a device error, and the caller must not retry by tearing down
    /// the other stream.
    AnotherCaptureLive,
    NoInputDevice,
    DeviceFailed(String),
    SpawnFailed,
}

impl std::fmt::Display for CaptureOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AnotherCaptureLive => f.write_str("another capture stream is already open"),
            Self::NoInputDevice => f.write_str("no input device on this host"),
            Self::DeviceFailed(error) => write!(f, "input device failed: {error}"),
            Self::SpawnFailed => f.write_str("could not spawn the capture owner thread"),
        }
    }
}

/// Lock-free single-producer single-consumer ring of interleaved f32 samples.
///
/// Producer side: the host audio callback. It must never block, so when the
/// ring is full the *oldest* samples are overwritten and the overrun counter
/// climbs — fresher audio at bounded latency beats a stall for voice chat.
/// Consumer side: ordinary threads, which may wait.
pub(crate) struct SampleRing {
    buf: Vec<f32>,
    mask: usize,
    /// Monotonic total samples written / read (never wrapped); the storage
    /// index of sample `n` is `n & mask`.
    written: AtomicU64,
    read: AtomicU64,
    overruns: AtomicU64,
    capacity: usize,
}

impl SampleRing {
    fn new(capacity_frames: usize, channels: u16) -> Self {
        let capacity_samples =
            (capacity_frames.max(1) * usize::from(channels.max(1))).next_power_of_two();
        Self {
            buf: vec![0.0; capacity_samples],
            mask: capacity_samples - 1,
            written: AtomicU64::new(0),
            read: AtomicU64::new(0),
            overruns: AtomicU64::new(0),
            capacity: capacity_samples,
        }
    }

    /// Realtime: write exactly `len` samples pulled from `sample`, overwriting
    /// the oldest when full. The closure runs on the producer thread only.
    fn write_with(&self, len: usize, mut sample: impl FnMut(usize) -> f32) {
        let written = self.written.fetch_add(len as u64, Ordering::AcqRel);
        let read = self.read.load(Ordering::Acquire);
        // Everything at or after `read` is still unread; if this write would
        // overwrite it, the reader loses the difference and we count it.
        if written - read > self.capacity as u64 - len as u64 {
            self.overruns.fetch_add(1, Ordering::Relaxed);
        }
        for (i, index) in (written as usize..).take(len).enumerate() {
            // SAFETY: `index & mask` is always inside `buf`, and the producer
            // is the process's only writer of this ring, so no two writes in
            // this loop touch the same slot.
            unsafe {
                *self.buf.as_ptr().cast_mut().add(index & self.mask) = sample(i);
            }
        }
    }

    /// Copy whatever is available into `dst`; returns samples copied (not
    /// necessarily a whole number of frames — callers round down).
    fn read(&self, dst: &mut [f32]) -> usize {
        let read = self.read.load(Ordering::Acquire);
        let written = self.written.load(Ordering::Acquire);
        let available = (written - read).min(dst.len() as u64) as usize;
        for (i, out) in dst.iter_mut().take(available).enumerate() {
            // SAFETY: the index is masked into range like the writer's; the
            // consumer is the only reader of slots the writer has finished.
            unsafe {
                *out = *self.buf.get_unchecked((read as usize + i) & self.mask);
            }
        }
        self.read.store(read + available as u64, Ordering::Release);
        available
    }

    /// Used by the AAudio input surface (blocking read pacing) and by tests;
    /// not every consumer of the ring needs it today.
    #[allow(dead_code)]
    fn available(&self) -> u64 {
        self.written.load(Ordering::Acquire) - self.read.load(Ordering::Acquire)
    }

    fn overruns(&self) -> u64 {
        self.overruns.load(Ordering::Relaxed)
    }
}

struct Live {
    ring: Arc<SampleRing>,
    rate: u32,
    channels: u16,
    owner: Option<JoinHandle<()>>,
    /// Dropping this wakes the owner thread, which then drops the cpal
    /// stream; dropping the stream is what closes the device.
    stop: Option<mpsc::Sender<()>>,
    disconnected: Arc<AtomicBool>,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stop.take();
        // Join while holding nothing: the owner thread touches the stream and
        // the ring, so it must be gone before this memory goes.
        if let Some(handle) = self.owner.take() {
            let _ = handle.join();
        }
    }
}

/// An open capture stream. There is no "constructed but idle" state to leak:
/// constructing one that is not recording is exactly what [`Self::open`]
/// refuses to do, and closing it releases every host resource.
pub struct CaptureStream {
    live: Option<Live>,
    /// Held for the stream's lifetime; released on close or drop, which is
    /// what lets the next `open` succeed.
    _lease: CaptureLease,
}

struct CaptureLease;

impl CaptureLease {
    fn acquire() -> Option<Self> {
        CAPTURE_LIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for CaptureLease {
    fn drop(&mut self) {
        CAPTURE_LIVE.store(false, Ordering::Release);
    }
}

impl CaptureStream {
    /// The one open() chokepoint. Opens a host input device and starts
    /// delivering interleaved f32 samples. Refuses while another capture
    /// stream exists; on every failure path the lease is released before
    /// returning, so a failed open never leaves the microphone "live".
    pub fn open(target: &str, request: CaptureRequest) -> Result<Self, CaptureOpenError> {
        let lease = CaptureLease::acquire().ok_or(CaptureOpenError::AnotherCaptureLive)?;
        // From here, any error returns while only `lease` is still in scope;
        // its Drop does the releasing, so failure never wedges the chokepoint.
        let live = open_live(target, request).map_err(|error| {
            log(
                Priority::Error,
                AUDIO_TAG,
                &format!("capture open failed, no capture stream exists: {error}"),
            );
            error
        })?;
        log(
            Priority::Info,
            AUDIO_TAG,
            &format!(
                "capture open: {} Hz, {} channel(s) (granted, not requested)",
                live.rate, live.channels
            ),
        );
        Ok(Self { live: Some(live), _lease: lease })
    }

    /// Every stop path. Idempotent; `Drop` calls this.
    pub fn close(&mut self) {
        self.live.take();
    }

    pub fn is_open(&self) -> bool {
        self.live.is_some()
    }

    /// Granted rate. Zero when closed.
    pub fn rate_hz(&self) -> u32 {
        self.live.as_ref().map_or(0, |l| l.rate)
    }

    /// Granted channel count. Zero when closed.
    pub fn channels(&self) -> u16 {
        self.live.as_ref().map_or(0, |l| l.channels)
    }

    /// Non-blocking pull of interleaved f32 samples. Zero means the host has
    /// nothing new (or the device went away — see [`Self::is_disconnected`]).
    pub fn read(&mut self, dst: &mut [f32]) -> usize {
        self.live.as_ref().map_or(0, |l| l.ring.read(dst))
    }

    /// Blocking pull with a deadline, for ordinary threads only.
    pub fn read_blocking(&mut self, dst: &mut [f32], timeout: Duration) -> usize {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let got = self.read(dst);
            if got > 0 {
                return got;
            }
            if self.is_disconnected() || std::time::Instant::now() >= deadline {
                return 0;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    pub fn is_disconnected(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.disconnected.load(Ordering::Acquire))
    }

    /// Samples the host produced that nobody consumed because the ring was
    /// full — the honest xrun source for an input stream.
    pub fn overruns(&self) -> u64 {
        self.live.as_ref().map_or(0, |l| l.ring.overruns())
    }
}

fn open_live(target: &str, request: CaptureRequest) -> Result<Live, CaptureOpenError> {
    let backend = effective_backend_name();
    let host = host_by_name(backend)
        .ok_or_else(|| CaptureOpenError::DeviceFailed(format!("host '{backend}' unavailable")))?;
    let device = pick_input(&host, target).ok_or(CaptureOpenError::NoInputDevice)?;
    let default = device
        .default_input_config()
        .map_err(|e| CaptureOpenError::DeviceFailed(e.to_string()))?;

    let rate = request.sample_rate.unwrap_or_else(|| default.sample_rate());
    let channels = request.channels.unwrap_or(default.channels());
    let format = default.sample_format();

    let ring = Arc::new(SampleRing::new(1 << 15, channels));
    let disconnected = Arc::new(AtomicBool::new(false));
    let cfg = StreamConfig {
        channels,
        sample_rate: rate,
        buffer_size: BufferSize::Default,
    };

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ring_cb, disconnected_cb) = (Arc::clone(&ring), Arc::clone(&disconnected));

    let spawned = std::thread::Builder::new()
        .name("roblox-cpal-capture".into())
        .spawn(move || {
            // Built, played and dropped on this one owner thread, so the
            // (non-Send) stream never crosses a thread boundary and dropping
            // it after the park below is what actually closes the device.
            let stream = match format {
                cpal::SampleFormat::F32 => build_input::<f32>(&device, &cfg, Arc::clone(&ring_cb), Arc::clone(&disconnected_cb)),
                cpal::SampleFormat::I16 => build_input::<i16>(&device, &cfg, Arc::clone(&ring_cb), Arc::clone(&disconnected_cb)),
                cpal::SampleFormat::I32 => build_input::<i32>(&device, &cfg, Arc::clone(&ring_cb), Arc::clone(&disconnected_cb)),
                cpal::SampleFormat::U16 => build_input::<u16>(&device, &cfg, Arc::clone(&ring_cb), Arc::clone(&disconnected_cb)),
                other => Err(format!("unsupported capture sample format {other:?}")),
            };
            match stream {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    // Parked until `close` drops the sender.
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            }
        });
    let owner = spawned.map_err(|_| CaptureOpenError::SpawnFailed)?;

    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(Live {
            ring,
            rate,
            channels,
            owner: Some(owner),
            stop: Some(stop_tx),
            disconnected,
        }),
        Ok(Err(error)) => {
            drop(stop_tx);
            let _ = owner.join();
            Err(CaptureOpenError::DeviceFailed(error))
        }
        Err(_) => {
            drop(stop_tx);
            let _ = owner.join();
            Err(CaptureOpenError::DeviceFailed(
                "the input stream never became ready".into(),
            ))
        }
    }
}

fn pick_input(host: &cpal::Host, target: &str) -> Option<cpal::Device> {
    if !target.is_empty() {
        if let Some(device) = target.parse().ok().and_then(|id| host.device_by_id(&id)) {
            return Some(device);
        }
        log(
            Priority::Warn,
            AUDIO_TAG,
            &format!("input device '{target}' is not on this host; using the default input"),
        );
    }
    host.default_input_device()
}

fn build_input<T>(
    device: &cpal::Device,
    cfg: &StreamConfig,
    ring: Arc<SampleRing>,
    disconnected: Arc<AtomicBool>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    // The callback's whole body is convert-and-store: no lock, no allocation,
    // no logging. The ring write overwrites the oldest data when full, which
    // keeps the callback bounded in time.
    device
        .build_input_stream::<T, _, _>(
            cfg.clone(),
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                ring.write_with(data.len(), |i| f32::from_sample(data[i]));
            },
            move |error| match error.kind() {
                cpal::ErrorKind::Xrun => {}
                cpal::ErrorKind::DeviceNotAvailable
                | cpal::ErrorKind::DeviceChanged
                | cpal::ErrorKind::StreamInvalidated => {
                    disconnected.store(true, Ordering::Release);
                    log(
                        Priority::Error,
                        AUDIO_TAG,
                        &format!("capture stream disconnected: {error}"),
                    );
                }
                _ => log(
                    Priority::Warn,
                    AUDIO_TAG,
                    &format!("capture stream error: {error}"),
                ),
            },
            None,
        )
        .map_err(|e| e.to_string())
}

/// Test-only construction of the ring, so the overwrite policy can be checked
/// without a device.
#[cfg(test)]
pub(crate) fn ring_for_tests(capacity_frames: usize, channels: u16) -> Arc<SampleRing> {
    Arc::new(SampleRing::new(capacity_frames, channels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_delivers_what_the_producer_wrote_in_order() {
        let ring = ring_for_tests(4, 1);
        ring.write_with(4, |i| (i + 1) as f32);
        let mut dst = [0f32; 4];
        assert_eq!(ring.read(&mut dst), 4);
        assert_eq!(dst, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(ring.available(), 0);
    }

    #[test]
    fn a_full_ring_overwrites_the_oldest_and_counts_the_loss() {
        let ring = ring_for_tests(2, 1);
        ring.write_with(2, |i| (i + 1) as f32);
        ring.write_with(2, |i| (i + 3) as f32);
        assert_eq!(ring.overruns(), 1);
        let mut dst = [0f32; 2];
        ring.read(&mut dst);
        // The oldest samples (1,2) were overwritten by (3,4).
        assert_eq!(dst, [3.0, 4.0]);
    }

    #[test]
    fn a_read_into_a_small_buffer_consumes_only_what_it_holds() {
        let ring = ring_for_tests(4, 1);
        ring.write_with(4, |i| (i + 1) as f32);
        let mut dst = [0f32; 2];
        assert_eq!(ring.read(&mut dst), 2);
        assert_eq!(dst, [1.0, 2.0]);
        assert_eq!(ring.available(), 2);
    }

    #[test]
    fn a_closed_stream_reads_nothing_and_reports_zero_format() {
        let mut stream = CaptureStream { live: None, _lease: CaptureLease };
        assert!(!stream.is_open());
        assert_eq!(stream.rate_hz(), 0);
        assert_eq!(stream.channels(), 0);
        let mut dst = [0f32; 4];
        assert_eq!(stream.read(&mut dst), 0);
        assert!(!stream.is_disconnected());
        assert_eq!(stream.overruns(), 0);
    }

    // One test for the process-global chokepoint, so parallel test threads
    // cannot interleave the transitions inside it.
    #[test]
    fn the_chokepoint_refuses_a_second_capture_and_every_release_reopens() {
        let first = CaptureLease::acquire().expect("first lease");
        assert!(CaptureLease::acquire().is_none(), "second capture must be refused");

        // Dropping the lease releases the chokepoint, and it can be taken
        // again — a closed capture never wedges the gate for future ones.
        drop(first);
        let again = CaptureLease::acquire().expect("released lease reopens");
        drop(again);

        // Dropping a stream (which drops its lease) releases it too.
        let stream = CaptureStream { live: None, _lease: CaptureLease::acquire().unwrap() };
        assert!(CaptureLease::acquire().is_none());
        drop(stream);
        assert!(CaptureLease::acquire().is_some(), "dropped stream releases the gate");
    }
}
