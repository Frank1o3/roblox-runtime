// Rust-owned Android and WebRTC audio Java surfaces. Host streams and capture
// policy live in roblox-audio; this file only adapts the descriptors Roblox
// calls to those implementations.

use roblox_audio::backend::VOICE_MUTED;
use roblox_audio::backend::capture::{CaptureRequest, CaptureStream};
use roblox_audio::backend::output::{OutputRequest, OutputStream};
use roblox_audio::backend::owned_pcm_queue::OwnedPcmQueue;
use roblox_audio::backend::{DeviceInfo, enumerate_devices, make_output_stream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

const AUDIO_DEFAULT_RATE: u32 = 48_000;
const AUDIO_PCM_S16: i32 = 2;
const AUDIO_SOURCE_UNINITIALIZED: i32 = 0;
const AUDIO_SOURCE_INITIALIZED: i32 = 1;
const AUDIO_RECORD_STOPPED: i32 = 1;
const AUDIO_RECORDING: i32 = 3;

type AudioObjectKey = (usize, jnivm::ObjectId);

fn audio_objects<T: Send + Sync + 'static>(
    slot: &'static OnceLock<Mutex<HashMap<AudioObjectKey, Arc<T>>>>,
) -> &'static Mutex<HashMap<AudioObjectKey, Arc<T>>> {
    slot.get_or_init(|| Mutex::new(HashMap::new()))
}

static AUDIO_DEVICE_INFOS: OnceLock<Mutex<HashMap<AudioObjectKey, Arc<DeviceInfo>>>> =
    OnceLock::new();
static AUDIO_RECORDS: OnceLock<Mutex<HashMap<AudioObjectKey, Arc<Mutex<AndroidAudioRecord>>>>> =
    OnceLock::new();
static WEBRTC_RECORDS: OnceLock<Mutex<HashMap<AudioObjectKey, Arc<WebRtcAudioRecord>>>> =
    OnceLock::new();
static WEBRTC_TRACKS: OnceLock<Mutex<HashMap<AudioObjectKey, Arc<WebRtcAudioTrack>>>> =
    OnceLock::new();
static WEBRTC_MIC_MUTED: AtomicBool = AtomicBool::new(false);

fn audio_key(vm: &Vm, object: jnivm::ObjectId) -> AudioObjectKey {
    (vm as *const Vm as usize, object)
}

fn audio_device_info(vm: &Vm, object: jnivm::ObjectId) -> Option<DeviceInfo> {
    audio_objects(&AUDIO_DEVICE_INFOS)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&audio_key(vm, object))
        .map(|device| device.as_ref().clone())
}

fn host_audio_devices() -> Vec<DeviceInfo> {
    match enumerate_devices() {
        Ok(devices) => devices,
        Err(error) => {
            eprintln!("[audio] host device enumeration failed: {error}");
            Vec::new()
        }
    }
}

fn android_audio_device_type(device: &DeviceInfo) -> i32 {
    let name = format!("{} {}", device.node_name, device.description).to_ascii_lowercase();
    if name.contains("bluetooth") || name.contains("bluez") {
        if device.is_source { 7 } else { 8 }
    } else if name.contains("hdmi") || name.contains("displayport") {
        9
    } else if name.contains("usb") && name.contains("headset") {
        22
    } else if name.contains("usb") {
        11
    } else if name.contains("headset") {
        3
    } else if name.contains("headphone") {
        4
    } else if device.is_source && (name.contains("mic") || name.contains("input")) {
        15
    } else if !device.is_source && (name.contains("speaker") || name.contains("output")) {
        2
    } else {
        0
    }
}

fn new_device_info(vm: &Vm, device: DeviceInfo) -> Option<jnivm::ObjectId> {
    let class = vm
        .find_or_define_class("android/media/AudioDeviceInfo")
        .ok()?;
    let env = attached_thread_env(vm);
    let object = vm.new_local_object(&env, class, ObjectValue::Opaque).ok()?;
    audio_objects(&AUDIO_DEVICE_INFOS)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(audio_key(vm, object), Arc::new(device));
    Some(object)
}

fn install_audio_methods(vm: &Vm) -> Result<(), String> {
    let info = vm
        .register_class("android/media/AudioDeviceInfo")
        .map_err(|e| e.to_string())?;
    for (name, descriptor, handler) in [
        ("getId", "()I", audio_info_id as jnivm::MethodHandler),
        ("getType", "()I", audio_info_type),
        ("isSource", "()Z", audio_info_is_source),
        (
            "getProductName",
            "()Ljava/lang/CharSequence;",
            audio_info_product_name,
        ),
    ] {
        install_instance_builtin(vm, info, name, descriptor, handler)?;
    }
    for method in ["getSampleRates", "getChannelCounts", "getEncodings"] {
        let handler = match method {
            "getSampleRates" => audio_info_sample_rates as jnivm::MethodHandler,
            "getChannelCounts" => audio_info_channel_counts,
            _ => audio_info_encodings,
        };
        install_instance_builtin(vm, info, method, "()[I", handler)?;
    }

    let manager = vm
        .register_class("android/media/AudioManager")
        .map_err(|e| e.to_string())?;
    for (name, descriptor, handler) in [
        (
            "getDevices",
            "(I)[Landroid/media/AudioDeviceInfo;",
            audio_manager_devices as jnivm::MethodHandler,
        ),
        (
            "getCommunicationDevice",
            "()Landroid/media/AudioDeviceInfo;",
            audio_manager_communication_device,
        ),
        ("isMicrophoneMute", "()Z", audio_manager_microphone_mute),
        ("isVolumeFixed", "()Z", audio_manager_volume_fixed),
        ("isSpeakerphoneOn", "()Z", audio_manager_speakerphone_on),
        ("isMusicActive", "()Z", audio_manager_music_active),
    ] {
        install_instance_builtin(vm, manager, name, descriptor, handler)?;
    }

    install_audio_record_methods(vm)?;
    install_webrtc_audio_methods(vm)?;
    Ok(())
}

fn audio_info_for_receiver(vm: &Vm, receiver: Option<jnivm::ObjectId>) -> Option<DeviceInfo> {
    audio_device_info(vm, receiver?)
}

fn audio_info_id(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Int(audio_info_for_receiver(vm, receiver).map_or(0, |d| d.id as i32))
}

fn audio_info_type(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Int(
        audio_info_for_receiver(vm, receiver).map_or(0, |d| android_audio_device_type(&d)),
    )
}

fn audio_info_is_source(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Boolean(audio_info_for_receiver(vm, receiver).is_some_and(|d| d.is_source))
}

fn audio_info_product_name(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    let Some(device) = audio_info_for_receiver(vm, receiver) else {
        return JniValue::Object(None);
    };
    let text = if !device.description.is_empty() {
        device.description
    } else if !device.nick.is_empty() {
        device.nick
    } else {
        device.node_name
    };
    let class = match vm.find_or_define_class("java/lang/String") {
        Ok(class) => class,
        Err(_) => return JniValue::Object(None),
    };
    let env = attached_thread_env(vm);
    match vm.new_local_object(
        &env,
        class,
        ObjectValue::String(text.encode_utf16().collect()),
    ) {
        Ok(object) => JniValue::Object(Some(object)),
        Err(_) => JniValue::Object(None),
    }
}

fn audio_info_int_array(vm: &Vm, values: &[i32]) -> JniValue {
    let env = attached_thread_env(vm);
    let Ok(array) = vm.new_primitive_array::<i32>(&env, values.len()) else {
        return JniValue::Object(None);
    };
    match vm.set_primitive_array_region::<i32>(&env, array, 0, values) {
        Ok(()) => JniValue::Object(Some(array)),
        Err(_) => JniValue::Object(None),
    }
}

fn audio_info_sample_rates(vm: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    audio_info_int_array(vm, &[8_000, 11_025, 16_000, 22_050, 32_000, 44_100, 48_000])
}

fn audio_info_channel_counts(vm: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    audio_info_int_array(vm, &[1, 2])
}

fn audio_info_encodings(vm: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    audio_info_int_array(vm, &[AUDIO_PCM_S16])
}

fn audio_manager_devices(vm: &Vm, _: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let flags = match args.first() {
        Some(JniValue::Int(flags)) => *flags,
        _ => 0,
    };
    let want_inputs = flags & 1 != 0;
    let want_outputs = flags & 2 != 0;
    let mut devices = host_audio_devices();
    devices.retain(|d| {
        if d.is_source {
            want_inputs
        } else {
            want_outputs
        }
    });
    devices.sort_by_key(|d| !d.is_default);
    let env = attached_thread_env(vm);
    let Ok(array) = vm.new_local_object_array(&env, devices.len(), None) else {
        return JniValue::Object(None);
    };
    for (index, device) in devices.into_iter().enumerate() {
        if let Some(object) = new_device_info(vm, device) {
            let _ = vm.set_object_array_element(&env, array, index, Some(object));
        }
    }
    JniValue::Object(Some(array))
}

fn audio_manager_communication_device(
    vm: &Vm,
    _: Option<jnivm::ObjectId>,
    _: &[JniValue],
) -> JniValue {
    host_audio_devices()
        .into_iter()
        .find(|d| d.is_source && d.is_default)
        .and_then(|device| new_device_info(vm, device))
        .map_or(JniValue::Object(None), |object| {
            JniValue::Object(Some(object))
        })
}

fn audio_manager_microphone_mute(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Boolean(false)
}

fn audio_manager_volume_fixed(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Boolean(false)
}

fn audio_manager_speakerphone_on(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Boolean(true)
}

fn audio_manager_music_active(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Boolean(false)
}

#[derive(Default)]
struct AndroidAudioRecord {
    sample_rate: i32,
    channels: u16,
    buffer_bytes: usize,
    recording_state: i32,
    capture: Option<CaptureStream>,
}

fn install_audio_record_methods(vm: &Vm) -> Result<(), String> {
    let class = vm
        .register_class("android/media/AudioRecord")
        .map_err(|e| e.to_string())?;
    install_builtin(
        vm,
        class,
        "<init>",
        "(IIIII)Landroid/media/AudioRecord;",
        audio_record_create,
    )?;
    install_builtin(
        vm,
        class,
        "getMinBufferSize",
        "(III)I",
        audio_record_min_buffer_size,
    )?;
    for (name, descriptor, handler) in [
        (
            "getState",
            "()I",
            audio_record_state as jnivm::MethodHandler,
        ),
        ("getRecordingState", "()I", audio_record_recording_state),
        ("getSampleRate", "()I", audio_record_sample_rate),
        ("getChannelCount", "()I", audio_record_channels),
        ("getAudioFormat", "()I", audio_record_format),
        ("getBufferSizeInFrames", "()I", audio_record_buffer_frames),
        ("getAudioSessionId", "()I", audio_record_session_id),
        ("startRecording", "()V", audio_record_start),
        ("stop", "()V", audio_record_stop),
        ("release", "()V", audio_record_release),
        ("read", "(Ljava/nio/ByteBuffer;I)I", audio_record_read),
    ] {
        install_instance_builtin(vm, class, name, descriptor, handler)?;
    }
    Ok(())
}

fn android_audio_record(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
) -> Option<Arc<Mutex<AndroidAudioRecord>>> {
    audio_objects(&AUDIO_RECORDS)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&audio_key(vm, receiver?))
        .cloned()
}

fn audio_record_create(vm: &Vm, _: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let int_arg = |index: usize, default: i32| match args.get(index) {
        Some(JniValue::Int(value)) => *value,
        _ => default,
    };
    let rate = int_arg(1, AUDIO_DEFAULT_RATE as i32);
    let channel_config = int_arg(2, 0x10);
    let encoding = int_arg(3, AUDIO_PCM_S16);
    let buffer_bytes = int_arg(4, 4096).max(1) as usize;
    let class = match vm.find_or_define_class("android/media/AudioRecord") {
        Ok(class) => class,
        Err(_) => return JniValue::Object(None),
    };
    let env = attached_thread_env(vm);
    let channels = if channel_config == 0x0c { 2 } else { 1 };
    let rate = if (8_000..=192_000).contains(&rate) && encoding == AUDIO_PCM_S16 {
        rate
    } else {
        0
    };
    if rate == 0 {
        eprintln!("[audio] AudioRecord rejected rate={rate}, encoding={encoding}");
    }
    let Ok(object) = vm.new_local_object(&env, class, ObjectValue::Opaque) else {
        return JniValue::Object(None);
    };
    audio_objects(&AUDIO_RECORDS)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(
            audio_key(vm, object),
            Arc::new(std::sync::Mutex::new(AndroidAudioRecord {
                sample_rate: rate,
                channels,
                buffer_bytes,
                recording_state: AUDIO_RECORD_STOPPED,
                capture: None,
            })),
        );
    JniValue::Object(Some(object))
}

fn audio_record_min_buffer_size(_: &Vm, _: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let rate = match args.first() {
        Some(JniValue::Int(rate)) if *rate > 0 => *rate,
        _ => AUDIO_DEFAULT_RATE as i32,
    };
    let channels = matches!(args.get(1), Some(JniValue::Int(0x0c))) as i32 + 1;
    JniValue::Int(channels * 2 * rate / 50)
}

fn audio_record_state(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Int(
        android_audio_record(vm, receiver).map_or(AUDIO_SOURCE_UNINITIALIZED, |record| {
            if record.lock().unwrap_or_else(|p| p.into_inner()).sample_rate > 0 {
                AUDIO_SOURCE_INITIALIZED
            } else {
                AUDIO_SOURCE_UNINITIALIZED
            }
        }),
    )
}

fn audio_record_recording_state(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    _: &[JniValue],
) -> JniValue {
    JniValue::Int(
        android_audio_record(vm, receiver).map_or(AUDIO_RECORD_STOPPED, |record| {
            record
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .recording_state
        }),
    )
}

fn audio_record_sample_rate(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    _: &[JniValue],
) -> JniValue {
    JniValue::Int(android_audio_record(vm, receiver).map_or(0, |record| {
        record.lock().unwrap_or_else(|p| p.into_inner()).sample_rate
    }))
}

fn audio_record_channels(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Int(android_audio_record(vm, receiver).map_or(0, |record| {
        i32::from(record.lock().unwrap_or_else(|p| p.into_inner()).channels)
    }))
}

fn audio_record_format(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Int(AUDIO_PCM_S16)
}

fn audio_record_buffer_frames(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    _: &[JniValue],
) -> JniValue {
    JniValue::Int(android_audio_record(vm, receiver).map_or(0, |record| {
        let record = record.lock().unwrap_or_else(|p| p.into_inner());
        (record.buffer_bytes / (usize::from(record.channels) * 2)) as i32
    }))
}

fn audio_record_session_id(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Int(0)
}

fn audio_capture_target() -> String {
    host_audio_devices()
        .into_iter()
        .find(|device| device.is_source && device.is_default)
        .map_or_else(String::new, |device| device.node_name)
}

fn open_capture(rate: i32, channels: u16) -> Result<CaptureStream, String> {
    CaptureStream::open(
        &audio_capture_target(),
        CaptureRequest {
            sample_rate: Some(rate as u32),
            channels: Some(channels),
        },
    )
    .map_err(|e| e.to_string())
}

fn audio_record_start(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    let Some(record) = android_audio_record(vm, receiver) else {
        return JniValue::Void;
    };
    let mut record = record.lock().unwrap_or_else(|p| p.into_inner());
    if record.sample_rate <= 0 || record.capture.is_some() {
        return JniValue::Void;
    }
    match open_capture(record.sample_rate, record.channels) {
        Ok(capture) => {
            record.capture = Some(capture);
            record.recording_state = AUDIO_RECORDING;
            eprintln!("[audio] AudioRecord capture started");
        }
        Err(error) => eprintln!("[audio] AudioRecord capture could not start: {error}"),
    }
    JniValue::Void
}

fn audio_record_stop(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    if let Some(record) = android_audio_record(vm, receiver) {
        let mut record = record.lock().unwrap_or_else(|p| p.into_inner());
        record.recording_state = AUDIO_RECORD_STOPPED;
        record.capture.take();
    }
    JniValue::Void
}

fn audio_record_release(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let _ = audio_record_stop(vm, receiver, args);
    if let Some(receiver) = receiver {
        audio_objects(&AUDIO_RECORDS)
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&audio_key(vm, receiver));
    }
    JniValue::Void
}

fn audio_record_read(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let Some(record) = android_audio_record(vm, receiver) else {
        return JniValue::Int(0);
    };
    let (Some(JniValue::Object(Some(buffer))), Some(JniValue::Int(requested))) =
        (args.first(), args.get(1))
    else {
        return JniValue::Int(0);
    };
    if *requested <= 0 {
        return JniValue::Int(0);
    }
    let env = attached_thread_env(vm);
    let Ok(ObjectValue::DirectByteBuffer { address, capacity }) = vm.object_value(&env, *buffer)
    else {
        return JniValue::Int(0);
    };
    let mut record = record.lock().unwrap_or_else(|p| p.into_inner());
    let channels = record.channels;
    let sample_rate = record.sample_rate.max(1);
    let Some(capture) = record.capture.as_mut() else {
        return JniValue::Int(0);
    };
    let bytes = (*requested as usize).min(capacity.max(0) as usize);
    let samples = bytes / 2;
    if samples == 0 || address == 0 {
        return JniValue::Int(0);
    }
    let mut pcm = vec![0.0_f32; samples];
    let frames_wait = (samples / usize::from(channels)).max(1);
    let timeout = Duration::from_secs_f64(frames_wait as f64 / sample_rate as f64);
    let read = capture.read_blocking(&mut pcm, timeout + Duration::from_millis(8));
    let count = read.min(samples);
    // SAFETY: the VM's DirectByteBuffer stores the caller-provided address and
    // capacity; count is bounded by both the request and that capacity.
    let dst = unsafe { std::slice::from_raw_parts_mut(address as *mut u8, count * 2) };
    for (sample, bytes) in pcm[..count].iter().zip(dst.chunks_exact_mut(2)) {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        bytes.copy_from_slice(&value.to_le_bytes());
    }
    JniValue::Int((count * 2) as i32)
}

static WEBRTC_SPEAKER_MUTED: AtomicBool = AtomicBool::new(false);

fn install_webrtc_audio_methods(vm: &Vm) -> Result<(), String> {
    install_webrtc_manager_methods(vm)?;
    install_webrtc_record_methods(vm)?;
    install_webrtc_track_methods(vm)
}

fn install_webrtc_manager_methods(vm: &Vm) -> Result<(), String> {
    let class = vm
        .register_class("org/webrtc/voiceengine/WebRtcAudioManager")
        .map_err(|e| e.to_string())?;
    install_builtin(
        vm,
        class,
        "<init>",
        "(J)Lorg/webrtc/voiceengine/WebRtcAudioManager;",
        webrtc_manager_create,
    )?;
    for (name, descriptor, handler) in [
        ("init", "()Z", webrtc_manager_init as jnivm::MethodHandler),
        ("dispose", "()V", webrtc_manager_dispose),
        (
            "getNativeOutputSampleRate",
            "()I",
            webrtc_manager_sample_rate,
        ),
        (
            "getSampleRateForApiLevel",
            "()I",
            webrtc_manager_sample_rate,
        ),
        (
            "getLowLatencyInputFramesPerBuffer",
            "()I",
            webrtc_manager_frames,
        ),
        (
            "getLowLatencyOutputFramesPerBuffer",
            "()I",
            webrtc_manager_frames,
        ),
        ("isLowLatencyInputSupported", "()Z", audio_false),
        ("isLowLatencyOutputSupported", "()Z", audio_false),
        ("isProAudioSupported", "()Z", audio_false),
        ("isAAudioSupported", "()Z", audio_false),
        ("hasEarpiece", "()Z", audio_false),
        ("isCommunicationModeEnabled", "()Z", audio_false),
        ("isDeviceBlacklistedForOpenSLESUsage", "()Z", audio_false),
    ] {
        install_instance_builtin(vm, class, name, descriptor, handler)?;
    }
    for (name, descriptor, handler) in [
        ("getStereoInput", "()Z", audio_false as jnivm::MethodHandler),
        ("getStereoOutput", "()Z", audio_false),
        ("getMinInputFrameSize", "(II)I", webrtc_manager_min_frames),
        ("getMinOutputFrameSize", "(II)I", webrtc_manager_min_frames),
        ("isAcousticEchoCancelerSupported", "()Z", audio_false),
        ("isNoiseSuppressorSupported", "()Z", audio_false),
    ] {
        install_builtin(vm, class, name, descriptor, handler)?;
    }
    Ok(())
}

#[derive(Default)]
struct WebRtcAudioManager {
    native_handle: i64,
}

static WEBRTC_MANAGERS: OnceLock<Mutex<HashMap<AudioObjectKey, Arc<WebRtcAudioManager>>>> =
    OnceLock::new();

fn webrtc_manager_create(vm: &Vm, _: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let native_handle = match args.first() {
        Some(JniValue::Long(handle)) => *handle,
        _ => 0,
    };
    let class = match vm.find_or_define_class("org/webrtc/voiceengine/WebRtcAudioManager") {
        Ok(class) => class,
        Err(_) => return JniValue::Object(None),
    };
    let env = attached_thread_env(vm);
    let Ok(object) = vm.new_local_object(&env, class, ObjectValue::Opaque) else {
        return JniValue::Object(None);
    };
    audio_objects(&WEBRTC_MANAGERS)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(
            audio_key(vm, object),
            Arc::new(WebRtcAudioManager { native_handle }),
        );
    JniValue::Object(Some(object))
}

fn webrtc_manager_init(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    let Some(receiver) = receiver else {
        return JniValue::Boolean(false);
    };
    let native = vm
        .find_class("org/webrtc/voiceengine/WebRtcAudioManager")
        .ok()
        .and_then(|class| vm.find_native(class, "nativeCacheAudioParameters", "(IIIZZZZZZZIIJ)V"));
    let Some(native) = native else {
        eprintln!("[audio] WebRtcAudioManager nativeCacheAudioParameters is not registered");
        return JniValue::Boolean(false);
    };
    let native_handle = audio_objects(&WEBRTC_MANAGERS)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&audio_key(vm, receiver))
        .map_or(0, |manager| manager.native_handle);
    let Some(env) = jnivm::current_env() else {
        return JniValue::Boolean(false);
    };
    type Cache = unsafe extern "system" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        i32,
        i32,
        i32,
        u8,
        u8,
        u8,
        u8,
        u8,
        u8,
        u8,
        i32,
        i32,
        i64,
    );
    // SAFETY: the address came from this VM's RegisterNatives table and the
    // ABI and field order match the dex-verified native descriptor.
    let cache: Cache = unsafe { std::mem::transmute(native) };
    unsafe {
        cache(
            env,
            receiver.raw() as usize as *mut std::ffi::c_void,
            AUDIO_DEFAULT_RATE as i32,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (AUDIO_DEFAULT_RATE / 100) as i32,
            (AUDIO_DEFAULT_RATE / 100) as i32,
            native_handle,
        );
    }
    JniValue::Boolean(true)
}

fn webrtc_manager_sample_rate(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Int(AUDIO_DEFAULT_RATE as i32)
}

fn webrtc_manager_dispose(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    _: &[JniValue],
) -> JniValue {
    if let Some(receiver) = receiver {
        audio_objects(&WEBRTC_MANAGERS)
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&audio_key(vm, receiver));
    }
    JniValue::Void
}

fn webrtc_manager_frames(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Int((AUDIO_DEFAULT_RATE / 100) as i32)
}

fn webrtc_manager_min_frames(_: &Vm, _: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let rate = match args.first() {
        Some(JniValue::Int(rate)) if *rate > 0 => *rate,
        _ => AUDIO_DEFAULT_RATE as i32,
    };
    let channels = match args.get(1) {
        Some(JniValue::Int(channels)) if *channels > 0 => *channels,
        _ => 1,
    };
    JniValue::Int(rate / 100 * channels)
}

fn audio_noop(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Void
}

fn audio_false(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Boolean(false)
}

struct WebRtcAudioRecord {
    native_handle: i64,
    sample_rate: AtomicU32,
    channels: AtomicU32,
    buffer: Mutex<Option<Box<[u8]>>>,
    capture: Mutex<Option<CaptureStream>>,
    running: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
    global_receiver: Mutex<Option<jnivm::ObjectId>>,
}

impl WebRtcAudioRecord {
    fn stop(&self) {
        self.running.store(false, Ordering::Release);
        let mut joined = true;
        if let Some(worker) = self.worker.lock().unwrap_or_else(|p| p.into_inner()).take() {
            if worker.thread().id() != std::thread::current().id() {
                let _ = worker.join();
            } else {
                joined = false;
            }
        }
        self.capture
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if joined {
            if let Some(receiver) = self
                .global_receiver
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
            {
                if let Some(vm) = jnivm::process_vm() {
                    let _ = vm.delete_global_ref(receiver);
                }
            }
        }
    }
}

impl Drop for WebRtcAudioRecord {
    fn drop(&mut self) {
        self.stop();
    }
}

fn webrtc_record(vm: &Vm, receiver: Option<jnivm::ObjectId>) -> Option<Arc<WebRtcAudioRecord>> {
    audio_objects(&WEBRTC_RECORDS)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&audio_key(vm, receiver?))
        .cloned()
}

fn install_webrtc_record_methods(vm: &Vm) -> Result<(), String> {
    let class = vm
        .register_class("org/webrtc/voiceengine/WebRtcAudioRecord")
        .map_err(|e| e.to_string())?;
    install_builtin(
        vm,
        class,
        "<init>",
        "(J)Lorg/webrtc/voiceengine/WebRtcAudioRecord;",
        webrtc_record_create,
    )?;
    for (name, descriptor, handler) in [
        (
            "initRecording",
            "(II)I",
            webrtc_record_init as jnivm::MethodHandler,
        ),
        ("startRecording", "()Z", webrtc_record_start),
        ("stopRecording", "()Z", webrtc_record_stop),
        ("releaseAudioResources", "()V", webrtc_record_release),
        ("enableBuiltInAEC", "(Z)Z", webrtc_record_effect),
        ("enableBuiltInNS", "(Z)Z", webrtc_record_effect),
    ] {
        install_instance_builtin(vm, class, name, descriptor, handler)?;
    }
    install_builtin(vm, class, "setMicrophoneMute", "(Z)V", webrtc_record_mute)?;
    install_builtin(
        vm,
        class,
        "getDefaultAudioSource",
        "()I",
        webrtc_record_default_source,
    )?;
    Ok(())
}

fn webrtc_record_create(vm: &Vm, _: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let native_handle = match args.first() {
        Some(JniValue::Long(handle)) => *handle,
        _ => 0,
    };
    let class = match vm.find_or_define_class("org/webrtc/voiceengine/WebRtcAudioRecord") {
        Ok(class) => class,
        Err(_) => return JniValue::Object(None),
    };
    let env = attached_thread_env(vm);
    let Ok(object) = vm.new_local_object(&env, class, ObjectValue::Opaque) else {
        return JniValue::Object(None);
    };
    audio_objects(&WEBRTC_RECORDS)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(
            audio_key(vm, object),
            Arc::new(WebRtcAudioRecord {
                native_handle,
                sample_rate: AtomicU32::new(AUDIO_DEFAULT_RATE),
                channels: AtomicU32::new(1),
                buffer: Mutex::new(None),
                capture: Mutex::new(None),
                running: AtomicBool::new(false),
                worker: Mutex::new(None),
                global_receiver: Mutex::new(None),
            }),
        );
    JniValue::Object(Some(object))
}

fn webrtc_record_init(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let Some(record) = webrtc_record(vm, receiver) else {
        return JniValue::Int(-1);
    };
    let (Some(JniValue::Int(rate)), Some(JniValue::Int(channels))) = (args.first(), args.get(1))
    else {
        return JniValue::Int(-1);
    };
    if !(8_000..=192_000).contains(rate)
        || !(1..=2).contains(channels)
        || record.running.load(Ordering::Acquire)
    {
        return JniValue::Int(-1);
    }
    let bytes = ((*rate as usize / 100).max(1)) * *channels as usize * 2;
    let mut buffer = vec![0u8; bytes].into_boxed_slice();
    let address = buffer.as_mut_ptr() as usize;
    let class = match vm.find_class("org/webrtc/voiceengine/WebRtcAudioRecord") {
        Ok(class) => class,
        Err(_) => return JniValue::Int(-1),
    };
    let Some(native) = vm.find_native(
        class,
        "nativeCacheDirectBufferAddress",
        "(Ljava/nio/ByteBuffer;J)V",
    ) else {
        eprintln!("[audio] WebRtcAudioRecord nativeCacheDirectBufferAddress is not registered");
        return JniValue::Int(-1);
    };
    let Ok(byte_buffer_class) = vm.find_or_define_class("java/nio/ByteBuffer") else {
        return JniValue::Int(-1);
    };
    let env_token = attached_thread_env(vm);
    let Ok(direct_buffer) = vm.new_local_object(
        &env_token,
        byte_buffer_class,
        ObjectValue::DirectByteBuffer {
            address,
            capacity: bytes as i64,
        },
    ) else {
        return JniValue::Int(-1);
    };
    let Some(receiver) = receiver else {
        return JniValue::Int(-1);
    };
    let Some(env) = jnivm::current_env() else {
        return JniValue::Int(-1);
    };
    type Cache = unsafe extern "system" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        i64,
    );
    // SAFETY: resolved from the VM's RegisterNatives table using its exact dex descriptor.
    let cache: Cache = unsafe { std::mem::transmute(native) };
    unsafe {
        cache(
            env,
            receiver.raw() as usize as *mut std::ffi::c_void,
            direct_buffer.raw() as usize as *mut std::ffi::c_void,
            record.native_handle,
        );
    }
    record.sample_rate.store(*rate as u32, Ordering::Release);
    record.channels.store(*channels as u32, Ordering::Release);
    *record.buffer.lock().unwrap_or_else(|p| p.into_inner()) = Some(buffer);
    JniValue::Int(bytes as i32)
}

fn webrtc_record_start(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    let Some(record) = webrtc_record(vm, receiver) else {
        return JniValue::Boolean(false);
    };
    if record.running.load(Ordering::Acquire) {
        return JniValue::Boolean(true);
    }
    if record
        .buffer
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .is_none()
    {
        return JniValue::Boolean(false);
    }
    let class = match vm.find_class("org/webrtc/voiceengine/WebRtcAudioRecord") {
        Ok(class) => class,
        Err(_) => return JniValue::Boolean(false),
    };
    let Some(data_native) = vm.find_native(class, "nativeDataIsRecorded", "(IJ)V") else {
        eprintln!("[audio] WebRtcAudioRecord nativeDataIsRecorded is not registered");
        return JniValue::Boolean(false);
    };
    let Some(receiver) = receiver else {
        return JniValue::Boolean(false);
    };
    let env_token = attached_thread_env(vm);
    let Ok(global_receiver) = vm.new_global_ref(&env_token, receiver) else {
        return JniValue::Boolean(false);
    };
    let rate = record.sample_rate.load(Ordering::Acquire) as i32;
    let channels = record.channels.load(Ordering::Acquire) as u16;
    let capture = match open_capture(rate, channels) {
        Ok(capture) => capture,
        Err(error) => {
            eprintln!("[audio] WebRtcAudioRecord capture could not start: {error}");
            let _ = vm.delete_global_ref(global_receiver);
            return JniValue::Boolean(false);
        }
    };
    *record
        .global_receiver
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(global_receiver);
    *record.capture.lock().unwrap_or_else(|p| p.into_inner()) = Some(capture);
    record.running.store(true, Ordering::Release);
    let worker_record = Arc::clone(&record);
    *record.worker.lock().unwrap_or_else(|p| p.into_inner()) =
        Some(std::thread::spawn(move || {
            let samples_len = (rate as usize / 100).max(1) * channels as usize;
            let mut samples = vec![0.0f32; samples_len];
            while worker_record.running.load(Ordering::Acquire) {
                let read = {
                    let mut capture = worker_record
                        .capture
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    capture.as_mut().map_or(0, |capture| {
                        capture.read_blocking(&mut samples, Duration::from_millis(20))
                    })
                };
                if read == 0 {
                    continue;
                }
                let muted =
                    WEBRTC_MIC_MUTED.load(Ordering::Acquire) || VOICE_MUTED.load(Ordering::Acquire);
                let count = {
                    let mut guard = worker_record
                        .buffer
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    let Some(buffer) = guard.as_mut() else { break };
                    let count = read.min(buffer.len() / 2);
                    for (sample, bytes) in samples[..count]
                        .iter()
                        .zip(buffer[..count * 2].chunks_exact_mut(2))
                    {
                        let value = if muted {
                            0
                        } else {
                            (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16
                        };
                        bytes.copy_from_slice(&value.to_le_bytes());
                    }
                    count
                };
                let Some(env) = jnivm::current_env() else {
                    break;
                };
                type Data = unsafe extern "system" fn(
                    *mut std::ffi::c_void,
                    *mut std::ffi::c_void,
                    i32,
                    i64,
                );
                // SAFETY: this engine callback came from RegisterNatives; thread is attached to the Rust VM.
                let data: Data = unsafe { std::mem::transmute(data_native) };
                unsafe {
                    data(
                        env,
                        global_receiver.raw() as usize as *mut std::ffi::c_void,
                        (count * 2) as i32,
                        worker_record.native_handle,
                    )
                };
            }
        }));
    eprintln!("[audio] WebRtcAudioRecord opened microphone capture");
    JniValue::Boolean(true)
}

fn webrtc_record_stop(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    if let Some(record) = webrtc_record(vm, receiver) {
        record.stop();
    }
    JniValue::Boolean(true)
}

fn webrtc_record_release(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    if let Some(record) = webrtc_record(vm, receiver) {
        record.stop();
    }
    if let Some(receiver) = receiver {
        audio_objects(&WEBRTC_RECORDS)
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&audio_key(vm, receiver));
    }
    JniValue::Void
}

fn webrtc_record_mute(_: &Vm, _: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    if let Some(JniValue::Boolean(mute)) = args.first() {
        WEBRTC_MIC_MUTED.store(*mute, Ordering::Release);
    }
    JniValue::Void
}

fn webrtc_record_effect(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Boolean(false)
}

fn webrtc_record_default_source(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Int(7)
}

struct WebRtcAudioTrack {
    native_handle: i64,
    sample_rate: AtomicU32,
    channels: AtomicU32,
    buffer: Mutex<Option<Box<[u8]>>>,
    queue: Mutex<Option<Arc<OwnedPcmQueue>>>,
    output: Mutex<Option<Box<dyn OutputStream>>>,
    playing: AtomicBool,
    volume_bits: AtomicU32,
    worker: Mutex<Option<JoinHandle<()>>>,
    global_receiver: Mutex<Option<jnivm::ObjectId>>,
}

impl WebRtcAudioTrack {
    fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    fn stop(&self) {
        self.playing.store(false, Ordering::Release);
        if let Some(queue) = self
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            queue.close();
        }
        if let Some(output) = self
            .output
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            output.set_running(false);
        }
        let mut joined = true;
        if let Some(worker) = self.worker.lock().unwrap_or_else(|p| p.into_inner()).take() {
            if worker.thread().id() != std::thread::current().id() {
                let _ = worker.join();
            } else {
                joined = false;
            }
        }
        if let Some(mut output) = self.output.lock().unwrap_or_else(|p| p.into_inner()).take() {
            output.close();
        }
        self.queue.lock().unwrap_or_else(|p| p.into_inner()).take();
        if joined {
            if let Some(receiver) = self
                .global_receiver
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
            {
                if let Some(vm) = jnivm::process_vm() {
                    let _ = vm.delete_global_ref(receiver);
                }
            }
        }
    }
}

impl Drop for WebRtcAudioTrack {
    fn drop(&mut self) {
        self.stop();
    }
}

fn webrtc_track(vm: &Vm, receiver: Option<jnivm::ObjectId>) -> Option<Arc<WebRtcAudioTrack>> {
    audio_objects(&WEBRTC_TRACKS)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&audio_key(vm, receiver?))
        .cloned()
}

fn install_webrtc_track_methods(vm: &Vm) -> Result<(), String> {
    let class = vm
        .register_class("org/webrtc/voiceengine/WebRtcAudioTrack")
        .map_err(|e| e.to_string())?;
    install_builtin(
        vm,
        class,
        "<init>",
        "(J)Lorg/webrtc/voiceengine/WebRtcAudioTrack;",
        webrtc_track_create,
    )?;
    for (name, descriptor, handler) in [
        (
            "initPlayout",
            "(IID)I",
            webrtc_track_init as jnivm::MethodHandler,
        ),
        ("startPlayout", "()Z", webrtc_track_start),
        ("stopPlayout", "()Z", webrtc_track_stop),
        ("releaseAudioResources", "()V", webrtc_track_release),
        ("getBufferSizeInFrames", "()I", webrtc_track_buffer_frames),
        ("getStreamMaxVolume", "()I", webrtc_track_max_volume),
        ("getStreamVolume", "()I", webrtc_track_volume),
        ("setStreamVolume", "(I)Z", webrtc_track_set_volume),
    ] {
        install_instance_builtin(vm, class, name, descriptor, handler)?;
    }
    install_builtin(
        vm,
        class,
        "setSpeakerMute",
        "(Z)V",
        webrtc_track_speaker_mute,
    )?;
    install_builtin(vm, class, "setAudioTrackUsageAttribute", "(I)V", audio_noop)?;
    Ok(())
}

fn webrtc_track_create(vm: &Vm, _: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let native_handle = match args.first() {
        Some(JniValue::Long(handle)) => *handle,
        _ => 0,
    };
    let class = match vm.find_or_define_class("org/webrtc/voiceengine/WebRtcAudioTrack") {
        Ok(class) => class,
        Err(_) => return JniValue::Object(None),
    };
    let env = attached_thread_env(vm);
    let Ok(object) = vm.new_local_object(&env, class, ObjectValue::Opaque) else {
        return JniValue::Object(None);
    };
    audio_objects(&WEBRTC_TRACKS)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(
            audio_key(vm, object),
            Arc::new(WebRtcAudioTrack {
                native_handle,
                sample_rate: AtomicU32::new(AUDIO_DEFAULT_RATE),
                channels: AtomicU32::new(1),
                buffer: Mutex::new(None),
                queue: Mutex::new(None),
                output: Mutex::new(None),
                playing: AtomicBool::new(false),
                volume_bits: AtomicU32::new(1.0_f32.to_bits()),
                worker: Mutex::new(None),
                global_receiver: Mutex::new(None),
            }),
        );
    JniValue::Object(Some(object))
}

fn webrtc_track_init(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let Some(track) = webrtc_track(vm, receiver) else {
        return JniValue::Int(-1);
    };
    let (Some(JniValue::Int(rate)), Some(JniValue::Int(channels)), Some(JniValue::Double(factor))) =
        (args.first(), args.get(1), args.get(2))
    else {
        return JniValue::Int(-1);
    };
    if !(8_000..=192_000).contains(rate)
        || !(1..=2).contains(channels)
        || track.playing.load(Ordering::Acquire)
    {
        return JniValue::Int(-1);
    }
    let factor = if *factor > 0.0 { *factor } else { 1.0 };
    let frames = ((*rate as f64 / 100.0 * factor) as usize).max(1);
    let bytes = frames.saturating_mul(*channels as usize).saturating_mul(2);
    let mut buffer = vec![0u8; bytes].into_boxed_slice();
    let address = buffer.as_mut_ptr() as usize;
    let class = match vm.find_class("org/webrtc/voiceengine/WebRtcAudioTrack") {
        Ok(class) => class,
        Err(_) => return JniValue::Int(-1),
    };
    let Some(native) = vm.find_native(
        class,
        "nativeCacheDirectBufferAddress",
        "(Ljava/nio/ByteBuffer;J)V",
    ) else {
        eprintln!("[audio] WebRtcAudioTrack nativeCacheDirectBufferAddress is not registered");
        return JniValue::Int(-1);
    };
    let Ok(byte_buffer_class) = vm.find_or_define_class("java/nio/ByteBuffer") else {
        return JniValue::Int(-1);
    };
    let env_token = attached_thread_env(vm);
    let Ok(direct_buffer) = vm.new_local_object(
        &env_token,
        byte_buffer_class,
        ObjectValue::DirectByteBuffer {
            address,
            capacity: bytes as i64,
        },
    ) else {
        return JniValue::Int(-1);
    };
    let Some(receiver) = receiver else {
        return JniValue::Int(-1);
    };
    let Some(env) = jnivm::current_env() else {
        return JniValue::Int(-1);
    };
    type Cache = unsafe extern "system" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        i64,
    );
    // SAFETY: resolved from the VM's RegisterNatives table using its exact dex descriptor.
    let cache: Cache = unsafe { std::mem::transmute(native) };
    unsafe {
        cache(
            env,
            receiver.raw() as usize as *mut std::ffi::c_void,
            direct_buffer.raw() as usize as *mut std::ffi::c_void,
            track.native_handle,
        );
    }
    track.sample_rate.store(*rate as u32, Ordering::Release);
    track.channels.store(*channels as u32, Ordering::Release);
    *track.buffer.lock().unwrap_or_else(|p| p.into_inner()) = Some(buffer);
    JniValue::Int(bytes as i32)
}

fn render_webrtc_pcm(
    queue: &OwnedPcmQueue,
    scratch: &mut [u8],
    dst: &mut [f32],
    volume: f32,
) -> bool {
    dst.fill(0.0);
    let byte_count = dst.len().saturating_mul(2).min(scratch.len());
    let bytes = &mut scratch[..byte_count];
    bytes.fill(0);
    let copied = queue.try_copy(bytes);
    for (sample, pair) in dst
        .iter_mut()
        .zip(bytes[..copied - copied % 2].chunks_exact(2))
    {
        *sample = f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0 * volume;
    }
    true
}

fn webrtc_track_start(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    let Some(track) = webrtc_track(vm, receiver) else {
        return JniValue::Boolean(false);
    };
    if track.playing.load(Ordering::Acquire) {
        return JniValue::Boolean(true);
    }
    let Some(buffer_bytes) = track
        .buffer
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|b| b.len())
    else {
        return JniValue::Boolean(false);
    };
    let class = match vm.find_class("org/webrtc/voiceengine/WebRtcAudioTrack") {
        Ok(class) => class,
        Err(_) => return JniValue::Boolean(false),
    };
    let Some(pull_native) = vm.find_native(class, "nativeGetPlayoutData", "(IJ)V") else {
        eprintln!("[audio] WebRtcAudioTrack nativeGetPlayoutData is not registered");
        return JniValue::Boolean(false);
    };
    let Some(receiver) = receiver else {
        return JniValue::Boolean(false);
    };
    let env_token = attached_thread_env(vm);
    let Ok(global_receiver) = vm.new_global_ref(&env_token, receiver) else {
        return JniValue::Boolean(false);
    };
    let rate = track.sample_rate.load(Ordering::Acquire);
    let channels = track.channels.load(Ordering::Acquire);
    let queue = Arc::new(OwnedPcmQueue::new(4));
    let mut output = make_output_stream();
    let queue_callback = Arc::clone(&queue);
    let volume = Arc::clone(&track);
    let mut scratch = vec![0u8; 1 << 16];
    let request = OutputRequest {
        sample_rate: Some(rate),
        channels: Some(channels),
        sample_bits: 16,
        sample_is_float: false,
        target: String::new(),
    };
    if !output.open(
        request,
        "roblox-runtime (WebRTC voice playout)",
        Box::new(move |dst, _| {
            render_webrtc_pcm(
                &queue_callback,
                &mut scratch,
                dst,
                if WEBRTC_SPEAKER_MUTED.load(Ordering::Acquire) {
                    0.0
                } else {
                    volume.volume()
                },
            )
        }),
    ) || output.rate_hz() != rate
        || output.channels() != channels
    {
        queue.close();
        output.close();
        let _ = vm.delete_global_ref(global_receiver);
        return JniValue::Boolean(false);
    }
    output.set_running(true);
    *track
        .global_receiver
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(global_receiver);
    *track.queue.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::clone(&queue));
    *track.output.lock().unwrap_or_else(|p| p.into_inner()) = Some(output);
    track.playing.store(true, Ordering::Release);
    let pump_track = Arc::clone(&track);
    *track.worker.lock().unwrap_or_else(|p| p.into_inner()) = Some(std::thread::spawn(move || {
        let mut pcm = vec![0u8; buffer_bytes];
        while pump_track.playing.load(Ordering::Acquire) {
            let Some(env) = jnivm::current_env() else {
                break;
            };
            type Pull =
                unsafe extern "system" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, i32, i64);
            // SAFETY: function address was registered by the engine and the worker has attached.
            let pull: Pull = unsafe { std::mem::transmute(pull_native) };
            unsafe {
                pull(
                    env,
                    global_receiver.raw() as usize as *mut std::ffi::c_void,
                    buffer_bytes as i32,
                    pump_track.native_handle,
                )
            };
            {
                let guard = pump_track.buffer.lock().unwrap_or_else(|p| p.into_inner());
                let Some(buffer) = guard.as_ref() else { break };
                pcm.copy_from_slice(buffer);
            }
            let mut pending = pcm;
            loop {
                while queue.take_reclaimed().is_some() {}
                if !pump_track.playing.load(Ordering::Acquire) {
                    break;
                }
                match queue.push(pending, Duration::from_millis(20)) {
                    Ok(()) => break,
                    Err((
                        roblox_audio::backend::owned_pcm_queue::PushError::TimedOut,
                        returned,
                    )) => pending = returned,
                    Err(_) => break,
                }
            }
            pcm = vec![0u8; buffer_bytes];
        }
    }));
    eprintln!("[audio] WebRtcAudioTrack opened host playout");
    JniValue::Boolean(true)
}

fn webrtc_track_stop(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    if let Some(track) = webrtc_track(vm, receiver) {
        track.stop();
    }
    JniValue::Boolean(true)
}

fn webrtc_track_release(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    if let Some(track) = webrtc_track(vm, receiver) {
        track.stop();
    }
    if let Some(receiver) = receiver {
        audio_objects(&WEBRTC_TRACKS)
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&audio_key(vm, receiver));
    }
    JniValue::Void
}

fn webrtc_track_buffer_frames(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    _: &[JniValue],
) -> JniValue {
    JniValue::Int(webrtc_track(vm, receiver).map_or(0, |track| {
        let channels = track.channels.load(Ordering::Acquire).max(1) as usize;
        track
            .buffer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map_or(0, |b| (b.len() / (channels * 2)) as i32)
    }))
}

fn webrtc_track_max_volume(_: &Vm, _: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Int(15)
}

fn webrtc_track_volume(vm: &Vm, receiver: Option<jnivm::ObjectId>, _: &[JniValue]) -> JniValue {
    JniValue::Int(
        webrtc_track(vm, receiver).map_or(15, |track| (track.volume() * 15.0).round() as i32),
    )
}

fn webrtc_track_set_volume(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let Some(track) = webrtc_track(vm, receiver) else {
        return JniValue::Boolean(false);
    };
    let Some(JniValue::Int(volume)) = args.first() else {
        return JniValue::Boolean(false);
    };
    let volume = (*volume).clamp(0, 15) as f32 / 15.0;
    track.volume_bits.store(volume.to_bits(), Ordering::Release);
    JniValue::Boolean(true)
}

fn webrtc_track_speaker_mute(_: &Vm, _: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    if let Some(JniValue::Boolean(mute)) = args.first() {
        WEBRTC_SPEAKER_MUTED.store(*mute, Ordering::Release);
    }
    JniValue::Void
}
