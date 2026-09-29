// JNI instance and static method call adapters.
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
        JniValue::Boolean(value) => Some(value as u8),
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
            JniValue::Boolean(value) => Some(value as u8),
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
        .and_then(|value| match value { JniValue::Boolean(value) => Some(value as u8), _ => None })
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
        .and_then(|value| match value { JniValue::Boolean(value) => Some(value as u8), _ => None })
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
                crate::Type::Boolean => JniValue::Boolean(value.z != 0),
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
