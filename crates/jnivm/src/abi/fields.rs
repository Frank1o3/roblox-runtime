// JNI field accessors share one typed store keyed by field ID and receiver.
unsafe extern "C" {
    fn roblox_ime_state_text(output: *mut std::ffi::c_char, capacity: i32) -> i32;
    fn roblox_ime_state_selection(start: *mut i32, end: *mut i32);
    fn roblox_ime_state_composition(start: *mut i32, end: *mut i32);
}

fn field_receiver(object: jni::jobject) -> Option<crate::ObjectId> {
    (!object.is_null()).then_some(crate::ObjectId(object as usize as u64))
}

fn read_field(field: jni::jfieldID, receiver: Option<crate::ObjectId>) -> Option<JniValue> {
    let Some(vm) = vm() else { return None };
    let field = crate::FieldId(field as usize as u64);
    if let Some(value) = read_game_text_input_state(vm, field, receiver) {
        return Some(value);
    }
    if vm.field_value_is_set(field, receiver) {
        return match vm.field_value(field, receiver) {
            Ok(value) => Some(value),
            Err(error) => {
                eprintln!("[jnivm] field access failed: {error}");
                None
            }
        };
    }
    if let Some(value) = fallback_cpp_field_get(vm, field, receiver) {
        return Some(value);
    }
    if let Some((class_name, name, descriptor, _)) = vm.field_info(field) {
        eprintln!(
            "[jnivm:fallback] no C++ getter for {class_name}.{name}:{descriptor}; returning JNI default"
        );
    }
    vm.field_value(field, receiver).ok()
}

/// `GameTextInput.State` is created by the companion VM, so its `jobject`
/// cannot be read from the Rust VM's per-object field table. The companion
/// `InputConnection.setState` hook already copies the complete state into the
/// runtime's shared IME snapshot; expose that snapshot through Rust's JNI
/// field table instead of making repeated, unsuccessful C++ field lookups.
fn read_game_text_input_state(
    vm: &Vm,
    field: crate::FieldId,
    receiver: Option<crate::ObjectId>,
) -> Option<JniValue> {
    let Some(_receiver) = receiver else { return None };
    let (class_name, name, descriptor, is_static) = vm.field_info(field)?;
    if class_name != "com/google/androidgamesdk/gametextinput/State" || is_static {
        return None;
    }

    let value = match name.as_str() {
        "text" if descriptor == "Ljava/lang/String;" => {
            let mut bytes = vec![0i8; 4096];
            // SAFETY: `bytes` has the advertised writable capacity.
            let length = unsafe { roblox_ime_state_text(bytes.as_mut_ptr(), bytes.len() as i32) };
            if length < 0 {
                return Some(JniValue::Object(None));
            }
            let raw = bytes[..(length as usize).min(bytes.len().saturating_sub(1))]
                .iter()
                .map(|byte| *byte as u8)
                .collect::<Vec<_>>();
            let text = String::from_utf8_lossy(&raw);
            let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
            let class = vm.find_or_define_class("java/lang/String").ok()?;
            let object = vm
                .new_local_object(
                    &env,
                    class,
                    crate::ObjectValue::String(text.encode_utf16().collect()),
                )
                .ok()?;
            JniValue::Object(Some(object))
        }
        "selectionStart" | "selectionEnd"
            if descriptor == "I" =>
        {
            let (mut start, mut end) = (0, 0);
            // SAFETY: both out-parameters are valid for this synchronous call.
            unsafe { roblox_ime_state_selection(&mut start, &mut end) };
            JniValue::Int(if name == "selectionStart" { start } else { end })
        }
        "composingRegionStart" | "composingRegionEnd" if descriptor == "I" => {
            let (mut start, mut end) = (-1, -1);
            // SAFETY: both out-parameters are valid for this synchronous call.
            unsafe { roblox_ime_state_composition(&mut start, &mut end) };
            JniValue::Int(if name == "composingRegionStart" { start } else { end })
        }
        _ => return None,
    };

    Some(value)
}

fn write_field(field: jni::jfieldID, receiver: Option<crate::ObjectId>, value: JniValue) {
    let Some(vm) = vm() else { return };
    if let Err(error) =
        vm.set_field_value(crate::FieldId(field as usize as u64), receiver, value.clone())
    {
        eprintln!("[jnivm] unimplemented field access: {error}");
        return;
    }
    if !fallback_cpp_field_set(
        vm,
        crate::FieldId(field as usize as u64),
        receiver,
        &value,
    ) {
        if let Some((class_name, name, descriptor, _)) =
            vm.field_info(crate::FieldId(field as usize as u64))
        {
            eprintln!(
                "[jnivm:fallback] C++ setter unavailable for {class_name}.{name}:{descriptor}; Rust value retained"
            );
        }
    }
}

macro_rules! scalar_field_access {
    ($get:ident, $get_static:ident, $set:ident, $set_static:ident, $ty:ty, $variant:ident) => {
        unsafe extern "system" fn $get(
            _env: *mut jni::JNIEnv,
            object: jni::jobject,
            field: jni::jfieldID,
        ) -> $ty {
            match read_field(field, field_receiver(object)) {
                Some(JniValue::$variant(value)) => value,
                Some(_) => {
                    eprintln!("[jnivm] JNI field getter type does not match its descriptor");
                    <$ty>::default()
                }
                None => <$ty>::default(),
            }
        }
        unsafe extern "system" fn $get_static(
            _env: *mut jni::JNIEnv,
            _class: jni::jclass,
            field: jni::jfieldID,
        ) -> $ty {
            match read_field(field, None) {
                Some(JniValue::$variant(value)) => value,
                Some(_) => {
                    eprintln!("[jnivm] JNI static field getter type does not match its descriptor");
                    <$ty>::default()
                }
                None => <$ty>::default(),
            }
        }
        unsafe extern "system" fn $set(
            _env: *mut jni::JNIEnv,
            object: jni::jobject,
            field: jni::jfieldID,
            value: $ty,
        ) {
            write_field(field, field_receiver(object), JniValue::$variant(value));
        }
        unsafe extern "system" fn $set_static(
            _env: *mut jni::JNIEnv,
            _class: jni::jclass,
            field: jni::jfieldID,
            value: $ty,
        ) {
            write_field(field, None, JniValue::$variant(value));
        }
    };
}

scalar_field_access!(
    get_boolean_field,
    get_static_boolean_field,
    set_boolean_field,
    set_static_boolean_field,
    jni::jboolean,
    Boolean
);
scalar_field_access!(
    get_byte_field,
    get_static_byte_field,
    set_byte_field,
    set_static_byte_field,
    jni::jbyte,
    Byte
);
scalar_field_access!(
    get_char_field,
    get_static_char_field,
    set_char_field,
    set_static_char_field,
    jni::jchar,
    Char
);
scalar_field_access!(
    get_short_field,
    get_static_short_field,
    set_short_field,
    set_static_short_field,
    jni::jshort,
    Short
);
scalar_field_access!(
    get_int_field,
    get_static_int_field,
    set_int_field,
    set_static_int_field,
    jni::jint,
    Int
);
scalar_field_access!(
    get_long_field,
    get_static_long_field,
    set_long_field,
    set_static_long_field,
    jni::jlong,
    Long
);
scalar_field_access!(
    get_float_field,
    get_static_float_field,
    set_float_field,
    set_static_float_field,
    jni::jfloat,
    Float
);
scalar_field_access!(
    get_double_field,
    get_static_double_field,
    set_double_field,
    set_static_double_field,
    jni::jdouble,
    Double
);

unsafe extern "system" fn get_object_field(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    field: jni::jfieldID,
) -> jni::jobject {
    match read_field(field, field_receiver(object)) {
        Some(JniValue::Object(Some(id))) => id.0 as usize as jni::jobject,
        Some(JniValue::Object(None)) | None => ptr::null_mut(),
        Some(_) => {
            eprintln!("[jnivm] JNI object field getter type does not match its descriptor");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn get_static_object_field(
    raw_env: *mut jni::JNIEnv,
    _class: jni::jclass,
    field: jni::jfieldID,
) -> jni::jobject {
    let Some(vm) = vm() else { return ptr::null_mut() };
    let Some(env) = env_token(vm, raw_env) else { return ptr::null_mut() };
    match read_field(field, None) {
        Some(JniValue::Object(Some(id))) => match vm.clone_local_ref(&env, id.0) {
            Ok(reference) => reference.0 as usize as jni::jobject,
            Err(error) => {
                eprintln!("[jnivm] GetStaticObjectField could not create a local ref: {error}");
                ptr::null_mut()
            }
        },
        Some(JniValue::Object(None)) | None => ptr::null_mut(),
        Some(_) => {
            eprintln!("[jnivm] JNI static object field getter type does not match its descriptor");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn set_object_field(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
    field: jni::jfieldID,
    value: jni::jobject,
) {
    write_field(
        field,
        field_receiver(object),
        JniValue::Object((!value.is_null()).then_some(crate::ObjectId(value as usize as u64))),
    );
}

unsafe extern "system" fn set_static_object_field(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    field: jni::jfieldID,
    value: jni::jobject,
) {
    write_field(
        field,
        None,
        JniValue::Object((!value.is_null()).then_some(crate::ObjectId(value as usize as u64))),
    );
}
