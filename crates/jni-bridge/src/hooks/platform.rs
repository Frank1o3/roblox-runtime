// Android framework behavior reached by the experimental JNI trace.

use std::fs;
use std::path::PathBuf;

unsafe extern "C" {
    fn roblox_get_display_physical_mm(width_mm: *mut i32, height_mm: *mut i32);
}

#[derive(Default)]
struct PreferenceEditor {
    owner: String,
    staged: HashMap<String, String>,
    removed: Vec<String>,
    clear: bool,
}

#[derive(Default)]
struct PreferenceState {
    stores: HashMap<String, HashMap<String, String>>,
    preference_objects: HashMap<jnivm::ObjectId, String>,
    editors: HashMap<jnivm::ObjectId, PreferenceEditor>,
}

static PREFERENCES: OnceLock<Mutex<PreferenceState>> = OnceLock::new();

fn preference_state() -> &'static Mutex<PreferenceState> {
    PREFERENCES.get_or_init(|| Mutex::new(PreferenceState::default()))
}

fn install_platform_methods(vm: &Vm) -> Result<(), String> {
    let activity_thread = vm
        .register_class("android/app/ActivityThread")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        activity_thread,
        "currentActivityThread",
        "()Landroid/app/ActivityThread;",
        activity_thread_current,
    )?;
    install_instance_builtin(
        vm,
        activity_thread,
        "getApplication",
        "()Landroid/app/Application;",
        activity_thread_application,
    )?;

    let context = vm
        .register_class("android/content/Context")
        .map_err(|error| error.to_string())?;
    let application = vm
        .register_class("android/app/Application")
        .map_err(|error| error.to_string())?;
    let activity = vm
        .register_class("android/app/Activity")
        .map_err(|error| error.to_string())?;
    for class in [application, activity] {
        install_instance_builtin(
            vm,
            class,
            "getSharedPreferences",
            "(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
            context_get_shared_preferences,
        )?;
    }
    install_instance_builtin(
        vm,
        context,
        "getSharedPreferences",
        "(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
        context_get_shared_preferences,
    )?;

    let preferences = vm
        .register_class("android/content/SharedPreferences")
        .map_err(|error| error.to_string())?;
    install_instance_builtin(
        vm,
        preferences,
        "edit",
        "()Landroid/content/SharedPreferences$Editor;",
        preferences_edit,
    )?;
    for (name, descriptor, handler) in [
        (
            "getString",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            preferences_get_string as jnivm::MethodHandler,
        ),
        ("getInt", "(Ljava/lang/String;I)I", preferences_get_int),
        ("getLong", "(Ljava/lang/String;J)J", preferences_get_long),
        ("getFloat", "(Ljava/lang/String;F)F", preferences_get_float),
        (
            "getBoolean",
            "(Ljava/lang/String;Z)Z",
            preferences_get_boolean,
        ),
        ("contains", "(Ljava/lang/String;)Z", preferences_contains),
    ] {
        install_instance_builtin(vm, preferences, name, descriptor, handler)?;
    }

    let editor = vm
        .register_class("android/content/SharedPreferences$Editor")
        .map_err(|error| error.to_string())?;
    for (name, descriptor, handler) in [
        (
            "putString",
            "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;",
            editor_put_string as jnivm::MethodHandler,
        ),
        (
            "putInt",
            "(Ljava/lang/String;I)Landroid/content/SharedPreferences$Editor;",
            editor_put_int,
        ),
        (
            "putLong",
            "(Ljava/lang/String;J)Landroid/content/SharedPreferences$Editor;",
            editor_put_long,
        ),
        (
            "putFloat",
            "(Ljava/lang/String;F)Landroid/content/SharedPreferences$Editor;",
            editor_put_float,
        ),
        (
            "putBoolean",
            "(Ljava/lang/String;Z)Landroid/content/SharedPreferences$Editor;",
            editor_put_boolean,
        ),
        (
            "remove",
            "(Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;",
            editor_remove,
        ),
        (
            "clear",
            "()Landroid/content/SharedPreferences$Editor;",
            editor_clear,
        ),
        ("apply", "()V", editor_apply),
        ("commit", "()Z", editor_commit),
    ] {
        install_instance_builtin(vm, editor, name, descriptor, handler)?;
    }

    let build_version = vm
        .register_class("android/os/Build$VERSION")
        .map_err(|error| error.to_string())?;
    let sdk_int = vm
        .register_field(build_version, "SDK_INT", "I", true)
        .map_err(|error| error.to_string())?;
    vm.set_field_value(sdk_int, None, JniValue::Int(33))
        .map_err(|error| error.to_string())?;

    let point = vm
        .register_class("android/graphics/Point")
        .map_err(|error| error.to_string())?;
    vm.register_field(point, "x", "I", false)
        .map_err(|error| error.to_string())?;
    vm.register_field(point, "y", "I", false)
        .map_err(|error| error.to_string())?;

    let device_utils = vm
        .register_class("com/roblox/platform/util/DeviceUtils")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        device_utils,
        "getScreenPhysicalSizeInMillimeters",
        "(Landroid/content/Context;)Landroid/graphics/Point;",
        device_utils_physical_size,
    )?;
    Ok(())
}

fn device_utils_physical_size(
    vm: &Vm,
    _receiver: Option<jnivm::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let mut width_mm = 0;
    let mut height_mm = 0;
    // The native runtime owns the same atomic dimensions used by the C++
    // DeviceUtils implementation. Zero means that the output's physical size
    // is unknown, for which the reference returns null.
    unsafe { roblox_get_display_physical_mm(&mut width_mm, &mut height_mm) };
    if width_mm <= 0 || height_mm <= 0 {
        return JniValue::Object(None);
    }

    let env = attached_thread_env(vm);
    let Ok(class) = vm.find_class("android/graphics/Point") else {
        return JniValue::Object(None);
    };
    let Ok(point) = vm.new_local_object(&env, class, jnivm::ObjectValue::Opaque) else {
        return JniValue::Object(None);
    };
    for (name, value) in [("x", width_mm), ("y", height_mm)] {
        let Ok(field) = vm.find_field(class, name, "I", false) else {
            return JniValue::Object(None);
        };
        if let Err(error) = vm.set_field_value(field, Some(point), JniValue::Int(value)) {
            eprintln!("[jnivm] DeviceUtils could not set Point.{name}: {error}");
            return JniValue::Object(None);
        }
    }
    JniValue::Object(Some(point))
}

fn install_fmod_methods(vm: &Vm) -> Result<(), String> {
    let fmod = vm
        .register_class("org/fmod/FMOD")
        .map_err(|error| error.to_string())?;
    install_builtin(vm, fmod, "checkInit", "()Z", fmod_check_init)?;
    install_builtin(
        vm,
        fmod,
        "supportsLowLatency",
        "()Z",
        fmod_supports_low_latency,
    )?;
    install_builtin(vm, fmod, "supportsAAudio", "()Z", fmod_supports_aaudio)?;
    install_builtin(
        vm,
        fmod,
        "getAssetManager",
        "()Landroid/content/res/AssetManager;",
        fmod_asset_manager,
    )?;

    let audio_device = vm
        .register_class("org/fmod/AudioDevice")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        audio_device,
        "<init>",
        "()Lorg/fmod/AudioDevice;",
        fmod_audio_device_create,
    )?;
    install_instance_builtin(vm, audio_device, "init", "(IIII)Z", fmod_audio_device_init)?;
    install_instance_builtin(vm, audio_device, "write", "([BI)V", fmod_audio_device_write)?;
    install_instance_builtin(vm, audio_device, "close", "()V", fmod_audio_device_close)?;
    Ok(())
}

fn activity_thread_current(
    vm: &Vm,
    _receiver: Option<jnivm::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    new_opaque_local(vm, "android/app/ActivityThread")
}

fn activity_thread_application(
    vm: &Vm,
    _receiver: Option<jnivm::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    new_opaque_local(vm, "android/app/Application")
}

fn context_get_shared_preferences(
    vm: &Vm,
    _receiver: Option<jnivm::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let name = java_string_arg(vm, args, 0).unwrap_or_else(|| "default".to_owned());
    let value = new_opaque_local(vm, "android/content/SharedPreferences");
    let JniValue::Object(Some(object)) = value else {
        return value;
    };
    let mut state = preference_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    ensure_preference_store(&mut state, &name);
    state.preference_objects.insert(object, name);
    JniValue::Object(Some(object))
}

fn preferences_edit(vm: &Vm, receiver: Option<jnivm::ObjectId>, _args: &[JniValue]) -> JniValue {
    let Some(owner_id) = receiver else {
        return JniValue::Object(None);
    };
    let mut state = preference_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(owner) = state.preference_objects.get(&owner_id).cloned() else {
        eprintln!("[jnivm] SharedPreferences.edit received an unknown receiver");
        return JniValue::Object(None);
    };
    let value = new_opaque_local(vm, "android/content/SharedPreferences$Editor");
    let JniValue::Object(Some(editor)) = value else {
        return value;
    };
    state.editors.insert(
        editor,
        PreferenceEditor {
            owner,
            ..PreferenceEditor::default()
        },
    );
    JniValue::Object(Some(editor))
}

fn preferences_get_string(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let Some(value) = preference_get(vm, receiver, args) else {
        return args.get(1).cloned().unwrap_or(JniValue::Object(None));
    };
    java_string(vm, &value, "SharedPreferences")
}

fn preferences_get_int(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    preference_get(vm, receiver, args)
        .and_then(|v| v.parse().ok())
        .map(JniValue::Int)
        .unwrap_or_else(|| args.get(1).cloned().unwrap_or(JniValue::Int(0)))
}

fn preferences_get_long(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    preference_get(vm, receiver, args)
        .and_then(|v| v.parse().ok())
        .map(JniValue::Long)
        .unwrap_or_else(|| args.get(1).cloned().unwrap_or(JniValue::Long(0)))
}

fn preferences_get_float(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    preference_get(vm, receiver, args)
        .and_then(|v| v.parse().ok())
        .map(JniValue::Float)
        .unwrap_or_else(|| args.get(1).cloned().unwrap_or(JniValue::Float(0.0)))
}

fn preferences_get_boolean(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    preference_get(vm, receiver, args)
        .map(|v| JniValue::Boolean(v == "true"))
        .unwrap_or_else(|| args.get(1).cloned().unwrap_or(JniValue::Boolean(false)))
}

fn preferences_contains(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    let found =
        receiver
            .and_then(|id| {
                let state = preference_state()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let owner = state.preference_objects.get(&id)?;
                Some(key.as_ref().is_some_and(|key| {
                    state.stores.get(owner).is_some_and(|s| s.contains_key(key))
                }))
            })
            .unwrap_or(false);
    JniValue::Boolean(found)
}

fn preference_get(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> Option<String> {
    let key = java_string_arg(vm, args, 0)?;
    let state = preference_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let owner = state.preference_objects.get(&receiver?)?;
    state.stores.get(owner)?.get(&key).cloned()
}

fn editor_put_string(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    let value = java_string_arg(vm, args, 1).unwrap_or_default();
    editor_stage(receiver, key, value)
}

fn editor_put_int(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    let value = match args.get(1) {
        Some(JniValue::Int(v)) => v.to_string(),
        _ => "0".to_owned(),
    };
    editor_stage(receiver, key, value)
}

fn editor_put_long(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    let value = match args.get(1) {
        Some(JniValue::Long(v)) => v.to_string(),
        _ => "0".to_owned(),
    };
    editor_stage(receiver, key, value)
}

fn editor_put_float(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    let value = match args.get(1) {
        Some(JniValue::Float(v)) => v.to_string(),
        _ => "0".to_owned(),
    };
    editor_stage(receiver, key, value)
}

fn editor_put_boolean(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    let value = matches!(args.get(1), Some(JniValue::Boolean(true)));
    editor_stage(receiver, key, value.to_string())
}

fn editor_remove(vm: &Vm, receiver: Option<jnivm::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    if let (Some(receiver), Some(key)) = (receiver, key) {
        let mut state = preference_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(editor) = state.editors.get_mut(&receiver) {
            editor.removed.push(key);
        }
    }
    JniValue::Object(receiver)
}

fn editor_clear(_vm: &Vm, receiver: Option<jnivm::ObjectId>, _args: &[JniValue]) -> JniValue {
    if let Some(receiver) = receiver {
        let mut state = preference_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(editor) = state.editors.get_mut(&receiver) {
            editor.clear = true;
        }
    }
    JniValue::Object(receiver)
}

fn editor_apply(_vm: &Vm, receiver: Option<jnivm::ObjectId>, _args: &[JniValue]) -> JniValue {
    if let Some(receiver) = receiver {
        flush_editor(receiver);
    }
    JniValue::Void
}

fn editor_commit(_vm: &Vm, receiver: Option<jnivm::ObjectId>, _args: &[JniValue]) -> JniValue {
    let committed = receiver.is_some_and(flush_editor);
    JniValue::Boolean(committed)
}

fn editor_stage(receiver: Option<jnivm::ObjectId>, key: Option<String>, value: String) -> JniValue {
    if let (Some(receiver), Some(key)) = (receiver, key) {
        let mut state = preference_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(editor) = state.editors.get_mut(&receiver) {
            editor.staged.insert(key, value);
        }
    }
    JniValue::Object(receiver)
}

fn flush_editor(receiver: jnivm::ObjectId) -> bool {
    let mut state = preference_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (owner, clear, removed, staged) = {
        let Some(editor) = state.editors.get_mut(&receiver) else {
            return false;
        };
        (
            editor.owner.clone(),
            std::mem::take(&mut editor.clear),
            std::mem::take(&mut editor.removed),
            std::mem::take(&mut editor.staged),
        )
    };
    let values = state.stores.entry(owner.clone()).or_default();
    if clear {
        values.clear();
    }
    for key in removed {
        values.remove(&key);
    }
    values.extend(staged);
    save_preference_store(&owner, values)
}

fn ensure_preference_store(state: &mut PreferenceState, name: &str) {
    if state.stores.contains_key(name) {
        return;
    }
    let mut values = HashMap::new();
    if let Some(path) = preference_path(name) {
        if let Ok(content) = fs::read_to_string(path) {
            for line in content.lines() {
                let mut parts = line.splitn(3, '\t');
                if parts.next() != Some("S") {
                    continue;
                }
                let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
                    continue;
                };
                values.insert(key.to_owned(), unescape_preference(value));
            }
        }
    }
    state.stores.insert(name.to_owned(), values);
}

fn save_preference_store(name: &str, values: &HashMap<String, String>) -> bool {
    let Some(path) = preference_path(name) else {
        return false;
    };
    let Some(parent) = path.parent() else {
        return false;
    };
    if let Err(error) = fs::create_dir_all(parent) {
        eprintln!("[jnivm] SharedPreferences directory creation failed: {error}");
        return false;
    }
    let mut content = String::new();
    let mut keys: Vec<_> = values.keys().collect();
    keys.sort();
    for key in keys {
        content.push_str("S\t");
        content.push_str(key);
        content.push('\t');
        content.push_str(&escape_preference(&values[key]));
        content.push('\n');
    }
    let mut temp = PathBuf::from(&path);
    temp.set_extension("prefs.tmp");
    if let Err(error) = fs::write(&temp, content).and_then(|()| fs::rename(&temp, &path)) {
        eprintln!("[jnivm] SharedPreferences write failed: {error}");
        let _ = fs::remove_file(temp);
        return false;
    }
    true
}

fn preference_path(name: &str) -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let base = name.rsplit('/').next().unwrap_or(name);
    let safe: String = base
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let safe = if safe.is_empty() { "prefs" } else { &safe };
    let hash = name
        .bytes()
        .fold(1_469_598_103_934_665_603_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211)
        });
    Some(
        cwd.join("shared_prefs")
            .join(format!("{safe}-{:08x}.prefs", hash as u32)),
    )
}

fn escape_preference(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\n', "\\n")
}

fn unescape_preference(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            output.push(match chars.next() {
                Some('n') => '\n',
                Some(other) => other,
                None => '\\',
            });
        } else {
            output.push(ch);
        }
    }
    output
}

fn java_string_arg(vm: &Vm, args: &[JniValue], index: usize) -> Option<String> {
    let Some(JniValue::Object(Some(id))) = args.get(index) else {
        return None;
    };
    let env = attached_thread_env(vm);
    match vm.object_value(&env, *id) {
        Ok(jnivm::ObjectValue::String(units)) => Some(String::from_utf16_lossy(&units)),
        _ => None,
    }
}

fn fmod_check_init(_vm: &Vm, _receiver: Option<jnivm::ObjectId>, _args: &[JniValue]) -> JniValue {
    eprintln!("[jnivm] FMOD.checkInit -> true");
    JniValue::Boolean(true)
}

fn fmod_supports_low_latency(
    _vm: &Vm,
    _receiver: Option<jnivm::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    eprintln!("[jnivm] FMOD.supportsLowLatency -> false");
    JniValue::Boolean(false)
}

fn fmod_supports_aaudio(
    _vm: &Vm,
    _receiver: Option<jnivm::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let supported = FMOD_AAUDIO_SUPPORTED
        .get()
        .is_some_and(|callback| callback());
    eprintln!("[jnivm] FMOD.supportsAAudio -> {supported}");
    JniValue::Boolean(supported)
}

fn fmod_asset_manager(vm: &Vm, _receiver: Option<jnivm::ObjectId>, _args: &[JniValue]) -> JniValue {
    new_opaque_local(vm, "android/content/res/AssetManager")
}

fn fmod_audio_device_create(
    vm: &Vm,
    _receiver: Option<jnivm::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let value = new_opaque_local(vm, "org/fmod/AudioDevice");
    if let JniValue::Object(Some(object)) = value {
        fmod_devices()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                (vm as *const Vm as usize, object),
                std::sync::Arc::new(FmodAudioDevice::default()),
            );
        JniValue::Object(Some(object))
    } else {
        value
    }
}

const FMOD_MAX_CALLBACK_BYTES: usize = 1 << 20;
const FMOD_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Default)]
struct FmodAudioDevice {
    lifecycle: Mutex<()>,
    active: Mutex<Option<FmodActiveOutput>>,
}

struct FmodActiveOutput {
    queue: std::sync::Arc<roblox_audio::backend::owned_pcm_queue::OwnedPcmQueue>,
    output: Box<dyn roblox_audio::backend::output::OutputStream>,
}

type FmodDeviceKey = (usize, jnivm::ObjectId);

fn fmod_devices() -> &'static Mutex<HashMap<FmodDeviceKey, std::sync::Arc<FmodAudioDevice>>> {
    static DEVICES: OnceLock<Mutex<HashMap<FmodDeviceKey, std::sync::Arc<FmodAudioDevice>>>> =
        OnceLock::new();
    DEVICES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn fmod_device(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
) -> Option<std::sync::Arc<FmodAudioDevice>> {
    let receiver = receiver?;
    fmod_devices()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&(vm as *const Vm as usize, receiver))
        .cloned()
}

fn render_fmod_pcm(
    queue: &roblox_audio::backend::owned_pcm_queue::OwnedPcmQueue,
    scratch: &mut [u8],
    output: &mut [f32],
) -> bool {
    output.fill(0.0);
    let Some(byte_count) = output.len().checked_mul(2) else {
        return true;
    };
    if byte_count > scratch.len() {
        return true;
    }
    let bytes = &mut scratch[..byte_count];
    bytes.fill(0);
    let copied = queue.try_copy(bytes);
    for (sample, pair) in output
        .iter_mut()
        .zip(bytes[..copied - copied % 2].chunks_exact(2))
    {
        *sample = f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0;
    }
    true
}

impl FmodAudioDevice {
    fn close(&self) {
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        let active = self.active.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(mut active) = active {
            // Closing first cancels blocked Java writers; stream teardown is
            // outside the active-state lock so the callback can finish.
            active.queue.close();
            active.output.close();
        }
    }

    fn init(&self, channels: i32, rate: i32, buffer_frames: i32, buffer_count: i32) -> bool {
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        self.close_active();
        if !(1..=8).contains(&channels)
            || !(8_000..=192_000).contains(&rate)
            || buffer_frames <= 0
            || buffer_count > 1024
        {
            return false;
        }
        let depth = if buffer_count > 0 {
            buffer_count as usize
        } else {
            4
        };
        let queue = std::sync::Arc::new(
            roblox_audio::backend::owned_pcm_queue::OwnedPcmQueue::new(depth),
        );
        let mut output = roblox_audio::backend::make_output_stream();
        let callback_queue = std::sync::Arc::clone(&queue);
        let mut scratch = vec![0u8; FMOD_MAX_CALLBACK_BYTES];
        let request = roblox_audio::backend::OutputRequest {
            sample_rate: Some(rate as u32),
            channels: Some(channels as u32),
            sample_bits: 16,
            sample_is_float: false,
            target: String::new(),
        };
        if !output.open(
            request,
            "roblox-runtime (Roblox FMOD Java AudioDevice)",
            Box::new(move |dst, _frames| render_fmod_pcm(&callback_queue, &mut scratch, dst)),
        ) {
            queue.close();
            return false;
        }
        if output.rate_hz() != rate as u32 || output.channels() != channels as u32 {
            queue.close();
            output.close();
            return false;
        }
        output.set_running(true);
        *self.active.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(FmodActiveOutput { queue, output });
        true
    }

    fn close_active(&self) {
        let active = self.active.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(mut active) = active {
            active.queue.close();
            active.output.close();
        }
    }

    fn write(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        let Some(queue) = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|active| std::sync::Arc::clone(&active.queue))
        else {
            return;
        };
        let deadline = std::time::Instant::now() + FMOD_WRITE_TIMEOUT;
        let mut bytes = bytes;
        loop {
            while queue.take_reclaimed().is_some() {}
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let wait = remaining.min(std::time::Duration::from_millis(10));
            match queue.push(bytes, wait) {
                Ok(()) => return,
                Err((roblox_audio::backend::owned_pcm_queue::PushError::TimedOut, returned)) => {
                    bytes = returned;
                }
                Err((roblox_audio::backend::owned_pcm_queue::PushError::Closed, _))
                | Err((roblox_audio::backend::owned_pcm_queue::PushError::Empty, _)) => return,
            }
        }
    }
}

impl Drop for FmodAudioDevice {
    fn drop(&mut self) {
        self.close_active();
    }
}

fn fmod_audio_device_init(
    _vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let channels = match args.first() {
        Some(JniValue::Int(value)) => *value,
        _ => 0,
    };
    let rate = match args.get(1) {
        Some(JniValue::Int(value)) => *value,
        _ => 0,
    };
    let buffer_frames = match args.get(2) {
        Some(JniValue::Int(value)) => *value,
        _ => 0,
    };
    let buffer_count = match args.get(3) {
        Some(JniValue::Int(value)) => *value,
        _ => 0,
    };
    let started = fmod_device(_vm, receiver)
        .is_some_and(|device| device.init(channels, rate, buffer_frames, buffer_count));
    JniValue::Boolean(started)
}

fn fmod_audio_device_write(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let Some(device) = fmod_device(vm, receiver) else {
        return JniValue::Void;
    };
    let (Some(JniValue::Object(Some(array))), Some(JniValue::Int(length))) =
        (args.first(), args.get(1))
    else {
        return JniValue::Void;
    };
    if *length <= 0 {
        return JniValue::Void;
    }
    let env = attached_thread_env(vm);
    let Ok(ObjectValue::ByteArray(array_bytes)) = vm.object_value(&env, *array) else {
        return JniValue::Void;
    };
    let length = (*length as usize).min(array_bytes.len());
    let bytes = array_bytes[..length]
        .iter()
        .map(|byte| *byte as u8)
        .collect();
    device.write(bytes);
    JniValue::Void
}

fn fmod_audio_device_close(
    vm: &Vm,
    receiver: Option<jnivm::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    if let Some(receiver) = receiver {
        if let Some(device) = fmod_devices()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&(vm as *const Vm as usize, receiver))
        {
            device.close();
        }
    }
    JniValue::Void
}

#[cfg(test)]
mod fmod_audio_device_tests {
    use super::*;

    #[test]
    fn render_converts_owned_pcm_and_reclaims_off_the_callback_path() {
        let queue = roblox_audio::backend::owned_pcm_queue::OwnedPcmQueue::new(2);
        queue
            .push(vec![0x00, 0x40, 0x00, 0xC0], std::time::Duration::ZERO)
            .unwrap();
        let mut scratch = [0u8; 8];
        let mut output = [1.0f32; 4];
        assert!(render_fmod_pcm(&queue, &mut scratch, &mut output));
        assert_eq!(output, [0.5, -0.5, 0.0, 0.0]);
        assert_eq!(queue.take_reclaimed().unwrap(), [0x00, 0x40, 0x00, 0xC0]);
    }

    #[test]
    fn render_preserves_partial_pcm_chunks_between_host_callbacks() {
        let queue = roblox_audio::backend::owned_pcm_queue::OwnedPcmQueue::new(1);
        queue
            .push(vec![0x00, 0x40, 0x00, 0xC0], std::time::Duration::ZERO)
            .unwrap();
        let mut scratch = [0u8; 2];
        let mut first = [0.0f32; 1];
        let mut second = [0.0f32; 1];
        render_fmod_pcm(&queue, &mut scratch, &mut first);
        assert!(queue.take_reclaimed().is_none());
        render_fmod_pcm(&queue, &mut scratch, &mut second);
        assert_eq!(first, [0.5]);
        assert_eq!(second, [-0.5]);
        assert_eq!(queue.take_reclaimed().unwrap(), [0x00, 0x40, 0x00, 0xC0]);
    }
}

static FMOD_AAUDIO_SUPPORTED: OnceLock<fn() -> bool> = OnceLock::new();

pub fn set_fmod_aaudio_support(callback: fn() -> bool) -> Result<(), &'static str> {
    FMOD_AAUDIO_SUPPORTED
        .set(callback)
        .map_err(|_| "FMOD AAudio support callback is already installed")
}
