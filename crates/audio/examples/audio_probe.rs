//! Drives the audio surface the way the engine drives it, so that "audio
//! works" can be a measurement rather than a reading of the code.
//!
//! Replaces the former `crates/native-sys/native/audio_probe.cpp`. What it exercises is the real
//! path and not a friendlier one: the
//! OpenSL objects go through `opensles::{sl_create_engine, get_engine, …}`,
//! the AAudio ones through `aaudio::stream`, and the capture accounting comes
//! from `backend::capture::live_capture_streams()` — the same chokepoint the
//! OpenSL recorder, the AAudio capture and the Java audio classes all funnel
//! through. The vtable-declaration half of the C++ probe (redeclaring Khronos
//! layouts so a layout mistake shows as a crash) has no Rust counterpart:
//! there are no vtables to get wrong here, and the FFI wiring that will cast
//! the `usize` handles into interface pointers is parity-gated in
//! `crates/abi/src/bionic/mod.rs` instead.
//!
//! Every printed line goes to stdout and is flushed immediately, because
//! outside processes (`pw-dump` samplers, `pw-record` captures) line up
//! against these lines as timestamps; a buffered line lies about when the
//! thing it describes happened. The crate's own narration goes to stderr in
//! Android log shape and is secondary.
//!
//! It needs a running audio session and makes real streams appear in it,
//! which is exactly why it is an example and never runs under `cargo test`:
//!
//!     cargo run -p roblox-audio --example audio_probe -- devices
//!     cargo run -p roblox-audio --example audio_probe -- record --seconds 2
//!
//! `play` defaults to an amplitude of 1/512 of full scale — about −54 dBFS,
//! inaudible at any sane desktop volume, but still hundreds of times the
//! noise floor of a sink monitor, which is digitally exact zero when nothing
//! is playing. `--amplitude` exists to raise it; there is no reason to.
//!
//! The five OpenSL scenarios and the three AAudio ones answer questions the
//! unit tests deliberately do not: does an enqueued buffer reach the host's
//! default sink, does the drain callback fire, does the microphone appear and
//! disappear exactly when the state machine says it does, does a second
//! capture owner get refused while a first is recording, and does a recorder
//! stopped from inside its own buffer callback survive. See the per-command
//! comments for what each one decides, and
//! `docs/manual-audio-verification.md` for the checklist that pairs the
//! automated exit codes with the outside observations that cannot be
//! automated.

use std::ffi::c_void;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use roblox_audio::aaudio::stream as aa;
use roblox_audio::backend::capture::{live_capture_streams, CaptureRequest, CaptureStream};
use roblox_audio::backend::enumerate_devices;
use roblox_audio::opensles::{
    get_engine, get_mix, get_player, get_recorder, sl_create_engine, EngineObject,
    PlayerIid, RecorderIid, SlInterfaceRequest, SlLocator,
    SlPcmFormat,
};
use roblox_audio::opensles::{
    android_configuration, android_simple_buffer_queue, engine, play as sl_play, record, volume,
};
use roblox_audio::types::*;

/// One mark: stdout, flushed, on its own line.
macro_rules! mark {
    ($($arg:tt)*) => {{
        println!($($arg)*);
        let _ = std::io::stdout().flush();
    }};
}

const USAGE: &str = "commands: devices | play | silence | record | record-interlock | record-selfstop | aaudio-play | aaudio-record | aaudio-record-never [--seconds N] [--amplitude A] [--hz F]";

struct Flags {
    seconds: f64,
    amplitude: f32,
    hz: f64,
}

fn parse_args(args: &[String]) -> Flags {
    let mut flags = Flags { seconds: 3.0, amplitude: 1.0 / 512.0, hz: 440.0 };
    let mut i = 0;
    while i < args.len() {
        let value = |i: &mut usize| {
            *i += 1;
            args.get(*i).cloned().unwrap_or_default()
        };
        match args[i].as_str() {
            "--seconds" => flags.seconds = value(&mut i).parse().unwrap_or(3.0),
            "--amplitude" => flags.amplitude = value(&mut i).parse().unwrap_or(1.0 / 512.0),
            "--hz" => flags.hz = value(&mut i).parse().unwrap_or(440.0),
            other => {
                eprintln!("unknown argument {other}\n{USAGE}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    flags
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let command = if args.is_empty() {
        "devices".to_owned()
    } else {
        args.remove(0)
    };
    let flags = parse_args(&args);
    let code = match command.as_str() {
        "devices" => cmd_devices(),
        "play" => cmd_play(&flags, false),
        "silence" => cmd_play(&flags, true),
        "record" => cmd_record(&flags),
        "record-interlock" => cmd_record_interlock(),
        "record-selfstop" => cmd_record_selfstop(),
        "aaudio-play" => cmd_aaudio_play(&flags),
        "aaudio-record" => cmd_aaudio_record(&flags),
        "aaudio-record-never" => cmd_aaudio_record_never(),
        other => {
            eprintln!("unknown command {other}\n{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

// ---------------------------------------------------------------------------
// devices
// ---------------------------------------------------------------------------

/// Enumerates the host's devices and checks the privacy invariant on the live
/// session: listing microphones must not open one. Exit 0 when any device was
/// found.
fn cmd_devices() -> i32 {
    let devices = match enumerate_devices() {
        Ok(devices) => devices,
        Err(error) => {
            mark!("enumerate_devices failed: {error}");
            return 1;
        }
    };
    mark!("enumerate_devices returned {} device(s)", devices.len());
    for device in &devices {
        mark!(
            "  id={} {}{} name='{}' description='{}' nick='{}' object.path='{}'",
            device.id,
            if device.is_source { "source" } else { "sink  " },
            if device.is_default { " DEFAULT" } else { "        " },
            device.node_name,
            device.description,
            device.nick,
            device.object_path,
        );
    }
    mark!("live_capture_streams after enumeration = {}", live_capture_streams());
    if devices.is_empty() {
        1
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// shared OpenSL engine setup
// ---------------------------------------------------------------------------

/// `slCreateEngine` + Realize + GetInterface(SL_IID_ENGINE), the same three
/// steps the engine takes. Any failure exits.
fn create_engine() -> Option<Arc<EngineObject>> {
    let mut engine_opt = None;
    let result = sl_create_engine(
        Some(&mut engine_opt),
        0,
        &[SlInterfaceRequest { iid: engine(), required: true }],
    );
    mark!("slCreateEngine -> {result}");
    let object = get_engine(engine_opt?)?;
    if object.realize(SL_BOOLEAN_FALSE) != SL_RESULT_SUCCESS {
        mark!("engine Realize -> failure");
        return None;
    }
    mark!("engine Realize -> {}", SL_RESULT_SUCCESS);
    if object.get_interface(engine()) != Ok(()) {
        mark!("engine GetInterface(SL_IID_ENGINE) -> failure");
        return None;
    }
    mark!("engine GetInterface(SL_IID_ENGINE) -> {}", SL_RESULT_SUCCESS);
    Some(object)
}

// ---------------------------------------------------------------------------
// play / silence
// ---------------------------------------------------------------------------

/// A 440 Hz s16 stereo tone, phase-continuous across its whole length and
/// across the wrap: 240 000 frames is exactly 200 periods of 440 Hz at
/// 48 kHz, so a slice cursor that wraps around the table is inaudibly
/// seamless. Precomputed and leaked once, so the drain callback refills
/// buffers by copying a slice — no allocation, no lock, no syscall on the
/// audio thread beyond the re-enqueue the buffer-queue pattern exists for.
fn build_tone_table(amplitude: f32, hz: f64) -> &'static [u8] {
    const PERIODS: usize = 200;
    const PERIOD_SAMPLES: usize = 1200; // 48 kHz / 440 Hz × 11 = 1200 samples
    let mut bytes = vec![0u8; PERIODS * PERIOD_SAMPLES * 4];
    let step = 2.0 * std::f64::consts::PI * hz / 48000.0;
    let mut phase = 0.0f64;
    for chunk in bytes.as_chunks_mut::<4>().0 {
        let s = (amplitude as f64 * 32767.0) * phase.sin();
        let s = s.clamp(-32768.0, 32767.0) as i16;
        // Both channels get the same sample, as the C++ feeder did.
        chunk[..2].copy_from_slice(&s.to_le_bytes());
        chunk[2..].copy_from_slice(&s.to_le_bytes());
        phase += step;
        if phase >= 2.0 * std::f64::consts::PI {
            phase -= 2.0 * std::f64::consts::PI;
        }
    }
    Box::leak(bytes.into_boxed_slice())
}

static DRAINS: AtomicU64 = AtomicU64::new(0);
/// The next slice of the tone table to hand a re-enqueued buffer, advanced
/// by the drain callback only.
static TONE_SLICE: AtomicU64 = AtomicU64::new(0);
static TABLE_BYTES: AtomicU64 = AtomicU64::new(0);
static BUFFER_BYTES: AtomicU64 = AtomicU64::new(0);
/// The four 10 ms buffers, as leaked addresses (never null after setup).
static TONE_PTRS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static TONE_TABLE: AtomicU64 = AtomicU64::new(0);

/// The buffer-queue pattern worth exercising: the drain callback re-fills and
/// re-enqueues the buffer that just drained, from inside the drain callback,
/// on the audio thread's own stack. The Rust fill wrapper fires drains
/// outside the queue lock precisely so this works. Drains happen in strict
/// round-robin (the initial queue is 0,1,2,3 and each drained buffer is
/// re-enqueued behind the rest), so the Nth drain belongs to buffer (N−1)%4.
unsafe extern "C" fn refill_and_reenqueue(queue: usize, _context: *mut c_void) {
    let n = DRAINS.fetch_add(1, Ordering::AcqRel) + 1;
    let buffer_addr = match TONE_PTRS.get(((n - 1) % 4) as usize) {
        Some(cell) if cell.load(Ordering::Acquire) != 0 => cell.load(Ordering::Acquire),
        _ => return,
    };
    let Some(player) = get_player(queue) else {
        return; // destroyed between drain and re-enqueue; nothing to do
    };
    let table_len = TABLE_BYTES.load(Ordering::Acquire) as usize;
    let buffer_bytes = BUFFER_BYTES.load(Ordering::Acquire) as usize;
    if table_len == 0 || buffer_bytes == 0 || buffer_bytes > table_len {
        return;
    }
    // The next sequential slice of the tone, wrapping around the table.
    let slice = TONE_SLICE.fetch_add(1, Ordering::AcqRel);
    let start = ((slice as usize) * buffer_bytes) % table_len;
    let table_ptr = TONE_TABLE.load(Ordering::Acquire) as *const u8;
    let buffer = buffer_addr as *mut u8;
    // SAFETY: both pointers were leaked at setup, the drained buffer is not
    // being read any more (the reader popped it before reporting the drain),
    // and `start + buffer_bytes <= table_len` by construction.
    unsafe { std::ptr::copy_nonoverlapping(table_ptr.add(start), buffer, buffer_bytes) };
    // SAFETY: the queue token is the player handle; the buffer stays valid
    // until its next drain, per the buffer-queue contract this file exists
    // to exercise.
    unsafe { player.enqueue(buffer, buffer_bytes as u32) };
}

/// The ordinary buffer-queue playback path: 4 × 480-frame buffers of a quiet
/// tone, refilled from inside the drain callback. Success is drains > 0 — the
/// one thing no amount of reading the code establishes.
fn cmd_play(flags: &Flags, silent: bool) -> i32 {
    // `silence` is the same code with the generated amplitude forced to
    // exactly zero; the tone clock keeps running so the window is the same
    // for an outside sampler.
    let amplitude = if silent { 0.0 } else { flags.amplitude };
    let label = if silent { "silence control" } else { "tone" };
    let Some(engine_object) = create_engine() else {
        return 1;
    };

    let mut mix = None;
    let mix_result = engine_object.create_output_mix(Some(&mut mix), &[]);
    mark!("CreateOutputMix -> {mix_result}");
    let Some(mix_handle) = mix else { return 1 };
    let mix_object = get_mix(mix_handle).unwrap();
    if mix_object.realize(SL_BOOLEAN_FALSE) != SL_RESULT_SUCCESS {
        mark!("output mix Realize -> failure");
        return 1;
    }
    mark!("output mix Realize -> {}", SL_RESULT_SUCCESS);

    let mut player = None;
    let create_result = engine_object.create_audio_player(
        Some(&mut player),
        // 4 × 480-frame buffers: the size WebRTC and most Android audio paths
        // use, and small enough that a stalled refill shows up as an underrun
        // rather than being papered over by a long buffer.
        &SlLocator::AndroidSimpleBufferQueue { num_buffers: 4 },
        &SlPcmFormat::Pcm {
            num_channels: 2,
            // milliHertz, as the spec has it.
            samples_per_sec_milli: 48_000 * 1000,
            bits_per_sample: 16,
            container_size: 16,
            channel_mask: 0x1 | 0x2, // FRONT_LEFT | FRONT_RIGHT
            endianness: SL_BYTEORDER_LITTLEENDIAN,
        },
        &SlLocator::OutputMix { mix: Some(mix_handle) },
        &[
            SlInterfaceRequest { iid: android_simple_buffer_queue(), required: true },
            SlInterfaceRequest { iid: volume(), required: true },
            SlInterfaceRequest { iid: android_configuration(), required: false },
        ],
    );
    mark!("CreateAudioPlayer -> {create_result}");
    let Some(player_handle) = player else { return 1 };
    let player_object = get_player(player_handle).unwrap();
    if player_object.realize() != SL_RESULT_SUCCESS {
        mark!("player Realize -> failure");
        return 1;
    }
    mark!("player Realize -> {}", SL_RESULT_SUCCESS);
    if player_object.get_interface(sl_play()) != Ok(PlayerIid::Play) {
        mark!("player GetInterface(SL_IID_PLAY) -> failure");
        return 1;
    }
    mark!("player GetInterface(SL_IID_PLAY) -> {}", SL_RESULT_SUCCESS);
    if player_object.get_interface(android_simple_buffer_queue())
        != Ok(PlayerIid::AndroidSimpleBufferQueue)
    {
        mark!("player GetInterface(SL_IID_ANDROIDSIMPLEBUFFERQUEUE) -> failure");
        return 1;
    }
    mark!("player GetInterface(SL_IID_ANDROIDSIMPLEBUFFERQUEUE) -> {}", SL_RESULT_SUCCESS);

    // The tone table and the four 10 ms buffers, leaked: the audio thread
    // reads them and nothing frees them.
    let table = build_tone_table(amplitude, flags.hz);
    let buffer_bytes = 480 * 4;
    let table_addr = table.as_ptr() as u64;
    TABLE_BYTES.store(table.len() as u64, Ordering::Release);
    BUFFER_BYTES.store(buffer_bytes as u64, Ordering::Release);
    TONE_TABLE.store(table_addr, Ordering::Release);
    TONE_SLICE.store(0, Ordering::Release);
    DRAINS.store(0, Ordering::Release);
    for (i, cell) in TONE_PTRS.iter().enumerate() {
        // SAFETY: the buffer is leaked and every byte is written from the
        // tone table before anything reads it.
        let buffer: *mut u8 =
            Box::leak(vec![0u8; buffer_bytes].into_boxed_slice()).as_mut_ptr();
        cell.store(buffer as u64, Ordering::Release);
        // SAFETY: both pointers valid for `buffer_bytes`; `i * buffer_bytes +
        // buffer_bytes <= table.len()`.
        unsafe {
            std::ptr::copy_nonoverlapping(table.as_ptr().add(i * buffer_bytes), buffer, buffer_bytes);
        }
    }

    player_object.absq_register_callback(Some(refill_and_reenqueue), std::ptr::null_mut());

    // Initial fill and enqueue of all four buffers.
    for cell in &TONE_PTRS {
        // SAFETY: leaked above and filled from the tone table.
        let result = unsafe { player_object.enqueue(cell.load(Ordering::Acquire) as *const u8, buffer_bytes as u32) };
        if result != SL_RESULT_SUCCESS {
            mark!("Enqueue -> {result}");
        }
    }

    mark!("PLAY-BEGIN {label} {:.0} Hz amplitude {:.5}", flags.hz, amplitude);
    let result = player_object.set_play_state(SL_PLAYSTATE_PLAYING);
    mark!("SetPlayState(PLAYING) -> {result}");
    std::thread::sleep(Duration::from_secs_f64(flags.seconds));
    let result = player_object.set_play_state(SL_PLAYSTATE_STOPPED);
    mark!("PLAY-END SetPlayState(STOPPED) -> {result}");
    let drains = DRAINS.load(Ordering::Acquire);
    mark!("drain callbacks fired: {drains}");

    player_object.destroy();
    get_mix(mix_handle).unwrap().destroy(mix_handle);
    if drains > 0 {
        0
    } else {
        1
    }
}

// ---------------------------------------------------------------------------
// record
// ---------------------------------------------------------------------------

static BUFFERS_FILLED: AtomicU64 = AtomicU64::new(0);
static PEAK_SAMPLE: AtomicI32 = AtomicI32::new(0);
static REC_BUFFERS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
const REC_BUFFER_BYTES: usize = 480 * 2; // 480 mono s16 frames

fn note_peak(sample: i16) {
    let peak = sample.abs() as i32;
    PEAK_SAMPLE.fetch_max(peak, Ordering::AcqRel);
}

/// Peak of the buffer that just filled, then re-enqueue: the ordinary fill
/// loop. The pump fills buffers in strict round-robin (the queue was seeded
/// with buffer 0 then 1, and each drained buffer is re-enqueued behind the
/// rest), so the Nth drain belongs to buffer N%2. The peak is a CAS on an
/// atomic, not a dump — "did real samples arrive from a real microphone" is
/// answered by a room's noise floor without anything being recorded that
/// anyone would want to keep.
unsafe extern "C" fn buffer_filled(queue: usize, _context: *mut c_void) {
    let n = BUFFERS_FILLED.fetch_add(1, Ordering::AcqRel);
    let buffer = REC_BUFFERS[(n % 2) as usize].load(Ordering::Acquire) as *mut u8;
    if !buffer.is_null() {
        // SAFETY: the buffer is leaked, was fully written by this same pump
        // thread before the drain fired, and is read here on that thread.
        let bytes = unsafe { std::slice::from_raw_parts(buffer, REC_BUFFER_BYTES) };
        for chunk in bytes.as_chunks::<2>().0 {
            note_peak(i16::from_le_bytes([chunk[0], chunk[1]]));
        }
    }
    let Some(recorder) = get_recorder(queue) else {
        return;
    };
    if buffer.is_null() {
        return;
    }
    // SAFETY: the leaked buffer stays valid until its next drain.
    unsafe { recorder.enqueue(buffer, REC_BUFFER_BYTES as u32) };
}

/// The recorder scenario. The mark lines embed the capture count at every
/// transition because that count *is* the microphone-lifetime rule: it must
/// read 0 realized-not-recording, 1 during RECORDING, 0 after PAUSED, and 0
/// after STOPPED — and an outside `pw-dump` sampler reading this log must
/// never see a capture node in the windows where the count says 0.
///
/// Exit 0 iff the capture count is 0 at the end. The peak line is
/// informational: whether the room was noisy is a fact about the room.
fn cmd_record(flags: &Flags) -> i32 {
    let Some(engine_object) = create_engine() else {
        return 1;
    };

    let mut recorder = None;
    let create_result = engine_object.create_audio_recorder(
        Some(&mut recorder),
        &SlLocator::IoDevice {
            device_type: SL_IODEVICE_AUDIOINPUT,
            device_id: SL_DEFAULTDEVICEID_AUDIOINPUT,
        },
        &SlLocator::AndroidSimpleBufferQueue { num_buffers: 2 },
        &SlPcmFormat::Pcm {
            num_channels: 1,
            samples_per_sec_milli: 48_000 * 1000,
            bits_per_sample: 16,
            container_size: 16,
            channel_mask: 0x4, // FRONT_CENTER
            endianness: SL_BYTEORDER_LITTLEENDIAN,
        },
        &[
            SlInterfaceRequest { iid: android_simple_buffer_queue(), required: true },
            SlInterfaceRequest { iid: android_configuration(), required: false },
        ],
    );
    mark!(
        "CreateAudioRecorder -> {create_result}  (capture streams open: {})",
        live_capture_streams()
    );
    let Some(recorder_handle) = recorder else {
        mark!("MIC-NEVER-OPENED: no recorder object exists, so nothing can open the microphone");
        return 1;
    };
    let recorder_object = get_recorder(recorder_handle).unwrap();

    if recorder_object.realize() != SL_RESULT_SUCCESS {
        mark!("RECORDER-REALIZED Realize -> failure");
        return 1;
    }
    mark!(
        "RECORDER-REALIZED Realize -> {}  (capture streams open: {})",
        SL_RESULT_SUCCESS,
        live_capture_streams()
    );
    if recorder_object.get_interface(record()) != Ok(RecorderIid::Record) {
        mark!("recorder GetInterface(SL_IID_RECORD) -> failure");
        return 1;
    }
    if recorder_object.get_interface(android_simple_buffer_queue())
        != Ok(RecorderIid::AndroidSimpleBufferQueue)
    {
        mark!("recorder GetInterface(SL_IID_ANDROIDSIMPLEBUFFERQUEUE) -> failure");
        return 1;
    }

    // The idle window: realized, not recording. This is the state a "muted
    // but open" backend would be indistinguishable from — except here, where
    // the pause is long enough for a pw-dump sampler to catch it.
    BUFFERS_FILLED.store(0, Ordering::Release);
    PEAK_SAMPLE.store(0, Ordering::Release);
    mark!("IDLE-BEGIN realized, not recording (capture streams open: {})", live_capture_streams());
    std::thread::sleep(Duration::from_millis(3000));
    mark!("IDLE-END (capture streams open: {})", live_capture_streams());

    recorder_object.absq_register_callback(Some(buffer_filled), recorder_handle as *mut c_void);

    // Two empty 480-frame mono s16 buffers, leaked: the pump writes into
    // them and nothing else touches their bytes.
    for cell in &REC_BUFFERS {
        // SAFETY: the buffer is leaked; only the pump writes it, and the
        // drain callback reads it on that same thread.
        let buffer = Box::leak(vec![0u8; REC_BUFFER_BYTES].into_boxed_slice()).as_mut_ptr();
        cell.store(buffer as u64, Ordering::Release);
    }
    // SAFETY: both buffers were leaked above and stay valid for the run.
    unsafe {
        let r = recorder_object.enqueue(REC_BUFFERS[0].load(Ordering::Acquire) as *mut u8, REC_BUFFER_BYTES as u32);
        if r != SL_RESULT_SUCCESS {
            mark!("Enqueue -> {r}");
        }
        let r = recorder_object.enqueue(REC_BUFFERS[1].load(Ordering::Acquire) as *mut u8, REC_BUFFER_BYTES as u32);
        if r != SL_RESULT_SUCCESS {
            mark!("Enqueue -> {r}");
        }
    }

    let result = recorder_object.set_record_state(SL_RECORDSTATE_RECORDING);
    mark!(
        "MIC-OPEN SetRecordState(RECORDING) -> {result}  (capture streams open: {})",
        live_capture_streams()
    );
    std::thread::sleep(Duration::from_secs_f64(flags.seconds));

    let mut state = 0;
    let _ = recorder_object.get_record_state(Some(&mut state));
    mark!("GetRecordState during recording -> {state} (3 == RECORDING)");
    let mut position = 0;
    let _ = recorder_object.get_position(Some(&mut position));
    mark!("GetPosition during recording -> {position} ms");

    let result = recorder_object.set_record_state(SL_RECORDSTATE_PAUSED);
    mark!(
        "MIC-PAUSE SetRecordState(PAUSED) -> {result}  (capture streams open: {})",
        live_capture_streams()
    );
    std::thread::sleep(Duration::from_millis(3000));
    mark!("PAUSED-SETTLED (capture streams open: {})", live_capture_streams());

    let result = recorder_object.set_record_state(SL_RECORDSTATE_RECORDING);
    mark!(
        "MIC-REOPEN SetRecordState(RECORDING) -> {result}  (capture streams open: {})",
        live_capture_streams()
    );
    std::thread::sleep(Duration::from_secs_f64(flags.seconds));

    let result = recorder_object.set_record_state(SL_RECORDSTATE_STOPPED);
    mark!(
        "MIC-CLOSE SetRecordState(STOPPED) -> {result}  (capture streams open: {})",
        live_capture_streams()
    );
    std::thread::sleep(Duration::from_millis(3000));
    mark!("STOPPED-SETTLED (capture streams open: {})", live_capture_streams());

    let filled = BUFFERS_FILLED.load(Ordering::Acquire);
    let peak = PEAK_SAMPLE.load(Ordering::Acquire);
    mark!("buffers filled from the microphone: {filled}, peak sample {peak} of 32767");

    recorder_object.destroy();
    mark!("recorder destroyed (capture streams open: {})", live_capture_streams());

    if live_capture_streams() == 0 {
        0
    } else {
        1
    }
}

// ---------------------------------------------------------------------------
// record-interlock
// ---------------------------------------------------------------------------

/// A second capture owner must be refused while a first is recording, and
/// allowed once the first closes. The first owner is the real OpenSL
/// recorder; the second is a bare `CaptureStream` standing in for the
/// Java-side capture owner (WebRtcAudioRecord's own member), which is what
/// the jnivm hooks will drive once their port lands.
fn cmd_record_interlock() -> i32 {
    let Some(engine_object) = create_engine() else {
        return 1;
    };

    let mut recorder = None;
    let create_result = engine_object.create_audio_recorder(
        Some(&mut recorder),
        &SlLocator::IoDevice {
            device_type: SL_IODEVICE_AUDIOINPUT,
            device_id: SL_DEFAULTDEVICEID_AUDIOINPUT,
        },
        &SlLocator::AndroidSimpleBufferQueue { num_buffers: 2 },
        &SlPcmFormat::Pcm {
            num_channels: 1,
            samples_per_sec_milli: 48_000 * 1000,
            bits_per_sample: 16,
            container_size: 16,
            channel_mask: 0x4,
            endianness: SL_BYTEORDER_LITTLEENDIAN,
        },
        &[SlInterfaceRequest { iid: android_simple_buffer_queue(), required: true }],
    );
    mark!("CreateAudioRecorder -> {create_result}");
    let Some(recorder_handle) = recorder else { return 1 };
    let recorder_object = get_recorder(recorder_handle).unwrap();
    if recorder_object.realize() != SL_RESULT_SUCCESS {
        return 1;
    }
    if recorder_object.get_interface(record()) != Ok(RecorderIid::Record) {
        mark!("no SLRecordItf");
        return 1;
    }

    let result = recorder_object.set_record_state(SL_RECORDSTATE_RECORDING);
    let count = live_capture_streams();
    mark!(
        "FIRST-OPEN (OpenSL) SetRecordState(RECORDING) -> {result}  (capture streams open: {count})"
    );
    if result != SL_RESULT_SUCCESS || count != 1 {
        mark!("FAIL: the first, uncontested capture stream did not open");
        return 1;
    }

    let second = CaptureStream::open(
        "",
        CaptureRequest { sample_rate: Some(48_000), channels: Some(1) },
    );
    let count = live_capture_streams();
    mark!(
        "SECOND-OPEN (bare CaptureStream, first still recording) open() -> {}  (capture streams open: {count})",
        match &second {
            Ok(_) => "ok".to_owned(),
            Err(error) => format!("refused: {error}"),
        }
    );
    if second.is_ok() || count != 1 {
        mark!("FAIL: a second capture stream opened while the first was still recording -- the interlock did not hold");
        recorder_object.destroy();
        return 1;
    }
    mark!("interlock held: second capture stream was refused, first is undisturbed");

    let result = recorder_object.set_record_state(SL_RECORDSTATE_STOPPED);
    mark!(
        "FIRST-CLOSE SetRecordState(STOPPED) -> {result}  (capture streams open: {})",
        live_capture_streams()
    );
    // Give the closed capture's teardown a moment to land.
    std::thread::sleep(Duration::from_millis(500));

    let retry = CaptureStream::open(
        "",
        CaptureRequest { sample_rate: Some(48_000), channels: Some(1) },
    );
    let count = live_capture_streams();
    mark!(
        "SECOND-OPEN-RETRY (first closed) open() -> {}  (capture streams open: {count})",
        match &retry {
            Ok(_) => "ok".to_owned(),
            Err(error) => format!("refused: {error}"),
        }
    );
    let mut ok = retry.is_ok() && count == 1;
    if let Ok(mut second) = retry {
        second.close();
    }
    mark!("second closed (capture streams open: {})", live_capture_streams());

    recorder_object.destroy();
    ok &= live_capture_streams() == 0;

    if ok {
        mark!("PASS: interlock refused concurrent capture, allowed sequential capture");
        0
    } else {
        mark!("FAIL: see the marks above");
        1
    }
}

// ---------------------------------------------------------------------------
// record-selfstop
// ---------------------------------------------------------------------------

/// An OpenSL caller that wants N buffers stops the recorder from inside the
/// buffer callback, which arrives on the pump thread — where a naive stop
/// joins that thread to itself and dies. Exercised here because "it does not
/// crash" is not a property worth asserting from a reading of the code.
fn cmd_record_selfstop() -> i32 {
    let Some(engine_object) = create_engine() else {
        return 1;
    };

    let mut recorder = None;
    let _create_result = engine_object.create_audio_recorder(
        Some(&mut recorder),
        &SlLocator::IoDevice {
            device_type: SL_IODEVICE_AUDIOINPUT,
            device_id: SL_DEFAULTDEVICEID_AUDIOINPUT,
        },
        &SlLocator::AndroidSimpleBufferQueue { num_buffers: 2 },
        &SlPcmFormat::Pcm {
            num_channels: 1,
            samples_per_sec_milli: 48_000 * 1000,
            bits_per_sample: 16,
            container_size: 16,
            channel_mask: 0x4,
            endianness: SL_BYTEORDER_LITTLEENDIAN,
        },
        &[SlInterfaceRequest { iid: android_simple_buffer_queue(), required: true }],
    );
    let Some(recorder_handle) = recorder else { return 1 };
    let recorder_object = get_recorder(recorder_handle).unwrap();
    if recorder_object.realize() != SL_RESULT_SUCCESS {
        return 1;
    }
    let _ = recorder_object.get_interface(record());

    recorder_object.absq_register_callback(Some(selfstop_callback), recorder_handle as *mut c_void);

    for cell in &SELFSTOP_BUFFERS {
        // SAFETY: the buffer is leaked; only the pump writes it, and the
        // callback reads it on that same thread.
        let buffer = Box::leak(vec![0u8; REC_BUFFER_BYTES].into_boxed_slice()).as_mut_ptr();
        cell.store(buffer as u64, Ordering::Release);
    }
    SELFSTOP_COUNT.store(0, Ordering::Release);
    SELFSTOP_DONE.store(false, Ordering::Release);
    SELFSTOP_SURVIVED.store(false, Ordering::Release);
    // SAFETY: both buffers were leaked above and stay valid for the run.
    unsafe {
        let _ = recorder_object.enqueue(
            SELFSTOP_BUFFERS[0].load(Ordering::Acquire) as *mut u8,
            REC_BUFFER_BYTES as u32,
        );
        let _ = recorder_object.enqueue(
            SELFSTOP_BUFFERS[1].load(Ordering::Acquire) as *mut u8,
            REC_BUFFER_BYTES as u32,
        );
    }
    let _ = recorder_object.set_record_state(SL_RECORDSTATE_RECORDING);
    mark!("SELFSTOP recording; will stop and destroy from the 20th buffer callback");

    let deadline = Instant::now() + Duration::from_millis(10_000);
    while !SELFSTOP_DONE.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    // Let the detached pump unwind and release its own Arc.
    std::thread::sleep(Duration::from_millis(1000));

    let count = SELFSTOP_COUNT.load(Ordering::Acquire);
    let captures = live_capture_streams();
    mark!("SELFSTOP survived: {count} buffers, capture streams open: {captures}");
    if SELFSTOP_SURVIVED.load(Ordering::Acquire) && captures == 0 {
        0
    } else {
        1
    }
}

static SELFSTOP_COUNT: AtomicU64 = AtomicU64::new(0);
static SELFSTOP_DONE: AtomicBool = AtomicBool::new(false);
static SELFSTOP_SURVIVED: AtomicBool = AtomicBool::new(false);
static SELFSTOP_BUFFERS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static SELFSTOP_NEXT: AtomicU64 = AtomicU64::new(0);

/// The 20th fill stops and destroys the recorder from inside its own
/// callback, on the pump thread. The Rust recorder's destroy has the same
/// on-pump-thread branch the C++ one does: registry removal happens
/// immediately, the pump's own Arc keeps the object alive until the loop
/// exits, and nothing joins itself.
unsafe extern "C" fn selfstop_callback(queue: usize, _context: *mut c_void) {
    let n = SELFSTOP_COUNT.fetch_add(1, Ordering::AcqRel) + 1;
    if n == 20 {
        mark!("SELFSTOP calling SetRecordState(STOPPED) from inside the buffer callback");
        let Some(recorder) = get_recorder(queue) else {
            return;
        };
        let result = recorder.set_record_state(SL_RECORDSTATE_STOPPED);
        mark!(
            "SELFSTOP SetRecordState(STOPPED) returned {result}  (capture streams open: {})",
            live_capture_streams()
        );
        mark!("SELFSTOP calling Destroy from inside the same callback");
        recorder.destroy();
        mark!(
            "SELFSTOP Destroy returned  (capture streams open: {})",
            live_capture_streams()
        );
        SELFSTOP_DONE.store(true, Ordering::Release);
        SELFSTOP_SURVIVED.store(true, Ordering::Release);
        return;
    }
    if n > 20 {
        return; // recorder is gone; nothing to re-enqueue into
    }
    let Some(recorder) = get_recorder(queue) else { return };
    let buffer = SELFSTOP_BUFFERS
        .get((SELFSTOP_NEXT.fetch_add(1, Ordering::AcqRel) as usize) % 2)
        .map(|cell| cell.load(Ordering::Acquire))
        .unwrap_or(0);
    if buffer == 0 {
        return;
    }
    // SAFETY: leaked buffers, pump-written, valid until the next drain.
    unsafe { recorder.enqueue(buffer as *mut u8, REC_BUFFER_BYTES as u32) };
}

// ---------------------------------------------------------------------------
// AAudio
// ---------------------------------------------------------------------------

/// `aaudio_drain`: read bursts until a deadline, return the peak of what came
/// back as a fraction of full scale, and accumulate frames. −1.0 means the
/// read path failed. 100 ms timeout: far longer than the 10 ms burst asks
/// for, so a read that comes back short is a read that had nothing to give
/// rather than one that was not waited for.
fn aaudio_drain(stream: aa::StreamHandle, seconds: f64, burst: i32, frames: &mut u64) -> f64 {
    let mut peak = 0.0f64;
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    let mut buffer = vec![0i16; burst.max(1) as usize * 2];
    while Instant::now() < deadline {
        // The read target is ours and outlives the call.
        let got =
            aa::read_stream(stream, buffer.as_mut_ptr() as *mut c_void, burst, 100_000_000);
        if got < 0 {
            mark!("READ FAILED -> {got}");
            return -1.0;
        }
        let samples = got as usize * 2;
        for &sample in &buffer[..samples] {
            let value = (sample.abs() as f64) / 32768.0;
            if value > peak {
                peak = value;
            }
        }
        *frames += got as u64;
    }
    peak
}

/// The AAudio capture lifetime, end to end: nothing before requestStart,
/// gone on pause/stop/close, frames flowing while started, reads refused
/// outside the started state. The SIGNAL line is reported separately and
/// deliberately not folded into pass/fail — a quiet room is not a bug, and a
/// *connected* stream reads exactly 0.00000 when its source is silent, so a
/// zero answers nothing about whether samples are being carried.
fn cmd_aaudio_record(flags: &Flags) -> i32 {
    aa::announce_backend();
    let mut ok = true;

    let builder = aa::create_stream_builder();
    mark!("AAudio_createStreamBuilder -> {AAUDIO_OK}");
    aa::builder_set_direction(builder, AAUDIO_DIRECTION_INPUT);
    aa::builder_set_format(builder, AAUDIO_FORMAT_UNSPECIFIED);
    // What a voice chat asks for on Android. Nothing downstream reads it
    // yet; it is set so the openStream log line shows a realistic request
    // rather than a zero.
    aa::builder_set_input_preset(builder, AAUDIO_INPUT_PRESET_VOICE_COMMUNICATION);

    let mut stream = None;
    let result = aa::open_stream(builder, Some(&mut stream));
    let _ = aa::delete_stream_builder(builder);
    mark!(
        "openStream(INPUT) -> {result}  (capture streams open: {})",
        live_capture_streams()
    );
    let Some(stream) = stream else { return 1 };
    if result != AAUDIO_OK {
        return 1;
    }
    if live_capture_streams() != 0 {
        mark!("FAIL openStream opened a microphone; it must not");
        ok = false;
    }

    mark!(
        "reported: {} Hz, {} channel(s), format {} (1 == PCM_I16), burst {} frames, capacity {} frames, state {}",
        aa::get_sample_rate(stream),
        aa::get_channel_count(stream),
        aa::get_format(stream),
        aa::get_frames_per_burst(stream),
        aa::get_buffer_capacity_in_frames(stream),
        aa::get_state(stream),
    );

    // Read before start: INVALID_STATE, never frames.
    let mut probe = [0i16; 64];
    let before = aa::read_stream(stream, probe.as_mut_ptr() as *mut c_void, 32, 0);
    mark!("read before requestStart -> {before} (must be negative: -895 is INVALID_STATE)");
    if before >= 0 {
        mark!("FAIL a stream that is not recording returned frames");
        ok = false;
    }

    mark!("IDLE-BEGIN opened, not started (capture streams open: {})", live_capture_streams());
    std::thread::sleep(Duration::from_millis(3000));
    mark!("IDLE-END (capture streams open: {})", live_capture_streams());
    if live_capture_streams() != 0 {
        mark!("FAIL a microphone appeared while the stream was merely open");
        ok = false;
    }

    let result = aa::request_start(stream);
    let count = live_capture_streams();
    mark!("MIC-OPEN requestStart -> {result}  (capture streams open: {count})");
    if result != AAUDIO_OK || count != 1 {
        mark!("FAIL requestStart did not open exactly one capture stream");
        ok = false;
    }

    let burst = aa::get_frames_per_burst(stream);
    let mut frames = 0u64;
    let peak = aaudio_drain(stream, flags.seconds, burst, &mut frames);
    mark!(
        "RECORDED {frames} frame(s) in {:.1} s, peak {peak:.5} of full scale, xruns {}",
        flags.seconds,
        aa::get_xrun_count(stream)
    );
    if peak < 0.0 {
        ok = false;
    }
    if (frames as f64) < flags.seconds * 48000.0 / 2.0 {
        mark!("FAIL far fewer frames than the reported rate implies");
        ok = false;
    }
    mark!("SIGNAL peak {peak:.5} of full scale (0.00000 means the source was silent, which is not the same as the path working)");

    let result = aa::request_pause(stream);
    mark!(
        "MIC-PAUSE requestPause -> {result}  (capture streams open: {})",
        live_capture_streams()
    );
    std::thread::sleep(Duration::from_millis(3000));
    mark!("PAUSED-SETTLED (capture streams open: {})", live_capture_streams());
    if live_capture_streams() != 0 {
        mark!("FAIL pause left the microphone open");
        ok = false;
    }

    let result = aa::request_start(stream);
    mark!(
        "MIC-REOPEN requestStart -> {result}  (capture streams open: {})",
        live_capture_streams()
    );
    let mut frames = 0u64;
    let peak = aaudio_drain(stream, flags.seconds, burst, &mut frames);
    mark!("RE-RECORDED {frames} frame(s), peak {peak:.5} of full scale");
    if peak < 0.0 || frames == 0 {
        mark!("FAIL nothing came back after re-starting");
        ok = false;
    }

    let result = aa::request_stop(stream);
    mark!(
        "MIC-STOP requestStop -> {result}  (capture streams open: {})",
        live_capture_streams()
    );
    std::thread::sleep(Duration::from_millis(3000));
    mark!("STOPPED-SETTLED (capture streams open: {})", live_capture_streams());
    if live_capture_streams() != 0 {
        mark!("FAIL stop left the microphone open");
        ok = false;
    }

    let after = aa::read_stream(stream, probe.as_mut_ptr() as *mut c_void, 32, 0);
    mark!("read after requestStop -> {after} (must be negative)");
    if after >= 0 {
        mark!("FAIL a stopped stream returned frames");
        ok = false;
    }

    let result = aa::close_stream(stream);
    mark!(
        "MIC-CLOSED close -> {result}  (capture streams open: {})",
        live_capture_streams()
    );
    if live_capture_streams() != 0 {
        mark!("FAIL close left the microphone open");
        ok = false;
    }

    if ok {
        mark!("AAUDIO-CAPTURE-LIFETIME PASS");
        0
    } else {
        mark!("AAUDIO-CAPTURE-LIFETIME FAIL");
        1
    }
}

/// The output half, so a change to the capture side cannot quietly break
/// playback without anything noticing. A data callback generates the tone and
/// the stream pulls it, the same path FMOD's callback takes. Success is the
/// callback cadence: within 5% of the negotiated rate.
fn cmd_aaudio_play(flags: &Flags) -> i32 {
    aa::announce_backend();
    let builder = aa::create_stream_builder();
    // Direction defaults to OUTPUT.
    aa::builder_set_data_callback(builder, Some(tone_callback), std::ptr::null_mut());

    let mut stream = None;
    let result = aa::open_stream(builder, Some(&mut stream));
    let _ = aa::delete_stream_builder(builder);
    mark!("openStream(OUTPUT, dataCallback) -> {result}");
    let Some(stream) = stream else { return 1 };
    if result != AAUDIO_OK {
        return 1;
    }

    let rate = aa::get_sample_rate(stream);
    let channels = aa::get_channel_count(stream);
    let step = if rate > 0 {
        2.0 * std::f64::consts::PI * flags.hz / f64::from(rate)
    } else {
        2.0 * std::f64::consts::PI * flags.hz / 48000.0
    };
    PLAY_AMPLITUDE.store(flags.amplitude.to_bits(), Ordering::Release);
    PLAY_STEP.store(step.to_bits(), Ordering::Release);
    PLAY_FRAMES.store(0, Ordering::Release);
    mark!(
        "negotiated {rate} Hz, {channels} channel(s), format {} (2 == PCM_FLOAT), burst {}, capacity {}",
        aa::get_format(stream),
        aa::get_frames_per_burst(stream),
        aa::get_buffer_capacity_in_frames(stream),
    );

    let result = aa::request_start(stream);
    mark!("requestStart -> {result}");
    std::thread::sleep(Duration::from_secs_f64(flags.seconds));
    let frames = PLAY_FRAMES.load(Ordering::Acquire);
    mark!(
        "PLAYED {frames} frame(s) in {:.1} s ({:.0} a second; the negotiated rate is {rate}), xruns {}",
        flags.seconds,
        frames as f64 / flags.seconds,
        aa::get_xrun_count(stream),
    );
    let _ = aa::request_stop(stream);
    let result = aa::close_stream(stream);
    mark!("close -> {result}");

    let ok = rate > 0
        && frames > 0
        && ((frames as f64 / flags.seconds - f64::from(rate)).abs()
            < f64::from(rate) * 0.05);
    if ok {
        mark!("AAUDIO-PLAYBACK PASS");
        0
    } else {
        mark!("AAUDIO-PLAYBACK FAIL");
        1
    }
}

static PLAY_FRAMES: AtomicU64 = AtomicU64::new(0);
static PLAY_AMPLITUDE: AtomicU32 = AtomicU32::new(0);
static PLAY_STEP: AtomicU64 = AtomicU64::new(0);
static PLAY_PHASE: AtomicU64 = AtomicU64::new(0);

/// f32 output, both channels the same sample, phase wrapping at 2π. Realtime
/// rule: atomics and arithmetic only.
unsafe extern "C" fn tone_callback(
    _user: *mut c_void,
    _stream: *mut c_void,
    data: *mut c_void,
    frames: i32,
) -> AAudioDataCallbackResult {
    let amplitude = f32::from_bits(PLAY_AMPLITUDE.load(Ordering::Acquire));
    let step = f64::from_bits(PLAY_STEP.load(Ordering::Acquire));
    let mut phase = f64::from_bits(PLAY_PHASE.load(Ordering::Acquire));
    // SAFETY: `data` is the engine's own interleaved f32 buffer for `frames`
    // frames, as the data-callback contract delivers it.
    let out = unsafe { std::slice::from_raw_parts_mut(data as *mut f32, frames.max(0) as usize) };
    for chunk in out.as_chunks_mut::<2>().0 {
        let value = (amplitude as f64 * phase.sin()) as f32;
        chunk[0] = value;
        chunk[1] = value;
        phase += step;
        if phase >= 2.0 * std::f64::consts::PI {
            phase -= 2.0 * std::f64::consts::PI;
        }
    }
    PLAY_PHASE.store(phase.to_bits(), Ordering::Release);
    PLAY_FRAMES.fetch_add(frames.max(0) as u64, Ordering::AcqRel);
    AAUDIO_CALLBACK_RESULT_CONTINUE
}

/// The case that matters most and is easiest to get wrong: a stream that is
/// opened and closed without ever recording must never have touched the
/// microphone, and must not leak one on the way out.
fn cmd_aaudio_record_never() -> i32 {
    aa::announce_backend();
    let builder = aa::create_stream_builder();
    aa::builder_set_direction(builder, AAUDIO_DIRECTION_INPUT);
    aa::builder_set_format(builder, AAUDIO_FORMAT_PCM_I16);

    let mut stream = None;
    let result = aa::open_stream(builder, Some(&mut stream));
    let _ = aa::delete_stream_builder(builder);
    let Some(stream) = stream else { return 1 };
    if result != AAUDIO_OK {
        return 1;
    }

    mark!("NEVER-RECORD opened (capture streams open: {})", live_capture_streams());
    std::thread::sleep(Duration::from_millis(2000));
    let during = live_capture_streams();
    let result = aa::close_stream(stream);
    let after = live_capture_streams();
    mark!("NEVER-RECORD closed (during: {during}, after: {after})");

    let ok = result == AAUDIO_OK && during == 0 && after == 0;
    if ok {
        mark!("MIC-NEVER-OPENED PASS");
        0
    } else {
        mark!("MIC-NEVER-OPENED FAIL");
        1
    }
}
