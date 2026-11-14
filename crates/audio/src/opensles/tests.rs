//! Tests for the OpenSL ES surface. No C++ OpenSL tests existed (the C++
//! probes drove real PipeWire); these pin the behavioral contract that file
//! established, against fakes installed through the same seams the real
//! backends use.
//!
//! The registries and fake backends are process-global, so the scenarios
//! serialize on one mutex and run in a fixed order, like the C++ main did.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::backend::{self, CaptureOpener, OpenSlCapture, OutputFactory};
use super::*;
use crate::backend::capture::CaptureOpenError;
use crate::backend::output::{FillCallback, OutputStream};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Serializes every scenario: the surface is process-global.
static TEST_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Fake backends
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeOutputInner {
    args: Mutex<Option<(u32, bool, String, String)>>,
    fill: Mutex<Option<FillCallback>>,
    running: AtomicBool,
    is_open: AtomicBool,
    closed: AtomicBool,
}

struct FakeOutput {
    inner: Arc<FakeOutputInner>,
}

impl OutputStream for FakeOutput {
    fn open(&mut self, bits: u32, is_float: bool, desc: &str, target: &str, fill: FillCallback) -> bool {
        *lock(&self.inner.args) = Some((bits, is_float, desc.to_owned(), target.to_owned()));
        *lock(&self.inner.fill) = Some(fill);
        self.inner.is_open.store(true, Ordering::Release);
        true
    }
    fn close(&mut self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.is_open.store(false, Ordering::Release);
    }
    fn is_open(&self) -> bool {
        self.inner.is_open.load(Ordering::Acquire)
    }
    fn set_running(&self, running: bool) {
        self.inner.running.store(running, Ordering::Release);
    }
    fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::Acquire)
    }
    fn rate_hz(&self) -> u32 {
        48_000
    }
    fn channels(&self) -> u32 {
        2
    }
    fn sample_bits(&self) -> u32 {
        32
    }
    fn sample_is_float(&self) -> bool {
        true
    }
    fn burst_frames(&self) -> u32 {
        480
    }
    fn silence_cycles(&self) -> u64 {
        0
    }
}

/// Shared by the whole test process; each test clears what it owns.
struct FakeOutputFactoryShared {
    created: Mutex<Vec<Arc<FakeOutputInner>>>,
}

impl OutputFactory for FakeOutputFactoryShared {
    fn make_output(&self) -> Box<dyn OutputStream> {
        let inner = Arc::new(FakeOutputInner::default());
        lock(&self.created).push(Arc::clone(&inner));
        Box::new(FakeOutput { inner })
    }
}

fn output_factory_shared() -> &'static Arc<FakeOutputFactoryShared> {
    static FACTORY: OnceLock<Arc<FakeOutputFactoryShared>> = OnceLock::new();
    FACTORY.get_or_init(|| {
        Arc::new(FakeOutputFactoryShared { created: Mutex::new(Vec::new()) })
    })
}

fn created_outputs() -> Vec<Arc<FakeOutputInner>> {
    lock(&output_factory_shared().created).clone()
}

/// The capture side: one shared counter of opens/lives plus a byte counter
/// the pump drains from.
struct FakeCaptureShared {
    opens: AtomicU32,
    live: AtomicU32,
    fail: AtomicBool,
    next_byte: AtomicU64,
}

impl FakeCaptureShared {
    fn reset(&self) {
        self.opens.store(0, Ordering::Release);
        self.live.store(0, Ordering::Release);
        self.fail.store(false, Ordering::Release);
        self.next_byte.store(0, Ordering::Release);
    }
}

fn capture_shared() -> &'static Arc<FakeCaptureShared> {
    static SHARED: OnceLock<Arc<FakeCaptureShared>> = OnceLock::new();
    SHARED.get_or_init(|| {
        Arc::new(FakeCaptureShared {
            opens: AtomicU32::new(0),
            live: AtomicU32::new(0),
            fail: AtomicBool::new(false),
            next_byte: AtomicU64::new(0),
        })
    })
}

struct FakeSource {
    shared: Arc<FakeCaptureShared>,
}

impl OpenSlCapture for FakeSource {
    fn read(&mut self, dst: &mut [u8]) -> usize {
        let first = self.shared.next_byte.fetch_add(dst.len() as u64, Ordering::AcqRel);
        for (i, out) in dst.iter_mut().enumerate() {
            *out = (first + i as u64) as u8;
        }
        dst.len()
    }
    fn is_open(&self) -> bool {
        true
    }
}

impl Drop for FakeSource {
    fn drop(&mut self) {
        self.shared.live.fetch_sub(1, Ordering::AcqRel);
    }
}

struct FakeOpener {
    shared: Arc<FakeCaptureShared>,
}

impl CaptureOpener for FakeOpener {
    fn open(&self, _rate_hz: u32, _channels: u16) -> Result<Box<dyn OpenSlCapture>, CaptureOpenError> {
        if self.shared.fail.load(Ordering::Acquire) {
            return Err(CaptureOpenError::NoInputDevice);
        }
        self.shared.opens.fetch_add(1, Ordering::AcqRel);
        self.shared.live.fetch_add(1, Ordering::AcqRel);
        Ok(Box::new(FakeSource { shared: Arc::clone(&self.shared) }))
    }
}

fn install_fakes() {
    capture_shared().reset();
    lock(&output_factory_shared().created).clear();
    let _ = backend::set_capture_opener(Arc::new(FakeOpener {
        shared: Arc::clone(capture_shared()),
    }));
    let shared_factory = Arc::clone(output_factory_shared());
    let factory: Arc<dyn OutputFactory> = shared_factory;
    let _ = backend::set_output_factory(factory);
}

fn live_captures() -> u32 {
    capture_shared().live.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// Drain-callback harness: a global record of (queue token, context) pairs.
// ---------------------------------------------------------------------------

fn drains() -> &'static Mutex<Vec<(usize, usize)>> {
    static DRAINS: OnceLock<Mutex<Vec<(usize, usize)>>> = OnceLock::new();
    DRAINS.get_or_init(|| Mutex::new(Vec::new()))
}

unsafe extern "C" fn record_drain(queue: usize, context: *mut c_void) {
    lock(drains()).push((queue, context as usize));
}

/// The destroy target for the destroy-from-inside-its-own-callback scenario.
static DESTROY_TARGET: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" fn destroying_drain(queue: usize, context: *mut c_void) {
    let target = DESTROY_TARGET.load(Ordering::Acquire) as usize;
    if target != 0
        && context as usize == target
        && let Some(recorder) = get_recorder(queue)
    {
        recorder.destroy();
    }
}

fn wait_for(deadline_ms: u64, check: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(deadline_ms);
    while !check() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    check()
}

// ---------------------------------------------------------------------------
// Shorthand builders
// ---------------------------------------------------------------------------

fn pcm_16_stereo(rate_milli: u32) -> SlPcmFormat {
    SlPcmFormat::Pcm {
        num_channels: 2,
        samples_per_sec_milli: rate_milli,
        bits_per_sample: 16,
        container_size: 16,
        channel_mask: 0,
        endianness: 0,
    }
}

fn absq_src(num_buffers: u32) -> SlLocator {
    SlLocator::AndroidSimpleBufferQueue { num_buffers }
}

fn mix_sink(mix: Option<MixHandle>) -> SlLocator {
    SlLocator::OutputMix { mix }
}

fn no_interfaces() -> Vec<SlInterfaceRequest> {
    Vec::new()
}

/// A realized player backed by one fake output.
fn realized_player(num_buffers: u32) -> (EngineHandle, PlayerHandle) {
    let engine = test_engine();
    let mut player = None;
    assert_eq!(
        get_engine(engine).unwrap().create_audio_player(
            Some(&mut player),
            &absq_src(num_buffers),
            &pcm_16_stereo(48_000_000),
            &mix_sink(Some(0)),
            &no_interfaces(),
        ),
        SL_RESULT_SUCCESS
    );
    let player = player.expect("create_audio_player wrote a handle");
    assert_eq!(get_player(player).unwrap().realize(), SL_RESULT_SUCCESS);
    (engine, player)
}

fn player_fill(player: PlayerHandle) -> FillCallback {
    let outputs = created_outputs();
    let inner = outputs.last().expect("realize created an output").clone();
    let fill = lock(&inner.fill).take().expect("open stored the fill callback");
    let _ = player;
    fill
}

fn run_fill(fill: &mut FillCallback, dst: &mut [f32], frames: u32) -> bool {
    fill(dst, frames)
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

#[test]
fn interface_ids_are_distinct_and_compared_by_identity() {
    let _g = lock(&TEST_LOCK);
    assert_eq!(engine(), engine());
    assert_ne!(engine(), play());
    for (a, b) in [
        (engine(), record()),
        (play(), volume()),
        (buffer_queue(), android_simple_buffer_queue()),
        (android_configuration(), output_mix()),
    ] {
        assert_ne!(a, b, "every ID object is distinct");
    }
}

#[test]
fn sl_create_engine_refuses_or_serves_depending_on_the_host_and_validates_requests() {
    let _g = lock(&TEST_LOCK);
    // A required non-engine interface is refused regardless of the host.
    let mut out = None;
    assert_eq!(
        sl_create_engine(
            Some(&mut out),
            0,
            &[SlInterfaceRequest { iid: volume(), required: true }],
        ),
        SL_RESULT_FEATURE_UNSUPPORTED
    );
    assert!(out.is_none());

    // Otherwise the answer is honest either way: an engine when the host has
    // a backend, FEATURE_UNSUPPORTED when it does not — never an engine with
    // nothing behind it.
    let mut out = None;
    let result = sl_create_engine(Some(&mut out), 0, &no_interfaces());
    if result == SL_RESULT_SUCCESS {
        let created = out.expect("success wrote a handle");
        let object = get_engine(created).unwrap();
        assert_eq!(
            object.get_state(None),
            SL_RESULT_PARAMETER_INVALID,
            "a null state pointer is a parameter error"
        );
        let mut state = 0;
        assert_eq!(object.get_state(Some(&mut state)), SL_RESULT_SUCCESS);
        assert_eq!(state, SL_OBJECT_STATE_UNREALIZED);
        assert_eq!(object.realize(SL_BOOLEAN_FALSE), SL_RESULT_SUCCESS);
        assert_eq!(object.get_state(Some(&mut state)), SL_RESULT_SUCCESS);
        assert_eq!(state, SL_OBJECT_STATE_REALIZED);
        assert_eq!(object.get_interface(engine()), Ok(()));
        assert_eq!(
            object.get_interface(play()),
            Err(SL_RESULT_FEATURE_UNSUPPORTED)
        );
        assert_eq!(object.destroy(created), SL_RESULT_SUCCESS);
        assert!(get_engine(created).is_none(), "destroy removed the engine");
    } else {
        assert_eq!(result, SL_RESULT_FEATURE_UNSUPPORTED);
        assert!(out.is_none());
    }
}

#[test]
fn the_engine_stub_creates_and_queries_say_unsupported_or_zero() {
    let _g = lock(&TEST_LOCK);
    let engine = test_engine();
    let object = get_engine(engine).unwrap();

    let mut out = None;
    for result in [
        object.create_led_device(Some(&mut out)),
        object.create_vibra_device(Some(&mut out)),
        object.create_listener(Some(&mut out)),
        object.create_3d_group(Some(&mut out)),
        object.create_metadata_extractor(Some(&mut out)),
        object.create_extension_object(Some(&mut out)),
    ] {
        assert_eq!(result, SL_RESULT_FEATURE_UNSUPPORTED);
        assert!(out.is_none(), "a refused create never leaves an object");
        out = None;
    }

    let mut midi = None;
    assert_eq!(
        object.create_midi_player(Some(&mut midi)),
        SL_RESULT_FEATURE_UNSUPPORTED
    );
    assert!(midi.is_none());

    let mut count = 99;
    assert_eq!(object.query_num_supported_interfaces(Some(&mut count)), SL_RESULT_SUCCESS);
    assert_eq!(count, 0);
    assert_eq!(object.query_supported_interfaces(0), SL_RESULT_PARAMETER_INVALID);
    let mut extensions = 99;
    assert_eq!(
        object.query_num_supported_extensions(Some(&mut extensions)),
        SL_RESULT_SUCCESS
    );
    assert_eq!(extensions, 0);
    assert_eq!(object.query_supported_extension(0), SL_RESULT_PARAMETER_INVALID);
    let mut supported = SL_BOOLEAN_TRUE;
    assert_eq!(
        object.is_extension_supported(Some(&mut supported)),
        SL_RESULT_SUCCESS
    );
    assert_eq!(supported, SL_BOOLEAN_FALSE);
}

#[test]
fn output_mix_lifecycle_and_its_three_methods() {
    let _g = lock(&TEST_LOCK);
    install_fakes();
    let engine = test_engine();
    let object = get_engine(engine).unwrap();

    let mut mix = None;
    assert_eq!(
        object.create_output_mix(
            Some(&mut mix),
            &[SlInterfaceRequest { iid: play(), required: true }],
        ),
        SL_RESULT_FEATURE_UNSUPPORTED
    );
    assert!(mix.is_none());

    // A non-required unknown interface is fine to list.
    let mut mix = None;
    assert_eq!(
        object.create_output_mix(
            Some(&mut mix),
            &[SlInterfaceRequest { iid: play(), required: false }],
        ),
        SL_RESULT_SUCCESS
    );
    let mix = mix.expect("create_output_mix wrote a handle");
    let mix_object = get_mix(mix).unwrap();
    let mut state = 0;
    assert_eq!(mix_object.get_state(Some(&mut state)), SL_RESULT_SUCCESS);
    assert_eq!(state, SL_OBJECT_STATE_UNREALIZED);
    assert_eq!(mix_object.realize(SL_BOOLEAN_FALSE), SL_RESULT_SUCCESS);
    assert_eq!(mix_object.get_interface(output_mix()), Ok(()));
    assert_eq!(
        mix_object.get_interface(volume()),
        Err(SL_RESULT_FEATURE_UNSUPPORTED)
    );

    let mut num_devices = -1;
    assert_eq!(
        mix_object.get_destination_output_device_ids(Some(&mut num_devices)),
        SL_RESULT_SUCCESS
    );
    assert_eq!(num_devices, 0, "one shared destination, described as none");
    assert_eq!(mix_object.re_route(1), SL_RESULT_FEATURE_UNSUPPORTED);
    assert_eq!(mix_object.register_device_change_callback(std::ptr::null_mut(), std::ptr::null_mut()), SL_RESULT_SUCCESS);

    assert_eq!(mix_object.destroy(mix), SL_RESULT_SUCCESS);
    assert!(get_mix(mix).is_none(), "destroy removed the mix");
}

#[test]
fn create_audio_player_validates_its_arguments() {
    let _g = lock(&TEST_LOCK);
    install_fakes();
    let engine = test_engine();
    let object = get_engine(engine).unwrap();

    let pcm_ex_unsupported = SlPcmFormat::PcmEx {
        num_channels: 2,
        sample_rate_milli: 48_000_000,
        bits_per_sample: 32,
        container_size: 32,
        channel_mask: 0,
        endianness: 0,
        representation: 2, // not SIGNED_INT: float PCM is not translated
    };
    let mut player = None;
    assert_eq!(
        object.create_audio_player(
            Some(&mut player),
            &absq_src(2),
            &pcm_ex_unsupported,
            &mix_sink(Some(0)),
            &no_interfaces(),
        ),
        SL_RESULT_CONTENT_UNSUPPORTED
    );
    assert!(player.is_none());

    let mut player = None;
    assert_eq!(
        object.create_audio_player(
            Some(&mut player),
            &SlLocator::Other(SL_DATALOCATOR_OUTPUTMIX),
            &pcm_16_stereo(48_000_000),
            &mix_sink(Some(0)),
            &no_interfaces(),
        ),
        SL_RESULT_CONTENT_UNSUPPORTED
    );

    let mut player = None;
    assert_eq!(
        object.create_audio_player(
            Some(&mut player),
            &absq_src(2),
            &SlPcmFormat::Other(7),
            &mix_sink(Some(0)),
            &no_interfaces(),
        ),
        SL_RESULT_CONTENT_UNSUPPORTED
    );

    // The mix locator itself must be an OutputMix...
    let mut player = None;
    assert_eq!(
        object.create_audio_player(
            Some(&mut player),
            &absq_src(2),
            &pcm_16_stereo(48_000_000),
            &absq_src(2),
            &no_interfaces(),
        ),
        SL_RESULT_CONTENT_UNSUPPORTED
    );

    // ...and name an actual object.
    let mut player = None;
    assert_eq!(
        object.create_audio_player(
            Some(&mut player),
            &absq_src(2),
            &pcm_16_stereo(48_000_000),
            &mix_sink(None),
            &no_interfaces(),
        ),
        SL_RESULT_PARAMETER_INVALID
    );

    // A required interface the player cannot serve.
    let mut player = None;
    assert_eq!(
        object.create_audio_player(
            Some(&mut player),
            &absq_src(2),
            &pcm_16_stereo(48_000_000),
            &mix_sink(Some(0)),
            &[SlInterfaceRequest { iid: record(), required: true }],
        ),
        SL_RESULT_FEATURE_UNSUPPORTED
    );
    assert!(player.is_none());
}

#[test]
fn player_realize_opens_the_output_with_the_caller_format_and_refuses_unmappable_layouts() {
    let _g = lock(&TEST_LOCK);
    install_fakes();
    let engine = test_engine();
    let object = get_engine(engine).unwrap();

    // 44.1 kHz stereo 16-bit, defaulting to two buffers.
    let mut player = None;
    assert_eq!(
        object.create_audio_player(
            Some(&mut player),
            &absq_src(0),
            &pcm_16_stereo(44_100_000),
            &mix_sink(Some(0)),
            &no_interfaces(),
        ),
        SL_RESULT_SUCCESS
    );
    let player = player.expect("create_audio_player wrote a handle");
    let shared = get_player(player).unwrap();
    assert_eq!(shared.rate_hz(), 44_100, "milliHertz divided down");
    assert_eq!(shared.channels(), 2);
    assert_eq!(shared.state(), SL_OBJECT_STATE_UNREALIZED);

    assert_eq!(shared.realize(), SL_RESULT_SUCCESS);
    assert_eq!(shared.state(), SL_OBJECT_STATE_REALIZED);
    let outputs = created_outputs();
    assert_eq!(outputs.len(), 1, "realize opened exactly one output");
    let args = lock(&outputs[0].args).clone().expect("open recorded its arguments");
    assert_eq!(args.0, 16, "the sample-bit hint is the caller's format");
    assert!(!args.1, "the hint is not float");
    let description = &args.2;
    assert!(description.contains("OpenSL"), "the description names the path: {description}");
    assert_eq!(args.3, "", "the sink follows the user's configured device");

    // An unmappable layout is refused at realize, before any device exists.
    let mut bad = None;
    assert_eq!(
        object.create_audio_player(
            Some(&mut bad),
            &absq_src(2),
            &SlPcmFormat::Pcm {
                num_channels: 2,
                samples_per_sec_milli: 48_000_000,
                bits_per_sample: 20,
                container_size: 24,
                channel_mask: 0,
                endianness: 0,
            },
            &mix_sink(Some(0)),
            &no_interfaces(),
        ),
        SL_RESULT_SUCCESS
    );
    let bad = bad.unwrap();
    assert_eq!(
        get_player(bad).unwrap().realize(),
        SL_RESULT_CONTENT_UNSUPPORTED,
        "20-bit samples in a 24-bit container were never mapped and still are not"
    );
    assert_eq!(created_outputs().len(), 1, "the refusal opened no output");

    // Zero channels: same story.
    let mut zero = None;
    assert_eq!(
        object.create_audio_player(
            Some(&mut zero),
            &absq_src(2),
            &SlPcmFormat::Pcm {
                num_channels: 0,
                samples_per_sec_milli: 48_000_000,
                bits_per_sample: 16,
                container_size: 16,
                channel_mask: 0,
                endianness: 0,
            },
            &mix_sink(Some(0)),
            &no_interfaces(),
        ),
        SL_RESULT_SUCCESS
    );
    assert_eq!(
        get_player(zero.unwrap()).unwrap().realize(),
        SL_RESULT_CONTENT_UNSUPPORTED
    );
    assert_eq!(created_outputs().len(), 1);

    // Interface dispatch.
    let shared = get_player(player).unwrap();
    assert_eq!(shared.get_interface(play()), Ok(PlayerIid::Play));
    assert_eq!(
        shared.get_interface(android_simple_buffer_queue()),
        Ok(PlayerIid::AndroidSimpleBufferQueue)
    );
    assert_eq!(shared.get_interface(volume()), Ok(PlayerIid::Volume));
    assert_eq!(
        shared.get_interface(android_configuration()),
        Ok(PlayerIid::AndroidConfiguration)
    );
    assert_eq!(
        shared.get_interface(buffer_queue()),
        Err(SL_RESULT_FEATURE_UNSUPPORTED),
        "BUFFERQUEUE is linked by the engine but never served: Android players use the \
         ANDROIDSIMPLEBUFFERQUEUE extension"
    );

    assert_eq!(shared.destroy(), SL_RESULT_SUCCESS);
    assert!(get_player(player).is_none(), "destroy removed the player");
    assert!(outputs[0].closed.load(Ordering::Acquire), "destroy closed the output");
}

#[test]
fn the_player_queue_drains_in_order_and_reports_each_buffer_once() {
    let _g = lock(&TEST_LOCK);
    install_fakes();
    lock(drains()).clear();
    let (_, player) = realized_player(2);
    let shared = get_player(player).unwrap();
    assert_eq!(
        shared.absq_register_callback(Some(record_drain), 0x1234 as *mut c_void),
        SL_RESULT_SUCCESS
    );

    let buffer_a = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let buffer_b = [9u8, 10, 11, 12, 13, 14, 15, 16];
    // SAFETY: both buffers outlive every drain reported in this test.
    unsafe {
        assert_eq!(shared.enqueue(buffer_a.as_ptr(), 8), SL_RESULT_SUCCESS);
        assert_eq!(shared.enqueue(buffer_b.as_ptr(), 8), SL_RESULT_SUCCESS);
    }
    let (mut count, mut index) = (0, 0);
    assert_eq!(shared.absq_get_state(Some(&mut count), Some(&mut index)), SL_RESULT_SUCCESS);
    assert_eq!(count, 2);
    assert_eq!(index, 2);

    // The queue is full at depth 2.
    // SAFETY: the buffer is valid; the refusal happens before any use.
    unsafe {
        assert_eq!(shared.enqueue(buffer_a.as_ptr(), 8), SL_RESULT_BUFFER_INSUFFICIENT);
    }
    // SAFETY: the null pointer is the point of the check.
    unsafe {
        assert_eq!(shared.enqueue(std::ptr::null(), 8), SL_RESULT_PARAMETER_INVALID);
        assert_eq!(shared.enqueue(buffer_a.as_ptr(), 0), SL_RESULT_PARAMETER_INVALID);
    }

    assert_eq!(shared.set_play_state(SL_PLAYSTATE_PLAYING), SL_RESULT_SUCCESS);
    let mut state = 0;
    assert_eq!(shared.get_play_state(Some(&mut state)), SL_RESULT_SUCCESS);
    assert_eq!(state, SL_PLAYSTATE_PLAYING);
    assert!(
        created_outputs()[0].running.load(Ordering::Acquire),
        "PLAYING activates the pull"
    );

    // Two frames of stereo, 16-bit: 8 bytes of caller PCM — exactly buffer
    // A, which is therefore consumed and drained by this fill. The samples
    // are the caller's little-endian s16 pairs scaled to f32.
    let mut fill = player_fill(player);
    let mut dst = [0f32; 4];
    assert!(run_fill(&mut fill, &mut dst, 2));
    let expected: [f32; 4] = [513.0 / 32768.0, 1027.0 / 32768.0, 1541.0 / 32768.0, 2055.0 / 32768.0];
    for (i, (got, want)) in dst.iter().zip(expected.iter()).enumerate() {
        assert!((got - want).abs() < 1e-6, "sample {i}: {got} vs {want}");
    }
    assert_eq!(
        lock(drains()).clone(),
        vec![(player, 0x1234)],
        "buffer A drained with the queue token and the registered context"
    );

    // The next fill drains B the same way.
    assert!(run_fill(&mut fill, &mut dst, 2));
    assert!(
        (dst[0] - 2569.0 / 32768.0).abs() < 1e-6,
        "the second buffer follows the first: {}",
        dst[0]
    );
    let drained = lock(drains()).clone();
    assert_eq!(
        drained,
        vec![(player, 0x1234), (player, 0x1234)],
        "one drain per buffer, in order"
    );

    // Pause stops the pull without clearing the queue.
    assert_eq!(shared.set_play_state(SL_PLAYSTATE_PAUSED), SL_RESULT_SUCCESS);
    assert!(
        !created_outputs()[0].running.load(Ordering::Acquire),
        "PAUSED deactivates the pull"
    );

    // STOPPED empties the queue as well: the next fill is silence.
    assert_eq!(shared.set_play_state(SL_PLAYSTATE_STOPPED), SL_RESULT_SUCCESS);
    assert_eq!(shared.absq_get_state(Some(&mut count), None), SL_RESULT_SUCCESS);
    assert_eq!(count, 0, "STOPPED abandons the queued buffers");
    lock(drains()).clear();
    let mut dst = [7f32; 4];
    assert!(run_fill(&mut fill, &mut dst, 2));
    assert!(dst.iter().all(|&s| s == 0.0), "silence after stop, not stale bytes");
    assert!(lock(drains()).is_empty(), "abandoned buffers never report a drain");

    assert_eq!(shared.destroy(), SL_RESULT_SUCCESS);
}

#[test]
fn player_volume_and_mute_follow_the_spec_curve() {
    let _g = lock(&TEST_LOCK);
    install_fakes();
    let (_, player) = realized_player(2);
    let shared = get_player(player).unwrap();
    let mut fill = player_fill(player);

    let mut max = 99;
    assert_eq!(shared.get_max_volume_level(Some(&mut max)), SL_RESULT_SUCCESS);
    assert_eq!(max, 0, "0 millibel = unity gain is the ceiling");

    // One full-scale sample to hear through the gain changes. 2 frames of
    // one channel: 4 bytes of caller PCM.
    let data = [0xFFu8, 0x7F, 0x00, 0x00]; // samples 32767, 0
    let mut dst = [0f32; 2];

    // -60 dB: 10^(-6000/2000) = 0.001; the round trip lands within one
    // millibel because f32 powf then f64 log10 truncates, exactly as the
    // C++ static_cast did.
    assert_eq!(shared.set_volume_level(-6000), SL_RESULT_SUCCESS);
    let mut level = 0;
    assert_eq!(shared.get_volume_level(Some(&mut level)), SL_RESULT_SUCCESS);
    assert!(
        (-6000..=-5999).contains(&level),
        "round trip within one millibel: {level}"
    );
    // SAFETY: `data` outlives the drain reported within this test.
    unsafe {
        assert_eq!(shared.enqueue(data.as_ptr(), 4), SL_RESULT_SUCCESS);
    }
    assert!(run_fill(&mut fill, &mut dst, 2));
    assert!(
        (dst[0] - 0.001 * 32767.0 / 32768.0).abs() < 1e-5,
        "-60 dB scaled the sample: {}",
        dst[0]
    );

    // The floor is silence, and reading it back reports the floor.
    assert_eq!(shared.set_volume_level(SL_MILLIBEL_MIN), SL_RESULT_SUCCESS);
    // SAFETY: as above.
    unsafe {
        assert_eq!(shared.enqueue(data.as_ptr(), 4), SL_RESULT_SUCCESS);
    }
    assert!(run_fill(&mut fill, &mut dst, 2));
    assert!(dst.iter().all(|&s| s == 0.0), "the floor is silence, not a negative gain");
    assert_eq!(shared.get_volume_level(Some(&mut level)), SL_RESULT_SUCCESS);
    assert_eq!(level, SL_MILLIBEL_MIN);

    assert_eq!(shared.set_volume_level(0), SL_RESULT_SUCCESS);
    assert_eq!(shared.get_volume_level(Some(&mut level)), SL_RESULT_SUCCESS);
    assert_eq!(level, 0);

    // Mute silences the output but the delivery machinery still runs.
    assert_eq!(shared.set_mute(SL_BOOLEAN_TRUE), SL_RESULT_SUCCESS);
    let mut mute = 0;
    assert_eq!(shared.get_mute(Some(&mut mute)), SL_RESULT_SUCCESS);
    assert_eq!(mute, SL_BOOLEAN_TRUE);
    // SAFETY: as above.
    unsafe {
        assert_eq!(shared.enqueue(data.as_ptr(), 4), SL_RESULT_SUCCESS);
    }
    let mut dst = [1f32; 2];
    assert!(run_fill(&mut fill, &mut dst, 2));
    assert!(dst.iter().all(|&s| s == 0.0), "mute silences the output");

    assert_eq!(shared.set_mute(SL_BOOLEAN_FALSE), SL_RESULT_SUCCESS);
    assert_eq!(shared.get_mute(Some(&mut mute)), SL_RESULT_SUCCESS);
    assert_eq!(mute, SL_BOOLEAN_FALSE);

    // Stereo position is a named feature this backend does not have.
    let mut enabled = SL_BOOLEAN_TRUE;
    assert_eq!(
        shared.enable_stereo_position(SL_BOOLEAN_TRUE),
        SL_RESULT_FEATURE_UNSUPPORTED
    );
    assert_eq!(shared.is_enabled_stereo_position(Some(&mut enabled)), SL_RESULT_SUCCESS);
    assert_eq!(enabled, SL_BOOLEAN_FALSE);
    assert_eq!(shared.set_stereo_position(500), SL_RESULT_FEATURE_UNSUPPORTED);
    let mut position = 99i16;
    assert_eq!(shared.get_stereo_position(Some(&mut position)), SL_RESULT_SUCCESS);
    assert_eq!(position, 0);

    assert_eq!(shared.destroy(), SL_RESULT_SUCCESS);
}

#[test]
fn player_position_is_not_tracked_and_says_so() {
    let _g = lock(&TEST_LOCK);
    install_fakes();
    let (_, player) = realized_player(2);
    let shared = get_player(player).unwrap();

    let mut position = 99;
    assert_eq!(shared.get_position(Some(&mut position)), SL_RESULT_SUCCESS);
    assert_eq!(position, 0, "zero rather than a fabricated estimate");
    let mut duration = 0;
    assert_eq!(shared.get_duration(Some(&mut duration)), SL_RESULT_SUCCESS);
    assert_eq!(duration, SL_TIME_UNKNOWN, "a buffer-queue source has no inherent duration");

    assert_eq!(shared.set_marker_position(500), SL_RESULT_SUCCESS);
    assert_eq!(shared.get_marker_position(Some(&mut position)), SL_RESULT_SUCCESS);
    assert_eq!(position, 500, "the marker is echoed back faithfully");
    assert_eq!(shared.clear_marker_position(), SL_RESULT_SUCCESS);
    assert_eq!(shared.get_marker_position(Some(&mut position)), SL_RESULT_SUCCESS);
    assert_eq!(position, 0);
    assert_eq!(shared.set_position_update_period(250), SL_RESULT_SUCCESS);
    let mut period = 0;
    assert_eq!(shared.get_position_update_period(Some(&mut period)), SL_RESULT_SUCCESS);
    assert_eq!(period, 250);
    assert_eq!(shared.set_callback_events_mask(0xFF), SL_RESULT_SUCCESS);
    let mut mask = 0xFF;
    assert_eq!(shared.get_callback_events_mask(Some(&mut mask)), SL_RESULT_SUCCESS);
    assert_eq!(mask, 0, "interest is accepted; the events structurally never occur");
    assert_eq!(shared.register_play_callback(std::ptr::null_mut(), std::ptr::null_mut()), SL_RESULT_SUCCESS);

    // Android configuration: accepted and discarded.
    assert_eq!(
        shared.set_configuration("androidStreamType", &[3, 0, 0, 0]),
        SL_RESULT_SUCCESS
    );
    let mut value_size = 99;
    assert_eq!(
        shared.get_configuration(Some(&mut value_size)),
        SL_RESULT_PARAMETER_INVALID
    );
    assert_eq!(value_size, 0);
    let mut proxy: *mut c_void = std::ptr::null_mut();
    assert_eq!(
        shared.acquire_java_proxy(Some(&mut proxy)),
        SL_RESULT_FEATURE_UNSUPPORTED
    );
    assert!(proxy.is_null());
    assert_eq!(shared.release_java_proxy(), SL_RESULT_SUCCESS);

    assert_eq!(shared.destroy(), SL_RESULT_SUCCESS);
}

#[test]
fn recorder_validation_refuses_everything_but_default_input_s16() {
    let _g = lock(&TEST_LOCK);
    install_fakes();
    let engine = test_engine();
    let object = get_engine(engine).unwrap();

    let recorder_sink = |num_buffers: u32, format: &SlPcmFormat| -> (SLresult, Option<RecorderHandle>) {
        let mut recorder = None;
        let result = object.create_audio_recorder(
            Some(&mut recorder),
            &SlLocator::IoDevice {
                device_type: SL_IODEVICE_AUDIOINPUT,
                device_id: SL_DEFAULTDEVICEID_AUDIOINPUT,
            },
            &absq_src(num_buffers),
            format,
            &no_interfaces(),
        );
        (result, recorder)
    };

    // The happy path first: 8 kHz mono s16, milliHertz divided down.
    let (result, recorder) = recorder_sink(
        0,
        &SlPcmFormat::Pcm {
            num_channels: 1,
            samples_per_sec_milli: 8_000_000,
            bits_per_sample: 16,
            container_size: 16,
            channel_mask: 0,
            endianness: 0,
        },
    );
    assert_eq!(result, SL_RESULT_SUCCESS);
    let recorder = recorder.expect("create_audio_recorder wrote a handle");
    let shared = get_recorder(recorder).unwrap();
    assert_eq!(shared.rate_hz(), 8_000);
    assert_eq!(shared.channels(), 1);
    assert_eq!(shared.state(), SL_OBJECT_STATE_UNREALIZED);
    assert_eq!(live_captures(), 0, "creating a recorder opens nothing");
    assert_eq!(shared.realize(), SL_RESULT_SUCCESS);
    assert_eq!(shared.state(), SL_OBJECT_STATE_REALIZED);
    assert_eq!(live_captures(), 0, "realizing a recorder still opens nothing");

    assert_eq!(shared.get_interface(record()), Ok(RecorderIid::Record));
    assert_eq!(
        shared.get_interface(android_simple_buffer_queue()),
        Ok(RecorderIid::AndroidSimpleBufferQueue)
    );
    assert_eq!(
        shared.get_interface(android_configuration()),
        Ok(RecorderIid::AndroidConfiguration)
    );
    assert_eq!(
        shared.get_interface(volume()),
        Err(SL_RESULT_FEATURE_UNSUPPORTED)
    );
    assert_eq!(shared.destroy(), SL_RESULT_SUCCESS);
    assert!(get_recorder(recorder).is_none());

    // A named input device is refused, loudly.
    let mut recorder = None;
    assert_eq!(
        object.create_audio_recorder(
            Some(&mut recorder),
            &SlLocator::IoDevice {
                device_type: SL_IODEVICE_AUDIOINPUT,
                device_id: 0x1234_5678,
            },
            &absq_src(2),
            &pcm_16_stereo(8_000_000),
            &no_interfaces(),
        ),
        SL_RESULT_CONTENT_UNSUPPORTED
    );
    assert!(recorder.is_none());

    // Wrong device type, wrong sink locator, non-PCM sink format.
    for (source, sink, format) in [
        (
            SlLocator::IoDevice { device_type: 0x99, device_id: SL_DEFAULTDEVICEID_AUDIOINPUT },
            absq_src(2),
            pcm_16_stereo(8_000_000),
        ),
        (
            SlLocator::IoDevice { device_type: SL_IODEVICE_AUDIOINPUT, device_id: SL_DEFAULTDEVICEID_AUDIOINPUT },
            mix_sink(Some(0)),
            pcm_16_stereo(8_000_000),
        ),
        (
            SlLocator::IoDevice { device_type: SL_IODEVICE_AUDIOINPUT, device_id: SL_DEFAULTDEVICEID_AUDIOINPUT },
            absq_src(2),
            SlPcmFormat::PcmEx {
                num_channels: 1,
                sample_rate_milli: 8_000_000,
                bits_per_sample: 16,
                container_size: 16,
                channel_mask: 0,
                endianness: 0,
                representation: SL_ANDROID_PCM_REPRESENTATION_SIGNED_INT,
            },
        ),
    ] {
        let mut recorder = None;
        assert_eq!(
            object.create_audio_recorder(
                Some(&mut recorder),
                &source,
                &sink,
                &format,
                &no_interfaces(),
            ),
            SL_RESULT_CONTENT_UNSUPPORTED,
            "source {source:?}"
        );
        assert!(recorder.is_none());
    }

    // Anything but 16-in-16 little-endian is refused: capture converts
    // nothing on the way in.
    for (bits, container, endianness) in [(8u32, 8u32, 0u32), (16, 32, 0), (16, 16, SL_BYTEORDER_BIGENDIAN)] {
        let mut recorder = None;
        assert_eq!(
            object.create_audio_recorder(
                Some(&mut recorder),
                &SlLocator::IoDevice {
                    device_type: SL_IODEVICE_AUDIOINPUT,
                    device_id: SL_DEFAULTDEVICEID_AUDIOINPUT,
                },
                &absq_src(2),
                &SlPcmFormat::Pcm {
                    num_channels: 1,
                    samples_per_sec_milli: 8_000_000,
                    bits_per_sample: bits,
                    container_size: container,
                    channel_mask: 0,
                    endianness,
                },
                &no_interfaces(),
            ),
            SL_RESULT_CONTENT_UNSUPPORTED,
            "{bits}-bit samples in a {container}-bit container, endianness {endianness}"
        );
        assert!(recorder.is_none());
    }

    // Zero channels or rate: parameter error.
    let mut recorder = None;
    assert_eq!(
        object.create_audio_recorder(
            Some(&mut recorder),
            &SlLocator::IoDevice {
                device_type: SL_IODEVICE_AUDIOINPUT,
                device_id: SL_DEFAULTDEVICEID_AUDIOINPUT,
            },
            &absq_src(2),
            &SlPcmFormat::Pcm {
                num_channels: 0,
                samples_per_sec_milli: 8_000_000,
                bits_per_sample: 16,
                container_size: 16,
                channel_mask: 0,
                endianness: 0,
            },
            &no_interfaces(),
        ),
        SL_RESULT_PARAMETER_INVALID
    );
    assert!(recorder.is_none());

    // A required interface the recorder cannot serve.
    let mut recorder = None;
    assert_eq!(
        object.create_audio_recorder(
            Some(&mut recorder),
            &SlLocator::IoDevice {
                device_type: SL_IODEVICE_AUDIOINPUT,
                device_id: SL_DEFAULTDEVICEID_AUDIOINPUT,
            },
            &absq_src(2),
            &pcm_16_stereo(8_000_000),
            &[SlInterfaceRequest { iid: play(), required: true }],
        ),
        SL_RESULT_FEATURE_UNSUPPORTED
    );
    assert!(recorder.is_none());
}

#[test]
fn the_recorder_microphone_rule_holds_across_the_whole_state_machine() {
    let _g = lock(&TEST_LOCK);
    install_fakes();
    lock(drains()).clear();
    let engine = test_engine();
    let object = get_engine(engine).unwrap();

    let mut recorder = None;
    assert_eq!(
        object.create_audio_recorder(
            Some(&mut recorder),
            &SlLocator::IoDevice {
                device_type: SL_IODEVICE_AUDIOINPUT,
                device_id: SL_DEFAULTDEVICEID_AUDIOINPUT,
            },
            &absq_src(2),
            &SlPcmFormat::Pcm {
                num_channels: 1,
                samples_per_sec_milli: 1_000_000, // 1 kHz: position maths stay visible
                bits_per_sample: 16,
                container_size: 16,
                channel_mask: 0,
                endianness: 0,
            },
            &no_interfaces(),
        ),
        SL_RESULT_SUCCESS
    );
    let recorder = recorder.unwrap();
    let shared = get_recorder(recorder).unwrap();
    assert_eq!(
        shared.absq_register_callback(Some(record_drain), 0xBEEF as *mut c_void),
        SL_RESULT_SUCCESS
    );

    // RECORDING opens the capture — the only thing that does.
    assert_eq!(
        shared.set_record_state(SL_RECORDSTATE_RECORDING),
        SL_RESULT_SUCCESS
    );
    assert_eq!(live_captures(), 1, "recording holds the device");
    let mut state = 0;
    assert_eq!(shared.get_record_state(Some(&mut state)), SL_RESULT_SUCCESS);
    assert_eq!(state, SL_RECORDSTATE_RECORDING);

    // Two empty buffers for the pump to fill.
    let mut buffer_a = [0u8; 8];
    let mut buffer_b = [0u8; 8];
    // SAFETY: both buffers are owned by this test and outlive the drains.
    unsafe {
        assert_eq!(shared.enqueue(buffer_a.as_mut_ptr(), 8), SL_RESULT_SUCCESS);
        assert_eq!(shared.enqueue(buffer_b.as_mut_ptr(), 8), SL_RESULT_SUCCESS);
    }
    assert!(
        wait_for(2000, || lock(drains()).len() >= 2),
        "the pump never reported both buffers drained"
    );
    assert_eq!(lock(drains()).len(), 2, "no extra drains");
    for (queue, context) in lock(drains()).iter() {
        assert_eq!(*queue, recorder);
        assert_eq!(*context, 0xBEEF);
    }
    // The pump filled the buffers from the counter: A got bytes 0..8.
    for (i, byte) in buffer_a.iter().enumerate() {
        assert_eq!(*byte, i as u8, "buffer A byte {i}");
    }
    for (i, byte) in buffer_b.iter().enumerate() {
        assert_eq!(*byte, (8 + i) as u8, "buffer B byte {i}");
    }

    // A genuinely tracked position: 8 frames at 1 kHz is 8 ms.
    let mut position = 0;
    assert_eq!(shared.get_position(Some(&mut position)), SL_RESULT_SUCCESS);
    assert_eq!(position, 8);

    // PAUSED destroys the capture, it does not deactivate it.
    assert_eq!(shared.set_record_state(SL_RECORDSTATE_PAUSED), SL_RESULT_SUCCESS);
    assert!(
        wait_for(2000, || live_captures() == 0),
        "pausing left the microphone open"
    );
    assert_eq!(shared.get_record_state(Some(&mut state)), SL_RESULT_SUCCESS);
    assert_eq!(state, SL_RECORDSTATE_PAUSED);

    // Recording again reopens it.
    assert_eq!(
        shared.set_record_state(SL_RECORDSTATE_RECORDING),
        SL_RESULT_SUCCESS
    );
    assert_eq!(live_captures(), 1);

    // STOPPED destroys it again.
    assert_eq!(shared.set_record_state(SL_RECORDSTATE_STOPPED), SL_RESULT_SUCCESS);
    assert!(wait_for(2000, || live_captures() == 0));
    assert_eq!(shared.get_record_state(Some(&mut state)), SL_RESULT_SUCCESS);
    assert_eq!(state, SL_RECORDSTATE_STOPPED);

    // Destroy after STOPPED is the ordinary sequence and is not an error.
    assert_eq!(shared.destroy(), SL_RESULT_SUCCESS);
    assert!(get_recorder(recorder).is_none());
}

#[test]
fn recorder_open_failure_stays_stopped_and_never_lies_about_recording() {
    let _g = lock(&TEST_LOCK);
    install_fakes();
    let engine = test_engine();
    let object = get_engine(engine).unwrap();
    let mut recorder = None;
    assert_eq!(
        object.create_audio_recorder(
            Some(&mut recorder),
            &SlLocator::IoDevice {
                device_type: SL_IODEVICE_AUDIOINPUT,
                device_id: SL_DEFAULTDEVICEID_AUDIOINPUT,
            },
            &absq_src(2),
            &pcm_16_stereo(8_000_000),
            &no_interfaces(),
        ),
        SL_RESULT_SUCCESS
    );
    let recorder = recorder.unwrap();
    let shared = get_recorder(recorder).unwrap();

    capture_shared().fail.store(true, Ordering::Release);
    assert_eq!(
        shared.set_record_state(SL_RECORDSTATE_RECORDING),
        SL_RESULT_RESOURCE_ERROR,
        "a failed open is a resource error, not a silent success"
    );
    let mut state = 0;
    assert_eq!(shared.get_record_state(Some(&mut state)), SL_RESULT_SUCCESS);
    assert_eq!(state, SL_RECORDSTATE_STOPPED, "stays stopped");
    assert_eq!(live_captures(), 0);

    capture_shared().fail.store(false, Ordering::Release);
    assert_eq!(shared.set_record_state(99), SL_RESULT_PARAMETER_INVALID);
    assert_eq!(shared.set_duration_limit(1000), SL_RESULT_FEATURE_UNSUPPORTED);
    let mut position = 99;
    assert_eq!(shared.get_position(Some(&mut position)), SL_RESULT_SUCCESS);
    assert_eq!(position, 0);
    assert_eq!(shared.set_callback_events_mask(0xFF), SL_RESULT_SUCCESS);
    let mut mask = 0xFF;
    assert_eq!(shared.get_callback_events_mask(Some(&mut mask)), SL_RESULT_SUCCESS);
    assert_eq!(mask, 0);
    assert_eq!(shared.register_record_callback(std::ptr::null_mut(), std::ptr::null_mut()), SL_RESULT_SUCCESS);

    assert_eq!(shared.destroy(), SL_RESULT_SUCCESS);
}

#[test]
fn recorder_destroy_from_inside_its_own_callback_releases_the_microphone() {
    let _g = lock(&TEST_LOCK);
    install_fakes();
    lock(drains()).clear();
    DESTROY_TARGET.store(0, Ordering::Release);
    let engine = test_engine();
    let object = get_engine(engine).unwrap();
    let mut recorder = None;
    assert_eq!(
        object.create_audio_recorder(
            Some(&mut recorder),
            &SlLocator::IoDevice {
                device_type: SL_IODEVICE_AUDIOINPUT,
                device_id: SL_DEFAULTDEVICEID_AUDIOINPUT,
            },
            &absq_src(2),
            &pcm_16_stereo(8_000_000),
            &no_interfaces(),
        ),
        SL_RESULT_SUCCESS
    );
    let recorder = recorder.unwrap();
    let shared = get_recorder(recorder).unwrap();
    // The context is the handle itself, so the drain knows what to destroy.
    assert_eq!(
        shared.absq_register_callback(Some(destroying_drain), recorder as *mut c_void),
        SL_RESULT_SUCCESS
    );
    DESTROY_TARGET.store(recorder as u64, Ordering::Release);

    assert_eq!(
        shared.set_record_state(SL_RECORDSTATE_RECORDING),
        SL_RESULT_SUCCESS
    );
    let mut buffer = [0u8; 8];
    // SAFETY: the buffer is owned by this test and outlives the drain.
    unsafe {
        assert_eq!(shared.enqueue(buffer.as_mut_ptr(), 8), SL_RESULT_SUCCESS);
    }

    // The drain callback destroys the recorder from the pump thread. The
    // microphone must be released promptly and the test must not hang.
    assert!(
        wait_for(2000, || live_captures() == 0),
        "destroy from the pump thread left the microphone open"
    );
    assert!(
        wait_for(2000, || get_recorder(recorder).is_none()),
        "destroy from the pump thread did not remove the registry entry"
    );

    // The pump keeps its own Arc, so the object outlives this call and the
    // loop ends on its own; nothing here may block on it.
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(live_captures(), 0);
    DESTROY_TARGET.store(0, Ordering::Release);
}

#[test]
fn recorder_absq_contract_matches_the_player_mirror() {
    let _g = lock(&TEST_LOCK);
    install_fakes();
    let engine = test_engine();
    let object = get_engine(engine).unwrap();
    let mut recorder = None;
    assert_eq!(
        object.create_audio_recorder(
            Some(&mut recorder),
            &SlLocator::IoDevice {
                device_type: SL_IODEVICE_AUDIOINPUT,
                device_id: SL_DEFAULTDEVICEID_AUDIOINPUT,
            },
            &absq_src(2),
            &pcm_16_stereo(8_000_000),
            &no_interfaces(),
        ),
        SL_RESULT_SUCCESS
    );
    let recorder = recorder.unwrap();
    let shared = get_recorder(recorder).unwrap();

    let mut buffer_a = [0u8; 8];
    let mut buffer_b = [0u8; 8];
    // SAFETY: the buffers are owned by this test; only the null/zero cases
    // deliberately pass invalid arguments, and those are refused.
    unsafe {
        assert_eq!(shared.enqueue(std::ptr::null_mut(), 8), SL_RESULT_PARAMETER_INVALID);
        assert_eq!(shared.enqueue(buffer_a.as_mut_ptr(), 0), SL_RESULT_PARAMETER_INVALID);
        assert_eq!(shared.enqueue(buffer_a.as_mut_ptr(), 8), SL_RESULT_SUCCESS);
        assert_eq!(shared.enqueue(buffer_b.as_mut_ptr(), 8), SL_RESULT_SUCCESS);
        assert_eq!(
            shared.enqueue(buffer_a.as_mut_ptr(), 8),
            SL_RESULT_BUFFER_INSUFFICIENT
        );
    }
    let (mut count, mut index) = (0, 0);
    assert_eq!(shared.absq_get_state(Some(&mut count), Some(&mut index)), SL_RESULT_SUCCESS);
    assert_eq!(count, 2);
    assert_eq!(index, 2);

    assert_eq!(shared.absq_clear(), SL_RESULT_SUCCESS);
    assert_eq!(shared.absq_get_state(Some(&mut count), Some(&mut index)), SL_RESULT_SUCCESS);
    assert_eq!(count, 0);
    assert_eq!(index, 2, "clear abandons the buffers; the index keeps rising");

    // And the recorder-side marker/period echo, for symmetry with the player.
    let mut position = 0;
    assert_eq!(shared.set_marker_position(500), SL_RESULT_SUCCESS);
    assert_eq!(shared.get_marker_position(Some(&mut position)), SL_RESULT_SUCCESS);
    assert_eq!(position, 500);
    assert_eq!(shared.set_position_update_period(250), SL_RESULT_SUCCESS);
    let mut period = 0;
    assert_eq!(shared.get_position_update_period(Some(&mut period)), SL_RESULT_SUCCESS);
    assert_eq!(period, 250);

    assert_eq!(shared.destroy(), SL_RESULT_SUCCESS);
}
