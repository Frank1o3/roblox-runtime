//! AudioRecorderObject: the microphone, and the one object here whose
//! *lifetime rules* are load-bearing rather than incidental. Stated once,
//! because every method below is arranged to keep them:
//!
//! ```text
//! create_audio_recorder            opens nothing
//! realize                          opens nothing
//! set_record_state(RECORDING)      opens the capture stream
//! set_record_state(PAUSED)         destroys it
//! set_record_state(STOPPED)        destroys it
//! destroy                          destroys it
//! ```
//!
//! `realize` opening nothing is the one that looks wrong next to the player,
//! which does open its stream. It is not symmetric on purpose: realizing a
//! player is inaudible, and realizing a recorder that opened a capture stream
//! would light the desktop's microphone indicator for an object the engine
//! has merely constructed. The cost of the asymmetry is that a microphone
//! that cannot be opened is reported at `set_record_state` rather than at
//! `realize`, which is later than Android would report it but is the only
//! place the answer can be found without holding the device to find it.
//!
//! Pausing destroys the stream rather than deactivating it for the same
//! reason `CaptureStream` gives: a paused-but-connected capture node is
//! still a lit indicator. Samples arriving during a pause are discarded
//! either way, so nothing is lost by not being there for them.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use super::backend::{capture_opener, OpenSlCapture};
use super::log_opensl;
use crate::logline::Priority;
use crate::types::*;

pub type RecorderHandle = usize;

pub type SlBufferQueueCallback = unsafe extern "C" fn(queue: usize, context: *mut c_void);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderIid {
    Record,
    AndroidSimpleBufferQueue,
    AndroidConfiguration,
}

/// One caller-owned buffer waiting to be filled with captured PCM. The
/// direction is the mirror image of the player's pending queue — the caller
/// supplies empty space rather than full data — but the ownership contract
/// is identical and comes from the same place: the buffer belongs to the
/// caller and must stay valid until its drain callback fires.
#[derive(Clone, Copy)]
struct RecordBuffer {
    data: *mut u8,
    size: u32,
    filled: u32,
}

/// Everything the pump thread and the surface calls share under one lock.
struct RecorderInner {
    pending: VecDeque<RecordBuffer>,
    enqueued_index: u32,
    callback: Option<SlBufferQueueCallback>,
    context: *mut c_void,
    frames_recorded: u64,
}

pub struct RecorderShared {
    state: AtomicU32,
    record_state: AtomicU32,
    rate_hz: u32,
    channels: u32,
    bytes_per_frame: u32,
    num_buffers: u32,
    inner: Mutex<RecorderInner>,
    /// The capture device, `Some` exactly while the caller is recording.
    capture: Mutex<Option<Box<dyn OpenSlCapture>>>,
    pumping: AtomicBool,
    pump: Mutex<Option<JoinHandle<()>>>,
    /// The pump's identity, set by the pump itself first thing so that a
    /// callback firing immediately still finds it set.
    pump_id: AtomicU64,
    /// Set when destroy ran on the pump thread and could not stop the pump
    /// synchronously; the pump's own exit then has nothing left to do but
    /// let go of its `Arc`.
    abandoned: AtomicBool,
    marker_position_ms: AtomicU32,
    position_update_period_ms: AtomicU32,
}

// SAFETY: the raw pointers are caller-owned record buffers (written in place,
// valid until their drain callback fires) and the engine's callback context
// (handed back verbatim, never dereferenced here). Both are protected by
// `inner`'s mutex for mutation.
unsafe impl Send for RecorderShared {}
// SAFETY: same reasoning — all access goes through `inner`'s mutex or the
// atomics.
unsafe impl Sync for RecorderShared {}

/// Distinguishes the pump thread of one recorder from another's (and from
/// every non-pump thread, which reads zero).
static NEXT_PUMP_ID: AtomicU64 = AtomicU64::new(1);
thread_local! {
    static PUMP_THREAD: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

impl RecorderShared {
    pub fn new(rate_hz: u32, channels: u32, num_buffers: u32) -> Arc<Self> {
        Arc::new(Self {
            state: AtomicU32::new(SL_OBJECT_STATE_UNREALIZED),
            record_state: AtomicU32::new(SL_RECORDSTATE_STOPPED),
            rate_hz,
            channels,
            bytes_per_frame: channels * 2, // s16, checked at create
            num_buffers,
            inner: Mutex::new(RecorderInner {
                pending: VecDeque::new(),
                enqueued_index: 0,
                callback: None,
                context: std::ptr::null_mut(),
                frames_recorded: 0,
            }),
            capture: Mutex::new(None),
            pumping: AtomicBool::new(false),
            pump: Mutex::new(None),
            pump_id: AtomicU64::new(0),
            abandoned: AtomicBool::new(false),
            marker_position_ms: AtomicU32::new(0),
            position_update_period_ms: AtomicU32::new(0),
        })
    }

    pub fn state(&self) -> SLuint32 {
        self.state.load(Ordering::Acquire)
    }

    pub fn rate_hz(&self) -> u32 {
        self.rate_hz
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    // -------------------------------------------------------------- realize

    /// `Realize` opens nothing; see the module comment. The engine gets a
    /// realized recorder and the microphone stays shut until it asks to
    /// record.
    pub fn realize(&self) -> SLresult {
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

    pub fn get_interface(&self, iid: super::SlIid) -> Result<RecorderIid, SLresult> {
        if iid == super::record() {
            Ok(RecorderIid::Record)
        } else if iid == super::android_simple_buffer_queue() {
            Ok(RecorderIid::AndroidSimpleBufferQueue)
        } else if iid == super::android_configuration() {
            Ok(RecorderIid::AndroidConfiguration)
        } else {
            Err(SL_RESULT_FEATURE_UNSUPPORTED)
        }
    }

    /// `Destroy`. Before anything else: a destroyed recorder must not leave a
    /// microphone behind it, including when the engine destroys one it never
    /// stopped. Idempotent, because destroy after STOPPED is the ordinary
    /// sequence and a double close must not be an error.
    ///
    /// The on-pump-thread branch is not defensive padding. Stopping a
    /// recorder from inside its own buffer-queue callback is a normal thing
    /// for an OpenSL caller to do — it is how you stop after N buffers — and
    /// it arrives here on the pump thread, where joining is a thread joining
    /// itself. The thread has already been told to stop and will return the
    /// moment the callback does, so skipping the join costs nothing; the
    /// registry entry goes away immediately, and the pump's own `Arc` keeps
    /// the object alive until the loop exits.
    pub fn destroy(self: Arc<Self>) -> SLresult {
        self.stop_capture();
        self.record_state.store(SL_RECORDSTATE_STOPPED, Ordering::Release);
        self.state.store(SL_OBJECT_STATE_UNREALIZED, Ordering::Release);
        super::remove_recorder(Arc::as_ptr(&self) as usize);
        if self.on_pump_thread() {
            // The pump is still on the stack below us; its own Arc keeps
            // everything alive until it unwinds.
            self.abandoned.store(true, Ordering::Release);
        }
        SL_RESULT_SUCCESS
    }

    // --------------------------------------------------------------- record

    pub fn set_record_state(self: &Arc<Self>, state: SLuint32) -> SLresult {
        match state {
            SL_RECORDSTATE_RECORDING => {
                if !self.start_capture() {
                    // Stays stopped. A recorder reporting RECORDING with no
                    // stream behind it is the stub-that-lies pattern this
                    // project keeps paying for: the engine would sit on a
                    // buffer queue that never drains, with nothing to say why.
                    log_opensl(
                        Priority::Error,
                        "SetRecordState(RECORDING) could not open a capture stream; staying \
                         STOPPED rather than reporting a recording that is not happening",
                    );
                    self.record_state
                        .store(SL_RECORDSTATE_STOPPED, Ordering::Release);
                    return SL_RESULT_RESOURCE_ERROR;
                }
            }
            SL_RECORDSTATE_PAUSED | SL_RECORDSTATE_STOPPED => {
                // Both destroy the stream; see the module comment on why
                // pausing is not allowed to mean "keep the device, discard
                // the samples".
                self.stop_capture();
            }
            _ => return SL_RESULT_PARAMETER_INVALID,
        }
        self.record_state.store(state, Ordering::Release);
        SL_RESULT_SUCCESS
    }

    pub fn get_record_state(&self, out: Option<&mut SLuint32>) -> SLresult {
        match out {
            Some(out) => {
                *out = self.record_state.load(Ordering::Acquire);
                SL_RESULT_SUCCESS
            }
            None => SL_RESULT_PARAMETER_INVALID,
        }
    }

    /// A promise to stop recording after N milliseconds. Nothing here
    /// enforces one, and accepting it would be a promise about the
    /// microphone specifically — the worst possible subject for a stub to be
    /// wrong about.
    pub fn set_duration_limit(&self, _ms: SLmillisecond) -> SLresult {
        SL_RESULT_FEATURE_UNSUPPORTED
    }

    /// Genuinely tracked, unlike the playback side's position: the pump
    /// counts every frame it hands to the caller, so this is a real position
    /// rather than an estimate that would have to be invented.
    pub fn get_position(&self, out: Option<&mut SLmillisecond>) -> SLresult {
        let Some(out) = out else {
            return SL_RESULT_PARAMETER_INVALID;
        };
        let inner = lock(&self.inner);
        *out = if self.rate_hz == 0 {
            0
        } else {
            (inner.frames_recorded * 1000 / u64::from(self.rate_hz)) as SLmillisecond
        };
        SL_RESULT_SUCCESS
    }

    /// Accepted; see the player side for why interest in an event that
    /// structurally never occurs is not a promise it will fire.
    pub fn register_record_callback(&self, _callback: *mut c_void, _context: *mut c_void) -> SLresult {
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

    // ------------------------------------------------------------------ absq

    /// Hands over one empty buffer for the pump to fill.
    ///
    /// # Safety
    /// `data` must point to `size` writable bytes that stay valid until this
    /// recorder reports them drained (or abandons them via clear/destroy).
    pub unsafe fn enqueue(&self, data: *mut u8, size: u32) -> SLresult {
        if data.is_null() || size == 0 {
            return SL_RESULT_PARAMETER_INVALID;
        }
        let mut inner = lock(&self.inner);
        if inner.pending.len() >= self.num_buffers as usize {
            return SL_RESULT_BUFFER_INSUFFICIENT;
        }
        inner.pending.push_back(RecordBuffer { data, size, filled: 0 });
        inner.enqueued_index += 1;
        SL_RESULT_SUCCESS
    }

    pub fn absq_clear(&self) -> SLresult {
        lock(&self.inner).pending.clear();
        SL_RESULT_SUCCESS
    }

    pub fn absq_get_state(
        &self,
        count: Option<&mut SLuint32>,
        index: Option<&mut SLuint32>,
    ) -> SLresult {
        let inner = lock(&self.inner);
        if let Some(count) = count {
            *count = inner.pending.len() as SLuint32;
        }
        if let Some(index) = index {
            *index = inner.enqueued_index;
        }
        SL_RESULT_SUCCESS
    }

    pub fn absq_register_callback(
        &self,
        callback: Option<SlBufferQueueCallback>,
        context: *mut c_void,
    ) -> SLresult {
        let mut inner = lock(&self.inner);
        inner.callback = callback;
        inner.context = context;
        SL_RESULT_SUCCESS
    }

    // ----------------------------------------------------------------- pump

    /// Opens the microphone and starts the pump. Called only from
    /// `set_record_state(SL_RECORDSTATE_RECORDING)`.
    fn start_capture(self: &Arc<Self>) -> bool {
        if lock(&self.capture).is_some() {
            return true;
        }
        // Empty target: the opener connects to whatever the host calls the
        // default source, read at this moment rather than cached, so changing
        // the desktop's default microphone between two recordings is picked
        // up without restarting the client.
        let capture = match capture_opener().open(self.rate_hz, self.channels as u16) {
            Ok(capture) => capture,
            Err(error) => {
                log_opensl(
                    Priority::Error,
                    &format!("SetRecordState(RECORDING): capture open failed: {error}"),
                );
                return false;
            }
        };
        *lock(&self.capture) = Some(capture);
        self.pumping.store(true, Ordering::Release);
        let worker = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("roblox-runtime-opensles-recorder".into())
            .spawn(move || run_pump(worker));
        match spawned {
            Ok(handle) => {
                *lock(&self.pump) = Some(handle);
                true
            }
            Err(error) => {
                // The capture must not outlive the pump that was supposed to
                // drain it: close it before reporting failure.
                *lock(&self.capture) = None;
                self.pumping.store(false, Ordering::Release);
                log_opensl(
                    Priority::Error,
                    &format!("could not start the recorder pump thread: {error}"),
                );
                false
            }
        }
    }

    /// Stops the pump and destroys the capture stream. Idempotent.
    fn stop_capture(&self) {
        self.pumping.store(false, Ordering::Release);
        if !self.on_pump_thread() {
            let handle = lock(&self.pump).take();
            if let Some(handle) = handle {
                let _ = handle.join();
            }
        }
        // Closing after the pump has stopped (or been left to finish on its
        // own): the pump only ever reads the capture it finds in place, so
        // taking it away ends the flow rather than racing it.
        *lock(&self.capture) = None;
    }

    fn on_pump_thread(&self) -> bool {
        let mine = PUMP_THREAD.with(|cell| cell.get());
        mine != 0 && self.pump_id.load(Ordering::Acquire) == mine
    }
}

/// Moves captured samples from the capture source into whatever buffer the
/// caller has at the front of the queue, and fires the drain callback for
/// each buffer that fills.
///
/// A thread rather than a host callback because `CaptureStream` was built for
/// `android.media.AudioRecord.read`, whose caller polls on its own schedule;
/// the buffering lives inside the capture and what is left here is a copy,
/// not a realtime deadline. 2 ms is a quarter of the shortest buffer either
/// caller uses (WebRTC's 10 ms frame), which is often enough that a buffer is
/// never late for want of asking and rare enough to cost nothing measurable.
fn run_pump(shared: Arc<RecorderShared>) {
    let id = NEXT_PUMP_ID.fetch_add(1, Ordering::AcqRel);
    // Recorded first thing, so that a callback firing immediately still
    // finds the answer set. Doing it from the thread that started the pump
    // would leave a window in which it was not.
    shared.pump_id.store(id, Ordering::Release);
    PUMP_THREAD.with(|cell| cell.set(id));

    while shared.pumping.load(Ordering::Acquire) {
        let mut completed = false;
        let mut got = 0usize;
        let callback: Option<SlBufferQueueCallback>;
        let context: *mut c_void;
        {
            let mut inner = lock(&shared.inner);
            let mut capture = lock(&shared.capture);
            if let Some(front) = inner.pending.front_mut() {
                // SAFETY: the buffer is the caller's, `size` writable bytes,
                // valid until the drain callback fires; `filled <= size` by
                // construction.
                let dst = unsafe {
                    std::slice::from_raw_parts_mut(
                        front.data.add(front.filled as usize),
                        (front.size - front.filled) as usize,
                    )
                };
                got = capture.as_mut().map_or(0, |c| c.read(dst));
                front.filled += got as u32;
                if front.filled >= front.size {
                    completed = true;
                }
            }
            if completed {
                inner.pending.pop_front();
            }
            // A partial frame is not a frame recorded; whole frames only.
            inner.frames_recorded += (got as u32).checked_div(shared.bytes_per_frame).unwrap_or(0) as u64;
            callback = inner.callback;
            context = inner.context;
        }
        let token = Arc::as_ptr(&shared) as usize;
        // Outside the lock, for the same reason the playback side does it:
        // the ordinary buffer-queue pattern is to re-enqueue from inside
        // this callback, which would deadlock against its own mutex.
        if completed {
            if let Some(callback) = callback {
                // SAFETY: the engine's own callback, invoked with the queue
                // token and the context it registered.
                unsafe { callback(token, context) };
            }
        } else {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    // An abandoned object (destroyed from inside its own callback) has
    // nothing left to free: the registry entry is gone, the capture is
    // closed, and this Arc clone was the last one.
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
