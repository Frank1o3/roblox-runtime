// Small Java-side behaviors used during the observed startup path.
use std::net::UdpSocket;
use std::time::Instant;

static STARTUP_BOOTSTRAP: OnceLock<extern "C" fn()> = OnceLock::new();
static TEXT_BOX_INFO: OnceLock<Mutex<HashMap<crate::ObjectId, NativeTextBoxInfo>>> =
    OnceLock::new();
static MESSAGEBUS_CONNECTION_HANDLES: OnceLock<Mutex<HashMap<crate::ObjectId, i64>>> =
    OnceLock::new();

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeTextBoxInfo {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    font_size: f32,
    multiline: i32,
    x_alignment: i32,
    y_alignment: i32,
    text_color: i32,
    font: i32,
    text_input_type: i32,
    return_key_type: i32,
    manual_focus_release: i32,
    text_wrapped: i32,
    z14: i32,
}

unsafe extern "C" {
    fn roblox_textbox_last_built(info: *const NativeTextBoxInfo);
    fn roblox_textbox_focused(handle: i64, text: *const std::ffi::c_char, info: *const NativeTextBoxInfo);
    fn roblox_textbox_blurred();
}

/// Install the runtime-owned startup callback for the Rust VM's Java-side
/// `GameActivity.bootstrapTheApp()` hook.
pub fn set_startup_bootstrap(callback: extern "C" fn()) -> Result<(), &'static str> {
    STARTUP_BOOTSTRAP
        .set(callback)
        .map_err(|_| "Rust JNI startup bootstrap is already installed")
}

fn install_builtin_methods(vm: &Vm) -> Result<(), String> {
    let system = vm
        .register_class("java/lang/System")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        system,
        "identityHashCode",
        "(Ljava/lang/Object;)I",
        system_identity_hash_code,
    )?;

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

    // GameActivity is the object the runtime supplies to
    // initializeNativeCode. The experimental backend constructs it in this
    // VM, so these same startup answers must also be available under its real
    // class instead of relying on libjnivm's separate Invalid fallback.
    let game_activity = vm
        .register_class("com/google/androidgamesdk/GameActivity")
        .map_err(|error| error.to_string())?;
    install_instance_builtin(
        vm,
        game_activity,
        "getResources",
        "()Landroid/content/res/Resources;",
        invalid_get_resources,
    )?;
    install_instance_builtin(
        vm,
        game_activity,
        "getDisplayMetrics",
        "()Landroid/util/DisplayMetrics;",
        invalid_get_display_metrics,
    )?;
    install_instance_builtin(
        vm,
        game_activity,
        "getNativeHelper",
        "()Lcom/roblox/client/startup/NativeHelper;",
        invalid_get_native_helper,
    )?;
    install_instance_builtin(
        vm,
        game_activity,
        "bootstrapTheApp",
        "()V",
        invalid_bootstrap,
    )?;
    install_instance_builtin(vm, game_activity, "finish", "()V", reporter_noop)?;
    install_instance_builtin(
        vm,
        game_activity,
        "setImeEditorInfoFields",
        "(III)V",
        reporter_noop,
    )?;
    install_instance_builtin(
        vm,
        game_activity,
        "setWindowFlags",
        "(II)V",
        reporter_noop,
    )?;
    vm.register_class("androidx/core/graphics/Insets")
        .map_err(|error| error.to_string())?;
    let insets = vm.find_or_define_class("androidx/core/graphics/Insets")
        .map_err(|error| error.to_string())?;
    for name in ["left", "top", "right", "bottom"] {
        vm.register_field(insets, name, "I", false).map_err(|error| error.to_string())?;
    }
    install_instance_builtin(
        vm,
        game_activity,
        "getWindowInsets",
        "(I)Landroidx/core/graphics/Insets;",
        zero_insets,
    )?;
    install_instance_builtin(
        vm,
        game_activity,
        "getWaterfallInsets",
        "()Landroidx/core/graphics/Insets;",
        zero_insets,
    )?;

    for class_name in [
        "android/content/res/AssetManager",
        "android/content/res/Configuration",
    ] {
        vm.register_class(class_name)
            .map_err(|error| error.to_string())?;
    }

    let configuration = vm
        .find_or_define_class("android/content/res/Configuration")
        .map_err(|error| error.to_string())?;
    for (name, descriptor) in [
        ("colorMode", "I"), ("densityDpi", "I"), ("fontScale", "F"),
        ("fontWeightAdjustment", "I"), ("hardKeyboardHidden", "I"),
        ("keyboard", "I"), ("keyboardHidden", "I"), ("mcc", "I"), ("mnc", "I"),
        ("navigation", "I"), ("navigationHidden", "I"), ("orientation", "I"),
        ("screenHeightDp", "I"), ("screenLayout", "I"), ("screenWidthDp", "I"),
        ("smallestScreenWidthDp", "I"), ("touchscreen", "I"), ("uiMode", "I"),
    ] {
        vm.register_field(configuration, name, descriptor, false)
            .map_err(|error| error.to_string())?;
    }
    install_instance_builtin(
        vm,
        configuration,
        "getLocales",
        "()Landroid/os/LocaleList;",
        configuration_get_locales,
    )?;

    let locale_list = vm
        .register_class("android/os/LocaleList")
        .map_err(|error| error.to_string())?;
    install_instance_builtin(vm, locale_list, "size", "()I", locale_list_size)?;
    install_instance_builtin(vm, locale_list, "isEmpty", "()Z", locale_list_is_empty)?;
    install_instance_builtin(
        vm,
        locale_list,
        "get",
        "(I)Ljava/util/Locale;",
        locale_list_get,
    )?;

    let locale = vm
        .register_class("java/util/Locale")
        .map_err(|error| error.to_string())?;
    for (method_name, handler) in [
        ("getLanguage", locale_language as crate::MethodHandler),
        ("getCountry", locale_country),
        ("getScript", locale_empty_string),
        ("getVariant", locale_empty_string),
        ("toString", locale_to_string),
    ] {
        install_instance_builtin(
            vm,
            locale,
            method_name,
            "()Ljava/lang/String;",
            handler,
        )?;
    }

    let insets_type = vm
        .register_class("androidx/core/view/WindowInsetsCompat$Type")
        .map_err(|error| error.to_string())?;
    for (method_name, handler) in [
        ("captionBar", inset_caption_bar as crate::MethodHandler),
        ("displayCutout", inset_display_cutout),
        ("ime", inset_ime),
        ("mandatorySystemGestures", inset_mandatory_gestures),
        ("navigationBars", inset_navigation_bars),
        ("statusBars", inset_status_bars),
        ("systemBars", inset_system_bars),
        ("systemGestures", inset_system_gestures),
        ("tappableElement", inset_tappable_element),
    ] {
        let method = vm
            .register_method(insets_type, method_name, "()I", true)
            .map_err(|error| error.to_string())?;
        vm.install_method_handler(method, handler)
            .map_err(|error| error.to_string())?;
    }

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
        ("gameActivity_onLuaTextBoxChanged", "(Ljava/lang/String;)V", native_helper_lua_text_box_changed),
        ("gameActivity_onLuaTextBoxPropertyChanged", "()V", native_helper_lua_text_box_property_changed),
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
        ("gameActivity_showKeyboard", "(JZ[BLcom/roblox/engine/jni/model/NativeTextBoxInfo;)V", native_helper_show_keyboard),
        ("gameActivity_hideKeyboard", "()V", native_helper_hide_keyboard),
    ] {
        install_instance_builtin(vm, native_helper, method_name, descriptor, handler)?;
    }
    install_native_text_box_info(vm)?;

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
    let byte_buffer = vm
        .register_class("java/nio/ByteBuffer")
        .map_err(|error| error.to_string())?;
    let _ = byte_buffer;
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

    let text_state = vm
        .register_class("com/google/androidgamesdk/gametextinput/State")
        .map_err(|error| error.to_string())?;
    for (name, descriptor) in [
        ("text", "Ljava/lang/String;"),
        ("selectionStart", "I"),
        ("selectionEnd", "I"),
        ("composingRegionStart", "I"),
        ("composingRegionEnd", "I"),
    ] {
        vm.register_field(text_state, name, descriptor, false)
            .map_err(|error| error.to_string())?;
    }
    let input_connection = vm.register_class("com/google/androidgamesdk/gametextinput/InputConnection")
        .map_err(|error| error.to_string())?;
    for (name, descriptor) in [
        ("setState", "(Lcom/google/androidgamesdk/gametextinput/State;)V"),
        ("setSoftKeyboardActive", "(ZI)V"),
        ("restartInput", "()V"),
    ] {
        register_unhandled_instance(vm, input_connection, name, descriptor)?;
    }

    // AGDK resolves these accessors while registering GameActivity, before it
    // receives any input. Runtime-created event objects are backed by the
    // companion C++ VM, whose reference implementations provide their values.
    let motion = vm.register_class("android/view/MotionEvent").map_err(|e| e.to_string())?;
    for (name, descriptor) in [
        ("getDeviceId", "()I"), ("getSource", "()I"), ("getAction", "()I"),
        ("getEventTime", "()J"), ("getDownTime", "()J"), ("getFlags", "()I"),
        ("getMetaState", "()I"), ("getActionButton", "()I"), ("getButtonState", "()I"),
        ("getClassification", "()I"), ("getEdgeFlags", "()I"), ("getHistorySize", "()I"),
        ("getHistoricalEventTime", "(I)J"), ("getPointerCount", "()I"),
        ("getPointerId", "(I)I"), ("getToolType", "(I)I"), ("getRawX", "(I)F"),
        ("getRawY", "(I)F"), ("getXPrecision", "()F"), ("getYPrecision", "()F"),
        ("getAxisValue", "(II)F"), ("getHistoricalAxisValue", "(III)F"),
    ] { register_unhandled_instance(vm, motion, name, descriptor)?; }
    let key = vm.register_class("android/view/KeyEvent").map_err(|e| e.to_string())?;
    for (name, descriptor) in [
        ("getDeviceId", "()I"), ("getSource", "()I"), ("getAction", "()I"),
        ("getEventTime", "()J"), ("getDownTime", "()J"), ("getFlags", "()I"),
        ("getMetaState", "()I"), ("getModifiers", "()I"), ("getRepeatCount", "()I"),
        ("getKeyCode", "()I"), ("getScanCode", "()I"), ("getUnicodeChar", "()I"),
    ] { register_unhandled_instance(vm, key, name, descriptor)?; }

    install_platform_methods(vm)?;
    install_fmod_methods(vm)?;
    install_flags_methods(vm)?;
    install_system_dialog_singleton(vm)?;
    install_system_dialog_methods(vm)?;
    install_system_theme(vm)?;
    install_build_manufacturer(vm)?;
    install_quote_interface(vm)?;

    // Startup resolves these Java types but the observed path makes no member
    // calls on them. Register their identities; method behavior remains
    // unresolved until a call or a reference implementation is available.
    for class_name in [
        "com/snapchat/djinni/NativeObjectManager",
        "com/roblox/audio/AppRtcDeviceWrapper",
        "org/fmod/MediaCodec",
        "com/roblox/engine/jni/memstorage/Connection",
        "com/roblox/universalapp/messagebus/Connection",
        "org/webrtc/voiceengine/WebRtcAudioManager",
        "com/roblox/client/flags/FlagJniInterface",
    ] {
        vm.register_class(class_name)
            .map_err(|error| error.to_string())?;
    }

    // roblox-runtime's message-bus bridge implements this factory in C++ and keeps
    // the returned native pointer with the Connection object. Keep that
    // constructor in the selected Rust VM too; otherwise each webview
    // subscription crosses into the companion VM just to create its wrapper.
    let connection = vm
        .find_or_define_class("com/roblox/universalapp/messagebus/Connection")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        connection,
        "<init>",
        "(J)Lcom/roblox/universalapp/messagebus/Connection;",
        messagebus_connection_init,
    )?;
    // roblox-runtime registers the factory with a Class receiver, while the engine
    // lookup observed in rusty-blox does not expose whether it asks for the
    // static or instance method table. Support both JNI lookup forms.
    install_instance_builtin(
        vm,
        connection,
        "<init>",
        "(J)Lcom/roblox/universalapp/messagebus/Connection;",
        messagebus_connection_init,
    )?;

    Ok(())
}

fn messagebus_connection_init(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let Some(JniValue::Long(native_handle)) = args.first() else {
        eprintln!("[jnivm] MessageBus Connection factory received no native handle");
        return JniValue::Object(None);
    };
    let Ok(class) = vm.find_or_define_class("com/roblox/universalapp/messagebus/Connection")
    else {
        return JniValue::Object(None);
    };
    let env = attached_thread_env(vm);
    match vm.new_local_object(&env, class, crate::ObjectValue::Opaque) {
        Ok(connection) => {
            MESSAGEBUS_CONNECTION_HANDLES
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(connection, *native_handle);
            JniValue::Object(Some(connection))
        }
        Err(error) => {
            eprintln!("[jnivm] could not create MessageBus Connection: {error}");
            JniValue::Object(None)
        }
    }
}

fn system_identity_hash_code(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    match args.first() {
        Some(JniValue::Object(Some(object))) => {
            let bits = object.0;
            JniValue::Int((bits ^ (bits >> 32)) as i32)
        }
        Some(JniValue::Object(None)) | None => JniValue::Int(0),
        _ => JniValue::Int(0),
    }
}

/// The APK's Kotlin `PlatformSystemDialogHandler` is an object singleton.
/// Keep its observed `INSTANCE` field non-null; dialog behavior is not modeled
/// here because the C++ compatibility runtime has no implementation for it.
fn install_system_dialog_singleton(vm: &Vm) -> Result<(), String> {
    const CLASS_NAME: &str = "com/roblox/protocols/systemdialog/PlatformSystemDialogHandler";
    let class = vm
        .register_class(CLASS_NAME)
        .map_err(|error| error.to_string())?;
    let field = vm
        .register_field(
            class,
            "INSTANCE",
            "Lcom/roblox/protocols/systemdialog/PlatformSystemDialogHandler;",
            true,
        )
        .map_err(|error| error.to_string())?;
    let env = attached_thread_env(vm);
    let local_instance = vm
        .new_local_object(&env, class, crate::ObjectValue::Opaque)
        .map_err(|error| error.to_string())?;
    let instance = vm
        .new_global_ref(&env, local_instance)
        .map_err(|error| error.to_string())?;
    vm.set_field_value(field, None, JniValue::Object(Some(instance)))
        .map_err(|error| error.to_string())
}

fn install_system_dialog_methods(vm: &Vm) -> Result<(), String> {
    const INTERFACE: &str =
        "com/roblox/protocols/systemdialogplatforminterface/generated/IPlatformSystemDialogHandler";
    const PROXY: &str = "com/roblox/protocols/systemdialogplatforminterface/generated/IPlatformSystemDialogHandler$CppProxy";
    const REQUEST: &str =
        "com/roblox/protocols/systemdialogplatforminterface/generated/SystemDialogRequest";
    const CALLBACK: &str =
        "com/roblox/protocols/systemdialogplatforminterface/generated/ISystemDialogCallback";

    let interface = vm
        .register_class(INTERFACE)
        .map_err(|error| error.to_string())?;
    install_instance_builtin(
        vm,
        interface,
        "isAvailable",
        "()Z",
        system_ui_unavailable,
    )?;
    install_instance_builtin(
        vm,
        interface,
        "open",
        &format!("(L{REQUEST};L{CALLBACK};)J"),
        system_dialog_open_unavailable,
    )?;
    install_instance_builtin(vm, interface, "dismiss", "(J)V", reporter_noop)?;
    install_instance_builtin(vm, interface, "dismissAll", "()V", reporter_noop)?;

    let proxy = vm.register_class(PROXY).map_err(|error| error.to_string())?;
    vm.register_field(proxy, "nativeRef", "J", false)
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        proxy,
        "<init>",
        &format!("(J)L{PROXY};"),
        system_dialog_proxy_init,
    )?;
    vm.register_class(REQUEST)
        .map_err(|error| error.to_string())?;
    vm.register_class(CALLBACK)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn install_system_theme(vm: &Vm) -> Result<(), String> {
    let class = vm
        .register_class("com/roblox/universalapp/systemtheme/SystemThemeProtocol")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        class,
        "isSystemThemeAvailable",
        "()Z",
        system_ui_unavailable,
    )
}

fn install_build_manufacturer(vm: &Vm) -> Result<(), String> {
    let class = vm
        .register_class("android/os/Build")
        .map_err(|error| error.to_string())?;
    let field = vm
        .register_field(class, "MANUFACTURER", "Ljava/lang/String;", true)
        .map_err(|error| error.to_string())?;
    let JniValue::Object(Some(manufacturer)) = java_string(vm, "roblox-runtime", "Build") else {
        return Err("could not allocate android/os/Build.MANUFACTURER".into());
    };
    let env = attached_thread_env(vm);
    let manufacturer = vm
        .new_global_ref(&env, manufacturer)
        .map_err(|error| error.to_string())?;
    vm.set_field_value(field, None, JniValue::Object(Some(manufacturer)))
        .map_err(|error| error.to_string())
}

fn install_quote_interface(vm: &Vm) -> Result<(), String> {
    let class = vm
        .register_class("com/roblox/engine/jni/NativeQuoteInterface")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        class,
        "requestResponse",
        "([B)[B",
        quote_request_unavailable,
    )
}

fn install_native_text_box_info(vm: &Vm) -> Result<(), String> {
    let class = vm
        .register_class("com/roblox/engine/jni/model/NativeTextBoxInfo")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        class,
        "<init>",
        "(FFFFFZIIIIIIZZZ)Lcom/roblox/engine/jni/model/NativeTextBoxInfo;",
        native_text_box_info_init,
    )
}

fn system_ui_unavailable(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    JniValue::Boolean(false)
}

fn system_dialog_open_unavailable(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    JniValue::Long(0)
}

fn quote_request_unavailable(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    // This runtime has no quote-response service; preserve the JNI null
    // default that the previous no-handler path returned.
    JniValue::Object(None)
}

fn native_text_box_info_init(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let Some(info) = NativeTextBoxInfo::from_args(args) else {
        eprintln!("[jnivm] NativeTextBoxInfo constructor received invalid arguments");
        return JniValue::Object(None);
    };
    let env = attached_thread_env(vm);
    let Ok(class) = vm.find_or_define_class("com/roblox/engine/jni/model/NativeTextBoxInfo") else {
        return JniValue::Object(None);
    };
    let Ok(object) = vm.new_local_object(&env, class, crate::ObjectValue::Opaque) else {
        return JniValue::Object(None);
    };
    TEXT_BOX_INFO
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(object, info);
    // SAFETY: the pointer refers to a stack value with the exact C ABI layout
    // shared with native/android_classes.cpp, which copies it before return.
    unsafe { roblox_textbox_last_built(&info) };
    JniValue::Object(Some(object))
}

impl NativeTextBoxInfo {
    fn from_args(args: &[JniValue]) -> Option<Self> {
        let floats = [
            float_arg(args, 0)?,
            float_arg(args, 1)?,
            float_arg(args, 2)?,
            float_arg(args, 3)?,
            float_arg(args, 4)?,
        ];
        let multiline = bool_arg(args, 5)?;
        Some(Self {
            x: floats[0],
            y: floats[1],
            width: floats[2],
            height: floats[3],
            font_size: floats[4],
            multiline: i32::from(multiline),
            x_alignment: int_arg(args, 6)?,
            y_alignment: int_arg(args, 7)?,
            text_color: int_arg(args, 8)?,
            font: int_arg(args, 9)?,
            text_input_type: int_arg(args, 10)?,
            return_key_type: int_arg(args, 11)?,
            manual_focus_release: i32::from(bool_arg(args, 12)?),
            text_wrapped: i32::from(bool_arg(args, 13)?),
            z14: i32::from(bool_arg(args, 14)?),
        })
    }
}

fn float_arg(args: &[JniValue], index: usize) -> Option<f32> {
    match args.get(index) {
        Some(JniValue::Float(value)) => Some(*value),
        _ => None,
    }
}

fn int_arg(args: &[JniValue], index: usize) -> Option<i32> {
    match args.get(index) {
        Some(JniValue::Int(value)) => Some(*value),
        _ => None,
    }
}

fn bool_arg(args: &[JniValue], index: usize) -> Option<bool> {
    match args.get(index) {
        Some(JniValue::Boolean(value)) => Some(*value),
        _ => None,
    }
}

fn native_helper_show_keyboard(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let handle = match args.first() {
        Some(JniValue::Long(handle)) => *handle,
        _ => 0,
    };
    let text = match args.get(2) {
        Some(JniValue::Object(Some(array))) => {
            let env = attached_thread_env(vm);
            match vm.object_value(&env, *array) {
                Ok(crate::ObjectValue::ByteArray(bytes)) => bytes
                    .into_iter()
                    .map(|byte| byte as u8)
                    .take_while(|byte| *byte != 0)
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    };
    let Ok(text) = CString::new(text) else {
        return JniValue::Void;
    };
    let info = match args.get(3) {
        Some(JniValue::Object(Some(object))) => TEXT_BOX_INFO
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(object)
            .copied(),
        _ => None,
    };
    let info_ptr = info
        .as_ref()
        .map_or(std::ptr::null(), |info| info as *const NativeTextBoxInfo);
    // SAFETY: `text` and optional `info` stay alive for the synchronous copy.
    unsafe { roblox_textbox_focused(handle, text.as_ptr(), info_ptr) };
    JniValue::Void
}

fn native_helper_hide_keyboard(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    // SAFETY: the C++ callback takes no pointers and updates its focus state.
    unsafe { roblox_textbox_blurred() };
    JniValue::Void
}

fn system_dialog_proxy_init(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let Some(JniValue::Long(native_ref)) = args.first() else {
        return JniValue::Object(None);
    };
    let Ok(class) = vm.find_or_define_class(
        "com/roblox/protocols/systemdialogplatforminterface/generated/IPlatformSystemDialogHandler$CppProxy",
    ) else {
        return JniValue::Object(None);
    };
    let env = attached_thread_env(vm);
    let Ok(object) = vm.new_local_object(&env, class, crate::ObjectValue::Opaque) else {
        return JniValue::Object(None);
    };
    if let Ok(field) = vm.resolve_field(class, "nativeRef", "J", false) {
        let _ = vm.set_field_value(field, Some(object), JniValue::Long(*native_ref));
    }
    JniValue::Object(Some(object))
}

fn new_opaque_local(vm: &Vm, class_name: &str) -> JniValue {
    let env = attached_thread_env(vm);
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

fn configuration_get_locales(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    new_opaque_local(vm, "android/os/LocaleList")
}

fn locale_list_size(_vm: &Vm, _receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
    JniValue::Int(1)
}

fn locale_list_is_empty(
    _vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    JniValue::Boolean(false)
}

fn locale_list_get(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    new_opaque_local(vm, "java/util/Locale")
}

fn locale_language(vm: &Vm, _receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
    java_string(vm, "en", "java/util/Locale")
}

fn locale_country(vm: &Vm, _receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
    java_string(vm, "US", "java/util/Locale")
}

fn locale_empty_string(
    vm: &Vm,
    _receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    java_string(vm, "", "java/util/Locale")
}

fn locale_to_string(vm: &Vm, _receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
    java_string(vm, "en_US", "java/util/Locale")
}

macro_rules! inset_mask_handler {
    ($name:ident, $mask:expr) => {
        fn $name(_vm: &Vm, _receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
            JniValue::Int($mask)
        }
    };
}

inset_mask_handler!(inset_caption_bar, 1);
inset_mask_handler!(inset_display_cutout, 2);
inset_mask_handler!(inset_ime, 4);
inset_mask_handler!(inset_mandatory_gestures, 8);
inset_mask_handler!(inset_navigation_bars, 16);
inset_mask_handler!(inset_status_bars, 32);
inset_mask_handler!(inset_system_bars, 64);
inset_mask_handler!(inset_system_gestures, 128);
inset_mask_handler!(inset_tappable_element, 256);

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

fn zero_insets(vm: &Vm, _receiver: Option<crate::ObjectId>, _args: &[JniValue]) -> JniValue {
    let Ok(class) = vm.find_or_define_class("androidx/core/graphics/Insets") else {
        return JniValue::Object(None);
    };
    let env = attached_thread_env(vm);
    vm.new_local_object(&env, class, crate::ObjectValue::Opaque)
        .ok()
        .map_or(JniValue::Object(None), |object| {
            for name in ["left", "top", "right", "bottom"] {
                if let Ok(field) = vm.resolve_field(class, name, "I", false) {
                    let _ = vm.set_field_value(field, Some(object), JniValue::Int(0));
                }
            }
            JniValue::Object(Some(object))
        })
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
    format!("{base}/roblox-runtime/instances/default/data")
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
    let env = attached_thread_env(vm);
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
    let env = attached_thread_env(vm);
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
    let env = attached_thread_env(vm);
    let Ok(crate::ObjectValue::String(units)) = vm.object_value(&env, *name) else {
        eprintln!("[jnivm] ClassLoader class lookup received a non-string name");
        return JniValue::Object(None);
    };
    let requested = String::from_utf16_lossy(&units);
    let name = requested.replace('.', "/");
    match vm.find_or_define_class(&name) {
        Ok(class) => {
            eprintln!("[jnivm] ClassLoader resolved {requested} as {name}");
            JniValue::Object(Some(crate::ObjectId(class.0)))
        }
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
    let env = attached_thread_env(vm);
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

fn register_unhandled_instance(vm: &Vm, class: crate::ClassId, name: &str, descriptor: &str) -> Result<(), String> {
    vm.register_method(class, name, descriptor, false)
        .map(|_| ())
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
    let env = attached_thread_env(vm);
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
