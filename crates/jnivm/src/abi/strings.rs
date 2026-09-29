// JNI string creation, access, and backing-buffer functions.
unsafe extern "system" fn new_string(
    env: *mut jni::JNIEnv,
    chars: *const jni::jchar,
    length: jni::jsize,
) -> jni::jstring {
    if length < 0 || (length > 0 && chars.is_null()) { return ptr::null_mut() }
    let Some(vm) = vm() else { return ptr::null_mut() };
    let Some(env_token) = env_token(vm, env) else { return ptr::null_mut() };
    // SAFETY: JNI requires `chars` to point at `length` UTF-16 code units.
    let chars = if length == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(chars, length as usize) } };
    let class = match vm.find_or_define_class("java/lang/String") { Ok(c) => c, Err(_) => return ptr::null_mut() };
    match vm.new_local_object(&env_token, class, crate::ObjectValue::String(chars.to_vec())) {
        Ok(object) => object.0 as usize as jni::jstring,
        Err(error) => { eprintln!("[jnivm] NewString failed: {error}"); ptr::null_mut() }
    }
}

unsafe extern "system" fn new_string_utf(env: *mut jni::JNIEnv, chars: *const c_char) -> jni::jstring {
    // libjnivm treats a null C string as an empty Java String.
    let value = if chars.is_null() {
        std::borrow::Cow::Borrowed("")
    } else {
        // SAFETY: JNI requires a NUL-terminated modified UTF-8 input string.
        unsafe { CStr::from_ptr(chars) }.to_string_lossy()
    };
    let units = value.encode_utf16().collect();
    let Some(vm) = vm() else { return ptr::null_mut() };
    let Some(env_token) = env_token(vm, env) else { return ptr::null_mut() };
    let class = match vm.find_or_define_class("java/lang/String") { Ok(c) => c, Err(_) => return ptr::null_mut() };
    match vm.new_local_object(&env_token, class, crate::ObjectValue::String(units)) {
        Ok(object) => object.0 as usize as jni::jstring,
        Err(error) => { eprintln!("[jnivm] NewStringUTF failed: {error}"); ptr::null_mut() }
    }
}

unsafe extern "system" fn get_string_length(env: *mut jni::JNIEnv, string: jni::jstring) -> jni::jsize {
    string_units(env, string).map_or(0, |units| units.len().min(i32::MAX as usize) as i32)
}

unsafe extern "system" fn get_string_utf_length(env: *mut jni::JNIEnv, string: jni::jstring) -> jni::jsize {
    string_units(env, string).map_or(0, |units| String::from_utf16_lossy(&units).len().min(i32::MAX as usize) as i32)
}

unsafe extern "system" fn get_string_chars(
    env: *mut jni::JNIEnv, string: jni::jstring, is_copy: *mut jni::jboolean,
) -> *const jni::jchar {
    if string.is_null() {
        let mut units = vec![0u16].into_boxed_slice();
        let pointer = units.as_mut_ptr();
        UTF16_BUFFERS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|p| p.into_inner()).insert(pointer as usize, units);
        return pointer;
    }
    let Some(units) = string_units(env, string) else { return ptr::null() };
    let mut units = units.into_boxed_slice();
    let pointer = units.as_mut_ptr();
    UTF16_BUFFERS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|p| p.into_inner()).insert(pointer as usize, units);
    if !is_copy.is_null() {
        // SAFETY: JNI marks the output flag writable when non-null.
        unsafe { *is_copy = jni::JNI_TRUE };
    }
    pointer
}

unsafe extern "system" fn release_string_chars(_env: *mut jni::JNIEnv, _string: jni::jstring, chars: *const jni::jchar) {
    if chars.is_null() { return }
    if let Some(buffers) = UTF16_BUFFERS.get() {
        buffers.lock().unwrap_or_else(|p| p.into_inner()).remove(&(chars as usize));
    }
}

unsafe extern "system" fn get_string_utf_chars(
    env: *mut jni::JNIEnv, string: jni::jstring, is_copy: *mut jni::jboolean,
) -> *const c_char {
    if string.is_null() {
        // This is the C++ libjnivm behavior: null Java strings expose an empty
        // C string, and the returned storage is not a copied Java string.
        let empty = CString::default();
        let pointer = empty.as_ptr();
        UTF8_BUFFERS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|p| p.into_inner()).insert(pointer as usize, empty);
        if !is_copy.is_null() {
            unsafe { *is_copy = jni::JNI_FALSE };
        }
        return pointer;
    }
    let Some(units) = string_units(env, string) else { return ptr::null() };
    let value = String::from_utf16_lossy(&units);
    let Ok(value) = CString::new(value) else { return ptr::null() };
    let pointer = value.as_ptr();
    UTF8_BUFFERS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|p| p.into_inner()).insert(pointer as usize, value);
    if !is_copy.is_null() {
        // SAFETY: JNI marks the output flag writable when non-null.
        unsafe { *is_copy = jni::JNI_TRUE };
    }
    pointer
}

unsafe extern "system" fn release_string_utf_chars(_env: *mut jni::JNIEnv, _string: jni::jstring, chars: *const c_char) {
    if chars.is_null() { return }
    if let Some(buffers) = UTF8_BUFFERS.get() {
        buffers.lock().unwrap_or_else(|p| p.into_inner()).remove(&(chars as usize));
    }
}

fn string_units(env: *mut jni::JNIEnv, string: jni::jstring) -> Option<Vec<u16>> {
    let vm = vm()?;
    let env = env_token(vm, env)?;
    match vm.object_value(&env, crate::ObjectId(string as usize as u64)).ok()? {
        crate::ObjectValue::String(units) => Some(units),
        _ => {
            eprintln!("[jnivm] string operation received a non-string object");
            None
        }
    }
}
