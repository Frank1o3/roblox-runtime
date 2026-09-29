// JNI instance and static method call adapters.
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
    _args: jni::va_list,
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
    invoke(method, None)
        .and_then(object_result)
        .unwrap_or(ptr::null_mut())
}

unsafe extern "system" fn call_object_v(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    _args: jni::va_list,
) -> jni::jobject {
    invoke(
        method,
        (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)),
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
    _args: jni::va_list,
) -> jni::jobject {
    invoke(method, None)
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
    _args: jni::va_list,
) -> jni::jboolean {
    invoke(
        method,
        (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)),
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
    _args: jni::va_list,
) -> jni::jint {
    invoke(
        method,
        (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)),
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
    _args: jni::va_list,
) -> jni::jlong {
    invoke(
        method,
        (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)),
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
    _args: jni::va_list,
) {
    let _ = invoke(
        method,
        (!object.is_null()).then_some(crate::ObjectId(object as usize as u64)),
    );
}
unsafe extern "system" fn call_static_boolean_v(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    method: jni::jmethodID,
    _args: jni::va_list,
) -> jni::jboolean {
    invoke(method, None)
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
    _args: jni::va_list,
) -> jni::jint {
    invoke(method, None)
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
    _args: jni::va_list,
) -> jni::jlong {
    invoke(method, None)
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
    _args: jni::va_list,
) {
    let _ = invoke(method, None);
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
    match vm.invoke_method(method, receiver, &arguments) {
        Ok(value) => Some(value),
        Err(error) => {
            eprintln!("[jnivm] call through unknown JNI method ID: {error}");
            None
        }
    }
}

fn invoke(method: jni::jmethodID, receiver: Option<crate::ObjectId>) -> Option<JniValue> {
    let Some(vm) = vm() else { return None };
    match vm.invoke_default(MethodId(method as usize as u64), receiver) {
        Ok(value) => Some(value),
        Err(error) => {
            eprintln!("[jnivm] call through unknown JNI method ID: {error}");
            None
        }
    }
}
