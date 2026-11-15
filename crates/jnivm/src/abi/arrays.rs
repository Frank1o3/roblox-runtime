// Object-array support used by Java methods that return arrays.
use crate::ObjectId;

/// The five entry points JNI defines per primitive array type: allocation,
/// direct element access, its release, and the two region copies.
///
/// The function names are JNI's; the bodies are one shape instantiated per
/// element type over the VM's generic primitive-array methods.
macro_rules! primitive_array_abi {
    (
        $element:ty, $elem:ty, $array:ty,
        $new:ident, $get_elements:ident, $release_elements:ident,
        $get_region:ident, $set_region:ident
    ) => {
        unsafe extern "system" fn $new(
            env: *mut jni::JNIEnv,
            length: jni::jsize,
        ) -> $array {
            let Some(vm) = vm() else { return ptr::null_mut() };
            let Some(env) = env_token(vm, env) else { return ptr::null_mut() };
            if length < 0 {
                eprintln!(
                    "[jnivm] {} received a negative length",
                    stringify!($new)
                );
                return ptr::null_mut();
            }
            match vm.new_primitive_array::<$element>(&env, length as usize) {
                Ok(array) => array.0 as usize as $array,
                Err(error) => {
                    eprintln!("[jnivm] {} failed: {error}", stringify!($new));
                    ptr::null_mut()
                }
            }
        }

        unsafe extern "system" fn $get_elements(
            env: *mut jni::JNIEnv,
            array: $array,
            is_copy: *mut jni::jboolean,
        ) -> *mut $elem {
            let Some(vm) = vm() else { return ptr::null_mut() };
            let Some(env) = env_token(vm, env) else { return ptr::null_mut() };
            if array.is_null() {
                return ptr::null_mut();
            }
            if !is_copy.is_null() {
                // SAFETY: JNI specifies a writable output pointer when
                // `isCopy` is non-null.
                unsafe { *is_copy = jni::JNI_FALSE };
            }
            match vm.primitive_array_elements::<$element>(&env, ObjectId(array as usize as u64)) {
                Ok(elements) => elements,
                Err(error) => {
                    eprintln!("[jnivm] {} failed: {error}", stringify!($get_elements));
                    ptr::null_mut()
                }
            }
        }

        unsafe extern "system" fn $release_elements(
            env: *mut jni::JNIEnv,
            array: $array,
            _elements: *mut $elem,
            _mode: jni::jint,
        ) {
            let Some(vm) = vm() else { return };
            let Some(env) = env_token(vm, env) else { return };
            if array.is_null() {
                return;
            }
            // The VM hands out its own backing allocation and nothing is
            // copied, so a release has nothing to write back — but an
            // invalid array is still worth reporting.
            if vm
                .primitive_array_elements::<$element>(&env, ObjectId(array as usize as u64))
                .is_err()
            {
                eprintln!(
                    "[jnivm] {} received an invalid array",
                    stringify!($release_elements)
                );
            }
        }

        unsafe extern "system" fn $get_region(
            env: *mut jni::JNIEnv,
            array: $array,
            start: jni::jsize,
            length: jni::jsize,
            buffer: *mut $elem,
        ) {
            let Some(vm) = vm() else { return };
            let Some(env) = env_token(vm, env) else { return };
            if array.is_null() || start < 0 || length < 0 || (length > 0 && buffer.is_null()) {
                return;
            }
            match vm.primitive_array_region::<$element>(
                &env,
                ObjectId(array as usize as u64),
                start as usize,
                length as usize,
            ) {
                Ok(values) if !values.is_empty() => {
                    // SAFETY: `values` holds exactly `values.len()` elements
                    // and the caller promised room for `length` of them.
                    unsafe { ptr::copy_nonoverlapping(values.as_ptr(), buffer, values.len()) }
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("[jnivm] {} failed: {error}", stringify!($get_region))
                }
            }
        }

        unsafe extern "system" fn $set_region(
            env: *mut jni::JNIEnv,
            array: $array,
            start: jni::jsize,
            length: jni::jsize,
            buffer: *const $elem,
        ) {
            let Some(vm) = vm() else { return };
            let Some(env) = env_token(vm, env) else { return };
            if array.is_null() || start < 0 || length < 0 || (length > 0 && buffer.is_null()) {
                return;
            }
            let values = if length == 0 {
                &[]
            } else {
                // SAFETY: the caller promised `length` readable elements.
                unsafe { std::slice::from_raw_parts(buffer, length as usize) }
            };
            if let Err(error) = vm.set_primitive_array_region::<$element>(
                &env,
                ObjectId(array as usize as u64),
                start as usize,
                values,
            ) {
                eprintln!("[jnivm] {} failed: {error}", stringify!($set_region));
            }
        }
    };
}

primitive_array_abi!(
    i8, jni::jbyte, jni::jbyteArray, new_byte_array, get_byte_array_elements,
    release_byte_array_elements, get_byte_array_region, set_byte_array_region
);
primitive_array_abi!(
    bool, jni::jboolean, jni::jbooleanArray, new_boolean_array,
    get_boolean_array_elements, release_boolean_array_elements,
    get_boolean_array_region, set_boolean_array_region
);
primitive_array_abi!(
    u16, jni::jchar, jni::jcharArray, new_char_array, get_char_array_elements,
    release_char_array_elements, get_char_array_region, set_char_array_region
);
primitive_array_abi!(
    i16, jni::jshort, jni::jshortArray, new_short_array, get_short_array_elements,
    release_short_array_elements, get_short_array_region, set_short_array_region
);
primitive_array_abi!(
    i32, jni::jint, jni::jintArray, new_int_array, get_int_array_elements,
    release_int_array_elements, get_int_array_region, set_int_array_region
);
primitive_array_abi!(
    i64, jni::jlong, jni::jlongArray, new_long_array, get_long_array_elements,
    release_long_array_elements, get_long_array_region, set_long_array_region
);
primitive_array_abi!(
    f32, jni::jfloat, jni::jfloatArray, new_float_array, get_float_array_elements,
    release_float_array_elements, get_float_array_region, set_float_array_region
);
primitive_array_abi!(
    f64, jni::jdouble, jni::jdoubleArray, new_double_array, get_double_array_elements,
    release_double_array_elements, get_double_array_region, set_double_array_region
);

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
        Ok(crate::ObjectValue::BooleanArray(values)) => values.len() as jni::jsize,
        Ok(crate::ObjectValue::CharArray(values)) => values.len() as jni::jsize,
        Ok(crate::ObjectValue::ShortArray(values)) => values.len() as jni::jsize,
        Ok(crate::ObjectValue::IntArray(values)) => values.len() as jni::jsize,
        Ok(crate::ObjectValue::LongArray(values)) => values.len() as jni::jsize,
        Ok(crate::ObjectValue::FloatArray(values)) => values.len() as jni::jsize,
        Ok(crate::ObjectValue::DoubleArray(values)) => values.len() as jni::jsize,
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
