//! Asks a named host output backend directly and takes two seconds.
//!
//! Ported from `crates/native-sys/native/alsa_backend_probe.cpp` and
//! `crates/native-sys/native/pulse_backend_probe.cpp` — which were byte-for-byte the same probe
//! against two C++ factories — onto one Rust example, since the Rust crate
//! has no per-backend factories: the backend is chosen by
//! `RBX_RUNTIME_AUDIO_HOST`, so the two probes become two invocations of one
//! binary:
//!
//!     cargo run -p roblox-audio --example backend_probe                    # whatever auto picks
//!     RBX_RUNTIME_AUDIO_HOST=alsa       cargo run -p roblox-audio --example backend_probe
//!     RBX_RUNTIME_AUDIO_HOST=pulseaudio cargo run -p roblox-audio --example backend_probe
//!
//! Standalone because waiting for the engine to open audio is not a test:
//! across six launches that reached a signed-in Home page, the engine opened
//! an audio stream on exactly one of them. So a client run with no sound
//! distinguishes nothing — not a broken backend from a working one, and not
//! either from a launch that simply never got that far. This asks the backend
//! directly and takes two seconds.
//!
//! One caveat survives from the ALSA probe: **`default` is not the same as
//! hardware, and the difference matters here.** On a PipeWire machine,
//! ALSA's `default` is PipeWire's ALSA plugin, so a pass proves the
//! sequencing and the writer thread, and does not prove a real device is
//! driven. For that, run against real hardware — which means taking the card
//! away from the session first. The same asymmetry applies to the cpal
//! pipeline: a named backend that ends up opening a Pro Audio node proves
//! the fill loop, not the speakers. Say what a green result shows; it does
//! not show more.
//!
//! It plays a quiet 440 Hz tone at −30 dBFS rather than silence, so the
//! answer is available by ear as well as by counter. `silence_cycles` reads
//! zero on a perfect stream and on one underrunning every cycle, which is
//! exactly why it is printed here but never *decided* on — a frame count
//! that climbs at the sample rate is the measurement; the counter is not.
//!
//! Exit 0 only when more than one second of audio at 48 kHz was generated.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use roblox_audio::backend::{effective_backend_name, host_backend_available, make_output_stream};

static FRAMES: AtomicU64 = AtomicU64::new(0);
static PHASE: AtomicU64 = AtomicU64::new(0);

fn main() {
    println!("backend: {}", effective_backend_name());
    println!(
        "host_backend_available(): {}",
        if host_backend_available() {
            "yes"
        } else {
            "NO"
        }
    );
    if !host_backend_available() {
        std::process::exit(1);
    }

    let mut stream = make_output_stream();
    let fill = Box::new(|dst: &mut [f32], frames: u32| -> bool {
        // A quiet 440 Hz tone at −30 dBFS: audible, not startling. Realtime
        // rule: atomics and arithmetic only.
        let step = 2.0 * std::f64::consts::PI * 440.0 / 48000.0;
        let mut phase = f64::from_bits(PHASE.load(Ordering::Acquire));
        for chunk in dst.chunks_mut(2) {
            let value = (0.03f64 * phase.sin()) as f32;
            chunk[0] = value;
            if chunk.len() > 1 {
                chunk[1] = value;
            }
            phase += step;
            if phase >= 2.0 * std::f64::consts::PI {
                phase -= 2.0 * std::f64::consts::PI;
            }
        }
        PHASE.store(phase.to_bits(), Ordering::Release);
        FRAMES.fetch_add(u64::from(frames), Ordering::AcqRel);
        true
    });
    // No format preference: whatever the device prefers is what it gets.
    if !stream.open(
        roblox_audio::backend::OutputRequest::default_device(0, false, ""),
        "roblox-runtime (backend probe)",
        fill,
    ) {
        println!("open failed");
        std::process::exit(1);
    }
    stream.set_running(true);
    println!(
        "open ok: {} Hz, {} ch, {} bits, float={}, burst {}",
        stream.rate_hz(),
        stream.channels(),
        stream.sample_bits(),
        u8::from(stream.sample_is_float()),
        stream.burst_frames(),
    );

    for i in 1..=5u32 {
        std::thread::sleep(Duration::from_millis(400));
        println!(
            "  t={:.1}s frames={} burst={} silence_cycles={}",
            f64::from(i) * 0.4,
            FRAMES.load(Ordering::Acquire),
            stream.burst_frames(),
            stream.silence_cycles(),
        );
    }

    // Must silence rather than tear down, and must be safe to call from
    // anywhere — the rule the whole file is arranged around.
    stream.set_running(false);
    let before = FRAMES.load(Ordering::Acquire);
    std::thread::sleep(Duration::from_millis(400));
    println!(
        "after set_running(false): engine frames {} -> {} (should not climb), silence_cycles {} (should climb)",
        before,
        FRAMES.load(Ordering::Acquire),
        stream.silence_cycles(),
    );

    stream.close();
    println!("closed; is_open={}", u8::from(stream.is_open()));

    let generated = FRAMES.load(Ordering::Acquire);
    std::process::exit(if generated > 48_000 { 0 } else { 2 });
}
