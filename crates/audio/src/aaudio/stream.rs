//! The AAudio surface: builders, streams, the state machine, and the
//! twenty-five entry points the engine resolves (`AAudio_createStreamBuilder`,
//! ten builder setters, `AAudioStreamBuilder_openStream`, and thirteen
//! `AAudioStream_*` calls).
//!
//! Written from the AAudio contract the engine actually exercises, with
//! `native/aaudio.cpp` consulted as a behavioral reference only. The rules
//! that shaped it, each pinned by a test in this file:
//!
//! - **Opening an input stream opens nothing.** The microphone exists only
//!   between `requestStart` and callback STOP / pause / stop / close; an
//!   opened-but-unstarted input stream is invisible to the host and leaves
//!   the desktop's microphone indicator out.
//! - **`supportsAAudio()` is a one-way door**: availability is only reported
//!   when a host backend can actually open.
//! - **Nothing on the fill path locks or allocates**: the output render
//!   closure touches only the engine's callback, its preallocated scratch,
//!   and one thread-local.
//! - **`close` from inside a data callback is refused**
//!   (`AAUDIO_ERROR_INVALID_STATE`) rather than deleting the stream the
//!   callback is running on; stop/pause from inside a callback never self-join
//!   and close the capture before returning.
//! - **Getters report what the device granted** (output) or the declared
//!   capture format (input), never what was asked.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use super::backend::{
    capture_backend, output_backend, Capture, DECLARED_BURST_FRAMES, DECLARED_BYTES_PER_FRAME,
    DECLARED_CAPTURE_RATE, DECLARED_CAPTURE_CHANNELS, DECLARED_RING_FRAMES,
};
use super::builder::{AAudioStreamDataCallback, AAudioStreamErrorCallback, StreamBuilder};
use crate::backend::output::{FillCallback, OutputStream};
use crate::input_callback::{DataFn, ExitFn, InputCallbackDriver, ReadFn};
use crate::logline::{log, Priority, AUDIO_TAG};
use crate::types::*;

/// Opaque builder handle handed to the engine: the registry key, which is the
/// address of the boxed builder. A call through a deleted builder's handle
/// answers honestly (`AAUDIO_ERROR_NULL` / a no-op) instead of reading freed
/// memory.
pub type BuilderHandle = usize;
/// Opaque stream handle: the registry key, the address of the stream's
/// `Arc`. Passed back to the engine as its `AAudioStream*`.
pub type StreamHandle = usize;

// ---------------------------------------------------------------------------
// Registries
// ---------------------------------------------------------------------------

static BUILDERS: OnceLock<Mutex<BTreeMap<BuilderHandle, Box<StreamBuilder>>>> = OnceLock::new();
static STREAMS: OnceLock<Mutex<BTreeMap<StreamHandle, Arc<StreamShared>>>> = OnceLock::new();

fn builders() -> &'static Mutex<BTreeMap<BuilderHandle, Box<StreamBuilder>>> {
    BUILDERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn streams() -> &'static Mutex<BTreeMap<StreamHandle, Arc<StreamShared>>> {
    STREAMS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// The thread that last ran an output fill or an input data callback,
// recorded so a `close` reaching this file from inside a callback can be
// refused instead of deleting a stream that is mid-callback (AAudio requires
// another thread; a hang is a far worse way to find out than an error code).
thread_local! {
    static CALLBACK_THREAD: Cell<usize> = const { Cell::new(0) };
}

fn running_on_callback_thread(handle: StreamHandle) -> bool {
    CALLBACK_THREAD.with(|c| c.get() == handle)
}

// ---------------------------------------------------------------------------
// Stream internals
// ---------------------------------------------------------------------------

/// The engine's callbacks and the values they were configured with, captured
/// at open time. Immutable after open (AAudio forbids changing callbacks on a
/// live stream), which is what lets the realtime paths read them without a
/// lock.
#[derive(Clone, Copy)]
struct EngineCallbacks {
    data: AAudioStreamDataCallback,
    error: AAudioStreamErrorCallback,
    /// SAFETY: the engine's `user_data`; passed back verbatim, never
    /// dereferenced here.
    user: *mut c_void,
}

// SAFETY: the two members are function pointers the engine registers and a
// `user_data` pointer the engine owns; both are handed back verbatim to the
// engine's own callbacks and never dereferenced here on this file's own
// authority, so sharing them across threads is as safe as the engine calling
// them from any thread.
unsafe impl Send for EngineCallbacks {}
// SAFETY: same reasoning as Send — the pointers are engine-owned values
// handed back verbatim; no thread mutates them through this type.
unsafe impl Sync for EngineCallbacks {}

/// A partial byte left over from the previous `read`: `Capture::read` counts
/// bytes, and this surface owes the engine whole frames. A producer that is
/// not frame-aligned costs one frame of latency instead of shifting every
/// subsequent sample by one byte.
struct Residue {
    buf: [u8; 8],
    len: usize,
}

impl Residue {
    const fn new() -> Self {
        Self { buf: [0; 8], len: 0 }
    }
}

pub(crate) struct StreamShared {
    direction: AAudioDirection,
    /// The granted format (output: what the device negotiated; input: the
    /// declared PCM_I16).
    format: AtomicI32,
    state: AtomicI32,
    /// Serialises external input lifecycle calls. Callback-thread lifecycle
    /// calls never take it: an external stop may hold it while joining that
    /// thread, so taking it from the callback would recreate the wait cycle
    /// this surface exists to avoid.
    input_lifecycle: Mutex<()>,
    /// Input only. `Some` while the capture device is held; this is the whole
    /// of the microphone rule as it applies to AAudio — opened by
    /// `requestStart`, destroyed by pause/stop/close and by every failure
    /// path.
    capture: Mutex<Option<Box<dyn Capture>>>,
    /// Input callback mode: the burst pump. Cancel-without-join is what makes
    /// stop-from-inside-the-callback safe; `join` is a no-op on the callback
    /// thread by the driver's own rule.
    driver: Mutex<Option<InputCallbackDriver>>,
    residue: Mutex<Residue>,
    engine: Mutex<EngineCallbacks>,
    /// Output: the host output stream, if opened.
    output: Mutex<Option<Box<dyn OutputStream>>>,
}

impl StreamShared {
    fn new(direction: AAudioDirection, format: AAudioFormat, engine: EngineCallbacks) -> Self {
        Self {
            direction,
            format: AtomicI32::new(format),
            state: AtomicI32::new(AAUDIO_STREAM_STATE_OPEN),
            input_lifecycle: Mutex::new(()),
            capture: Mutex::new(None),
            driver: Mutex::new(None),
            residue: Mutex::new(Residue::new()),
            engine: Mutex::new(engine),
            output: Mutex::new(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Builder surface
// ---------------------------------------------------------------------------

pub fn create_stream_builder() -> BuilderHandle {
    let builder = Box::new(StreamBuilder::default());
    let key = &*builder as *const StreamBuilder as usize;
    lock(builders()).insert(key, builder);
    key
}

fn builder_for(handle: BuilderHandle) -> Option<Box<StreamBuilder>> {
    lock(builders()).get(&handle).cloned()
}

/// Mutates a stored builder in place. A builder is stateful between setters
/// and openStream, so a throwaway clone of it would silently lose the
/// configuration — the bug this exists to prevent.
fn with_builder_mut<R>(
    handle: BuilderHandle,
    f: impl FnOnce(&mut StreamBuilder) -> R,
) -> Option<R> {
    let mut map = lock(builders());
    map.get_mut(&handle).map(|b| f(b))
}

pub fn delete_stream_builder(handle: BuilderHandle) -> AAudioResult {
    match lock(builders()).remove(&handle) {
        Some(_) => AAUDIO_OK,
        None => AAUDIO_ERROR_NULL,
    }
}

pub fn builder_set_direction(handle: BuilderHandle, direction: AAudioDirection) {
    with_builder_mut(handle, |b| b.direction = direction);
}

pub fn builder_set_format(handle: BuilderHandle, format: AAudioFormat) {
    with_builder_mut(handle, |b| b.format = format);
}

pub fn builder_set_buffer_capacity_in_frames(handle: BuilderHandle, frames: i32) {
    with_builder_mut(handle, |b| b.buffer_capacity_in_frames = frames);
}

pub fn builder_set_performance_mode(handle: BuilderHandle, mode: AAudioPerformanceMode) {
    with_builder_mut(handle, |b| b.performance_mode = mode);
}

pub fn builder_set_usage(handle: BuilderHandle, usage: AAudioUsage) {
    with_builder_mut(handle, |b| b.usage = usage);
}

pub fn builder_set_input_preset(handle: BuilderHandle, preset: AAudioInputPreset) {
    with_builder_mut(handle, |b| b.input_preset = preset);
}

pub fn builder_set_data_callback(
    handle: BuilderHandle,
    callback: AAudioStreamDataCallback,
    user: *mut c_void,
) {
    with_builder_mut(handle, |b| {
        b.data_callback = callback;
        b.data_user = user;
    });
}

pub fn builder_set_error_callback(
    handle: BuilderHandle,
    callback: AAudioStreamErrorCallback,
    user: *mut c_void,
) {
    with_builder_mut(handle, |b| {
        b.error_callback = callback;
        b.error_user = user;
    });
}

// ---------------------------------------------------------------------------
// openStream
// ---------------------------------------------------------------------------

/// Translates one engine format into the bits/float pair the output backends
/// speak. `false` for anything this surface cannot carry, which the caller
/// reports as a refused open.
fn format_to_bits(format: AAudioFormat) -> Option<(u32, bool)> {
    match format {
        // No preference: the backend offers its own float, which converts
        // nothing anywhere.
        AAUDIO_FORMAT_UNSPECIFIED => Some((0, false)),
        AAUDIO_FORMAT_PCM_I16 => Some((16, false)),
        AAUDIO_FORMAT_PCM_FLOAT => Some((32, true)),
        AAUDIO_FORMAT_PCM_I24_PACKED => Some((24, false)),
        AAUDIO_FORMAT_PCM_I32 => Some((32, false)),
        _ => None,
    }
}

fn bits_to_format(bits: u32, is_float: bool) -> AAudioFormat {
    if is_float {
        return if bits == 32 { AAUDIO_FORMAT_PCM_FLOAT } else { AAUDIO_FORMAT_INVALID };
    }
    match bits {
        16 => AAUDIO_FORMAT_PCM_I16,
        24 => AAUDIO_FORMAT_PCM_I24_PACKED,
        32 => AAUDIO_FORMAT_PCM_I32,
        _ => AAUDIO_FORMAT_INVALID,
    }
}

fn format_name(format: AAudioFormat) -> &'static str {
    match format {
        AAUDIO_FORMAT_UNSPECIFIED => "UNSPECIFIED",
        AAUDIO_FORMAT_PCM_I16 => "PCM_I16",
        AAUDIO_FORMAT_PCM_FLOAT => "PCM_FLOAT",
        AAUDIO_FORMAT_PCM_I24_PACKED => "PCM_I24_PACKED",
        AAUDIO_FORMAT_PCM_I32 => "PCM_I32",
        _ => "?",
    }
}

/// `AAudioStreamBuilder_openStream`. Everything the engine configured is
/// logged whatever the outcome, because this is the only place the
/// *requested* configuration is visible — the getters report the granted one,
/// and the difference is the whole answer to "what does this engine want".
pub fn open_stream(
    builder_handle: BuilderHandle,
    out: Option<&mut Option<StreamHandle>>,
) -> AAudioResult {
    let Some(builder) = builder_for(builder_handle) else {
        return AAUDIO_ERROR_NULL;
    };
    let Some(out) = out else {
        return AAUDIO_ERROR_NULL;
    };
    *out = None;
    log(
        Priority::Info,
        AUDIO_TAG,
        &format!(
            "openStream requested: direction={} format={} bufferCapacity={} performanceMode={} \
             usage={} inputPreset={} dataCallback={} errorCallback={}",
            if builder.direction == AAUDIO_DIRECTION_INPUT { "INPUT" } else { "OUTPUT" },
            format_name(builder.format),
            builder.buffer_capacity_in_frames,
            builder.performance_mode,
            builder.usage,
            builder.input_preset,
            builder.data_callback.is_some(),
            builder.error_callback.is_some(),
        ),
    );

    if format_to_bits(builder.format).is_none() {
        log(
            Priority::Error,
            AUDIO_TAG,
            &format!(
                "no host equivalent for aaudio_format_t {}; refusing rather than substituting a \
                 nearby format",
                builder.format
            ),
        );
        return AAUDIO_ERROR_ILLEGAL_ARGUMENT;
    }

    if !crate::backend::host_backend_available() {
        // Should be unreachable: the engine's supportsAAudio() asks the same
        // question first precisely so it never gets here. Kept because a
        // refusal at this point is not recoverable — FMOD abandons audio
        // rather than falling back — so the one place that could still
        // produce it says what it cost.
        log(
            Priority::Error,
            AUDIO_TAG,
            "no host backend reachable, and the engine has already committed to AAudio; there \
             will be no audio this run and no fallback",
        );
        return AAUDIO_ERROR_UNAVAILABLE;
    }

    if builder.direction == AAUDIO_DIRECTION_INPUT {
        open_input_stream(*builder, Some(out))
    } else {
        open_output_stream(*builder, Some(out))
    }
}
/// `AAudioStreamBuilder_openStream` for `AAUDIO_DIRECTION_INPUT`.
///
/// **Opening an input stream opens nothing.** That sentence is the whole
/// design. The stream this creates holds no capture device, so between here
/// and `requestStart` there is no microphone indicator and nothing for another
/// application to see. The getters cannot report a negotiated format the way
/// the output path does — negotiating one would mean connecting a stream —
/// so they report the declared capture format, and the real capture is asked
/// for exactly that at `requestStart`.
fn open_input_stream(
    builder: StreamBuilder,
    out: Option<&mut Option<StreamHandle>>,
) -> AAudioResult {
    // Mono s16 is all the capture path delivers, so it is all this can
    // honestly report. UNSPECIFIED means "you choose"; an explicit request
    // for anything else is refused rather than quietly answered with I16,
    // because the engine lays its recording buffers out against getFormat
    // and a wrong answer there is noise rather than an error.
    if builder.format != AAUDIO_FORMAT_UNSPECIFIED && builder.format != AAUDIO_FORMAT_PCM_I16 {
        log(
            Priority::Error,
            AUDIO_TAG,
            &format!(
                "refusing an input stream asking for {}: the capture path delivers PCM_I16 and \
                 nothing else. Refusing costs recording; answering with I16 anyway would cost it \
                 audibly and silently",
                format_name(builder.format)
            ),
        );
        return AAUDIO_ERROR_ILLEGAL_ARGUMENT;
    }

    let shared = Arc::new(StreamShared::new(
        AAUDIO_DIRECTION_INPUT,
        AAUDIO_FORMAT_PCM_I16,
        EngineCallbacks {
            data: builder.data_callback,
            error: builder.error_callback,
            // SAFETY: engine-owned; passed back verbatim.
            user: builder.data_user,
        },
    ));
    let handle = insert_stream(Arc::clone(&shared));
    log(
        Priority::Info,
        AUDIO_TAG,
        &format!(
            "input stream opened: will report {} Hz, {} channel(s), {}, {} frames per burst, \
             delivery={}. **No microphone is open yet** — the capture stream is created by \
             requestStart and destroyed by requestPause, requestStop and close",
            DECLARED_CAPTURE_RATE,
            DECLARED_CAPTURE_CHANNELS,
            format_name(AAUDIO_FORMAT_PCM_I16),
            DECLARED_BURST_FRAMES,
            if builder.data_callback.is_some() { "callback" } else { "blocking read" },
        ),
    );
    if let Some(out) = out {
        *out = Some(handle);
    }
    AAUDIO_OK
}

fn insert_stream(shared: Arc<StreamShared>) -> StreamHandle {
    let key = Arc::as_ptr(&shared) as usize;
    lock(streams()).insert(key, shared);
    key
}

/// `AAudioStreamBuilder_openStream` for output.
///
/// A stream with no data callback is not a mistake and must not be refused:
/// the engine opens probe streams with no callback purely to read
/// `getSampleRate`/`getFramesPerBurst` off them. The fill closure returns
/// silence for a null callback and every cycle is silence — that is what a
/// probe is *for*.
fn open_output_stream(
    builder: StreamBuilder,
    out: Option<&mut Option<StreamHandle>>,
) -> AAudioResult {
    let Some((bits, is_float)) = format_to_bits(builder.format) else {
        return AAUDIO_ERROR_ILLEGAL_ARGUMENT;
    };

    let shared = Arc::new(StreamShared::new(
        AAUDIO_DIRECTION_OUTPUT,
        AAUDIO_FORMAT_UNSPECIFIED,
        EngineCallbacks {
            data: builder.data_callback,
            error: builder.error_callback,
            // SAFETY: engine-owned; passed back verbatim.
            user: builder.data_user,
        },
    ));
    let handle = insert_stream(Arc::clone(&shared));

    let mut output = output_backend().make_output();
    let ctx = Arc::new(OutputFillCtx {
        handle,
        callbacks: *lock(&shared.engine),
    });
    let fill: FillCallback = Box::new(move |dst: &mut [f32], frames| ctx.render(dst, frames));
    // The user's sink, or empty for the session default. The engine cannot
    // express a device choice on this surface; the choice arrives from
    // outside the engine entirely (client settings).
    if !output.open(bits, is_float, "roblox-runtime (Roblox via AAudio)", "", fill) {
        lock(streams()).remove(&handle);
        return AAUDIO_ERROR_UNAVAILABLE;
    }
    let granted = bits_to_format(output.sample_bits(), output.sample_is_float());
    if granted == AAUDIO_FORMAT_INVALID {
        log(
            Priority::Error,
            AUDIO_TAG,
            &format!(
                "the host backend granted a {}-bit {} format with no aaudio_format_t to describe \
                 it; refusing rather than reporting a format the engine would lay its mixer out \
                 against wrongly",
                output.sample_bits(),
                if output.sample_is_float() { "float" } else { "integer" }
            ),
        );
        output.close();
        lock(streams()).remove(&handle);
        return AAUDIO_ERROR_UNAVAILABLE;
    }
    shared.format.store(granted, Ordering::Release);
    *lock(&shared.output) = Some(output);

    log(
        Priority::Info,
        AUDIO_TAG,
        &format!(
            "openStream ok: host backend granted {} Hz, {} channel(s), {}, {} frames per burst. \
             Requested format was {}; rate and channel count were left for the host to choose, \
             which is why nothing resamples",
            get_sample_rate(handle),
            get_channel_count(handle),
            format_name(granted),
            get_frames_per_burst(handle),
            format_name(builder.format),
        ),
    );
    if let Some(out) = out {
        *out = Some(handle);
    }
    AAUDIO_OK
}

// ---------------------------------------------------------------------------
// Output fill (realtime)
// ---------------------------------------------------------------------------

struct OutputFillCtx {
    handle: StreamHandle,
    callbacks: EngineCallbacks,
}

impl OutputFillCtx {
    /// Runs on the host audio thread. Its whole body: mark, invoke the
    /// engine's callback, convert, return. No lock, no allocation, no log —
    /// the realtime rule.
    fn render(&self, dst: &mut [f32], frames: u32) -> bool {
        CALLBACK_THREAD.with(|c| c.set(self.handle));
        let Some(data) = self.callbacks.data else {
            // No engine callback: silence. A probe stream is *meant* to
            // deliver nothing.
            dst.fill(0.0);
            return false;
        };
        // SAFETY: `dst` is the backend's output buffer with exactly
        // `frames * channels` float slots; the engine's callback fills it,
        // per the AAudio data-callback contract (stream pointer, user data,
        // buffer, frame count in, callback-result out).
        let result = unsafe {
            data(
                self.handle as *mut c_void,
                self.callbacks.user,
                dst.as_mut_ptr() as *mut c_void,
                frames as i32,
            )
        };
        result == AAUDIO_CALLBACK_RESULT_CONTINUE
    }
}

// ---------------------------------------------------------------------------
// Stream surface
// ---------------------------------------------------------------------------

fn stream_for(handle: StreamHandle) -> Option<Arc<StreamShared>> {
    lock(streams()).get(&handle).cloned()
}

fn engine_callbacks(shared: &Arc<StreamShared>) -> EngineCallbacks {
    *lock(&shared.engine)
}

fn take_capture(shared: &Arc<StreamShared>) -> Option<Box<dyn Capture>> {
    lock(&shared.capture).take()
}

fn reset_residue(shared: &Arc<StreamShared>) {
    lock(&shared.residue).len = 0;
}

/// Input stop/pause path. Cancels the driver without holding anything it
/// needs (safe from inside the callback), closes the capture immediately, and
/// only then joins when off the callback thread.
fn stop_capture(shared: &Arc<StreamShared>, to: AAudioStreamState) {
    let was_started = shared.state.load(Ordering::Acquire) == AAUDIO_STREAM_STATE_STARTED;
    shared.state.store(to, Ordering::Release);
    // Take the driver out of its mutex first, so nothing below holds a lock
    // the callback thread might need.
    let driver = lock(&shared.driver).take();
    if let Some(d) = driver.as_ref() {
        d.cancel();
    }
    // A callback may call pause/stop and then never return. Closing before
    // the join makes this call's AAUDIO_OK mean the microphone is already
    // gone rather than promising cleanup in the callback's exit hook.
    take_capture(shared);
    reset_residue(shared);
    if let Some(d) = driver {
        // A no-op when called from the callback thread, by the driver's own
        // rule — which is what keeps this safe from inside the callback.
        d.join();
    }
    if was_started {
        log(
            Priority::Info,
            AUDIO_TAG,
            &format!(
                "recording {}: capture stream destroyed, not deactivated",
                if to == AAUDIO_STREAM_STATE_PAUSED { "paused" } else { "stopped" }
            ),
        );
    }
}

pub fn request_start(handle: StreamHandle) -> AAudioResult {
    let Some(shared) = stream_for(handle) else {
        return AAUDIO_ERROR_NULL;
    };
    if shared.direction == AAUDIO_DIRECTION_OUTPUT {
        let output_guard = lock(&shared.output);
        let Some(output) = output_guard.as_ref() else {
            return AAUDIO_ERROR_INVALID_STATE;
        };
        if !output.is_open() {
            return AAUDIO_ERROR_INVALID_STATE;
        }
        // Realtime rule: flips one atomic and touches nothing else.
        output.set_running(true);
        shared.state.store(AAUDIO_STREAM_STATE_STARTED, Ordering::Release);
        return AAUDIO_OK;
    }

    let engine = engine_callbacks(&shared);
    if running_on_callback_thread(handle) {
        // From the callback thread: no lifecycle lock, no join — either it is
        // already running or the request is invalid.
        return if shared.state.load(Ordering::Acquire) == AAUDIO_STREAM_STATE_STARTED {
            AAUDIO_OK
        } else {
            AAUDIO_ERROR_INVALID_STATE
        };
    }

    let _lifecycle = lock(&shared.input_lifecycle);
    if engine.data.is_some() {
        // An external requestStart is also a restart barrier: waiting for the
        // old worker prevents its exit hook from closing the newly opened
        // capture after a concurrent callback STOP.
        let old = lock(&shared.driver).take();
        if let Some(d) = old.as_ref() {
            d.cancel();
        }
        if let Some(d) = old {
            d.join();
        }
        take_capture(&shared);
        reset_residue(&shared);
    }

    if capture_is_open(&shared) {
        shared.state.store(AAUDIO_STREAM_STATE_STARTED, Ordering::Release);
        return AAUDIO_OK;
    }

    // **The only place in the runtime that opens a microphone**, and the only
    // place that may be. `requestStart` is AAudio's "I am recording now";
    // everything earlier is preparation and must leave the capture device
    // alone.
    let capture = match capture_backend().open() {
        Ok(capture) => capture,
        Err(error) => {
            // Stays STOPPED with nothing open. A stream that reported OK here
            // would have the engine reading zeroes out of a device it does
            // not hold — the "stub that lies" failure, refused on principle.
            log(
                Priority::Error,
                AUDIO_TAG,
                &format!("requestStart could not open a capture stream ({error}); staying \
                          stopped with no microphone open rather than reporting a recording that \
                          is not happening"),
            );
            shared.state.store(AAUDIO_STREAM_STATE_STOPPED, Ordering::Release);
            return AAUDIO_ERROR_UNAVAILABLE;
        }
    };
    *lock(&shared.capture) = Some(capture);
    reset_residue(&shared);
    shared.state.store(AAUDIO_STREAM_STATE_STARTED, Ordering::Release);
    if engine.data.is_some() && !start_input_driver(&shared, handle, engine) {
        shared.state.store(AAUDIO_STREAM_STATE_STOPPED, Ordering::Release);
        take_capture(&shared);
        log(
            Priority::Error,
            AUDIO_TAG,
            "requestStart could not start the input callback consumer; staying stopped with \
             no microphone open",
        );
        return AAUDIO_ERROR_UNAVAILABLE;
    }
    AAUDIO_OK
}

/// Whether a capture is held. Only ever called with the lifecycle barrier
/// held or before any concurrent path can race it.
fn capture_is_open(shared: &Arc<StreamShared>) -> bool {
    lock(&shared.capture).is_some()
}

pub fn request_pause(handle: StreamHandle) -> AAudioResult {
    let Some(shared) = stream_for(handle) else {
        return AAUDIO_ERROR_NULL;
    };
    if shared.direction == AAUDIO_DIRECTION_OUTPUT {
        if let Some(output) = lock(&shared.output).as_ref() {
            // Never tears down, never takes a lock that matters: silence.
            output.set_running(false);
        }
        shared.state.store(AAUDIO_STREAM_STATE_PAUSED, Ordering::Release);
        return AAUDIO_OK;
    }
    if running_on_callback_thread(handle) {
        stop_capture(&shared, AAUDIO_STREAM_STATE_PAUSED);
    } else {
        let _lifecycle = lock(&shared.input_lifecycle);
        stop_capture(&shared, AAUDIO_STREAM_STATE_PAUSED);
    }
    AAUDIO_OK
}

pub fn request_stop(handle: StreamHandle) -> AAudioResult {
    let Some(shared) = stream_for(handle) else {
        return AAUDIO_ERROR_NULL;
    };
    if shared.direction == AAUDIO_DIRECTION_OUTPUT {
        if let Some(output) = lock(&shared.output).as_ref() {
            output.set_running(false);
        }
        shared.state.store(AAUDIO_STREAM_STATE_STOPPED, Ordering::Release);
        return AAUDIO_OK;
    }
    if running_on_callback_thread(handle) {
        stop_capture(&shared, AAUDIO_STREAM_STATE_STOPPED);
    } else {
        let _lifecycle = lock(&shared.input_lifecycle);
        stop_capture(&shared, AAUDIO_STREAM_STATE_STOPPED);
    }
    AAUDIO_OK
}

pub fn close_stream(handle: StreamHandle) -> AAudioResult {
    let Some(shared) = stream_for(handle) else {
        return AAUDIO_ERROR_NULL;
    };
    if running_on_callback_thread(handle) {
        // AAudio requires another thread for close; refusing keeps the bug
        // visible instead of deleting the callback's stream underneath it.
        log(
            Priority::Error,
            AUDIO_TAG,
            "AAudioStream_close called from inside the data callback; refusing rather than \
             deleting the callback's stream underneath it",
        );
        return AAUDIO_ERROR_INVALID_STATE;
    }
    if shared.direction == AAUDIO_DIRECTION_INPUT {
        let _lifecycle = lock(&shared.input_lifecycle);
        shared.state.store(AAUDIO_STREAM_STATE_CLOSED, Ordering::Release);
        let driver = lock(&shared.driver).take();
        if let Some(d) = driver.as_ref() {
            d.cancel();
        }
        // Close before joining: an engine callback is untrusted and may not
        // return promptly, but it must not extend the microphone's lifetime
        // after another thread asks to close the stream.
        take_capture(&shared);
        if let Some(d) = driver {
            d.join();
        }
        lock(streams()).remove(&handle);
        return AAUDIO_OK;
    }
    // Output: drop the host stream (its owner thread joins internally), then
    // delete.
    lock(&shared.output).take();
    shared.state.store(AAUDIO_STREAM_STATE_CLOSED, Ordering::Release);
    lock(streams()).remove(&handle);
    AAUDIO_OK
}

// ---------------------------------------------------------------------------
// Input driver plumbing
// ---------------------------------------------------------------------------

/// Starts the burst pump. Runs only with the lifecycle barrier held.
fn start_input_driver(
    shared: &Arc<StreamShared>,
    handle: StreamHandle,
    engine: EngineCallbacks,
) -> bool {
    let driver = InputCallbackDriver::new();
    let read: ReadFn = make_read_fn(Arc::clone(shared), handle);
    let deliver = DeliverCtx {
        shared: Arc::clone(shared),
        callbacks: engine,
        handle,
    };
    let data: DataFn = Box::new(move |bytes: &mut [u8], frames: u32| deliver.deliver(bytes, frames));
    let watch = Arc::clone(shared);
    let exited: ExitFn = Box::new(move |callback_stopped: bool| {
        // The exit hook: the capture closes on every way out of the worker,
        // panics included.
        take_capture(&watch);
        if callback_stopped {
            log(
                Priority::Info,
                AUDIO_TAG,
                "input data callback returned AAUDIO_CALLBACK_RESULT_STOP; capture stream \
                 destroyed and recording stopped",
            );
        }
        CALLBACK_THREAD.with(|c| c.set(0));
    });
    match driver.start(
        DECLARED_BYTES_PER_FRAME,
        DECLARED_BURST_FRAMES,
        read,
        data,
        exited,
    ) {
        Ok(()) => {
            *lock(&shared.driver) = Some(driver);
            true
        }
        Err(error) => {
            log(
                Priority::Error,
                AUDIO_TAG,
                &format!("could not start the input callback driver: {error:?}"),
            );
            false
        }
    }
}

/// The realtime-safe reader the driver calls every cycle: check for a failed
/// capture, then pull the declared byte stream. Runs on the driver thread,
/// never on a host audio thread.
fn make_read_fn(shared: Arc<StreamShared>, handle: StreamHandle) -> ReadFn {
    Box::new(move |dst: &mut [u8]| {
        if disconnect_failed_input(&shared, handle) {
            return 0;
        }
        let mut capture = lock(&shared.capture);
        capture.as_mut().map_or(0, |c| c.read(dst))
    })
}

/// The capture backend gave up. Runs on the driver thread, where cancel is
/// deliberately non-joining: the capture is gone before the engine receives
/// the error.
fn disconnect_failed_input(shared: &Arc<StreamShared>, handle: StreamHandle) -> bool {
    let failed = lock(&shared.capture).as_ref().is_some_and(|c| c.failed());
    if !failed {
        return false;
    }
    if shared
        .state
        .compare_exchange(
            AAUDIO_STREAM_STATE_STARTED,
            AAUDIO_STREAM_STATE_DISCONNECTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
    {
        if let Some(d) = lock(&shared.driver).as_ref() {
            d.cancel();
        }
        take_capture(shared);
        reset_residue(shared);
        log(
            Priority::Error,
            AUDIO_TAG,
            "capture backend disconnected; microphone closed and input stream moved to \
             DISCONNECTED",
        );
        let engine = engine_callbacks(shared);
        if let Some(error) = engine.error {
            // SAFETY: the engine's error callback receives the stream handle
            // it opened and its own user_data, per the AAudio contract.
            unsafe { error(handle as *mut c_void, engine.user, AAUDIO_ERROR_DISCONNECTED) };
        }
    }
    true
}

struct DeliverCtx {
    shared: Arc<StreamShared>,
    callbacks: EngineCallbacks,
    handle: StreamHandle,
}

impl DeliverCtx {
    /// One whole burst to the engine. Runs on the driver thread; the engine's
    /// callback may call back into this surface (stop/pause/close), which is
    /// why nothing here holds a lock the callback thread would need.
    fn deliver(&self, bytes: &mut [u8], frames: u32) -> bool {
        CALLBACK_THREAD.with(|c| c.set(self.handle));
        // Muting zeroes what the engine is about to see, after the microphone
        // actually delivered: the capture keeps running, and anything that
        // watches the ring still sees the real world.
        if crate::backend::VOICE_MUTED.load(Ordering::Relaxed) {
            bytes.fill(0);
        }
        let Some(data) = self.callbacks.data else {
            return false;
        };
        // SAFETY: the engine's data callback receives the stream handle it
        // opened, its own user_data, and a buffer of exactly
        // `frames * bytes_per_frame` s16 samples, per the AAudio contract.
        let result = unsafe {
            data(
                self.handle as *mut c_void,
                self.callbacks.user,
                bytes.as_mut_ptr() as *mut c_void,
                frames as i32,
            )
        };
        if result != AAUDIO_CALLBACK_RESULT_CONTINUE {
            let _ = self.shared.state.compare_exchange(
                AAUDIO_STREAM_STATE_STARTED,
                AAUDIO_STREAM_STATE_STOPPED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            return false;
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Blocking read
// ---------------------------------------------------------------------------

/// Blocking capture. Callback capture leaves through `deliver`.
///
/// AAudio's contract, kept exactly: a non-positive timeout is a non-blocking
/// read that returns what is there; a positive one waits up to that long and
/// returns however many frames it got. A short read is not an error, and zero
/// frames is a legitimate answer, never a TIMEOUT code.
pub fn read_stream(
    handle: StreamHandle,
    dst: *mut c_void,
    frames: i32,
    timeout_nanoseconds: i64,
) -> AAudioResult {
    let Some(shared) = stream_for(handle) else {
        return AAUDIO_ERROR_NULL;
    };
    if dst.is_null() {
        return AAUDIO_ERROR_NULL;
    }
    if shared.direction != AAUDIO_DIRECTION_INPUT {
        return AAUDIO_ERROR_UNIMPLEMENTED;
    }
    if engine_callbacks(&shared).data.is_some() {
        return AAUDIO_ERROR_INVALID_STATE;
    }
    if frames < 0 {
        return AAUDIO_ERROR_ILLEGAL_ARGUMENT;
    }
    if frames == 0 {
        return 0;
    }
    // Not started, or started and then stopped: there is no microphone open,
    // and the engine must hear that rather than be handed a quiet room.
    if shared.state.load(Ordering::Acquire) != AAUDIO_STREAM_STATE_STARTED
        || !capture_is_open(&shared)
    {
        return AAUDIO_ERROR_INVALID_STATE;
    }

    let bpf = DECLARED_BYTES_PER_FRAME as usize;
    let want = frames as usize * bpf;
    // SAFETY: the engine supplies a buffer sized for the frames it asked
    // for; everything written stays inside that bound.
    let buf = unsafe { std::slice::from_raw_parts_mut(dst.cast::<u8>(), want) };

    // The partial frame the previous call could not report. Always shorter
    // than one frame and `want` is at least one frame, so this never fills
    // the buffer on its own.
    let mut got = {
        let mut residue = lock(&shared.residue);
        buf[..residue.len].copy_from_slice(&residue.buf[..residue.len]);
        let len = residue.len;
        residue.len = 0;
        len
    };

    let deadline = (timeout_nanoseconds > 0).then(|| Instant::now() + Duration::from_nanos(timeout_nanoseconds as u64));
    loop {
        let n = {
            let mut capture = lock(&shared.capture);
            capture.as_mut().map_or(0, |c| c.read(&mut buf[got..]))
        };
        got += n;
        if got >= want || timeout_nanoseconds <= 0 {
            break;
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            break;
        }
        // A stop from another thread must end the wait rather than let it
        // run out the clock; there will be no more samples, so waiting for
        // them is waiting for nothing.
        if shared.state.load(Ordering::Acquire) != AAUDIO_STREAM_STATE_STARTED {
            break;
        }
        // A millisecond, or whatever is left of the timeout if that is less.
        let remaining = deadline.map_or(1_000_000, |d| {
            d.saturating_duration_since(Instant::now()).as_nanos().min(1_000_000) as u64
        });
        std::thread::sleep(Duration::from_nanos(remaining.max(1)));
    }

    let got_frames = got / bpf;
    let remainder = got % bpf;
    if remainder > 0 && remainder <= 8 {
        let mut residue = lock(&shared.residue);
        residue.buf[..remainder].copy_from_slice(&buf[got_frames * bpf..got]);
        residue.len = remainder;
    }

    // Muting is applied to what is delivered, after reading: the microphone
    // itself still runs, and the desktop sees what it sees.
    if crate::backend::VOICE_MUTED.load(Ordering::Relaxed) {
        buf[..got_frames * bpf].fill(0);
    }

    got_frames as i32
}

// ---------------------------------------------------------------------------
// Getters
// ---------------------------------------------------------------------------

pub fn get_state(handle: StreamHandle) -> AAudioStreamState {
    stream_for(handle).map_or(AAUDIO_STREAM_STATE_CLOSED, |s| s.state.load(Ordering::Acquire))
}

pub fn get_sample_rate(handle: StreamHandle) -> i32 {
    let Some(shared) = stream_for(handle) else {
        return 0;
    };
    if shared.direction == AAUDIO_DIRECTION_INPUT {
        return DECLARED_CAPTURE_RATE as i32;
    }
    lock(&shared.output).as_ref().map_or(0, |o| o.rate_hz() as i32)
}

pub fn get_channel_count(handle: StreamHandle) -> i32 {
    let Some(shared) = stream_for(handle) else {
        return 0;
    };
    if shared.direction == AAUDIO_DIRECTION_INPUT {
        return DECLARED_CAPTURE_CHANNELS as i32;
    }
    lock(&shared.output).as_ref().map_or(0, |o| o.channels() as i32)
}

pub fn get_format(handle: StreamHandle) -> AAudioFormat {
    stream_for(handle).map_or(AAUDIO_FORMAT_INVALID, |s| s.format.load(Ordering::Acquire))
}

pub fn get_frames_per_burst(handle: StreamHandle) -> i32 {
    let Some(shared) = stream_for(handle) else {
        return 0;
    };
    if shared.direction == AAUDIO_DIRECTION_INPUT {
        return DECLARED_BURST_FRAMES as i32;
    }
    lock(&shared.output).as_ref().map_or(0, |o| o.burst_frames() as i32)
}

/// On output, capacity and size are both the burst, and that is the truth
/// rather than a shortcut: there is no buffer between the engine and the host
/// on that path — the host pulls one quantum and the engine fills it in the
/// same call.
///
/// Input is genuinely different and reports so: the capture ring is
/// roblox-runtime's own, so there *is* a buffer, and its size is the honest
/// answer to how far behind a reader may fall before it loses samples.
pub fn get_buffer_capacity_in_frames(handle: StreamHandle) -> i32 {
    let Some(shared) = stream_for(handle) else {
        return 0;
    };
    if shared.direction == AAUDIO_DIRECTION_INPUT {
        return DECLARED_RING_FRAMES as i32;
    }
    get_frames_per_burst(handle)
}

pub fn get_buffer_size_in_frames(handle: StreamHandle) -> i32 {
    get_buffer_capacity_in_frames(handle)
}

/// Returns the size actually in force, which AAudio documents as the correct
/// answer when the request is clamped.
pub fn set_buffer_size_in_frames(handle: StreamHandle, frames: i32) -> i32 {
    let actual = get_buffer_capacity_in_frames(handle);
    if frames != actual {
        log(
            Priority::Info,
            AUDIO_TAG,
            &format!(
                "setBufferSizeInFrames({frames}) -> {actual}: on output the buffer is the host's \
                 quantum with nothing between it and the engine to resize, and on input it is \
                 the capture ring, which is sized in one place"
            ),
        );
    }
    actual
}

/// **This is not Android's xrun count, and saying so is the point.** On
/// output, cycles filled with silence while the stream was running — a flat
/// zero is *not* evidence of a clean run, because the server-side glitch
/// count is not visible here. On input it means something: samples the ring
/// dropped because a reader fell behind, reported in frames.
pub fn get_xrun_count(handle: StreamHandle) -> i32 {
    let Some(shared) = stream_for(handle) else {
        return 0;
    };
    let count = if shared.direction == AAUDIO_DIRECTION_INPUT {
        lock(&shared.capture).as_ref().map_or(0, |c| c.dropped_bytes())
            / u64::from(DECLARED_BYTES_PER_FRAME)
    } else {
        lock(&shared.output).as_ref().map_or(0, |o| o.silence_cycles())
    };
    count.min(i32::MAX as u64) as i32
}

// ---------------------------------------------------------------------------
// Availability (the one-way door)
// ---------------------------------------------------------------------------

/// True only when a host backend can actually open. The engine treats a
/// `true` answer as permanent; this is the gate that keeps that promise
/// honest.
pub fn supports_aaudio() -> bool {
    crate::backend::host_backend_available()
}

/// **Said out loud, because a user whose variable was silently ignored had no
/// way to tell.** Both names, because they answer different questions: what
/// was asked for, and what is actually there.
pub fn announce_backend() {
    let asked = crate::backend::host_backend_name();
    let got = crate::backend::effective_backend_name();
    if asked == got {
        log(Priority::Info, AUDIO_TAG, &format!("host backend: {got}"));
    } else {
        log(
            Priority::Info,
            AUDIO_TAG,
            &format!("host backend: {got} (RBX_RUNTIME_AUDIO_HOST={asked})"),
        );
    }
}

// ---------------------------------------------------------------------------
// Tests: the twelve contract checks, ported from native/aaudio_input_test.cpp
// against a fake backend installed through the same seams the real one uses.
// They share one process-global surface, so they serialize on a mutex and run
// in a fixed order, like the C++ main does.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::capture::CaptureOpenError;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64};
    use std::sync::Condvar;

    /// Serializes the twelve scenarios: the surface is process-global and the
    /// fake capture counts are too.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    // -- the fake backend ---------------------------------------------------

    static ACTIVE_CAPTURES: AtomicU32 = AtomicU32::new(0);
    static STALL_CAPTURE: AtomicBool = AtomicBool::new(false);
    static CAPTURE_FAILED: AtomicBool = AtomicBool::new(false);
    static FAKE_NEXT_BYTE: AtomicU64 = AtomicU64::new(0);

    fn active_capture_streams() -> u32 {
        ACTIVE_CAPTURES.load(Ordering::Acquire)
    }

    struct FakeCapture;

    impl Capture for FakeCapture {
        fn read(&mut self, dst: &mut [u8]) -> usize {
            if STALL_CAPTURE.load(Ordering::Acquire) {
                return 0;
            }
            let first = FAKE_NEXT_BYTE.fetch_add(dst.len() as u64, Ordering::AcqRel);
            for (i, out) in dst.iter_mut().enumerate() {
                *out = (first + i as u64) as u8;
            }
            dst.len()
        }

        fn is_open(&self) -> bool {
            true
        }

        fn failed(&self) -> bool {
            CAPTURE_FAILED.load(Ordering::Acquire)
        }

        fn dropped_bytes(&self) -> u64 {
            0
        }
    }

    impl Drop for FakeCapture {
        fn drop(&mut self) {
            ACTIVE_CAPTURES.fetch_sub(1, Ordering::AcqRel);
        }
    }

    struct FakeCaptureBackend;

    impl super::super::backend::CaptureBackend for FakeCaptureBackend {
        fn open(&self) -> Result<Box<dyn Capture>, CaptureOpenError> {
            // The same single-capture rule the real chokepoint enforces.
            if ACTIVE_CAPTURES
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err(CaptureOpenError::AnotherCaptureLive);
            }
            FAKE_NEXT_BYTE.store(0, Ordering::Release);
            CAPTURE_FAILED.store(false, Ordering::Release);
            Ok(Box::new(FakeCapture))
        }
    }

    struct FakeOutputStream;

    impl OutputStream for FakeOutputStream {
        fn open(&mut self, _: u32, _: bool, _: &str, _: &str, _: FillCallback) -> bool {
            false
        }
        fn close(&mut self) {}
        fn is_open(&self) -> bool {
            false
        }
        fn set_running(&self, _: bool) {}
        fn is_running(&self) -> bool {
            false
        }
        fn rate_hz(&self) -> u32 {
            0
        }
        fn channels(&self) -> u32 {
            0
        }
        fn sample_bits(&self) -> u32 {
            0
        }
        fn sample_is_float(&self) -> bool {
            false
        }
        fn burst_frames(&self) -> u32 {
            0
        }
        fn silence_cycles(&self) -> u64 {
            0
        }
    }

    struct FakeOutputBackend;

    impl super::super::backend::OutputBackend for FakeOutputBackend {
        fn make_output(&self) -> Box<dyn OutputStream> {
            Box::new(FakeOutputStream)
        }
    }

    fn install_fakes() {
        super::super::backend::set_capture_backend(Arc::new(FakeCaptureBackend));
        super::super::backend::set_output_backend(Arc::new(FakeOutputBackend));
    }

    // -- the engine's test harness -----------------------------------------

    #[derive(Clone, Copy, PartialEq)]
    enum RequestKind {
        Stop,
        Pause,
        Close,
    }

    struct Inner {
        calls: u32,
        frames: i32,
        bytes_match: bool,
        result: i32,
        request_from_callback: Option<RequestKind>,
        request_result: i32,
        continue_calls_before_stop: u32,
        first_call_stops_then_continues: bool,
        wait_after_request: bool,
        release_callback: bool,
        errors: u32,
        last_error: i32,
    }

    struct CallbackState {
        inner: Mutex<Inner>,
        changed: Condvar,
    }

    impl CallbackState {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                inner: Mutex::new(Inner {
                    calls: 0,
                    frames: 0,
                    bytes_match: true,
                    result: 1,
                    request_from_callback: None,
                    request_result: i32::MIN,
                    continue_calls_before_stop: 0,
                    first_call_stops_then_continues: false,
                    wait_after_request: false,
                    release_callback: false,
                    errors: 0,
                    last_error: 0,
                }),
                changed: Condvar::new(),
            })
        }
    }

    unsafe extern "C" fn receive(
        stream: *mut c_void,
        user: *mut c_void,
        audio: *mut c_void,
        frames: i32,
    ) -> i32 {
        // SAFETY: the harness passed the engine's stream handle as the first
        // argument and its own state pointer as user_data; both are read back
        // here, and the audio pointer covers exactly frames * 2 s16 bytes.
        let state = unsafe { &*(user as *const CallbackState) };
        let handle = stream as usize;
        let request_result = {
            let inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
            match inner.request_from_callback {
                Some(RequestKind::Stop) => request_stop(handle),
                Some(RequestKind::Pause) => request_pause(handle),
                Some(RequestKind::Close) => close_stream(handle),
                None => i32::MIN,
            }
        };
        // SAFETY: `audio` is the engine's own interleaved s16 buffer for
        // `frames` frames, exactly as the data callback contract delivers
        // it; only its bytes are read here.
        let bytes = unsafe { std::slice::from_raw_parts(audio as *const u8, (frames * 2) as usize) };
        let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.calls += 1;
        let call = inner.calls;
        let mut match_bytes = frames == 480;
        let first = (call as u64 - 1) * 480 * 2;
        for (i, &b) in bytes.iter().enumerate() {
            if b != (first + i as u64) as u8 {
                match_bytes = false;
            }
        }
        inner.frames = frames;
        inner.bytes_match = if call == 1 { match_bytes } else { inner.bytes_match && match_bytes };
        inner.request_result = request_result;
        state.changed.notify_all();
        if inner.wait_after_request {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !inner.release_callback && Instant::now() < deadline {
                let (guard, _) = state
                    .changed
                    .wait_timeout(inner, Duration::from_millis(100))
                    .unwrap_or_else(|p| p.into_inner());
                inner = guard;
            }
        }
        if inner.first_call_stops_then_continues {
            return if call == 1 { 1 } else { 0 };
        }
        if inner.continue_calls_before_stop != 0 && call <= inner.continue_calls_before_stop {
            return AAUDIO_CALLBACK_RESULT_CONTINUE;
        }
        inner.result
    }

    unsafe extern "C" fn receive_error(_stream: *mut c_void, user: *mut c_void, error: i32) {
        // SAFETY: the harness's state pointer, valid for the stream's life.
        let state = unsafe { &*(user as *const CallbackState) };
        let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.errors += 1;
        inner.last_error = error;
        state.changed.notify_all();
    }

    fn wait_for(state: &CallbackState, check: impl Fn(&Inner) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        while !check(&inner) && Instant::now() < deadline {
            let (guard, _) = state
                .changed
                .wait_timeout(inner, Duration::from_millis(50))
                .unwrap_or_else(|p| p.into_inner());
            inner = guard;
        }
        check(&inner)
    }

    fn wait_for_calls(state: &CallbackState, count: u32) -> bool {
        wait_for(state, |i| i.calls >= count)
    }

    fn wait_for_errors(state: &CallbackState, count: u32) -> bool {
        wait_for(state, |i| i.errors >= count)
    }

    fn wait_for_request(state: &CallbackState) -> bool {
        wait_for(state, |i| i.request_result != i32::MIN)
    }

    fn release_callback(state: &CallbackState) {
        let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.release_callback = true;
        state.changed.notify_all();
    }

    /// `open_input` from the C++: an input stream, optionally with callbacks,
    /// whose getters report the declared values while nothing is recording.
    fn open_input(callback: Option<&Arc<CallbackState>>) -> StreamHandle {
        let builder = create_stream_builder();
        builder_set_direction(builder, AAUDIO_DIRECTION_INPUT);
        if let Some(state) = callback {
            builder_set_data_callback(
                builder,
                Some(receive),
                Arc::as_ptr(state) as *mut c_void,
            );
            builder_set_error_callback(
                builder,
                Some(receive_error),
                Arc::as_ptr(state) as *mut c_void,
            );
        }
        let mut stream = None;
        assert_eq!(open_stream(builder, Some(&mut stream)), AAUDIO_OK);
        assert_eq!(delete_stream_builder(builder), AAUDIO_OK);
        let stream = stream.expect("openStream wrote a handle");
        assert_eq!(get_sample_rate(stream), 48_000);
        assert_eq!(get_channel_count(stream), 1);
        assert_eq!(get_format(stream), AAUDIO_FORMAT_PCM_I16);
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_OPEN);
        assert_eq!(active_capture_streams(), 0);
        stream
    }

    fn spin_until_no_capture() {
        let deadline = Instant::now() + Duration::from_secs(2);
        while active_capture_streams() != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn callback_input_delivers_s16_frames_and_callback_stop_releases_capture() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let state = CallbackState::new();
        let stream = open_input(Some(&state));
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert!(wait_for_calls(&state, 1));

        spin_until_no_capture();
        let inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(inner.calls, 1);
        assert_eq!(inner.frames, 480);
        assert!(inner.bytes_match);
        drop(inner);
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_STOPPED);
        assert_eq!(active_capture_streams(), 0);
        assert_eq!(close_stream(stream), AAUDIO_OK);
    }

    #[test]
    fn stop_from_inside_callback_does_not_self_join_or_leak_capture() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let state = CallbackState::new();
        {
            let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.result = AAUDIO_CALLBACK_RESULT_CONTINUE;
            inner.request_from_callback = Some(RequestKind::Stop);
        }
        let stream = open_input(Some(&state));
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert!(wait_for_calls(&state, 1));

        spin_until_no_capture();
        let inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(inner.request_result, AAUDIO_OK);
        assert_eq!(inner.calls, 1);
        drop(inner);
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_STOPPED);
        assert_eq!(active_capture_streams(), 0);
        assert_eq!(close_stream(stream), AAUDIO_OK);
    }

    #[test]
    fn callback_stop_closes_capture_before_a_blocked_callback_returns() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let state = CallbackState::new();
        {
            let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.result = AAUDIO_CALLBACK_RESULT_CONTINUE;
            inner.request_from_callback = Some(RequestKind::Stop);
            inner.wait_after_request = true;
        }
        let stream = open_input(Some(&state));
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert!(wait_for_request(&state));

        let inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(inner.request_result, AAUDIO_OK);
        drop(inner);
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_STOPPED);
        assert_eq!(
            active_capture_streams(),
            0,
            "callback-thread requestStop returned while the microphone remained open"
        );
        release_callback(&state);
        assert_eq!(close_stream(stream), AAUDIO_OK);
    }

    #[test]
    fn callback_pause_closes_capture_before_a_blocked_callback_returns() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let state = CallbackState::new();
        {
            let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.result = AAUDIO_CALLBACK_RESULT_CONTINUE;
            inner.request_from_callback = Some(RequestKind::Pause);
            inner.wait_after_request = true;
        }
        let stream = open_input(Some(&state));
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert!(wait_for_request(&state));

        let inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(inner.request_result, AAUDIO_OK);
        drop(inner);
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_PAUSED);
        assert_eq!(
            active_capture_streams(),
            0,
            "callback-thread requestPause returned while the microphone remained open"
        );
        release_callback(&state);
        assert_eq!(close_stream(stream), AAUDIO_OK);
    }

    #[test]
    fn callback_continue_delivers_multiple_bursts_before_stop() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let state = CallbackState::new();
        {
            let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.continue_calls_before_stop = 3;
        }
        let stream = open_input(Some(&state));
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert!(wait_for_calls(&state, 4));

        spin_until_no_capture();
        let inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(inner.calls, 4);
        assert_eq!(inner.frames, 480);
        assert!(inner.bytes_match);
        drop(inner);
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_STOPPED);
        assert_eq!(active_capture_streams(), 0);
        assert_eq!(close_stream(stream), AAUDIO_OK);
    }

    #[test]
    fn external_close_waits_for_active_callback_then_releases_capture() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let state = CallbackState::new();
        {
            let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.result = AAUDIO_CALLBACK_RESULT_CONTINUE;
            inner.wait_after_request = true;
        }
        let stream = open_input(Some(&state));
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert!(wait_for_calls(&state, 1));
        assert_eq!(active_capture_streams(), 1);

        let close_result = Arc::new(AtomicI32::new(i32::MIN));
        let closer_state = Arc::clone(&close_result);
        let closer = std::thread::spawn(move || {
            closer_state.store(close_stream(stream), Ordering::Release);
        });
        // The capture must be released promptly, while close itself is still
        // waiting for the blocked callback.
        spin_until_no_capture();
        assert_eq!(
            active_capture_streams(),
            0,
            "external close left the microphone open while waiting for the callback"
        );
        assert_eq!(
            close_result.load(Ordering::Acquire),
            i32::MIN,
            "external close deleted a stream while its callback was still active"
        );
        release_callback(&state);
        closer.join().unwrap();

        assert_eq!(close_result.load(Ordering::Acquire), AAUDIO_OK);
        let inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(inner.calls, 1);
        drop(inner);
        assert_eq!(active_capture_streams(), 0);
    }

    #[test]
    fn concurrent_callback_stop_and_restart_cannot_close_the_new_capture() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let state = CallbackState::new();
        {
            let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.first_call_stops_then_continues = true;
            inner.wait_after_request = true;
        }
        let stream = open_input(Some(&state));
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert!(wait_for_calls(&state, 1));

        let restart_result = Arc::new(AtomicI32::new(i32::MIN));
        let restarter_state = Arc::clone(&restart_result);
        let restarter = std::thread::spawn(move || {
            restarter_state.store(request_start(stream), Ordering::Release);
        });
        // The restart must not return while the first callback is still
        // blocked: requestStart waits for the old worker so its exit hook
        // cannot close the new capture.
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            restart_result.load(Ordering::Acquire),
            i32::MIN,
            "requestStart returned before the concurrent callback STOP completed"
        );

        release_callback(&state);
        restarter.join().unwrap();

        assert_eq!(restart_result.load(Ordering::Acquire), AAUDIO_OK);
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_STARTED);
        assert_eq!(
            active_capture_streams(),
            1,
            "the old callback exit closed the newly restarted capture"
        );
        assert_eq!(request_stop(stream), AAUDIO_OK);
        spin_until_no_capture();
        assert_eq!(active_capture_streams(), 0);
        assert_eq!(close_stream(stream), AAUDIO_OK);
    }

    #[test]
    fn capture_failure_stops_callback_and_reports_disconnected() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let state = CallbackState::new();
        {
            let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.result = AAUDIO_CALLBACK_RESULT_CONTINUE;
        }
        let stream = open_input(Some(&state));
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert!(wait_for_calls(&state, 1));

        STALL_CAPTURE.store(true, Ordering::Release);
        CAPTURE_FAILED.store(true, Ordering::Release);
        assert!(
            wait_for_errors(&state, 1),
            "capture failure never reached the AAudio error callback"
        );
        STALL_CAPTURE.store(false, Ordering::Release);

        let inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(inner.last_error, AAUDIO_ERROR_DISCONNECTED);
        drop(inner);
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_DISCONNECTED);
        spin_until_no_capture();
        assert_eq!(active_capture_streams(), 0);
        assert_eq!(close_stream(stream), AAUDIO_OK);
    }

    #[test]
    fn close_from_inside_callback_is_refused_without_deleting_the_live_stream() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let state = CallbackState::new();
        {
            let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.request_from_callback = Some(RequestKind::Close);
        }
        let stream = open_input(Some(&state));
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert!(wait_for_calls(&state, 1));

        spin_until_no_capture();
        let inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(inner.request_result, AAUDIO_ERROR_INVALID_STATE);
        assert_eq!(inner.calls, 1);
        drop(inner);
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_STOPPED);
        assert_eq!(active_capture_streams(), 0);
        assert_eq!(close_stream(stream), AAUDIO_OK);
    }

    #[test]
    fn pause_cancels_capture_and_start_reopens_it() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let state = CallbackState::new();
        {
            let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.result = AAUDIO_CALLBACK_RESULT_CONTINUE;
        }
        let stream = open_input(Some(&state));
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert!(wait_for_calls(&state, 1));
        assert_eq!(request_pause(stream), AAUDIO_OK);
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_PAUSED);
        spin_until_no_capture();
        assert_eq!(active_capture_streams(), 0);

        let first_calls = state.inner.lock().unwrap_or_else(|p| p.into_inner()).calls;
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert!(wait_for_calls(&state, first_calls + 1));
        assert_eq!(request_stop(stream), AAUDIO_OK);
        spin_until_no_capture();
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_STOPPED);
        assert_eq!(active_capture_streams(), 0);
        assert_eq!(close_stream(stream), AAUDIO_OK);
    }

    #[test]
    fn stop_cancels_an_idle_callback_and_releases_capture_promptly() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let state = CallbackState::new();
        {
            let mut inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.result = AAUDIO_CALLBACK_RESULT_CONTINUE;
        }
        STALL_CAPTURE.store(true, Ordering::Release);
        let stream = open_input(Some(&state));
        assert_eq!(request_start(stream), AAUDIO_OK);
        assert_eq!(active_capture_streams(), 1);

        let before = Instant::now();
        assert_eq!(request_stop(stream), AAUDIO_OK);
        let elapsed = before.elapsed();
        STALL_CAPTURE.store(false, Ordering::Release);

        assert!(
            elapsed < Duration::from_millis(250),
            "stop from an external thread must not wait behind a stalled capture ({elapsed:?})"
        );
        let inner = state.inner.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(inner.calls, 0);
        drop(inner);
        assert_eq!(get_state(stream), AAUDIO_STREAM_STATE_STOPPED);
        spin_until_no_capture();
        assert_eq!(active_capture_streams(), 0);
        assert_eq!(close_stream(stream), AAUDIO_OK);
    }

    #[test]
    fn blocking_reads_remain_supported() {
        let _g = lock(&TEST_LOCK);
        install_fakes();
        let stream = open_input(None);
        assert_eq!(request_start(stream), AAUDIO_OK);
        let mut bytes = [0u8; 64];
        // SAFETY: the test supplies the buffer read_stream writes into, sized
        // for the 32 frames requested.
        let got = read_stream(stream, bytes.as_mut_ptr() as *mut c_void, 32, 0);
        assert_eq!(got, 32);
        for (i, &b) in bytes.iter().enumerate() {
            assert_eq!(b, i as u8, "byte {i} of the blocking read");
        }
        assert_eq!(request_stop(stream), AAUDIO_OK);
        spin_until_no_capture();
        assert_eq!(active_capture_streams(), 0);
        assert_eq!(close_stream(stream), AAUDIO_OK);
    }
}
