// Android framework behavior reached by the experimental JNI trace.

use std::fs;
use std::path::PathBuf;

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
    preference_objects: HashMap<crate::ObjectId, String>,
    editors: HashMap<crate::ObjectId, PreferenceEditor>,
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
            preferences_get_string as crate::MethodHandler,
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
            editor_put_string as crate::MethodHandler,
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
    Ok(())
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
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    new_opaque_local(vm, "android/app/ActivityThread")
}

fn activity_thread_application(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    new_opaque_local(vm, "android/app/Application")
}

fn context_get_shared_preferences(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
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

fn preferences_edit(vm: &Vm, receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
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
    receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let Some(value) = preference_get(vm, receiver, args) else {
        return args.get(1).cloned().unwrap_or(JniValue::Object(None));
    };
    java_string(vm, &value, "SharedPreferences")
}

fn preferences_get_int(vm: &Vm, receiver: Option<crate::ObjectId>, args: &[JniValue]) -> JniValue {
    preference_get(vm, receiver, args)
        .and_then(|v| v.parse().ok())
        .map(JniValue::Int)
        .unwrap_or_else(|| args.get(1).cloned().unwrap_or(JniValue::Int(0)))
}

fn preferences_get_long(vm: &Vm, receiver: Option<crate::ObjectId>, args: &[JniValue]) -> JniValue {
    preference_get(vm, receiver, args)
        .and_then(|v| v.parse().ok())
        .map(JniValue::Long)
        .unwrap_or_else(|| args.get(1).cloned().unwrap_or(JniValue::Long(0)))
}

fn preferences_get_float(
    vm: &Vm,
    receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    preference_get(vm, receiver, args)
        .and_then(|v| v.parse().ok())
        .map(JniValue::Float)
        .unwrap_or_else(|| args.get(1).cloned().unwrap_or(JniValue::Float(0.0)))
}

fn preferences_get_boolean(
    vm: &Vm,
    receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    preference_get(vm, receiver, args)
        .map(|v| JniValue::Boolean(v == "true"))
        .unwrap_or_else(|| args.get(1).cloned().unwrap_or(JniValue::Boolean(false)))
}

fn preferences_contains(vm: &Vm, receiver: Option<crate::ObjectId>, args: &[JniValue]) -> JniValue {
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

fn preference_get(vm: &Vm, receiver: Option<crate::ObjectId>, args: &[JniValue]) -> Option<String> {
    let key = java_string_arg(vm, args, 0)?;
    let state = preference_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let owner = state.preference_objects.get(&receiver?)?;
    state.stores.get(owner)?.get(&key).cloned()
}

fn editor_put_string(vm: &Vm, receiver: Option<crate::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    let value = java_string_arg(vm, args, 1).unwrap_or_default();
    editor_stage(receiver, key, value)
}

fn editor_put_int(vm: &Vm, receiver: Option<crate::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    let value = match args.get(1) {
        Some(JniValue::Int(v)) => v.to_string(),
        _ => "0".to_owned(),
    };
    editor_stage(receiver, key, value)
}

fn editor_put_long(vm: &Vm, receiver: Option<crate::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    let value = match args.get(1) {
        Some(JniValue::Long(v)) => v.to_string(),
        _ => "0".to_owned(),
    };
    editor_stage(receiver, key, value)
}

fn editor_put_float(vm: &Vm, receiver: Option<crate::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    let value = match args.get(1) {
        Some(JniValue::Float(v)) => v.to_string(),
        _ => "0".to_owned(),
    };
    editor_stage(receiver, key, value)
}

fn editor_put_boolean(vm: &Vm, receiver: Option<crate::ObjectId>, args: &[JniValue]) -> JniValue {
    let key = java_string_arg(vm, args, 0);
    let value = matches!(args.get(1), Some(JniValue::Boolean(true)));
    editor_stage(receiver, key, value.to_string())
}

fn editor_remove(vm: &Vm, receiver: Option<crate::ObjectId>, args: &[JniValue]) -> JniValue {
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

fn editor_clear(_vm: &Vm, receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
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

fn editor_apply(_vm: &Vm, receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
    if let Some(receiver) = receiver {
        flush_editor(receiver);
    }
    JniValue::Void
}

fn editor_commit(_vm: &Vm, receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
    let committed = receiver.is_some_and(flush_editor);
    JniValue::Boolean(committed)
}

fn editor_stage(receiver: Option<crate::ObjectId>, key: Option<String>, value: String) -> JniValue {
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

fn flush_editor(receiver: crate::ObjectId) -> bool {
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
    let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    match vm.object_value(&env, *id) {
        Ok(crate::ObjectValue::String(units)) => Some(String::from_utf16_lossy(&units)),
        _ => None,
    }
}

fn fmod_check_init(_vm: &Vm, _receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
    eprintln!("[jnivm] FMOD.checkInit -> true");
    JniValue::Boolean(true)
}

fn fmod_supports_low_latency(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    eprintln!("[jnivm] FMOD.supportsLowLatency -> false");
    JniValue::Boolean(false)
}

fn fmod_supports_aaudio(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let supported = FMOD_AAUDIO_SUPPORTED
        .get()
        .is_some_and(|callback| callback());
    eprintln!("[jnivm] FMOD.supportsAAudio -> {supported}");
    JniValue::Boolean(supported)
}

fn fmod_audio_device_create(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    new_opaque_local(vm, "org/fmod/AudioDevice")
}

fn fmod_audio_device_init(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
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
    eprintln!(
        "[jnivm] FMOD Java AudioDevice sink is unavailable in this VM (channels={channels}, rate={rate}); AAudio should be selected when supported"
    );
    JniValue::Boolean(false)
}

fn fmod_audio_device_write(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    // `init` refuses this Java fallback, so there is no active sink to write.
    JniValue::Void
}

fn fmod_audio_device_close(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    JniValue::Void
}

static FMOD_AAUDIO_SUPPORTED: OnceLock<fn() -> bool> = OnceLock::new();

pub fn set_fmod_aaudio_support(callback: fn() -> bool) -> Result<(), &'static str> {
    FMOD_AAUDIO_SUPPORTED
        .set(callback)
        .map_err(|_| "FMOD AAudio support callback is already installed")
}
