// Small Java-side behaviors used during the observed startup path.
fn install_builtin_methods(vm: &Vm) -> Result<(), String> {
    let class = vm
        .find_or_define_class("java/lang/Class")
        .map_err(|error| error.to_string())?;
    install_instance_builtin(
        vm,
        class,
        "getClassLoader",
        "()Ljava/lang/ClassLoader;",
        class_get_class_loader,
    )?;

    let class_loader = vm
        .find_or_define_class("java/lang/ClassLoader")
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

    Ok(())
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
