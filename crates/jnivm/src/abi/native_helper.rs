// NativeHelper callback implementations mirrored from native/init_params.cpp.

static GAME_LOADED_CALLBACK: OnceLock<extern "C" fn(i64)> = OnceLock::new();

pub fn set_game_loaded_callback(callback: extern "C" fn(i64)) -> Result<(), &'static str> {
    GAME_LOADED_CALLBACK
        .set(callback)
        .map_err(|_| "Rust JNI game-loaded callback is already installed")
}

fn native_helper_flags_failed(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    eprintln!("[roblox] flags: engine reported onFlagsFailed");
    JniValue::Void
}

fn native_helper_flags_loaded(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let capacity = match args.first() {
        Some(JniValue::Object(Some(buffer))) => match vm.object_value(
            &vm.get_env().unwrap_or_else(|| vm.attach_current_thread()),
            *buffer,
        ) {
            Ok(crate::ObjectValue::DirectByteBuffer { capacity, .. }) => capacity,
            _ => -1,
        },
        _ => -1,
    };
    eprintln!("[roblox] flags loaded ({capacity} bytes)");
    JniValue::Void
}

fn native_helper_app_ready(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    eprintln!("[roblox] app ready: {}", native_helper_string(vm, args, 0));
    JniValue::Void
}

fn native_helper_experience_start(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    eprintln!("[roblox] experience start");
    JniValue::Void
}

fn native_helper_game_loaded(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let place_id = match args.first() {
        Some(JniValue::Long(value)) => *value,
        _ => 0,
    };
    eprintln!("[roblox] game loaded: place {place_id}");
    if let Some(callback) = GAME_LOADED_CALLBACK.get() {
        callback(place_id);
    }
    JniValue::Void
}

fn native_helper_logged_in(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    // This is a session identifier; mirror the C++ bridge and never print it.
    eprintln!(
        "[roblox] logged in ({} bytes, not shown)",
        native_helper_string(vm, args, 0).len()
    );
    JniValue::Void
}

fn native_helper_orientation_changed(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let orientation = match args.first() {
        Some(JniValue::Int(value)) => *value,
        _ => 0,
    };
    let locked = matches!(args.get(1), Some(JniValue::Boolean(true)));
    eprintln!(
        "[roblox] orientation {orientation} (locked: {})",
        if locked { "yes" } else { "no" }
    );
    JniValue::Void
}

macro_rules! native_helper_notice {
    ($name:ident, $message:literal) => {
        fn $name(_vm: &Vm, _receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
            eprintln!("[roblox] {}", $message);
            JniValue::Void
        }
    };
}

native_helper_notice!(native_helper_engine_initialized, "engine initialised");
native_helper_notice!(native_helper_logged_out, "logged out");
native_helper_notice!(native_helper_account_switched, "account switched");
native_helper_notice!(native_helper_lua_app_returned, "lua app returned");
native_helper_notice!(native_helper_restart_lua_app, "lua app restart requested");
native_helper_notice!(
    native_helper_scan_qr_code,
    "QR scan requested (no camera handler)"
);

fn native_helper_signed_up(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    eprintln!(
        "[roblox] signed up ({} bytes, not shown)",
        native_helper_string(vm, args, 0).len()
    );
    JniValue::Void
}

fn native_helper_streaming_status(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    eprintln!(
        "[roblox] game streaming status: {}",
        native_helper_string(vm, args, 0)
    );
    JniValue::Void
}

fn native_helper_screenshot_ready(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    eprintln!(
        "[roblox] screenshot ready: {}",
        native_helper_string(vm, args, 0)
    );
    JniValue::Void
}

fn native_helper_motion_listening(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    eprintln!(
        "[roblox] motion event listening: {}",
        native_helper_string(vm, args, 0)
    );
    JniValue::Void
}

fn native_helper_experience_stopped(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let seconds = match args.first() {
        Some(JniValue::Double(value)) => *value,
        _ => 0.0,
    };
    eprintln!("[roblox] experience stop ({seconds:.3} s)");
    JniValue::Void
}

fn native_helper_upgrade_status(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let status = match args.first() {
        Some(JniValue::Int(value)) => *value,
        _ => 0,
    };
    let flags = match args.get(1) {
        Some(JniValue::Int(value)) => *value,
        _ => 0,
    };
    eprintln!(
        "[roblox] app upgrade status {status}/{flags} {} {}",
        native_helper_string(vm, args, 2),
        native_helper_string(vm, args, 3)
    );
    JniValue::Void
}

fn native_helper_string(vm: &Vm, args: &[JniValue], index: usize) -> String {
    let Some(JniValue::Object(Some(object))) = args.get(index) else {
        return String::new();
    };
    let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    match vm.object_value(&env, *object) {
        Ok(crate::ObjectValue::String(units)) => String::from_utf16_lossy(&units),
        _ => String::new(),
    }
}
