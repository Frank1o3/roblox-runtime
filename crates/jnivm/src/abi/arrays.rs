// Object-array support used by Java methods that return arrays.
use crate::ObjectId;

unsafe extern "system" fn new_byte_array(
    env: *mut jni::JNIEnv,
    length: jni::jsize,
) -> jni::jbyteArray {
    let Some(vm) = vm() else { return ptr::null_mut() };
    let Some(env) = env_token(vm, env) else { return ptr::null_mut() };
    if length < 0 {
        eprintln!("[jnivm] NewByteArray received a negative length");
        return ptr::null_mut();
    }
    let Ok(class) = vm.find_or_define_class("java/lang/Object") else {
        return ptr::null_mut();
    };
    match vm.new_local_object(
        &env,
        class,
        crate::ObjectValue::ByteArray(vec![0; length as usize]),
    ) {
        Ok(array) => array.0 as usize as jni::jbyteArray,
        Err(error) => {
            eprintln!("[jnivm] NewByteArray failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn get_byte_array_elements(
    env: *mut jni::JNIEnv,
    array: jni::jbyteArray,
    is_copy: *mut jni::jboolean,
) -> *mut jni::jbyte {
    let Some(vm) = vm() else { return ptr::null_mut() };
    let Some(env) = env_token(vm, env) else { return ptr::null_mut() };
    if array.is_null() {
        return ptr::null_mut();
    }
    if !is_copy.is_null() {
        unsafe { *is_copy = jni::JNI_FALSE };
    }
    match vm.byte_array_elements(&env, ObjectId(array as usize as u64)) {
        Ok(elements) => elements,
        Err(error) => {
            eprintln!("[jnivm] GetByteArrayElements failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn release_byte_array_elements(
    env: *mut jni::JNIEnv,
    array: jni::jbyteArray,
    elements: *mut jni::jbyte,
    _mode: jni::jint,
) {
    let Some(vm) = vm() else { return };
    let Some(env) = env_token(vm, env) else { return };
    if array.is_null() || elements.is_null() {
        return;
    }
    // libjnivm exposes the backing allocation directly and its release is a no-op.
    if vm
        .byte_array_elements(&env, ObjectId(array as usize as u64))
        .is_err()
    {
        eprintln!("[jnivm] ReleaseByteArrayElements received an invalid array");
    }
}

unsafe extern "system" fn get_byte_array_region(
    env: *mut jni::JNIEnv,
    array: jni::jbyteArray,
    start: jni::jsize,
    length: jni::jsize,
    buffer: *mut jni::jbyte,
) {
    let Some(vm) = vm() else { return };
    let Some(env) = env_token(vm, env) else { return };
    if array.is_null() || start < 0 || length < 0 || (length > 0 && buffer.is_null()) {
        return;
    }
    match vm.byte_array_region(&env, ObjectId(array as usize as u64), start as usize, length as usize) {
        Ok(values) if !values.is_empty() => unsafe {
            ptr::copy_nonoverlapping(values.as_ptr(), buffer, values.len())
        },
        Ok(_) => {},
        Err(error) => eprintln!("[jnivm] GetByteArrayRegion failed: {error}"),
    }
}

unsafe extern "system" fn set_byte_array_region(
    env: *mut jni::JNIEnv,
    array: jni::jbyteArray,
    start: jni::jsize,
    length: jni::jsize,
    buffer: *const jni::jbyte,
) {
    let Some(vm) = vm() else { return };
    let Some(env) = env_token(vm, env) else { return };
    if array.is_null() || start < 0 || length < 0 || (length > 0 && buffer.is_null()) {
        return;
    }
    let values = if length == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(buffer, length as usize) }
    };
    if let Err(error) = vm.set_byte_array_region(
        &env,
        ObjectId(array as usize as u64),
        start as usize,
        values,
    ) {
        eprintln!("[jnivm] SetByteArrayRegion failed: {error}");
    }
}

unsafe extern "system" fn new_object_array(
    env: *mut jni::JNIEnv,
    length: jni::jsize,
    element_class: jni::jclass,
    initial: jni::jobject,
) -> jni::jobjectArray {
    let Some(vm) = vm() else {
        return ptr::null_mut();
    };
    if length < 0 || element_class.is_null() {
        eprintln!("[jnivm] NewObjectArray received an invalid length or class");
        return ptr::null_mut();
    }
    if vm.class_name(ClassId(element_class as usize as u64)).is_none() {
        eprintln!("[jnivm] NewObjectArray received an unknown element class");
        return ptr::null_mut();
    }
    let Some(env) = env_token(vm, env) else {
        return ptr::null_mut();
    };
    match vm.new_local_object_array(
        &env,
        length as usize,
        (!initial.is_null()).then_some(ObjectId(initial as usize as u64)),
    ) {
        Ok(array) => array.0 as usize as jni::jobjectArray,
        Err(error) => {
            eprintln!("[jnivm] NewObjectArray failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn get_array_length(
    env: *mut jni::JNIEnv,
    array: jni::jarray,
) -> jni::jsize {
    let Some(vm) = vm() else { return 0 };
    let Some(env) = env_token(vm, env) else { return 0 };
    if array.is_null() {
        eprintln!("[jnivm] GetArrayLength received null");
        return 0;
    }
    match vm.object_value(&env, ObjectId(array as usize as u64)) {
        Ok(crate::ObjectValue::ObjectArray(values)) => values.len() as jni::jsize,
        Ok(crate::ObjectValue::ByteArray(values)) => values.len() as jni::jsize,
        Ok(crate::ObjectValue::IntArray(values)) => values.len() as jni::jsize,
        Ok(crate::ObjectValue::LongArray(values)) => values.len() as jni::jsize,
        Ok(_) => {
            eprintln!("[jnivm] GetArrayLength received a non-array object");
            0
        }
        Err(error) => {
            eprintln!("[jnivm] GetArrayLength failed: {error}");
            0
        }
    }
}

unsafe extern "system" fn get_object_array_element(
    env: *mut jni::JNIEnv,
    array: jni::jobjectArray,
    index: jni::jsize,
) -> jni::jobject {
    let Some(vm) = vm() else { return ptr::null_mut() };
    let Some(env) = env_token(vm, env) else { return ptr::null_mut() };
    if array.is_null() || index < 0 {
        return ptr::null_mut();
    }
    match vm.object_array_element(
        &env,
        ObjectId(array as usize as u64),
        index as usize,
    ) {
        Ok(Some(object)) => object.0 as usize as jni::jobject,
        Ok(None) => ptr::null_mut(),
        Err(error) => {
            eprintln!("[jnivm] GetObjectArrayElement failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn set_object_array_element(
    env: *mut jni::JNIEnv,
    array: jni::jobjectArray,
    index: jni::jsize,
    value: jni::jobject,
) {
    let Some(vm) = vm() else { return };
    let Some(env) = env_token(vm, env) else { return };
    if array.is_null() || index < 0 {
        return;
    }
    if let Err(error) = vm.set_object_array_element(
        &env,
        ObjectId(array as usize as u64),
        index as usize,
        (!value.is_null()).then_some(ObjectId(value as usize as u64)),
    ) {
        eprintln!("[jnivm] SetObjectArrayElement failed: {error}");
    }
}
