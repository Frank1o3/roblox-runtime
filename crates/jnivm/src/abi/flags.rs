// The small flag-result/map surface used by Roblox during startup. These
// handlers mirror NativeFlagsInitResult and JavaMap in native/init_params.cpp
// while keeping every object inside the selected Rust VM.

#[derive(Default)]
struct RustFlagResult {
    provider_id: i32,
    entries: HashMap<String, bool>,
    map: Option<crate::ObjectId>,
}

static RUST_FLAG_RESULTS: OnceLock<Mutex<HashMap<crate::ObjectId, RustFlagResult>>> =
    OnceLock::new();
static RUST_FLAG_MAP_OWNERS: OnceLock<Mutex<HashMap<crate::ObjectId, crate::ObjectId>>> =
    OnceLock::new();

fn install_flags_methods(vm: &Vm) -> Result<(), String> {
    let flags = vm
        .register_class("com/roblox/client/flags/NativeFlagsInitResult")
        .map_err(|error| error.to_string())?;
    install_builtin(
        vm,
        flags,
        "<init>",
        "(I)Lcom/roblox/client/flags/NativeFlagsInitResult;",
        flags_create,
    )?;
    install_instance_builtin(
        vm,
        flags,
        "addBoolean",
        "(Ljava/lang/String;ZZ)V",
        flags_add_boolean,
    )?;
    install_instance_builtin(vm, flags, "getNativeFlagProviderId", "()I", flags_provider_id)?;
    install_instance_builtin(
        vm,
        flags,
        "getBooleanCachedMap",
        "()Ljava/util/Map;",
        flags_boolean_map,
    )?;
    install_instance_builtin(
        vm,
        flags,
        "resolveFlagValue",
        "(Ljava/lang/String;)Z",
        flags_resolve,
    )?;

    let map = vm
        .register_class("java/util/Map")
        .map_err(|error| error.to_string())?;
    install_instance_builtin(vm, map, "size", "()I", flags_map_size)?;
    install_instance_builtin(vm, map, "isEmpty", "()Z", flags_map_is_empty)?;
    Ok(())
}

fn rust_java_string(vm: &Vm, value: &JniValue) -> Option<String> {
    let JniValue::Object(Some(object)) = value else {
        return None;
    };
    let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    let ObjectValue::String(units) = vm.object_value(&env, *object).ok()? else {
        return None;
    };
    Some(String::from_utf16_lossy(&units))
}

fn flags_create(vm: &Vm, _receiver: Option<crate::ObjectId>, args: &[JniValue]) -> JniValue {
    let JniValue::Object(Some(result)) = new_opaque_local(vm, "com/roblox/client/flags/NativeFlagsInitResult") else {
        return JniValue::Object(None);
    };
    let JniValue::Object(Some(map)) = new_opaque_local(vm, "java/util/Map") else {
        return JniValue::Object(None);
    };
    let provider_id = match args.first() {
        Some(JniValue::Int(value)) => *value,
        _ => 0,
    };
    RUST_FLAG_RESULTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            result,
            RustFlagResult {
                provider_id,
                map: Some(map),
                ..RustFlagResult::default()
            },
        );
    RUST_FLAG_MAP_OWNERS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(map, result);
    JniValue::Object(Some(result))
}

fn flags_add_boolean(
    vm: &Vm,
    receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let (Some(receiver), Some(name), Some(JniValue::Boolean(value))) = (
        receiver,
        args.first().and_then(|value| rust_java_string(vm, value)),
        args.get(1),
    ) else {
        return JniValue::Void;
    };
    if let Some(result) = RUST_FLAG_RESULTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_mut(&receiver)
    {
        result.entries.insert(name, *value);
    }
    JniValue::Void
}

fn flags_provider_id(
    _vm: &Vm,
    receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let provider_id = receiver
        .and_then(|receiver| {
            RUST_FLAG_RESULTS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&receiver)
                .map(|result| result.provider_id)
        })
        .unwrap_or_default();
    JniValue::Int(provider_id)
}

fn flags_boolean_map(
    _vm: &Vm,
    receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let map = receiver.and_then(|receiver| {
        RUST_FLAG_RESULTS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&receiver)
            .and_then(|result| result.map)
    });
    JniValue::Object(map)
}

fn flags_resolve(
    vm: &Vm,
    receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let value = receiver
        .zip(args.first().and_then(|value| rust_java_string(vm, value)))
        .and_then(|(receiver, name)| {
            RUST_FLAG_RESULTS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&receiver)
                .and_then(|result| result.entries.get(&name).copied())
        })
        .unwrap_or(false);
    JniValue::Boolean(value)
}

fn flags_map_size(
    _vm: &Vm,
    receiver: Option<crate::ObjectId>,
    _args: &[JniValue],
) -> JniValue {
    let size = receiver
        .and_then(|map| {
            let owner = RUST_FLAG_MAP_OWNERS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&map)
                .copied()?;
            RUST_FLAG_RESULTS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&owner)
                .map(|result| result.entries.len() as i32)
        })
        .unwrap_or(0);
    JniValue::Int(size)
}

fn flags_map_is_empty(
    vm: &Vm,
    receiver: Option<crate::ObjectId>,
    args: &[JniValue],
) -> JniValue {
    let JniValue::Int(size) = flags_map_size(vm, receiver, args) else {
        unreachable!()
    };
    JniValue::Boolean(size == 0)
}
