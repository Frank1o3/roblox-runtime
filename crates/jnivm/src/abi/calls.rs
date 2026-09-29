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
    env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    _args: *const jni::jvalue,
) -> jni::jobject {
    unsafe { call_object_v(env, object, method, ptr::null_mut()) }
}
unsafe extern "system" fn call_boolean_a(
    env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    _args: *const jni::jvalue,
) -> jni::jboolean {
    unsafe { call_boolean_v(env, object, method, ptr::null_mut()) }
}
unsafe extern "system" fn call_int_a(
    env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    _args: *const jni::jvalue,
) -> jni::jint {
    unsafe { call_int_v(env, object, method, ptr::null_mut()) }
}
unsafe extern "system" fn call_long_a(
    env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    _args: *const jni::jvalue,
) -> jni::jlong {
    unsafe { call_long_v(env, object, method, ptr::null_mut()) }
}
unsafe extern "system" fn call_void_a(
    env: *mut jni::JNIEnv,
    object: jni::jobject,
    method: jni::jmethodID,
    _args: *const jni::jvalue,
) {
    unsafe { call_void_v(env, object, method, ptr::null_mut()) }
}
unsafe extern "system" fn call_static_object_a(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    method: jni::jmethodID,
    _args: *const jni::jvalue,
) -> jni::jobject {
    unsafe { call_static_object_v(env, class, method, ptr::null_mut()) }
}
unsafe extern "system" fn call_static_boolean_a(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    method: jni::jmethodID,
    _args: *const jni::jvalue,
) -> jni::jboolean {
    unsafe { call_static_boolean_v(env, class, method, ptr::null_mut()) }
}
unsafe extern "system" fn call_static_int_a(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    method: jni::jmethodID,
    _args: *const jni::jvalue,
) -> jni::jint {
    unsafe { call_static_int_v(env, class, method, ptr::null_mut()) }
}
unsafe extern "system" fn call_static_long_a(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    method: jni::jmethodID,
    _args: *const jni::jvalue,
) -> jni::jlong {
    unsafe { call_static_long_v(env, class, method, ptr::null_mut()) }
}
unsafe extern "system" fn call_static_void_a(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    method: jni::jmethodID,
    _args: *const jni::jvalue,
) {
    unsafe { call_static_void_v(env, class, method, ptr::null_mut()) }
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

