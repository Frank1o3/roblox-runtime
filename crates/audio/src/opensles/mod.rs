//! OpenSL ES: the surface the engine resolves through `libOpenSLES.so`.
//!
//! `libOpenSLES.so` was once listed among the empty libraries on the basis
//! that the engine consulted it only through `dlsym`, if at all. That was
//! true of the build this runtime was first developed against; current builds
//! reference eight OpenSL symbols directly:
//!
//! ```text
//! slCreateEngine
//! SL_IID_ENGINE  SL_IID_PLAY  SL_IID_RECORD  SL_IID_VOLUME
//! SL_IID_BUFFERQUEUE  SL_IID_ANDROIDSIMPLEBUFFERQUEUE
//! SL_IID_ANDROIDCONFIGURATION
//! ```
//!
//! Seven of those are *data*, not functions — an `SLInterfaceID` is a pointer
//! to a UUID struct — and a missing data symbol fails the dynamic linker's
//! walk outright. So the ID objects exist here whether or not the path they
//! unlock is implemented, and `slCreateEngine` reports honestly what is
//! behind them.
//!
//! Written against `native/opensles.cpp` as a behavioral reference only. The
//! interface-ID rule carried over from the spec's own reading: the IDs are
//! compared by *pointer identity*, never by their (zeroed) value bytes, so
//! each is a distinct static and nothing here needs a real UUID.
//!
//! The object model is Rust-facing: each object kind is a struct behind an
//! `Arc` in a registry keyed by address, and the address doubles as the
//! opaque handle the future FFI wrapper passes as the spec's interface
//! pointers. The vtable-pointer-first-field layout the C++ needed for the
//! spec's `self`-recovery trick is deliberately absent here — that is FFI
//! wiring, done in the linker phase, not policy.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::logline::{log, Priority, OPENSL_TAG};
use crate::types::*;

pub mod backend;
pub mod player;
pub mod recorder;

pub use player::{PlayerHandle, PlayerIid};
pub use recorder::{RecorderHandle, RecorderIid};

/// One log line under the OpenSL tag.
pub(crate) fn log_opensl(priority: Priority, message: &str) {
    log(priority, OPENSL_TAG, message);
}

// ---------------------------------------------------------------------------
// Interface IDs
// ---------------------------------------------------------------------------

/// A handle to one of the runtime's interface-ID objects: compared only by
/// identity, never by value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlIid {
    addr: usize,
}

macro_rules! iid {
    ($storage:ident, $accessor:ident) => {
        // Zeroed: the value carries no meaning, only the address does.
        static $storage: SLInterfaceID = SLInterfaceID {
            time_low: 0,
            time_mid: 0,
            time_hi_and_version: 0,
            clock_seq: 0,
            node: [0; 6],
        };
        pub fn $accessor() -> SlIid {
            SlIid { addr: &$storage as *const SLInterfaceID as usize }
        }
    };
}

iid!(IID_STORAGE_ENGINE, engine);
iid!(IID_STORAGE_PLAY, play);
iid!(IID_STORAGE_RECORD, record);
iid!(IID_STORAGE_VOLUME, volume);
iid!(IID_STORAGE_BUFFER_QUEUE, buffer_queue);
iid!(IID_STORAGE_ANDROID_SIMPLE_BUFFER_QUEUE, android_simple_buffer_queue);
iid!(IID_STORAGE_ANDROID_CONFIGURATION, android_configuration);
// Not exported: the engine never references SL_IID_OUTPUTMIX as a linked
// symbol (see the eight-symbol list above, which does not include it), so
// nothing outside can ever hold this pointer to compare against it. Kept
// only so create_output_mix's own interface-request validation has a real
// identity to check, in case that ever changes.
iid!(IID_STORAGE_OUTPUT_MIX, output_mix);

// ---------------------------------------------------------------------------
// Locators and formats, as the engine's Create* calls describe them
// ---------------------------------------------------------------------------

/// A data locator from a `SLDataSource`/`SLDataSink`, already decoded by the
/// caller of this layer (the FFI wrapper during wiring). Variants this
/// backend cannot serve are carried faithfully so validation refuses them
/// with the spec's code rather than normalizing to something nearby.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlLocator {
    /// `SLDataLocator_AndroidSimpleBufferQueue`: `numBuffers`.
    AndroidSimpleBufferQueue { num_buffers: u32 },
    /// `SLDataLocator_OutputMix`: which mix object the player sinks into.
    OutputMix { mix: Option<MixHandle> },
    /// `SLDataLocator_IODevice`: which input device a recorder reads from.
    IoDevice { device_type: SLuint32, device_id: SLuint32 },
    /// Any other locator type, by its type code.
    Other(SLuint32),
}

/// A PCM format from a source or sink descriptor. The classic
/// `SLDataFormat_PCM` (7 fields) and the Android `SLAndroidDataFormat_PCM_EX`
/// extension (the same 7 plus `representation`) are kept as separate variants
/// so `formatType` decides which fields exist, exactly as the C++ reads
/// `formatType` and only that field before choosing a struct — reading the
/// extension's `representation` through the shorter type would read past
/// what the caller allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlPcmFormat {
    Pcm {
        num_channels: u32,
        /// milliHertz: 44100000 means 44.1 kHz.
        samples_per_sec_milli: u32,
        bits_per_sample: u32,
        container_size: u32,
        channel_mask: SLuint32,
        endianness: SLuint32,
    },
    PcmEx {
        num_channels: u32,
        /// Same milliHertz units, different field name.
        sample_rate_milli: u32,
        bits_per_sample: u32,
        container_size: u32,
        channel_mask: SLuint32,
        endianness: SLuint32,
        representation: SLuint32,
    },
    /// MIME/compressed sources and anything else: no decoder here.
    Other(SLuint32),
}

/// One requested interface on a Create call: which ID and whether the caller
/// made it mandatory.
#[derive(Debug, Clone, Copy)]
pub struct SlInterfaceRequest {
    pub iid: SlIid,
    pub required: bool,
}

// ---------------------------------------------------------------------------
// Registries
// ---------------------------------------------------------------------------

pub type EngineHandle = usize;
pub type MixHandle = usize;

static ENGINES: OnceLock<Mutex<BTreeMap<EngineHandle, Arc<EngineObject>>>> = OnceLock::new();
static MIXES: OnceLock<Mutex<BTreeMap<MixHandle, Arc<MixObject>>>> = OnceLock::new();
static PLAYERS: OnceLock<Mutex<BTreeMap<PlayerHandle, Arc<player::PlayerShared>>>> = OnceLock::new();
static RECORDERS: OnceLock<Mutex<BTreeMap<RecorderHandle, Arc<recorder::RecorderShared>>>> =
    OnceLock::new();

fn engines() -> &'static Mutex<BTreeMap<EngineHandle, Arc<EngineObject>>> {
    ENGINES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn mixes() -> &'static Mutex<BTreeMap<MixHandle, Arc<MixObject>>> {
    MIXES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn players() -> &'static Mutex<BTreeMap<PlayerHandle, Arc<player::PlayerShared>>> {
    PLAYERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn recorders() -> &'static Mutex<BTreeMap<RecorderHandle, Arc<recorder::RecorderShared>>> {
    RECORDERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn insert<T>(map: &Mutex<BTreeMap<usize, Arc<T>>>, object: Arc<T>) -> usize {
    let key = Arc::as_ptr(&object) as usize;
    lock(map).insert(key, object);
    key
}

pub(crate) fn remove_player(key: usize) {
    lock(players()).remove(&key);
}

pub(crate) fn remove_recorder(key: usize) {
    lock(recorders()).remove(&key);
}

fn engine_for(handle: EngineHandle) -> Option<Arc<EngineObject>> {
    lock(engines()).get(&handle).cloned()
}

fn mix_for(handle: MixHandle) -> Option<Arc<MixObject>> {
    lock(mixes()).get(&handle).cloned()
}

fn player_for(handle: PlayerHandle) -> Option<Arc<player::PlayerShared>> {
    lock(players()).get(&handle).cloned()
}

fn recorder_for(handle: RecorderHandle) -> Option<Arc<recorder::RecorderShared>> {
    lock(recorders()).get(&handle).cloned()
}

// ---------------------------------------------------------------------------
// EngineObject and OutputMixObject
// ---------------------------------------------------------------------------

pub struct EngineObject {
    state: AtomicU32,
}

pub struct MixObject {
    state: AtomicU32,
}

impl EngineObject {
    /// `Realize` succeeds without doing anything: PipeWire reachability was
    /// already confirmed by `slCreateEngine`.
    pub fn realize(&self, _async_: SLboolean) -> SLresult {
        self.state.store(SL_OBJECT_STATE_REALIZED, Ordering::Release);
        SL_RESULT_SUCCESS
    }

    pub fn get_state(&self, out: Option<&mut SLuint32>) -> SLresult {
        match out {
            Some(out) => {
                *out = self.state.load(Ordering::Acquire);
                SL_RESULT_SUCCESS
            }
            None => SL_RESULT_PARAMETER_INVALID,
        }
    }

    pub fn get_interface(&self, iid: SlIid) -> Result<(), SLresult> {
        if iid == engine() {
            Ok(())
        } else {
            Err(SL_RESULT_FEATURE_UNSUPPORTED)
        }
    }

    pub fn destroy(&self, handle: EngineHandle) -> SLresult {
        self.state.store(SL_OBJECT_STATE_UNREALIZED, Ordering::Release);
        lock(engines()).remove(&handle);
        SL_RESULT_SUCCESS
    }

    // ------------------------------------------------------------ Create*

    /// `CreateAudioPlayer`. Validation order mirrors the C++: source locator,
    /// source format, sink locator, then the interface requests. Refusals
    /// say what was asked for; a success is silent apart from the
    /// configuration the getters will echo back anyway.
    pub fn create_audio_player(
        &self,
        out: Option<&mut Option<PlayerHandle>>,
        source: &SlLocator,
        source_format: &SlPcmFormat,
        sink: &SlLocator,
        interfaces: &[SlInterfaceRequest],
    ) -> SLresult {
        let Some(out) = out else {
            return SL_RESULT_PARAMETER_INVALID;
        };
        *out = None;

        // Only the raw-PCM push model Android's own audio players actually
        // use is implemented; a real OpenSL ES source type (URI, ANDROIDFD,
        // plain BUFFERQUEUE, ...) is refused as unsupported.
        let SlLocator::AndroidSimpleBufferQueue { num_buffers } = source else {
            return SL_RESULT_CONTENT_UNSUPPORTED;
        };

        let (num_channels, samples_per_sec_milli, bits_per_sample, container_size, endianness) =
            match source_format {
                SlPcmFormat::Pcm {
                    num_channels,
                    samples_per_sec_milli,
                    bits_per_sample,
                    container_size,
                    endianness,
                    ..
                } => (
                    *num_channels,
                    *samples_per_sec_milli,
                    *bits_per_sample,
                    *container_size,
                    *endianness,
                ),
                SlPcmFormat::PcmEx {
                    num_channels,
                    sample_rate_milli,
                    bits_per_sample,
                    container_size,
                    endianness,
                    representation,
                    ..
                } => {
                    if *representation != SL_ANDROID_PCM_REPRESENTATION_SIGNED_INT {
                        // Float and unsigned-int PCM: not translated.
                        // Rejecting here is the honest answer; silently
                        // reinterpreting the bytes as signed would produce
                        // audio, just wrong audio.
                        return SL_RESULT_CONTENT_UNSUPPORTED;
                    }
                    (
                        *num_channels,
                        *sample_rate_milli,
                        *bits_per_sample,
                        *container_size,
                        *endianness,
                    )
                }
                SlPcmFormat::Other(_) => return SL_RESULT_CONTENT_UNSUPPORTED,
            };

        let SlLocator::OutputMix { mix } = sink else {
            return SL_RESULT_CONTENT_UNSUPPORTED;
        };
        // The mix object itself is not otherwise consulted: this backend has
        // exactly one destination (the shared host session), so there is
        // nothing routing-relevant to read from it. A null one is still a
        // caller bug worth reporting rather than silently playing anyway.
        if mix.is_none() {
            return SL_RESULT_PARAMETER_INVALID;
        }

        for request in interfaces {
            let known = request.iid == play()
                || request.iid == android_simple_buffer_queue()
                || request.iid == volume()
                || request.iid == android_configuration();
            if request.required && !known {
                return SL_RESULT_FEATURE_UNSUPPORTED;
            }
        }

        let player = player::PlayerShared::new(
            samples_per_sec_milli / 1000, // milliHertz -> Hz; 44100000 means 44.1 kHz
            num_channels,
            bits_per_sample,
            container_size,
            endianness == SL_BYTEORDER_BIGENDIAN,
            if *num_buffers != 0 { *num_buffers } else { 2 },
        );
        *out = Some(insert(players(), player));
        SL_RESULT_SUCCESS
    }

    /// `CreateAudioRecorder`. Validation order mirrors the C++: source
    /// locator and device identity, sink locator, sink format, then the
    /// interface requests.
    pub fn create_audio_recorder(
        &self,
        out: Option<&mut Option<RecorderHandle>>,
        source: &SlLocator,
        sink: &SlLocator,
        sink_format: &SlPcmFormat,
        interfaces: &[SlInterfaceRequest],
    ) -> SLresult {
        let Some(out) = out else {
            return SL_RESULT_PARAMETER_INVALID;
        };
        *out = None;

        // Source: an IO device, and specifically the default audio input. A
        // request for a *named* input device is refused rather than quietly
        // served from the default — the caller asked for a particular
        // microphone and getting a different one without being told is worse
        // than being refused.
        let SlLocator::IoDevice { device_type, device_id } = source else {
            return SL_RESULT_CONTENT_UNSUPPORTED;
        };
        if *device_type != SL_IODEVICE_AUDIOINPUT {
            return SL_RESULT_CONTENT_UNSUPPORTED;
        }
        if *device_id != SL_DEFAULTDEVICEID_AUDIOINPUT {
            log_opensl(
                Priority::Error,
                &format!(
                    "CreateAudioRecorder asked for input device id 0x{device_id:08X}; only \
                     SL_DEFAULTDEVICEID_AUDIOINPUT is implemented, and serving a different \
                     microphone than the one asked for would be worse than refusing"
                ),
            );
            return SL_RESULT_CONTENT_UNSUPPORTED;
        }

        let SlLocator::AndroidSimpleBufferQueue { num_buffers } = sink else {
            return SL_RESULT_CONTENT_UNSUPPORTED;
        };

        let SlPcmFormat::Pcm {
            num_channels,
            samples_per_sec_milli,
            bits_per_sample,
            container_size,
            endianness,
            ..
        } = sink_format
        else {
            return SL_RESULT_CONTENT_UNSUPPORTED;
        };
        // The capture path delivers signed 16-bit native-endian PCM and only
        // that, so anything else is refused here rather than delivered as
        // reinterpreted bytes. Playback can afford a conversion table because
        // the host converts on the way out; capture would have to convert on
        // the way in, which is work nothing has asked for.
        if *bits_per_sample != 16 || *container_size != 16 || *endianness == SL_BYTEORDER_BIGENDIAN {
            log_opensl(
                Priority::Error,
                &format!(
                    "CreateAudioRecorder asked for {bits_per_sample}-bit samples in a \
                     {container_size}-bit container; this backend records signed 16-bit \
                     native-endian PCM only"
                ),
            );
            return SL_RESULT_CONTENT_UNSUPPORTED;
        }
        if *num_channels == 0 || *samples_per_sec_milli == 0 {
            return SL_RESULT_PARAMETER_INVALID;
        }

        for request in interfaces {
            let known = request.iid == record()
                || request.iid == android_simple_buffer_queue()
                || request.iid == android_configuration();
            if request.required && !known {
                return SL_RESULT_FEATURE_UNSUPPORTED;
            }
        }

        let recorder = recorder::RecorderShared::new(
            samples_per_sec_milli / 1000, // milliHertz -> Hz, as for playback
            *num_channels,
            if *num_buffers != 0 { *num_buffers } else { 2 },
        );
        // Worth a line even on the success path: this is the moment an engine
        // that records through OpenSL ES becomes distinguishable from one
        // that does not. No microphone is opened here.
        log_opensl(
            Priority::Info,
            &format!(
                "CreateAudioRecorder({} Hz, {} channel(s), {} buffers) — recorder created; \
                 no capture stream exists until SetRecordState(RECORDING)",
                samples_per_sec_milli / 1000,
                num_channels,
                if *num_buffers != 0 { *num_buffers } else { 2 },
            ),
        );
        *out = Some(insert(recorders(), recorder));
        SL_RESULT_SUCCESS
    }

    pub fn create_output_mix(
        &self,
        out: Option<&mut Option<MixHandle>>,
        interfaces: &[SlInterfaceRequest],
    ) -> SLresult {
        let Some(out) = out else {
            return SL_RESULT_PARAMETER_INVALID;
        };
        *out = None;
        for request in interfaces {
            if request.required && request.iid != output_mix() {
                return SL_RESULT_FEATURE_UNSUPPORTED;
            }
        }
        *out = Some(insert(mixes(), Arc::new(MixObject {
            state: AtomicU32::new(SL_OBJECT_STATE_UNREALIZED),
        })));
        SL_RESULT_SUCCESS
    }

    // Every remaining Create* is a feature the spec names and nothing here
    // implements; each says so with the spec's code rather than a null
    // object that pretends otherwise.
    pub fn create_led_device(&self, out: Option<&mut Option<EngineHandle>>) -> SLresult {
        if let Some(out) = out {
            *out = None;
        }
        SL_RESULT_FEATURE_UNSUPPORTED
    }

    pub fn create_vibra_device(&self, out: Option<&mut Option<EngineHandle>>) -> SLresult {
        if let Some(out) = out {
            *out = None;
        }
        SL_RESULT_FEATURE_UNSUPPORTED
    }

    pub fn create_midi_player(&self, out: Option<&mut Option<PlayerHandle>>) -> SLresult {
        if let Some(out) = out {
            *out = None;
        }
        SL_RESULT_FEATURE_UNSUPPORTED
    }

    pub fn create_listener(&self, out: Option<&mut Option<EngineHandle>>) -> SLresult {
        if let Some(out) = out {
            *out = None;
        }
        SL_RESULT_FEATURE_UNSUPPORTED
    }

    pub fn create_3d_group(&self, out: Option<&mut Option<EngineHandle>>) -> SLresult {
        if let Some(out) = out {
            *out = None;
        }
        SL_RESULT_FEATURE_UNSUPPORTED
    }

    pub fn create_metadata_extractor(&self, out: Option<&mut Option<EngineHandle>>) -> SLresult {
        if let Some(out) = out {
            *out = None;
        }
        SL_RESULT_FEATURE_UNSUPPORTED
    }

    pub fn create_extension_object(&self, out: Option<&mut Option<EngineHandle>>) -> SLresult {
        if let Some(out) = out {
            *out = None;
        }
        SL_RESULT_FEATURE_UNSUPPORTED
    }

    pub fn query_num_supported_interfaces(&self, out: Option<&mut SLuint32>) -> SLresult {
        if let Some(out) = out {
            *out = 0;
        }
        SL_RESULT_SUCCESS
    }

    /// Index is always out of range: `query_num_supported_interfaces` above
    /// always reports zero.
    pub fn query_supported_interfaces(&self, _index: SLuint32) -> SLresult {
        SL_RESULT_PARAMETER_INVALID
    }

    pub fn query_num_supported_extensions(&self, out: Option<&mut SLuint32>) -> SLresult {
        if let Some(out) = out {
            *out = 0;
        }
        SL_RESULT_SUCCESS
    }

    pub fn query_supported_extension(&self, _index: SLuint32) -> SLresult {
        SL_RESULT_PARAMETER_INVALID
    }

    pub fn is_extension_supported(&self, out: Option<&mut SLboolean>) -> SLresult {
        if let Some(out) = out {
            *out = SL_BOOLEAN_FALSE;
        }
        SL_RESULT_SUCCESS
    }
}

impl MixObject {
    pub fn realize(&self, _async_: SLboolean) -> SLresult {
        self.state.store(SL_OBJECT_STATE_REALIZED, Ordering::Release);
        SL_RESULT_SUCCESS
    }

    pub fn get_state(&self, out: Option<&mut SLuint32>) -> SLresult {
        match out {
            Some(out) => {
                *out = self.state.load(Ordering::Acquire);
                SL_RESULT_SUCCESS
            }
            None => SL_RESULT_PARAMETER_INVALID,
        }
    }

    pub fn get_interface(&self, iid: SlIid) -> Result<(), SLresult> {
        if iid == output_mix() {
            Ok(())
        } else {
            Err(SL_RESULT_FEATURE_UNSUPPORTED)
        }
    }

    pub fn destroy(&self, handle: MixHandle) -> SLresult {
        self.state.store(SL_OBJECT_STATE_UNREALIZED, Ordering::Release);
        lock(mixes()).remove(&handle);
        SL_RESULT_SUCCESS
    }

    /// No per-device routing surface: this backend has exactly one
    /// destination (the shared host session), chosen by the host's own
    /// session manager rather than by device ID.
    pub fn get_destination_output_device_ids(
        &self,
        num_devices: Option<&mut SLint32>,
    ) -> SLresult {
        if let Some(out) = num_devices {
            *out = 0;
        }
        SL_RESULT_SUCCESS
    }

    /// Accepted; never fires, because device changes are never surfaced
    /// above.
    pub fn register_device_change_callback(
        &self,
        _callback: *mut std::ffi::c_void,
        _context: *mut std::ffi::c_void,
    ) -> SLresult {
        SL_RESULT_SUCCESS
    }

    pub fn re_route(&self, _num_output_devices: SLint32) -> SLresult {
        SL_RESULT_FEATURE_UNSUPPORTED
    }
}

// ---------------------------------------------------------------------------
// slCreateEngine
// ---------------------------------------------------------------------------

/// `slCreateEngine`. Returns `SL_RESULT_FEATURE_UNSUPPORTED` when no host
/// backend is reachable — no device, no session. Handing back an engine
/// object in that case would let the engine proceed on an engine with no
/// audio behind it and fail somewhere with no relationship to the real
/// cause; that is the mistake this file's C++ reference avoided since it
/// first existed, now for a more specific reason than "nothing is
/// implemented yet".
///
/// `SL_ENGINEOPTION_THREADSAFE` / `_LOSSOFCONTROL`: this engine is internally
/// synchronised unconditionally (every mutable object is behind an atomic or
/// a mutex), so there is no laxer mode for these to opt out of.
pub fn sl_create_engine(
    mut out: Option<&mut Option<EngineHandle>>,
    num_options: u32,
    interfaces: &[SlInterfaceRequest],
) -> SLresult {
    // Announced on entry, before anything can decline. Whether the engine
    // calls this at all is the first question about this runtime's audio,
    // and with no line printed on the way in an absent engine and a refused
    // one look identical in a log.
    log_opensl(
        Priority::Info,
        &format!(
            "slCreateEngine called by the engine ({num_options} option(s), {} interface(s) \
             requested)",
            interfaces.len()
        ),
    );

    if let Some(out) = out.as_deref_mut() {
        *out = None;
    }

    for request in interfaces {
        if request.required && request.iid != engine() {
            return SL_RESULT_FEATURE_UNSUPPORTED;
        }
    }

    if !crate::backend::host_backend_available() {
        return SL_RESULT_FEATURE_UNSUPPORTED;
    }
    let Some(out) = out else {
        return SL_RESULT_PARAMETER_INVALID;
    };

    *out = Some(insert(engines(), Arc::new(EngineObject {
        state: AtomicU32::new(SL_OBJECT_STATE_UNREALIZED),
    })));
    SL_RESULT_SUCCESS
}

/// The handle the engine's getters work through. `None` when the handle is
/// not (or no longer) in the registry — a caller bug worth an error, not a
/// silent success.
pub fn get_engine(handle: EngineHandle) -> Option<Arc<EngineObject>> {
    engine_for(handle)
}

pub fn get_mix(handle: MixHandle) -> Option<Arc<MixObject>> {
    mix_for(handle)
}

pub fn get_player(handle: PlayerHandle) -> Option<Arc<player::PlayerShared>> {
    player_for(handle)
}

pub fn get_recorder(handle: RecorderHandle) -> Option<Arc<recorder::RecorderShared>> {
    recorder_for(handle)
}

/// Tests construct objects directly so object-lifecycle coverage does not
/// depend on the test host having an audio device (only `slCreateEngine`'s
/// availability gate does, and its test tolerates either answer).
#[cfg(test)]
pub(crate) fn test_engine() -> EngineHandle {
    insert(engines(), Arc::new(EngineObject {
        state: AtomicU32::new(SL_OBJECT_STATE_UNREALIZED),
    }))
}

#[cfg(test)]
mod tests;
