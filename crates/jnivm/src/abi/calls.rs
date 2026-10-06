// JNI instance and static method call adapters.
static CPP_FALLBACK_OBJECTS: OnceLock<Mutex<HashMap<u64, usize>>> = OnceLock::new();

unsafe extern "C" {
    fn roblox_jni_decode_va_list(
        signature: *const std::ffi::c_char,
        args: jni::va_list,
        output: *mut jni::jvalue,
        capacity: i32,
    ) -> i32;
    fn roblox_jni_fallback_has_method(
        class_name: *const std::ffi::c_char,
        name: *const std::ffi::c_char,
        signature: *const std::ffi::c_char,
        is_static: i32,
    ) -> i32;
    fn roblox_jni_fallback_has_field(
        class_name: *const std::ffi::c_char,
        name: *const std::ffi::c_char,
        descriptor: *const std::ffi::c_char,
        is_static: i32,
        is_set: i32,
    ) -> i32;
    fn roblox_jni_fallback_get_field(
        class_name: *const std::ffi::c_char,
        name: *const std::ffi::c_char,
        descriptor: *const std::ffi::c_char,
        is_static: i32,
        receiver: *mut std::ffi::c_void,
        result: *mut jni::jvalue,
    ) -> i32;
    fn roblox_jni_fallback_set_field(
        class_name: *const std::ffi::c_char,
        name: *const std::ffi::c_char,
        descriptor: *const std::ffi::c_char,
        is_static: i32,
        receiver: *mut std::ffi::c_void,
        value: jni::jvalue,
    ) -> i32;
    fn roblox_jni_fallback_new_object(class_name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    fn roblox_jni_fallback_new_string(chars: *const u16, length: i32) -> *mut std::ffi::c_void;
    fn roblox_jni_fallback_invoke(
        class_name: *const std::ffi::c_char,
        name: *const std::ffi::c_char,
        signature: *const std::ffi::c_char,
        is_static: i32,
        receiver: *mut std::ffi::c_void,
        args: *mut jni::jvalue,
        result: *mut jni::jvalue,
    ) -> i32;
    fn roblox_jni_fallback_copy_string(
        string_ref: *mut std::ffi::c_void,
        output: *mut u16,
        capacity: i32,
    ) -> i32;
}

unsafe extern "system" fn alloc_object(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
) -> jni::jobject {
    let Some(vm) = vm() else {
        return ptr::null_mut();
    };
    let Some(env) = env_token(vm, env) else {
        return ptr::null_mut();
    };
    if class.is_null() {
        return ptr::null_mut();
    }
    let class = ClassId(class as usize as u64);
    if vm.class_name(class).is_none() {
        eprintln!("[jnivm] AllocObject received an unknown class");
        return ptr::null_mut();
    }
    match vm.new_local_object(&env, class, crate::ObjectValue::Opaque) {
        Ok(object) => object.0 as usize as jni::jobject,
        Err(error) => {
            eprintln!("[jnivm] AllocObject failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn new_object_a(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    method: jni::jmethodID,
    args: *const jni::jvalue,
) -> jni::jobject {
    let Some(vm) = vm() else {
        return ptr::null_mut();
    };
    if env_token(vm, env).is_none() || class.is_null() || method.is_null() {
        return ptr::null_mut();
    }
    let class_id = ClassId(class as usize as u64);
    let method_id = MethodId(method as usize as u64);
    if vm.class_name(class_id).is_none()
        || vm.method_descriptor(method_id).is_none()
    {
        eprintln!("[jnivm] NewObjectA received an unknown class or constructor");
        return ptr::null_mut();
    }
    let Some(JniValue::Object(Some(object))) = invoke_a(method, None, args) else {
        return ptr::null_mut();
    };
    object.0 as usize as jni::jobject
}

unsafe extern "system" fn new_object_v(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    method: jni::jmethodID,
    args: jni::va_list,
) -> jni::jobject {
    let Some(vm) = vm() else {
        return ptr::null_mut();
    };
    if env_token(vm, env).is_none() || class.is_null() || method.is_null() {
        return ptr::null_mut();
    }
    if vm.class_name(ClassId(class as usize as u64)).is_none() {
        eprintln!("[jnivm] NewObjectV received an unknown class");
        return ptr::null_mut();
    }
    invoke(method, None, args)
        .and_then(object_result)
        .unwrap_or(ptr::null_mut())
}

unsafe extern "system" fn call_object_v(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    args: jni::va_list,
) -> jni::jobject {
    invoke(
        method,
        (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)),
        args,
    )
    .and_then(|value| match value {
        JniValue::Object(Some(id)) => Some(id.0 as usize as jni::jobject),
        _ => None,
    })
    .unwrap_or(ptr::null_mut())
}

unsafe extern "system" fn call_static_object_v(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    method: jni::jmethodID,
    args: jni::va_list,
) -> jni::jobject {
    invoke(method, None, args)
        .and_then(|value| match value {
            JniValue::Object(Some(id)) => Some(id.0 as usize as jni::jobject),
            _ => None,
        })
        .unwrap_or(ptr::null_mut())
}

unsafe extern "system" fn call_boolean_v(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    args: jni::va_list,
) -> jni::jboolean {
    invoke(
        method,
        (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)),
        args,
    )
    .and_then(|value| match value {
        JniValue::Boolean(value) => Some(value),
        _ => None,
    })
    .unwrap_or(jni::JNI_FALSE)
}
unsafe extern "system" fn call_int_v(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    args: jni::va_list,
) -> jni::jint {
    invoke(
        method,
        (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)),
        args,
    )
    .and_then(|value| match value {
        JniValue::Int(value) => Some(value),
        _ => None,
    })
    .unwrap_or(0)
}
unsafe extern "system" fn call_long_v(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    args: jni::va_list,
) -> jni::jlong {
    invoke(
        method,
        (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)),
        args,
    )
    .and_then(|value| match value {
        JniValue::Long(value) => Some(value),
        _ => None,
    })
    .unwrap_or(0)
}
unsafe extern "system" fn call_void_v(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    args: jni::va_list,
) {
    let _ = invoke(
        method,
        (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)),
        args,
    );
}
unsafe extern "system" fn call_static_boolean_v(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    method: jni::jmethodID,
    args: jni::va_list,
) -> jni::jboolean {
    invoke(method, None, args)
        .and_then(|value| match value {
            JniValue::Boolean(value) => Some(value),
            _ => None,
        })
        .unwrap_or(jni::JNI_FALSE)
}
unsafe extern "system" fn call_static_int_v(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    method: jni::jmethodID,
    args: jni::va_list,
) -> jni::jint {
    invoke(method, None, args)
        .and_then(|value| match value {
            JniValue::Int(value) => Some(value),
            _ => None,
        })
        .unwrap_or(0)
}
unsafe extern "system" fn call_static_long_v(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    method: jni::jmethodID,
    args: jni::va_list,
) -> jni::jlong {
    invoke(method, None, args)
        .and_then(|value| match value {
            JniValue::Long(value) => Some(value),
            _ => None,
        })
        .unwrap_or(0)
}
unsafe extern "system" fn call_static_void_v(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    method: jni::jmethodID,
    args: jni::va_list,
) {
    let _ = invoke(method, None, args);
}

unsafe extern "system" fn call_object_a(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    args: *const jni::jvalue,
) -> jni::jobject {
    invoke_a(method, (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)), args)
        .and_then(object_result)
        .unwrap_or(ptr::null_mut())
}
unsafe extern "system" fn call_boolean_a(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    args: *const jni::jvalue,
) -> jni::jboolean {
    invoke_a(method, (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)), args)
        .and_then(|value| match value { JniValue::Boolean(value) => Some(value), _ => None })
        .unwrap_or(jni::JNI_FALSE)
}
unsafe extern "system" fn call_int_a(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    args: *const jni::jvalue,
) -> jni::jint {
    invoke_a(method, (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)), args)
        .and_then(|value| match value { JniValue::Int(value) => Some(value), _ => None }).unwrap_or(0)
}
unsafe extern "system" fn call_long_a(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    args: *const jni::jvalue,
) -> jni::jlong {
    invoke_a(method, (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)), args)
        .and_then(|value| match value { JniValue::Long(value) => Some(value), _ => None }).unwrap_or(0)
}
unsafe extern "system" fn call_void_a(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    args: *const jni::jvalue,
) {
    let _ = invoke_a(method, (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)), args);
}
unsafe extern "system" fn call_static_object_a(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    method: jni::jmethodID,
    args: *const jni::jvalue,
) -> jni::jobject {
    invoke_a(method, None, args).and_then(object_result).unwrap_or(ptr::null_mut())
}
unsafe extern "system" fn call_static_boolean_a(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    method: jni::jmethodID,
    args: *const jni::jvalue,
) -> jni::jboolean {
    invoke_a(method, None, args)
        .and_then(|value| match value { JniValue::Boolean(value) => Some(value), _ => None })
        .unwrap_or(jni::JNI_FALSE)
}
unsafe extern "system" fn call_static_int_a(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    method: jni::jmethodID,
    args: *const jni::jvalue,
) -> jni::jint {
    invoke_a(method, None, args).and_then(|value| match value { JniValue::Int(value) => Some(value), _ => None }).unwrap_or(0)
}
unsafe extern "system" fn call_static_long_a(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    method: jni::jmethodID,
    args: *const jni::jvalue,
) -> jni::jlong {
    invoke_a(method, None, args).and_then(|value| match value { JniValue::Long(value) => Some(value), _ => None }).unwrap_or(0)
}
unsafe extern "system" fn call_static_void_a(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    method: jni::jmethodID,
    args: *const jni::jvalue,
) {
    let _ = invoke_a(method, None, args);
}

fn object_result(value: JniValue) -> Option<jni::jobject> {
    match value {
        JniValue::Object(Some(id)) => Some(id.0 as usize as jni::jobject),
        JniValue::Object(None) => None,
        _ => None,
    }
}

fn invoke_a(
    method: jni::jmethodID,
    receiver: Option<crate::ObjectId>,
    raw_arguments: *const jni::jvalue,
) -> Option<JniValue> {
    let vm = vm()?;
    let method = MethodId(method as usize as u64);
    let descriptor = vm.method_descriptor(method)?;
    let arguments = if descriptor.parameters.is_empty() {
        Vec::new()
    } else if raw_arguments.is_null() {
        eprintln!("[jnivm] unimplemented JNI call: non-empty Call*MethodA argument vector is null");
        descriptor.parameters.iter().map(JniValue::default_for).collect()
    } else {
        descriptor.parameters.iter().enumerate().map(|(index, ty)| {
            // SAFETY: JNI Call*MethodA supplies one jvalue per descriptor argument.
            let value = unsafe { raw_arguments.add(index).read() };
            // SAFETY: the method descriptor selects the active jvalue union member.
            unsafe { match ty {
                crate::Type::Boolean => JniValue::Boolean(value.z),
                crate::Type::Byte => JniValue::Byte(value.b),
                crate::Type::Char => JniValue::Char(value.c),
                crate::Type::Short => JniValue::Short(value.s),
                crate::Type::Int => JniValue::Int(value.i),
                crate::Type::Long => JniValue::Long(value.j),
                crate::Type::Float => JniValue::Float(value.f),
                crate::Type::Double => JniValue::Double(value.d),
                crate::Type::Object(_) | crate::Type::Array(_) => {
                    JniValue::Object((!value.l.is_null()).then_some(crate::ObjectId(value.l as usize as u64)))
                }
                crate::Type::Void => JniValue::Void,
            }}
        }).collect()
    };
    match dispatch_method(vm, method, receiver, &arguments) {
        Ok(value) => Some(value),
        Err(error) => {
            eprintln!("[jnivm] call through unknown JNI method ID: {error}");
            None
        }
    }
}

fn invoke(
    method: jni::jmethodID,
    receiver: Option<crate::ObjectId>,
    raw_args: jni::va_list,
) -> Option<JniValue> {
    let Some(vm) = vm() else { return None };
    let method = MethodId(method as usize as u64);
    let descriptor = vm.method_descriptor(method)?;
    let (class_name, name, signature, _, _) = vm.method_info(method)?;
    let signature_c = std::ffi::CString::new(signature.as_bytes()).ok()?;
    let mut raw_arguments = vec![unsafe { std::mem::zeroed::<jni::jvalue>() }; descriptor.parameters.len()];
    let capacity = i32::try_from(raw_arguments.len()).ok()?;
    // The platform owns `va_list`'s representation. The native shim copies
    // and decodes it with the target C ABI so handlers such as ClassLoader's
    // `loadClass(String)` receive the class name instead of a default null.
    let status = unsafe {
        roblox_jni_decode_va_list(
            signature_c.as_ptr(),
            raw_args,
            raw_arguments.as_mut_ptr(),
            capacity,
        )
    };
    if status != 0 {
        eprintln!(
            "[jnivm] could not decode Call*MethodV arguments for {class_name}.{name}{signature} (status {status}); call skipped"
        );
        return None;
    }
    let arguments = descriptor
        .parameters
        .iter()
        .zip(raw_arguments)
        .map(|(ty, raw)| unsafe { from_raw_jvalue(ty, raw) })
        .collect::<Vec<_>>();
    match dispatch_method(vm, method, receiver, &arguments) {
        Ok(value) => Some(value),
        Err(error) => {
            eprintln!("[jnivm] call through unknown JNI method ID: {error}");
            None
        }
    }
}

unsafe fn from_raw_jvalue(ty: &crate::Type, value: jni::jvalue) -> JniValue {
    match ty {
        crate::Type::Boolean => JniValue::Boolean(unsafe { value.z }),
        crate::Type::Byte => JniValue::Byte(unsafe { value.b }),
        crate::Type::Char => JniValue::Char(unsafe { value.c }),
        crate::Type::Short => JniValue::Short(unsafe { value.s }),
        crate::Type::Int => JniValue::Int(unsafe { value.i }),
        crate::Type::Long => JniValue::Long(unsafe { value.j }),
        crate::Type::Float => JniValue::Float(unsafe { value.f }),
        crate::Type::Double => JniValue::Double(unsafe { value.d }),
        crate::Type::Object(_) | crate::Type::Array(_) => {
            let object = unsafe { value.l };
            JniValue::Object(
                (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)),
            )
        }
        crate::Type::Void => JniValue::Void,
    }
}

fn dispatch_method(
    vm: &Vm,
    method: MethodId,
    receiver: Option<crate::ObjectId>,
    arguments: &[JniValue],
) -> Result<JniValue, crate::JniError> {
    let Some((class_name, name, signature, is_static, has_rust_handler)) = vm.method_info(method)
    else {
        return vm.invoke_method(method, receiver, arguments);
    };
    if has_rust_handler {
        return vm.invoke_method(method, receiver, arguments);
    }

    if !crate::cpp_fallback_enabled() {
        eprintln!(
            "[jnivm:fallback] C++ fallback disabled for {class_name}.{name}{signature}; returning JNI default"
        );
        return vm.invoke_method(method, receiver, arguments);
    }

    eprintln!(
        "[jnivm:fallback] Rust handler missing; checking C++ libjnivm for {class_name}.{name}{signature}"
    );
    if let Some(value) = invoke_cpp_fallback(
        vm,
        method,
        &class_name,
        &name,
        &signature,
        is_static,
        receiver,
        arguments,
    ) {
        return Ok(value);
    }

    eprintln!(
        "[jnivm:fallback] no usable C++ handler for {class_name}.{name}{signature}; returning JNI default"
    );
    vm.invoke_method(method, receiver, arguments)
}

#[allow(clippy::too_many_arguments)]
fn invoke_cpp_fallback(
    vm: &Vm,
    method: MethodId,
    class_name: &str,
    name: &str,
    signature: &str,
    is_static: bool,
    receiver: Option<crate::ObjectId>,
    arguments: &[JniValue],
) -> Option<JniValue> {
    let class_c = std::ffi::CString::new(class_name).ok()?;
    let name_c = std::ffi::CString::new(name).ok()?;
    let signature_c = std::ffi::CString::new(signature).ok()?;
    // SAFETY: the C strings remain alive for the synchronous bridge call.
    let has_cpp = unsafe {
        roblox_jni_fallback_has_method(
            class_c.as_ptr(),
            name_c.as_ptr(),
            signature_c.as_ptr(),
            i32::from(is_static),
        )
    };
    if has_cpp != 1 {
        return None;
    }

    let descriptor = vm.method_descriptor(method)?;
    let mut raw_arguments = Vec::with_capacity(arguments.len());
    for (index, value) in arguments.iter().enumerate() {
        let ty = descriptor.parameters.get(index)?;
        let Some(raw) = to_cpp_jvalue(vm, ty, value) else {
            eprintln!(
                "[jnivm:fallback] could not mirror argument {index} ({ty:?}) for {class_name}.{name}{signature}"
            );
            return None;
        };
        raw_arguments.push(raw);
    }

    let cpp_receiver = if is_static {
        ptr::null_mut()
    } else {
        let receiver = receiver?;
        let receiver_class = vm.object_class_name(receiver.0).unwrap_or_else(|| class_name.to_owned());
        cpp_reference_for(vm, receiver, &receiver_class)? as *mut std::ffi::c_void
    };
    let mut result: jni::jvalue = unsafe { std::mem::zeroed() };
    // SAFETY: argument storage and C strings live through the synchronous C++ call.
    let status = unsafe {
        roblox_jni_fallback_invoke(
            class_c.as_ptr(),
            name_c.as_ptr(),
            signature_c.as_ptr(),
            i32::from(is_static),
            cpp_receiver,
            if raw_arguments.is_empty() {
                ptr::null_mut()
            } else {
                raw_arguments.as_mut_ptr()
            },
            &mut result,
        )
    };
    if status != 0 {
        eprintln!(
            "[jnivm:fallback] C++ invocation failed for {class_name}.{name}{signature} (status {status})"
        );
        return None;
    }
    from_cpp_jvalue(vm, &descriptor.result, result)
}

fn to_cpp_jvalue(vm: &Vm, ty: &crate::Type, value: &JniValue) -> Option<jni::jvalue> {
    let mut raw: jni::jvalue = unsafe { std::mem::zeroed() };
    match (ty, value) {
            (crate::Type::Boolean, JniValue::Boolean(value)) => raw.z = *value,
            (crate::Type::Byte, JniValue::Byte(value)) => raw.b = *value,
            (crate::Type::Char, JniValue::Char(value)) => raw.c = *value,
            (crate::Type::Short, JniValue::Short(value)) => raw.s = *value,
            (crate::Type::Int, JniValue::Int(value)) => raw.i = *value,
            (crate::Type::Long, JniValue::Long(value)) => raw.j = *value,
            (crate::Type::Float, JniValue::Float(value)) => raw.f = *value,
            (crate::Type::Double, JniValue::Double(value)) => raw.d = *value,
            (crate::Type::Object(_) | crate::Type::Array(_), JniValue::Object(None)) => {
                raw.l = ptr::null_mut();
            }
            (crate::Type::Object(_) | crate::Type::Array(_), JniValue::Object(Some(object))) => {
                let class_name = vm.object_class_name(object.0)?;
                raw.l = cpp_reference_for(vm, *object, &class_name)? as *mut _;
            }
            (crate::Type::Void, JniValue::Void) => {}
            _ => return None,
    }
    Some(raw)
}

fn cpp_reference_for(vm: &Vm, object: crate::ObjectId, class_name: &str) -> Option<usize> {
    let value = vm.object_value(&attached_thread_env(vm), object).ok()?;
    if let crate::ObjectValue::CppObject(reference) = value {
        return Some(reference);
    }
    let cache = CPP_FALLBACK_OBJECTS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(reference) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(&object.0) {
        return Some(*reference);
    }
    let reference = match value {
        crate::ObjectValue::String(ref units) => {
            let length = i32::try_from(units.len()).ok()?;
            // SAFETY: UTF-16 storage remains valid for the synchronous call.
            unsafe { roblox_jni_fallback_new_string(units.as_ptr(), length) as usize }
        }
        crate::ObjectValue::Opaque => {
            let class = std::ffi::CString::new(class_name).ok()?;
            // SAFETY: the class name remains alive for the synchronous call.
            unsafe { roblox_jni_fallback_new_object(class.as_ptr()) as usize }
        }
        crate::ObjectValue::ByteArray(_)
        | crate::ObjectValue::IntArray(_)
        | crate::ObjectValue::LongArray(_)
        | crate::ObjectValue::ObjectArray(_)
        | crate::ObjectValue::DirectByteBuffer { .. } => {
            eprintln!("[jnivm:fallback] cannot mirror array reference of class {class_name} into C++ libjnivm");
            0
        }
        crate::ObjectValue::CppObject(_) => unreachable!(),
    };
    if reference == 0 {
        eprintln!("[jnivm:fallback] C++ libjnivm could not create a mirror for {class_name}");
        return None;
    }
    cache
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(object.0, reference);
    Some(reference)
}

fn remove_cpp_reference(object: crate::ObjectId) {
    let Some(cache) = CPP_FALLBACK_OBJECTS.get() else { return };
    if let Some(reference) = cache.lock().unwrap_or_else(|p| p.into_inner()).remove(&object.0) {
        if reference != 0 {
            // SAFETY: this mirror was created via roblox_jni_fallback_new_object or
            // roblox_jni_fallback_new_string and is released when the Rust object dies.
            unsafe { roblox_jni_fallback_release_ref(reference as *mut std::ffi::c_void) };
        }
    }
}

fn from_cpp_jvalue(vm: &Vm, ty: &crate::Type, value: jni::jvalue) -> Option<JniValue> {
    // SAFETY: the method descriptor selects the active union member.
    Some(unsafe {
        match ty {
            crate::Type::Boolean => JniValue::Boolean(value.z),
            crate::Type::Byte => JniValue::Byte(value.b),
            crate::Type::Char => JniValue::Char(value.c),
            crate::Type::Short => JniValue::Short(value.s),
            crate::Type::Int => JniValue::Int(value.i),
            crate::Type::Long => JniValue::Long(value.j),
            crate::Type::Float => JniValue::Float(value.f),
            crate::Type::Double => JniValue::Double(value.d),
            crate::Type::Void => JniValue::Void,
            crate::Type::Object(class_name) => {
                if value.l.is_null() {
                    return Some(JniValue::Object(None));
                }
                if class_name == "java/lang/String" {
                    return cpp_string_to_rust(vm, value.l as *mut std::ffi::c_void);
                }
                let env = attached_thread_env(vm);
                let class = vm.find_or_define_class(class_name).ok()?;
                let object = vm
                    .new_local_object(
                        &env,
                        class,
                        crate::ObjectValue::CppObject(value.l as usize),
                    )
                    .ok()?;
                JniValue::Object(Some(object))
            }
            crate::Type::Array(_) => {
                if !value.l.is_null() {
                    // SAFETY: this is a persistent global reference returned by the bridge.
                    roblox_jni_fallback_release_ref(value.l as *mut std::ffi::c_void);
                }
                eprintln!("[jnivm:fallback] array return cannot be represented in the Rust VM yet");
                JniValue::Object(None)
            }
        }
    })
}

fn cpp_string_to_rust(vm: &Vm, string_ref: *mut std::ffi::c_void) -> Option<JniValue> {
    // SAFETY: a null output with zero capacity requests the UTF-16 length.
    let length = unsafe { roblox_jni_fallback_copy_string(string_ref, ptr::null_mut(), 0) };
    if length < 0 {
        eprintln!("[jnivm:fallback] C++ returned an unreadable Java String");
        // SAFETY: release the global reference returned by the fallback bridge.
        unsafe { roblox_jni_fallback_release_ref(string_ref) };
        return None;
    }
    let mut units = vec![0u16; length as usize];
    // SAFETY: the allocated UTF-16 buffer has exactly the required capacity.
    let copied = unsafe {
        roblox_jni_fallback_copy_string(string_ref, units.as_mut_ptr(), length)
    };
    // SAFETY: the copied string is now owned by the Rust VM.
    unsafe { roblox_jni_fallback_release_ref(string_ref) };
    if copied < 0 || copied != length {
        eprintln!("[jnivm:fallback] could not copy C++ Java String contents");
        return None;
    }
    let env = attached_thread_env(vm);
    let class = vm.find_or_define_class("java/lang/String").ok()?;
    let object = vm
        .new_local_object(&env, class, crate::ObjectValue::String(units))
        .ok()?;
    Some(JniValue::Object(Some(object)))
}

fn fallback_cpp_field_get(
    vm: &Vm,
    field: crate::FieldId,
    receiver: Option<crate::ObjectId>,
) -> Option<JniValue> {
    if !crate::cpp_fallback_enabled() {
        return None;
    }
    let (class_name, name, descriptor, is_static) = vm.field_info(field)?;
    let class_c = std::ffi::CString::new(class_name.as_str()).ok()?;
    let name_c = std::ffi::CString::new(name.as_str()).ok()?;
    let descriptor_c = std::ffi::CString::new(descriptor.as_str()).ok()?;
    // SAFETY: the strings remain alive for this synchronous lookup.
    let has_cpp = unsafe {
        roblox_jni_fallback_has_field(
            class_c.as_ptr(),
            name_c.as_ptr(),
            descriptor_c.as_ptr(),
            i32::from(is_static),
            0,
        )
    };
    if has_cpp != 1 {
        return None;
    }
    eprintln!(
        "[jnivm:fallback] Rust field value missing; using C++ libjnivm for {class_name}.{name}:{descriptor}"
    );
    let cpp_receiver = if is_static {
        ptr::null_mut()
    } else {
        let object = receiver?;
        let receiver_class = vm
            .object_class_name(object.0)
            .unwrap_or_else(|| class_name.clone());
        cpp_reference_for(vm, object, &receiver_class)? as *mut std::ffi::c_void
    };
    let mut result: jni::jvalue = unsafe { std::mem::zeroed() };
    // SAFETY: class/member C strings and output storage live through the call.
    let status = unsafe {
        roblox_jni_fallback_get_field(
            class_c.as_ptr(),
            name_c.as_ptr(),
            descriptor_c.as_ptr(),
            i32::from(is_static),
            cpp_receiver,
            &mut result,
        )
    };
    if status != 0 {
        eprintln!(
            "[jnivm:fallback] C++ field read failed for {class_name}.{name}:{descriptor} (status {status})"
        );
        return None;
    }
    let ty = vm.field_type(field)?;
    from_cpp_jvalue(vm, &ty, result)
}

fn fallback_cpp_field_set(
    vm: &Vm,
    field: crate::FieldId,
    receiver: Option<crate::ObjectId>,
    value: &JniValue,
) -> bool {
    if !crate::cpp_fallback_enabled() {
        return false;
    }
    let Some((class_name, name, descriptor, is_static)) = vm.field_info(field) else {
        return false;
    };
    let (Ok(class_c), Ok(name_c), Ok(descriptor_c)) = (
        std::ffi::CString::new(class_name.as_str()),
        std::ffi::CString::new(name.as_str()),
        std::ffi::CString::new(descriptor.as_str()),
    ) else {
        return false;
    };
    // SAFETY: the strings remain alive for this synchronous lookup.
    let has_cpp = unsafe {
        roblox_jni_fallback_has_field(
            class_c.as_ptr(),
            name_c.as_ptr(),
            descriptor_c.as_ptr(),
            i32::from(is_static),
            1,
        )
    };
    if has_cpp != 1 {
        return false;
    }
    let Some(ty) = vm.field_type(field) else {
        return false;
    };
    let Some(value) = to_cpp_jvalue(vm, &ty, value) else {
        return false;
    };
    let cpp_receiver = if is_static {
        ptr::null_mut()
    } else {
        let Some(object) = receiver else { return false };
        let class = vm
            .object_class_name(object.0)
            .unwrap_or_else(|| class_name.clone());
        let Some(reference) = cpp_reference_for(vm, object, &class) else {
            return false;
        };
        reference as *mut std::ffi::c_void
    };
    // SAFETY: class/member C strings and the copied jvalue live through the call.
    let status = unsafe {
        roblox_jni_fallback_set_field(
            class_c.as_ptr(),
            name_c.as_ptr(),
            descriptor_c.as_ptr(),
            i32::from(is_static),
            cpp_receiver,
            value,
        )
    };
    if status != 0 {
        eprintln!(
            "[jnivm:fallback] C++ field write failed for {class_name}.{name}:{descriptor} (status {status})"
        );
        return false;
    }
    eprintln!(
        "[jnivm:fallback] Rust field setter mirrored to C++ libjnivm: {class_name}.{name}:{descriptor}"
    );
    true
}
