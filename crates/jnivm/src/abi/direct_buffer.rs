// Direct ByteBuffer JNI entries mirrored from libjnivm's internal/bytebuffer.cpp.

unsafe extern "system" fn new_direct_byte_buffer(
    env: *mut jni::JNIEnv,
    address: *mut c_void,
    capacity: jni::jlong,
) -> jni::jobject {
    let Some(vm) = vm() else {
        return ptr::null_mut();
    };
    let Some(env) = env_token(vm, env) else {
        return ptr::null_mut();
    };
    let Ok(class) = vm.find_or_define_class("java/nio/ByteBuffer") else {
        return ptr::null_mut();
    };
    match vm.new_local_object(
        &env,
        class,
        crate::ObjectValue::DirectByteBuffer {
            address: address as usize,
            capacity,
        },
    ) {
        Ok(object) => object.0 as usize as jni::jobject,
        Err(error) => {
            eprintln!("[jnivm] NewDirectByteBuffer failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn get_direct_buffer_address(
    env: *mut jni::JNIEnv,
    object: jni::jobject,
) -> *mut c_void {
    if object.is_null() {
        return ptr::null_mut();
    }
    let Some(vm) = vm() else {
        return ptr::null_mut();
    };
    let Some(env) = env_token(vm, env) else {
        return ptr::null_mut();
    };
    match vm.object_value(&env, crate::ObjectId(object as usize as u64)) {
        Ok(crate::ObjectValue::DirectByteBuffer { address, .. }) => *address as *mut c_void,
        _ => ptr::null_mut(),
    }
}

unsafe extern "system" fn get_direct_buffer_capacity(
    env: *mut jni::JNIEnv,
    object: jni::jobject,
) -> jni::jlong {
    if object.is_null() {
        return -1;
    }
    let Some(vm) = vm() else {
        return -1;
    };
    let Some(env) = env_token(vm, env) else {
        return -1;
    };
    match vm.object_value(&env, crate::ObjectId(object as usize as u64)) {
        Ok(crate::ObjectValue::DirectByteBuffer { capacity, .. }) => *capacity,
        _ => -1,
    }
}
