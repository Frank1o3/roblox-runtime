// Small Java-side behaviors used during the observed startup path.
use std::net::UdpSocket;
use std::time::Instant;

static STARTUP_BOOTSTRAP: OnceLock<extern "C" fn()> = OnceLock::new();

/// Install the runtime-owned startup callback for the Rust VM's Java-side
/// `GameActivity.bootstrapTheApp()` hook.
pub fn set_startup_bootstrap(callback: extern "C" fn()) -> Result<(), &'static str> {
    STARTUP_BOOTSTRAP
        .set(callback)
        .map_err(|_| "Rust JNI startup bootstrap is already installed")
}

fn install_builtin_methods(vm: &Vm) -> Result<(), String> {
    let class = vm
        .register_class("java/lang/Class")
        .map_err(|error| error.to_string())?;
    install_instance_builtin(
        vm,
        class,
        "getClassLoader",
        "()Ljava/lang/ClassLoader;",
        class_get_class_loader,
    )?;

    let class_loader = vm
        .register_class("java/lang/ClassLoader")
        .map_err(|error| error.to_string())?;
    for method_name in ["findClass", "loadClass"] {
        install_instance_builtin(
            vm,
            class_loader,
            method_name,
            "(Ljava/lang/String;)Ljava/lang/Class;",
            class_loader_find_class,
        )?;
    }

    let locale = vm
        .register_class("com/roblox/engine/jni/locale/NativeLocaleJavaInterface")
        .map_err(|error| error.to_string())?;
    for method_name in ["getLocale", "getRobloxLocale", "getGameLocale"] {
        install_builtin(vm, locale, method_name, "()Ljava/lang/String;", locale_method)?;
    }

    let user = vm
        .register_class("com/roblox/engine/jni/user/NativeUserJavaInterface")
        .map_err(|error| error.to_string())?;
    for method_name in ["getUserId"] {
        install_builtin(vm, user, method_name, "()J", anonymous_user_id)?;
    }
    for method_name in ["getIsUnder13", "getHasRobloxSubscription"] {
        install_builtin(vm, user, method_name, "()Z", anonymous_user_boolean)?;
    }
    install_builtin(vm, user, "getMembershipType", "()I", anonymous_user_membership)?;
    for method_name in ["getUsername", "getDisplayName", "getAlternateName"] {
        install_builtin(vm, user, method_name, "()Ljava/lang/String;", anonymous_user_string)?;
    }
    install_builtin(
        vm,
        user,
        "getPlatformName",
        "()Ljava/lang/String;",
        platform_name_string,
    )?;
    install_builtin(vm, user, "getTheme", "()Ljava/lang/String;", theme_string)?;

    let native_gl = vm
        .register_class("com/roblox/engine/jni/NativeGLJavaInterface")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        native_gl,
        "getDeviceStaticParams",
        "()Lcom/roblox/engine/jni/model/DeviceStaticParams;",
        device_static_params,
    )?;
    for (method_name, descriptor) in [
        ("showKeyboard", "(JZ[BLcom/roblox/engine/jni/model/NativeTextBoxInfo;)V"),
        ("hideKeyboard", "()V"),
        ("promptNativePurchase", "(JLjava/lang/String;Ljava/lang/String;)V"),
        ("promptNativePurchase", "(JLjava/lang/String;)V"),
        ("promptNativePurchaseWithPayload", "(JLjava/lang/String;Ljava/lang/String;)V"),
        (
            "promptNativePurchaseWithPaymentSessionId",
            "(JLjava/lang/String;Ljava/lang/String;)V",
        ),
        (
            "promptNativePurchaseWithPaymentSessionId",
            "(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
        ),
        ("exitGameWithError", "(I)V"),
        ("gameDidLeave", "()V"),
        ("onAppShellReloadNeeded", "()V"),
        ("listenToMotionEvents", "(Ljava/lang/String;)V"),
        ("screenOrientationChanged", "(I)V"),
        ("openNativeOverlay", "(Ljava/lang/String;Ljava/lang/String;)V"),
        (
            "onDataModelNotificationCallback",
            "(Ljava/lang/String;Ljava/lang/String;)V",
        ),
        ("onLuaTextBoxChangedCallback", "(Ljava/lang/String;)V"),
        ("onLuaTextBoxPropertyChangedCallback", "()V"),
        ("onAppBridgeNotification", "(Ljava/lang/String;Ljava/lang/String;)V"),
        ("onExtendedAnalyticsRecvCallback", "([BI)V"),
        ("saveImageToAlbum", "(Ljava/lang/String;)V"),
        ("onVrSessionStateUpdate", "(I)V"),
        ("getWebViewUserAgent", "()V"),
        ("getMobileAdvertisingId", "()V"),
    ] {
        install_builtin(vm, native_gl, method_name, descriptor, reporter_noop)?;
    }
    install_builtin(
        vm,
        native_gl,
        "gameLoadedCallback",
        "(J)V",
        native_helper_game_loaded,
    )?;

    let media_codec = vm
        .register_class("com/roblox/engine/jni/video/MediaCodecInfoUtils")
        .map_err(|error| error.to_string())?;
    let _video_codec = vm
        .register_class("com/roblox/engine/jni/video/VideoCodecCapability")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        media_codec,
        "getVideoCodecs",
        "()[Lcom/roblox/engine/jni/video/VideoCodecCapability;",
        empty_video_codecs,
    )?;
    install_builtin(
        vm,
        media_codec,
        "hevcHardwareEncodingSupported",
        "(III)Z",
        no_hardware_codec,
    )?;

    let network_utils = vm
        .register_class("com/roblox/engine/jni/util/NetworkUtils")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        network_utils,
        "getPublicIPv4Addresseses",
        "()Ljava/lang/String;",
        network_ipv4_address,
    )?;

    let logging = vm
        .register_class("com/roblox/universalapp/logging/LoggingProtocol")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        logging,
        "getProcessTimestamp",
        "()J",
        process_timestamp,
    )?;

    let reporter = vm
        .register_class("com/roblox/engine/jni/reporter/SessionReporterJavaInterface")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        reporter,
        "getFilesDir",
        "()Ljava/lang/String;",
        reporter_files_dir,
    )?;
    for method_name in ["getAppVersion", "getLastLoggedInUser", "getLastLoggedInUserId"] {
        install_builtin(
            vm,
            reporter,
            method_name,
            "()Ljava/lang/String;",
            reporter_empty_string,
        )?;
    }
    install_builtin(
        vm,
        reporter,
        "sendSessionReport",
        "(Ljava/lang/String;Ljava/lang/String;)V",
        reporter_noop,
    )?;
    install_builtin(
        vm,
        reporter,
        "setEventTrackingGoogleAnalytics",
        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;J)V",
        reporter_noop,
    )?;

    // The current experimental trace reaches an Activity through an unknown
    // Java reference, so GetObjectClass reports `Invalid`. Keep the same
    // minimal context answers the C++ Activity hooks provide, allowing the
    // startup path to obtain Resources and display density even before the
    // cross-VM object bridge is implemented.
    let invalid = vm
        .register_class("Invalid")
        .map_err(|error| error.to_string())?;
    install_instance_builtin(
        vm,
        invalid,
        "getResources",
        "()Landroid/content/res/Resources;",
        invalid_get_resources,
    )?;
    install_instance_builtin(
        vm,
        invalid,
        "getDisplayMetrics",
        "()Landroid/util/DisplayMetrics;",
        invalid_get_display_metrics,
    )?;
    install_instance_builtin(
        vm,
        invalid,
        "getNativeHelper",
        "()Lcom/roblox/client/startup/NativeHelper;",
        invalid_get_native_helper,
    )?;
    install_instance_builtin(vm, invalid, "bootstrapTheApp", "()V", invalid_bootstrap)?;

    let resources = vm
        .register_class("android/content/res/Resources")
        .map_err(|error| error.to_string())?;
    install_instance_builtin(
        vm,
        resources,
        "getDisplayMetrics",
        "()Landroid/util/DisplayMetrics;",
        invalid_get_display_metrics,
    )?;

    let native_helper = vm
        .register_class("com/roblox/client/startup/NativeHelper")
        .map_err(|error| error.to_string())?;
    for (method_name, descriptor, handler) in [
        ("gameActivity_onFlagsFailed", "()V", native_helper_flags_failed as crate::MethodHandler),
        ("gameActivity_onFlagsLoaded", "(Ljava/nio/ByteBuffer;)V", native_helper_flags_loaded),
        ("gameActivity_onAppReady", "(Ljava/lang/String;)V", native_helper_app_ready),
        ("gameActivity_onExperienceStart", "()V", native_helper_experience_start),
        ("gameActivity_onGameLoaded", "(J)V", native_helper_game_loaded),
        ("gameActivity_onDidLogInReceived", "(Ljava/lang/String;)V", native_helper_logged_in),
        ("gameActivity_onScreenOrientationChanged", "(IZ)V", native_helper_orientation_changed),
        ("gameActivity_onEngineInitialized", "()V", native_helper_engine_initialized),
        ("gameActivity_onDidLogOutReceived", "()V", native_helper_logged_out),
        ("gameActivity_onDidSwitchAccountReceived", "()V", native_helper_account_switched),
        ("gameActivity_onLuaAppDidReturn", "()V", native_helper_lua_app_returned),
        ("gameActivity_onRestartLuaApp", "()V", native_helper_restart_lua_app),
        ("gameActivity_onScanQrCode", "()V", native_helper_scan_qr_code),
        ("gameActivity_onDidSignUp", "(Ljava/lang/String;)V", native_helper_signed_up),
        ("gameActivity_onGameStreamingStatusChanged", "(Ljava/lang/String;)V", native_helper_streaming_status),
        ("gameActivity_onScreenshotReady", "(Ljava/lang/String;)V", native_helper_screenshot_ready),
        ("gameActivity_onMotionEventListening", "(Ljava/lang/String;)V", native_helper_motion_listening),
        ("gameActivity_onExperienceStop", "(D)V", native_helper_experience_stopped),
        ("gameActivity_setAppUpgradeStatus", "(IILjava/lang/String;Ljava/lang/String;)V", native_helper_upgrade_status),
    ] {
        install_instance_builtin(vm, native_helper, method_name, descriptor, handler)?;
    }

    // Register the Java classes and fields backed by values above so these
    // intentional implementations do not surface as placeholder lookups.
    let string_class = vm
        .register_class("java/lang/String")
        .map_err(|error| error.to_string())?;
    let _ = string_class;
    let java_object = vm
        .register_class("java/lang/Object")
        .map_err(|error| error.to_string())?;
    let _ = java_object;
    let device_static_params_class = vm
        .register_class("com/roblox/engine/jni/model/DeviceStaticParams")
        .map_err(|error| error.to_string())?;
    for (name, descriptor) in [
        ("osVersion", "Ljava/lang/String;"),
        ("deviceName", "Ljava/lang/String;"),
        ("appVersion", "Ljava/lang/String;"),
        ("manufacturer", "Ljava/lang/String;"),
        ("deviceSku", "Ljava/lang/String;"),
        ("appBuildVariant", "Ljava/lang/String;"),
        ("socModel", "Ljava/lang/String;"),
        ("cpu64Bit", "Z"),
    ] {
        vm.register_field(device_static_params_class, name, descriptor, false)
            .map_err(|error| error.to_string())?;
    }
    let display_metrics = vm
        .register_class("android/util/DisplayMetrics")
        .map_err(|error| error.to_string())?;
    vm.register_field(display_metrics, "density", "F", false)
        .map_err(|error| error.to_string())?;

    install_platform_methods(vm)?;
    install_fmod_methods(vm)?;

    Ok(())
}

fn new_opaque_local(vm: &Vm, class_name: &str) -> JniValue {
    let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    let Ok(class) = vm.find_or_define_class(class_name) else {
        return JniValue::Object(None);
    };
    match vm.new_local_object(&env, class, crate::ObjectValue::Opaque) {
        Ok(object) => JniValue::Object(Some(object)),
        Err(error) => {
            eprintln!("[jnivm] {class_name} allocation failed: {error}");
            JniValue::Object(None)
        }
    }
}

fn invalid_get_resources(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    new_opaque_local(vm, "android/content/res/Resources")
}

fn invalid_get_display_metrics(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let value = new_opaque_local(vm, "android/util/DisplayMetrics");
    let JniValue::Object(Some(object)) = value else {
        return value;
    };
    let Ok(class) = vm.find_or_define_class("android/util/DisplayMetrics") else {
        return JniValue::Object(Some(object));
    };
    if let Ok(field) = vm.resolve_field(class, "density", "F", false) {
        let _ = vm.set_field_value(field, Some(object), JniValue::Float(1.0));
    }
    JniValue::Object(Some(object))
}

fn invalid_get_native_helper(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    new_opaque_local(vm, "com/roblox/client/startup/NativeHelper")
}

fn invalid_bootstrap(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    if let Some(callback) = STARTUP_BOOTSTRAP.get() {
        callback();
    } else {
        eprintln!("[jnivm] Invalid.bootstrapTheApp has no Rust host bootstrap callback");
    }
    JniValue::Void
}

fn process_timestamp(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    static START: OnceLock<Instant> = OnceLock::new();
    JniValue::Long(START.get_or_init(Instant::now).elapsed().as_millis() as i64)
}

fn reporter_files_dir(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let path = std::env::var("RBX_RUNTIME_FILES_DIR")
        .ok()
        .filter(|path| !path.is_empty())
        .unwrap_or_else(default_files_dir);
    java_string(vm, &path, "SessionReporterJavaInterface")
}

fn default_files_dir() -> String {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|path| !path.is_empty())
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|path| !path.is_empty())
                .map(|home| format!("{home}/.local/share"))
        })
        .unwrap_or_else(|| "/tmp".to_owned());
    format!("{base}/cordial/instances/default/data")
}

fn reporter_empty_string(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    java_string(vm, "", "SessionReporterJavaInterface")
}

fn reporter_noop(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    JniValue::Void
}

fn no_hardware_codec(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    // C++ reports false because this runtime has no Android MediaCodec.
    JniValue::Boolean(false)
}

fn empty_video_codecs(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    match vm.new_local_object_array(&env, 0, None) {
        Ok(array) => JniValue::Object(Some(array)),
        Err(error) => {
            eprintln!("[jnivm] MediaCodecInfoUtils returned no codec array: {error}");
            JniValue::Object(None)
        }
    }
}

fn network_ipv4_address(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    // Match the C++ handler's local-address semantics without contacting a
    // public service. UDP connect selects the host's default IPv4 route but
    // sends no packet; this reports that route's address when one exists.
    let address = UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.connect("192.0.2.1:9")?;
            socket.local_addr()
        })
        .map(|address| address.ip().to_string())
        .unwrap_or_default();
    java_string(vm, &address, "NetworkUtils")
}

fn class_get_class_loader(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    let Ok(class) = vm.find_or_define_class("java/lang/ClassLoader") else {
        return JniValue::Object(None);
    };
    match vm.new_local_object(&env, class, crate::ObjectValue::Opaque) {
        Ok(object) => JniValue::Object(Some(object)),
        Err(error) => {
            eprintln!("[jnivm] Class.getClassLoader failed: {error}");
            JniValue::Object(None)
        }
    }
}

fn class_loader_find_class(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let Some(JniValue::Object(Some(name))) = args.first() else {
        return JniValue::Object(None);
    };
    let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    let Ok(crate::ObjectValue::String(units)) = vm.object_value(&env, *name) else {
        eprintln!("[jnivm] ClassLoader class lookup received a non-string name");
        return JniValue::Object(None);
    };
    let mut name = String::from_utf16_lossy(&units);
    name = name.replace('.', "/");
    match vm.find_or_define_class(&name) {
        Ok(class) => JniValue::Object(Some(crate::ObjectId(class.0))),
        Err(error) => {
            eprintln!("[jnivm] ClassLoader class lookup failed for {name}: {error}");
            JniValue::Object(None)
        }
    }
}

fn device_static_params(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    let Ok(class) = vm.find_or_define_class("com/roblox/engine/jni/model/DeviceStaticParams") else {
        return JniValue::Object(None);
    };
    let Ok(object) = vm.new_local_object(&env, class, crate::ObjectValue::Opaque) else {
        return JniValue::Object(None);
    };
    let profile = std::env::var("RBX_RUNTIME_DEVICE_PROFILE")
        .unwrap_or_else(|_| "pc-windows-11".to_owned());
    let device_name = std::env::var("RBX_RUNTIME_DEVICE_NAME")
        .unwrap_or_else(|_| "Roblox Runtime".to_owned());
    let fields = [
        ("osVersion", "33"),
        ("deviceName", device_name.as_str()),
        ("appVersion", ""),
        ("manufacturer", "Roblox Runtime"),
        ("deviceSku", profile.as_str()),
        ("appBuildVariant", "release"),
        ("socModel", "unknown"),
    ];
    for (name, value) in fields {
        let Ok(field) = vm.resolve_field(class, name, "Ljava/lang/String;", false) else {
            continue;
        };
        let JniValue::Object(Some(value)) = java_string(vm, value, "DeviceStaticParams") else {
            continue;
        };
        if let Err(error) = vm.set_field_value(field, Some(object), JniValue::Object(Some(value))) {
            eprintln!("[jnivm] DeviceStaticParams.{name} setup failed: {error}");
        }
    }
    if let Ok(field) = vm.resolve_field(class, "cpu64Bit", "Z", false) {
        let _ = vm.set_field_value(field, Some(object), JniValue::Boolean(true));
    }
    JniValue::Object(Some(object))
}

fn install_builtin(
    vm: &Vm,
    class: crate::ClassId,
    name: &str,
    descriptor: &str,
    handler: crate::MethodHandler,
) -> Result<(), String> {
    let method = vm
        .register_method(class, name, descriptor, true)
        .map_err(|error| error.to_string())?;
    vm.install_method_handler(method, handler)
        .map_err(|error| error.to_string())
}

fn install_instance_builtin(
    vm: &Vm,
    class: crate::ClassId,
    name: &str,
    descriptor: &str,
    handler: crate::MethodHandler,
) -> Result<(), String> {
    let method = vm
        .register_method(class, name, descriptor, false)
        .map_err(|error| error.to_string())?;
    vm.install_method_handler(method, handler)
        .map_err(|error| error.to_string())
}

fn locale_method(vm: &Vm, _receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
    let mut locale = std::env::var("LC_ALL")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("LC_MESSAGES")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .or_else(|| std::env::var("LANG").ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "en_us".to_owned());
    if let Some(encoding) = locale.find('.') {
        locale.truncate(encoding);
    }
    if locale.is_empty() || locale == "C" || locale == "POSIX" {
        locale = "en_us".to_owned();
    }
    locale.make_ascii_lowercase();
    java_string(vm, &locale, "NativeLocaleJavaInterface")
}

fn anonymous_user_id(_vm: &Vm, _receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
    JniValue::Long(0)
}

fn anonymous_user_boolean(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    JniValue::Boolean(false)
}

fn anonymous_user_membership(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    JniValue::Int(0)
}

fn anonymous_user_string(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    java_string(vm, "", "NativeUserJavaInterface")
}

fn platform_name_string(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let name = if let Ok(name) = std::env::var("RBX_RUNTIME_PLATFORM_NAME") {
        name
    } else {
        match std::env::var("RBX_RUNTIME_DEVICE_PROFILE")
            .unwrap_or_else(|_| "pc-windows-11".to_owned())
            .to_ascii_lowercase()
            .as_str()
        {
            "roblox-app" | "app" | "roblox" | "android-tablet" | "android" | "tablet" => {
                "Android".to_owned()
            }
            _ => "Windows".to_owned(),
        }
    };
    java_string(vm, &name, "NativeUserJavaInterface")
}

fn theme_string(vm: &Vm, _receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
    java_string(vm, "Dark", "NativeUserJavaInterface")
}

fn java_string(vm: &Vm, value: &str, source: &str) -> JniValue {
    let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    let Ok(class) = vm.find_or_define_class("java/lang/String") else {
        return JniValue::Object(None);
    };
    match vm.new_local_object(
        &env,
        class,
        crate::ObjectValue::String(value.encode_utf16().collect()),
    ) {
        Ok(object) => JniValue::Object(Some(object)),
        Err(error) => {
            eprintln!("[jnivm] {source} returned no string: {error}");
            JniValue::Object(None)
        }
    }
}
