//! AudioPlayerObject: buffer-queue playback over the host output backend.
//!
//! The engine enqueues whole PCM buffers (`Enqueue`, no copy — the buffer
//! belongs to the caller until its drain is reported), and the output
//! backend's audio thread pulls from that queue, converts the caller's format
//! to f32, applies volume and mute, and reports each drained buffer through
//! the engine's callback — the same shape as the C++ `PlaybackStream` +
//! `AudioPlayerObject`, re-derived here against the Rust backend seam.
//!
//! Rules pinned by tests in `super::tests`:
//! - accepted layouts are exactly the ones the C++ backend mapped:
//!   (8,8) signed, (16,16), (24,24), (24,32), (32,32), each LE and BE;
//!   anything else is refused with `SL_RESULT_CONTENT_UNSUPPORTED`, never
//!   rounded to the nearest thing that compiles;
//! - the buffer-queue contract is the caller's: bytes are read in place and
//!   the drain callback fires once per buffer, in order;
//! - volume is the spec's millibel→linear curve, applied in the fill path;
//! - position is not tracked and says so (zero, `SL_TIME_UNKNOWN` duration).

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::backend::output_factory;
use super::log_opensl;
use crate::backend::output::{FillCallback, OutputStream};
use crate::backend::pcm::{fill_pcm, PendingBuffer};
use crate::logline::Priority;
use crate::types::*;

pub type PlayerHandle = usize;

/// The engine's `slAndroidSimpleBufferQueueCallback`: the queue token (this
/// player's handle, standing in for the interface pointer the FFI wrapper
/// will pass) and the caller's registered context.
pub type SlBufferQueueCallback = unsafe extern "C" fn(queue: usize, context: *mut c_void);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerIid {
    Play,
    AndroidSimpleBufferQueue,
    Volume,
    AndroidConfiguration,
}

/// The registered drain callback and its context. Behind a mutex because the
/// engine may re-register from any thread while the audio thread is reading.
#[derive(Clone, Copy)]
struct DrainCallback {
    callback: Option<SlBufferQueueCallback>,
    context: *mut c_void,
}

// SAFETY: the context pointer is the engine's, stored only to be handed back
// verbatim to the engine's own callback and never dereferenced here.
unsafe impl Send for DrainCallback {}
// SAFETY: same reasoning as Send — no thread mutates the pointer through
// this type; the engine owns what it points at.
unsafe impl Sync for DrainCallback {}

/// Newtype so the caller-owned buffer pointers can cross threads under the
/// lock: `PendingBuffer` holds raw pointers whose validity is the buffer-
/// queue contract (valid and unmodified until the drain callback fires).
#[derive(Default)]
struct PendingQueue(VecDeque<PendingBuffer>);

// SAFETY: the pointers are engine-owned enqueue buffers; they are read in
// place under the queue mutex and never dereferenced on this file's own
// authority, which is exactly the contract the engine signed up to.
unsafe impl Send for PendingQueue {}
// SAFETY: same reasoning — all access is mutex-protected.
unsafe impl Sync for PendingQueue {}

/// The drain contexts reported by one fill.
#[derive(Default)]
struct DrainedBuffers(Vec<*mut c_void>);

// SAFETY: engine-owned context pointers, collected under the queue mutex and
// handed back verbatim to the engine's own callback.
unsafe impl Send for DrainedBuffers {}
// SAFETY: same reasoning.
unsafe impl Sync for DrainedBuffers {}

/// State the fill closure owns exclusively once the audio thread runs it.
/// Everything shared with the engine-facing surface lives behind an `Arc`
/// inside it.
struct PlayerFillState {
    queue: Arc<Mutex<PendingQueue>>,
    callback: Arc<Mutex<DrainCallback>>,
    volume_linear: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
    /// Preallocated: drains reported in one call can never exceed the queue
    /// depth, so this never grows after the first fill.
    drained: DrainedBuffers,
    /// Caller-format bytes staged between `fill_pcm` and conversion.
    /// Preallocated on the first call; every later resize stays within
    /// capacity, so the steady-state fill allocates nothing.
    scratch: Vec<u8>,
    container_bytes: usize,
    bits_per_sample: u32,
    big_endian: bool,
    token: usize,
}

pub struct PlayerShared {
    state: AtomicU32,
    play_state: AtomicU32,
    // Format, captured at create time and used at realize.
    rate_hz: u32,
    channels: u32,
    bits_per_sample: u32,
    container_bits: u32,
    big_endian: bool,
    num_buffers: u32,
    /// The pending queue, shared with the fill closure the audio thread runs.
    queue: Arc<Mutex<PendingQueue>>,
    enqueued_index: AtomicU32,
    callback: Arc<Mutex<DrainCallback>>,
    volume_linear: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
    // SetMarkerPosition/SetPositionUpdatePeriod: accepted and echoed back
    // faithfully, but the events they configure are never emitted — this
    // player does not track playback position, so there is no moment to
    // compare against them.
    marker_position_ms: AtomicU32,
    position_update_period_ms: AtomicU32,
    /// The host output stream, created at realize.
    output: Mutex<Option<Box<dyn OutputStream>>>,
}

// SAFETY: the only raw pointers live in `queue`'s `PendingBuffer` entries
// (caller-owned enqueue buffers, valid until their drain is reported, read
// in place) and in `DrainCallback` (engine-owned, handed back verbatim).
unsafe impl Send for PlayerShared {}
// SAFETY: same reasoning — no thread dereferences the engine's pointers on
// this file's own authority.
unsafe impl Sync for PlayerShared {}

impl PlayerShared {
    pub fn new(
        rate_hz: u32,
        channels: u32,
        bits_per_sample: u32,
        container_bits: u32,
        big_endian: bool,
        num_buffers: u32,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: AtomicU32::new(SL_OBJECT_STATE_UNREALIZED),
            play_state: AtomicU32::new(SL_PLAYSTATE_STOPPED),
            rate_hz,
            channels,
            bits_per_sample,
            container_bits,
            big_endian,
            num_buffers,
            queue: Arc::new(Mutex::new(PendingQueue::default())),
            enqueued_index: AtomicU32::new(0),
            callback: Arc::new(Mutex::new(DrainCallback {
                callback: None,
                context: std::ptr::null_mut(),
            })),
            volume_linear: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            muted: Arc::new(AtomicBool::new(false)),
            marker_position_ms: AtomicU32::new(0),
            position_update_period_ms: AtomicU32::new(0),
            output: Mutex::new(None),
        })
    }

    pub fn state(&self) -> SLuint32 {
        self.state.load(Ordering::Acquire)
    }

    /// The engine's format, as captured at create time.
    pub fn rate_hz(&self) -> u32 {
        self.rate_hz
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    // -------------------------------------------------------------- realize

    /// `Realize`: opens the output stream. The third output path, and it
    /// honours the user's sink through the host selector for the same reason
    /// the other two do — a device choice that only applied to some of the
    /// three paths would be a setting that works or does not depending on a
    /// decision the user never sees.
    pub fn realize(self: &Arc<Self>) -> SLresult {
        // Layout validation, mirroring the C++ backend's map_format: the
        // combinations that exist in the spec but that nothing uses fail
        // rather than being rounded to the nearest thing that compiles.
        let layout_ok = matches!(
            (self.bits_per_sample, self.container_bits),
            (8, 8) | (16, 16) | (24, 24) | (24, 32) | (32, 32)
        );
        if !layout_ok {
            log_opensl(
                Priority::Error,
                &format!(
                    "unsupported PCM layout: {}-bit samples in a {}-bit container; no audio \
                     for this player",
                    self.bits_per_sample, self.container_bits
                ),
            );
            return SL_RESULT_CONTENT_UNSUPPORTED;
        }
        if self.channels == 0 || self.channels > 64 {
            log_opensl(
                Priority::Error,
                &format!("unsupported channel count {}; no audio for this player", self.channels),
            );
            return SL_RESULT_CONTENT_UNSUPPORTED;
        }

        let mut output = output_factory().make_output();
        let mut fill_state = PlayerFillState {
            queue: Arc::clone(&self.queue),
            callback: Arc::clone(&self.callback),
            volume_linear: Arc::clone(&self.volume_linear),
            muted: Arc::clone(&self.muted),
            drained: DrainedBuffers(Vec::with_capacity(self.num_buffers.max(1) as usize)),
            scratch: Vec::new(),
            container_bytes: (self.container_bits / 8) as usize,
            bits_per_sample: self.bits_per_sample,
            big_endian: self.big_endian,
            // The token the engine's callbacks receive: this object's
            // registry key, which the FFI wrapper hands out as the
            // buffer-queue interface pointer.
            token: Arc::as_ptr(self) as usize,
        };
        let fill: FillCallback = Box::new(move |dst: &mut [f32], frames: u32| {
            fill_state.render(dst, frames)
        });
        if !output.open(
            self.bits_per_sample,
            false,
            "roblox-runtime (Roblox via OpenSL ES)",
            "",
            fill,
        ) {
            return SL_RESULT_CONTENT_UNSUPPORTED;
        }
        *lock(&self.output) = Some(output);
        self.state.store(SL_OBJECT_STATE_REALIZED, Ordering::Release);
        SL_RESULT_SUCCESS
    }

    pub fn get_state(&self, out: Option<&mut SLuint32>) -> SLresult {
        match out {
            Some(out) => {
                *out = self.state();
                SL_RESULT_SUCCESS
            }
            None => SL_RESULT_PARAMETER_INVALID,
        }
    }

    pub fn get_interface(&self, iid: super::SlIid) -> Result<PlayerIid, SLresult> {
        if iid == super::play() {
            Ok(PlayerIid::Play)
        } else if iid == super::android_simple_buffer_queue() {
            Ok(PlayerIid::AndroidSimpleBufferQueue)
        } else if iid == super::volume() {
            Ok(PlayerIid::Volume)
        } else if iid == super::android_configuration() {
            Ok(PlayerIid::AndroidConfiguration)
        } else {
            // Includes SL_IID_BUFFERQUEUE deliberately: the engine links
            // against it, but Android's own audio players use the
            // ANDROIDSIMPLEBUFFERQUEUE extension, not the generic one, so
            // nothing here ever needs to hand one out.
            Err(SL_RESULT_FEATURE_UNSUPPORTED)
        }
    }

    /// `Destroy`: stops being pulled, closes the host stream, unregisters.
    pub fn destroy(self: Arc<Self>) -> SLresult {
        let mut output = lock(&self.output).take();
        // Explicit close, not a drop: dropping the box releases the object,
        // but `close` is the method that actually shuts the device down.
        if let Some(output) = output.as_mut() {
            output.close();
        }
        self.play_state.store(SL_PLAYSTATE_STOPPED, Ordering::Release);
        lock(&self.queue).0.clear();
        self.state.store(SL_OBJECT_STATE_UNREALIZED, Ordering::Release);
        super::remove_player(Arc::as_ptr(&self) as usize);
        SL_RESULT_SUCCESS
    }

    // ----------------------------------------------------------------- play

    pub fn set_play_state(&self, state: SLuint32) -> SLresult {
        let output_guard = lock(&self.output);
        match state {
            SL_PLAYSTATE_PLAYING => {
                if let Some(output) = output_guard.as_ref() {
                    output.set_running(true);
                }
            }
            SL_PLAYSTATE_PAUSED => {
                if let Some(output) = output_guard.as_ref() {
                    output.set_running(false);
                }
            }
            SL_PLAYSTATE_STOPPED => {
                if let Some(output) = output_guard.as_ref() {
                    output.set_running(false);
                }
                // STOPPED also empties the queue: a stopped player resumes
                // from silence, not from audio it was told to abandon.
                lock(&self.queue).0.clear();
            }
            _ => return SL_RESULT_PARAMETER_INVALID,
        }
        drop(output_guard);
        self.play_state.store(state, Ordering::Release);
        SL_RESULT_SUCCESS
    }

    pub fn get_play_state(&self, out: Option<&mut SLuint32>) -> SLresult {
        match out {
            Some(out) => {
                *out = self.play_state.load(Ordering::Acquire);
                SL_RESULT_SUCCESS
            }
            None => SL_RESULT_PARAMETER_INVALID,
        }
    }

    /// Spec-correct for a buffer-queue source: there is no inherent duration,
    /// the queue is fed indefinitely.
    pub fn get_duration(&self, out: Option<&mut SLmillisecond>) -> SLresult {
        match out {
            Some(out) => {
                *out = SL_TIME_UNKNOWN;
                SL_RESULT_SUCCESS
            }
            None => SL_RESULT_PARAMETER_INVALID,
        }
    }

    /// Not tracked: zero rather than a fabricated estimate.
    pub fn get_position(&self, out: Option<&mut SLmillisecond>) -> SLresult {
        match out {
            Some(out) => {
                *out = 0;
                SL_RESULT_SUCCESS
            }
            None => SL_RESULT_PARAMETER_INVALID,
        }
    }

    /// Accepted; the events it configures structurally never occur, and
    /// registering interest in one is not the same claim as promising it
    /// will fire.
    pub fn register_play_callback(&self, _callback: *mut c_void, _context: *mut c_void) -> SLresult {
        SL_RESULT_SUCCESS
    }

    pub fn set_callback_events_mask(&self, _mask: SLuint32) -> SLresult {
        SL_RESULT_SUCCESS
    }

    pub fn get_callback_events_mask(&self, out: Option<&mut SLuint32>) -> SLresult {
        match out {
            Some(out) => {
                *out = 0;
                SL_RESULT_SUCCESS
            }
            None => SL_RESULT_PARAMETER_INVALID,
        }
    }

    pub fn set_marker_position(&self, ms: SLmillisecond) -> SLresult {
        self.marker_position_ms.store(ms, Ordering::Release);
        SL_RESULT_SUCCESS
    }

    pub fn clear_marker_position(&self) -> SLresult {
        self.marker_position_ms.store(0, Ordering::Release);
        SL_RESULT_SUCCESS
    }

    pub fn get_marker_position(&self, out: Option<&mut SLmillisecond>) -> SLresult {
        match out {
            Some(out) => {
                *out = self.marker_position_ms.load(Ordering::Acquire);
                SL_RESULT_SUCCESS
            }
            None => SL_RESULT_PARAMETER_INVALID,
        }
    }

    pub fn set_position_update_period(&self, ms: SLmillisecond) -> SLresult {
        self.position_update_period_ms.store(ms, Ordering::Release);
        SL_RESULT_SUCCESS
    }

    pub fn get_position_update_period(&self, out: Option<&mut SLmillisecond>) -> SLresult {
        match out {
            Some(out) => {
                *out = self.position_update_period_ms.load(Ordering::Acquire);
                SL_RESULT_SUCCESS
            }
            None => SL_RESULT_PARAMETER_INVALID,
        }
    }

    // ----------------------------------------------------------------- absq

    /// `Enqueue`: hands over one full buffer. Not copied — the buffer-queue
    /// contract is that `data` stays valid and unmodified until the drain
    /// callback fires for it, exactly as real Android's implementation
    /// requires; copying here would silently give the engine a looser
    /// contract than the one it was compiled against.
    ///
    /// # Safety
    /// `data` must point to `size` readable bytes that stay valid and
    /// unmodified until this player reports them drained (and must remain
    /// valid if the queue is cleared — those buffers are abandoned without a
    /// callback, which is the C++ behaviour too).
    pub unsafe fn enqueue(&self, data: *const u8, size: u32) -> SLresult {
        if data.is_null() || size == 0 {
            return SL_RESULT_PARAMETER_INVALID;
        }
        {
            let mut queue = lock(&self.queue);
            if queue.0.len() >= self.num_buffers as usize {
                return SL_RESULT_BUFFER_INSUFFICIENT;
            }
            queue.0.push_back(PendingBuffer {
                data,
                size,
                offset: 0,
                context: std::ptr::null_mut(),
            });
        }
        self.enqueued_index.fetch_add(1, Ordering::AcqRel);
        SL_RESULT_SUCCESS
    }

    pub fn absq_clear(&self) -> SLresult {
        lock(&self.queue).0.clear();
        SL_RESULT_SUCCESS
    }

    pub fn absq_get_state(
        &self,
        count: Option<&mut SLuint32>,
        index: Option<&mut SLuint32>,
    ) -> SLresult {
        let queue = lock(&self.queue);
        if let Some(count) = count {
            *count = queue.0.len() as SLuint32;
        }
        if let Some(index) = index {
            *index = self.enqueued_index.load(Ordering::Acquire);
        }
        SL_RESULT_SUCCESS
    }

    pub fn absq_register_callback(
        &self,
        callback: Option<SlBufferQueueCallback>,
        context: *mut c_void,
    ) -> SLresult {
        *lock(&self.callback) = DrainCallback { callback, context };
        SL_RESULT_SUCCESS
    }

    // --------------------------------------------------------------- volume

    pub fn set_volume_level(&self, level: SLmillibel) -> SLresult {
        // The spec's own curve: millibels are hundredths of a dB, so
        // 10^(mb/2000) is the linear gain. The floor is silence, not a
        // negative gain.
        // `SL_MILLIBEL_MIN` is i16's minimum, so `==` is the whole story:
        // nothing can be smaller.
        let linear = if level == SL_MILLIBEL_MIN {
            0.0f32
        } else {
            10.0f32.powf(level as f32 / 2000.0)
        };
        self.volume_linear.store(linear.to_bits(), Ordering::Release);
        SL_RESULT_SUCCESS
    }

    pub fn get_volume_level(&self, out: Option<&mut SLmillibel>) -> SLresult {
        let Some(out) = out else {
            return SL_RESULT_PARAMETER_INVALID;
        };
        let linear = f32::from_bits(self.volume_linear.load(Ordering::Acquire));
        if linear <= 0.0 {
            *out = SL_MILLIBEL_MIN;
        } else {
            // f64 not for precision but because the C++ needed a glibc symbol
            // that does not vanish on older hosts; the value is truncated to
            // an integer millibel two lines down either way.
            let mb = 2000.0f64 * f64::from(linear).log10();
            *out = mb.clamp(SL_MILLIBEL_MIN as f64, SL_MILLIBEL_MAX as f64) as SLmillibel;
        }
        SL_RESULT_SUCCESS
    }

    /// 0 millibel = unity gain, the ceiling the host's own control expects.
    pub fn get_max_volume_level(&self, out: Option<&mut SLmillibel>) -> SLresult {
        if let Some(out) = out {
            *out = 0;
        }
        SL_RESULT_SUCCESS
    }

    pub fn set_mute(&self, mute: SLboolean) -> SLresult {
        self.muted.store(mute != SL_BOOLEAN_FALSE, Ordering::Release);
        SL_RESULT_SUCCESS
    }

    pub fn get_mute(&self, out: Option<&mut SLboolean>) -> SLresult {
        match out {
            Some(out) => {
                *out = if self.muted.load(Ordering::Acquire) {
                    SL_BOOLEAN_TRUE
                } else {
                    SL_BOOLEAN_FALSE
                };
                SL_RESULT_SUCCESS
            }
            None => SL_RESULT_PARAMETER_INVALID,
        }
    }

    pub fn enable_stereo_position(&self, _enable: SLboolean) -> SLresult {
        SL_RESULT_FEATURE_UNSUPPORTED
    }

    pub fn is_enabled_stereo_position(&self, out: Option<&mut SLboolean>) -> SLresult {
        if let Some(out) = out {
            *out = SL_BOOLEAN_FALSE;
        }
        SL_RESULT_SUCCESS
    }

    pub fn set_stereo_position(&self, _position: i16) -> SLresult {
        SL_RESULT_FEATURE_UNSUPPORTED
    }

    pub fn get_stereo_position(&self, out: Option<&mut i16>) -> SLresult {
        if let Some(out) = out {
            *out = 0;
        }
        SL_RESULT_SUCCESS
    }

    // -------------------------------------------------- android configuration

    /// Every key defined for this interface is a hint about which physical
    /// Android audio path or scheduling class to use. None of that exists on
    /// this host — the request genuinely does not apply, which is different
    /// from an unimplemented feature — so accepting and discarding is the
    /// correct translation, not a shortcut.
    ///
    /// Discarded, but not unrecorded: which keys the engine sets is the
    /// cheapest available evidence of how it separates voice from game audio.
    /// One line per distinct key, so a client that sets one per player does
    /// not turn this into a log flood.
    pub fn set_configuration(&self, key: &str, value: &[u8]) -> SLresult {
        static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let mut seen = lock(&SEEN);
        if seen.iter().all(|k| k != key) {
            seen.push(key.to_owned());
            let mut value_u32 = 0u32;
            if value.len() >= 4 {
                value_u32 = u32::from_le_bytes([value[0], value[1], value[2], value[3]]);
            }
            log_opensl(
                Priority::Info,
                &format!(
                    "SLAndroidConfigurationItf::SetConfiguration('{key}', {value_u32}, {} bytes) \
                     — accepted and discarded; there is no Android audio path to select on \
                     this host",
                    value.len()
                ),
            );
        }
        SL_RESULT_SUCCESS
    }

    /// Nothing was ever stored, so there is no value to hand back;
    /// `PARAMETER_INVALID` is the spec's answer for an unrecognised key,
    /// which this is indistinguishable from.
    pub fn get_configuration(&self, value_size: Option<&mut SLuint32>) -> SLresult {
        if let Some(out) = value_size {
            *out = 0;
        }
        SL_RESULT_PARAMETER_INVALID
    }

    /// No JNI environment behind this player to proxy into.
    pub fn acquire_java_proxy(&self, out: Option<&mut *mut c_void>) -> SLresult {
        if let Some(out) = out {
            *out = std::ptr::null_mut();
        }
        SL_RESULT_FEATURE_UNSUPPORTED
    }

    pub fn release_java_proxy(&self) -> SLresult {
        SL_RESULT_SUCCESS
    }
}

// -------------------------------------------------------------------- fill

/// The fill closure the audio thread runs. Converts the caller's queued PCM
/// bytes to f32, applies volume and mute, and reports drains.
impl PlayerFillState {
    fn convert_sample(&self, bytes: &[u8]) -> f32 {
        match (self.bits_per_sample, self.big_endian) {
            (8, _) => i8::from_le_bytes([bytes[0]]) as f32 / 128.0,
            (16, false) => i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32768.0,
            (16, true) => i16::from_be_bytes([bytes[0], bytes[1]]) as f32 / 32768.0,
            (24, false) => {
                let v = (bytes[0] as i32 | (bytes[1] as i32) << 8 | (bytes[2] as i32) << 16) << 8;
                v as f32 / (1 << 31) as f32
            }
            (24, true) => {
                let v = (bytes[2] as i32 | (bytes[1] as i32) << 8 | (bytes[0] as i32) << 16) << 8;
                v as f32 / (1 << 31) as f32
            }
            (32, false) => {
                i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32
                    / (1 << 31) as f32
            }
            (32, true) => {
                i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32
                    / (1 << 31) as f32
            }
            // Realize refuses every other layout before this runs.
            _ => 0.0,
        }
    }

    fn render(&mut self, dst: &mut [f32], frames: u32) -> bool {
        let frames = frames as usize;
        // Zero frames is a degenerate call; zero channels keeps the fill a
        // no-op rather than dividing by it.
        let channels = dst.len().checked_div(frames).unwrap_or(0);
        let want_bytes = frames * channels * self.container_bytes;
        if self.scratch.len() < want_bytes {
            self.scratch.resize(want_bytes, 0);
        }
        self.drained.0.clear();
        {
            let mut queue = lock(&self.queue);
            fill_pcm(&mut queue.0, &mut self.scratch[..want_bytes], &mut self.drained.0);
        }
        let linear = f32::from_bits(self.volume_linear.load(Ordering::Relaxed));
        for (sample, out) in dst.iter_mut().enumerate() {
            let base = sample * self.container_bytes;
            let end = base + self.container_bytes;
            *out = self.convert_sample(&self.scratch[base..end]) * linear;
        }
        if self.muted.load(Ordering::Relaxed) {
            dst.fill(0.0);
        }
        // Drain reports happen outside the queue lock, in order: the ordinary
        // buffer-queue pattern is to re-enqueue from inside this callback,
        // which would deadlock against the queue's own mutex.
        let (callback, context) = {
            let drain = lock(&self.callback);
            (drain.callback, drain.context)
        };
        if let Some(callback) = callback {
            for _ in self.drained.0.drain(..) {
                // SAFETY: the engine's own callback, invoked with the queue
                // token it opened this player through and the context it
                // registered, per the buffer-queue contract.
                unsafe { callback(self.token, context) };
            }
        }
        // The queue model keeps playing; stopping happens through play state.
        true
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
