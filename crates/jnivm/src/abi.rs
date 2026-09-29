//! JNI C ABI tables and process/thread handles.
//!
//! Only ABI entries with defined behavior are installed. A missing Java
//! method or field is logged and offered to the companion C++ libjnivm bridge;
//! handlers that cannot be mirrored safely return descriptor-correct defaults.
//! The remaining function-table surface is filled from runtime observations.

#![allow(unsafe_code)]

use crate::{ClassId, JniValue, MethodId, Vm};
use jni_sys as jni;
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use std::sync::{Mutex, OnceLock};

static VM: OnceLock<Vm> = OnceLock::new();
static ENV_TABLE: OnceLock<usize> = OnceLock::new();
static VM_TABLE: OnceLock<usize> = OnceLock::new();
static VM_HANDLE: OnceLock<usize> = OnceLock::new();
static UTF8_BUFFERS: OnceLock<Mutex<HashMap<usize, CString>>> = OnceLock::new();
static UTF16_BUFFERS: OnceLock<Mutex<HashMap<usize, Box<[u16]>>>> = OnceLock::new();

thread_local! {
    /// Points to a boxed `JNIEnv` table pointer, the layout expected by JNI.
    static THREAD_ENV: Cell<usize> = const { Cell::new(0) };
}

fn vm() -> Option<&'static Vm> {
    VM.get()
}

include!("abi/builtins.rs");
include!("abi/platform.rs");
include!("abi/native_helper.rs");
include!("abi/arrays.rs");

fn env_table() -> *const jni::JNINativeInterface_ {
    *ENV_TABLE.get_or_init(|| {
        // All-zero is a valid initial value for a C function table: null
        // function pointers mean an entry has not been implemented yet.
        let mut table: jni::JNINativeInterface_ = unsafe { std::mem::zeroed() };
        // jni-sys 0.4 exposes each versioned prefix as a union member and
        // stores plain C function pointers rather than Option<fn>.
        let slots = unsafe { &mut table.v24 };
        slots.GetVersion = get_version;
        slots.FindClass = find_class;
        slots.GetObjectClass = get_object_class;
        slots.GetMethodID = get_method_id;
        slots.GetStaticMethodID = get_static_method_id;
        slots.GetFieldID = get_field_id;
        slots.GetStaticFieldID = get_static_field_id;
        slots.GetObjectField = get_object_field;
        slots.GetBooleanField = get_boolean_field;
        slots.GetByteField = get_byte_field;
        slots.GetCharField = get_char_field;
        slots.GetShortField = get_short_field;
        slots.GetIntField = get_int_field;
        slots.GetLongField = get_long_field;
        slots.GetFloatField = get_float_field;
        slots.GetDoubleField = get_double_field;
        slots.SetObjectField = set_object_field;
        slots.SetBooleanField = set_boolean_field;
        slots.SetByteField = set_byte_field;
        slots.SetCharField = set_char_field;
        slots.SetShortField = set_short_field;
        slots.SetIntField = set_int_field;
        slots.SetLongField = set_long_field;
        slots.SetFloatField = set_float_field;
        slots.SetDoubleField = set_double_field;
        slots.GetStaticObjectField = get_static_object_field;
        slots.GetStaticBooleanField = get_static_boolean_field;
        slots.GetStaticByteField = get_static_byte_field;
        slots.GetStaticCharField = get_static_char_field;
        slots.GetStaticShortField = get_static_short_field;
        slots.GetStaticIntField = get_static_int_field;
        slots.GetStaticLongField = get_static_long_field;
        slots.GetStaticFloatField = get_static_float_field;
        slots.GetStaticDoubleField = get_static_double_field;
        slots.SetStaticObjectField = set_static_object_field;
        slots.SetStaticBooleanField = set_static_boolean_field;
        slots.SetStaticByteField = set_static_byte_field;
        slots.SetStaticCharField = set_static_char_field;
        slots.SetStaticShortField = set_static_short_field;
        slots.SetStaticIntField = set_static_int_field;
        slots.SetStaticLongField = set_static_long_field;
        slots.SetStaticFloatField = set_static_float_field;
        slots.SetStaticDoubleField = set_static_double_field;
        slots.RegisterNatives = register_natives;
        slots.Throw = throw;
        slots.ThrowNew = throw_new;
        slots.ExceptionOccurred = exception_occurred;
        slots.ExceptionDescribe = exception_describe;
        slots.NewLocalRef = new_local_ref;
        slots.DeleteLocalRef = delete_local_ref;
        slots.NewGlobalRef = new_global_ref;
        slots.DeleteGlobalRef = delete_global_ref;
        slots.IsSameObject = is_same_object;
        slots.ExceptionCheck = exception_check;
        slots.ExceptionClear = exception_clear;
        slots.NewString = new_string;
        slots.NewStringUTF = new_string_utf;
        slots.GetStringLength = get_string_length;
        slots.GetStringUTFLength = get_string_utf_length;
        slots.GetStringChars = get_string_chars;
        slots.ReleaseStringChars = release_string_chars;
        slots.GetStringUTFChars = get_string_utf_chars;
        slots.ReleaseStringUTFChars = release_string_utf_chars;
        slots.GetJavaVM = get_java_vm;
        slots.AllocObject = alloc_object;
        slots.NewObjectV = new_object_v;
        slots.NewObjectA = new_object_a;
        slots.GetArrayLength = get_array_length;
        slots.NewObjectArray = new_object_array;
        slots.GetObjectArrayElement = get_object_array_element;
        slots.SetObjectArrayElement = set_object_array_element;
        slots.CallObjectMethodV = call_object_v;
        slots.CallBooleanMethodV = call_boolean_v;
        slots.CallIntMethodV = call_int_v;
        slots.CallLongMethodV = call_long_v;
        slots.CallVoidMethodV = call_void_v;
        slots.CallStaticObjectMethodV = call_static_object_v;
        slots.CallStaticBooleanMethodV = call_static_boolean_v;
        slots.CallStaticIntMethodV = call_static_int_v;
        slots.CallStaticLongMethodV = call_static_long_v;
        slots.CallStaticVoidMethodV = call_static_void_v;
        slots.CallObjectMethodA = call_object_a;
        slots.CallBooleanMethodA = call_boolean_a;
        slots.CallIntMethodA = call_int_a;
        slots.CallLongMethodA = call_long_a;
        slots.CallVoidMethodA = call_void_a;
        slots.CallStaticObjectMethodA = call_static_object_a;
        slots.CallStaticBooleanMethodA = call_static_boolean_a;
        slots.CallStaticIntMethodA = call_static_int_a;
        slots.CallStaticLongMethodA = call_static_long_a;
        slots.CallStaticVoidMethodA = call_static_void_a;
        Box::into_raw(Box::new(table)) as usize
    }) as *const jni::JNINativeInterface_
}

fn vm_table() -> *const jni::JNIInvokeInterface_ {
    *VM_TABLE.get_or_init(|| {
        let mut table: jni::JNIInvokeInterface_ = unsafe { std::mem::zeroed() };
        let slots = unsafe { &mut table.v1_4 };
        slots.DestroyJavaVM = destroy_vm;
        slots.AttachCurrentThread = attach_current_thread;
        slots.AttachCurrentThreadAsDaemon = attach_current_thread;
        slots.DetachCurrentThread = detach_current_thread;
        slots.GetEnv = get_env;
        Box::into_raw(Box::new(table)) as usize
    }) as *const jni::JNIInvokeInterface_
}

/// Create the Rust JavaVM and attach the calling thread.
pub fn create_vm() -> Result<*mut c_void, String> {
    if VM.get().is_some() {
        return Err("a Rust JavaVM already exists".into());
    }
    VM.set(Vm::new())
        .map_err(|_| "a Rust JavaVM already exists")?;
    install_builtin_methods(vm().expect("VM was just initialized"))?;
    let _ = env_table();
    let _ = vm_table();
    vm().expect("VM was just initialized")
        .attach_current_thread();
    let handle = Box::new(vm_table() as jni::JavaVM);
    let raw = Box::into_raw(handle) as usize;
    VM_HANDLE
        .set(raw)
        .map_err(|_| "a Rust JavaVM already exists")?;
    eprintln!("[jnivm] experimental pure Rust JavaVM initialized");
    Ok(raw as *mut c_void)
}

pub fn vm_exists() -> bool {
    VM_HANDLE.get().is_some()
}

/// Return the current thread's `JNIEnv*` handle, attaching it if necessary.
pub fn current_env() -> Option<*mut c_void> {
    let vm = vm()?;
    Some(thread_env(vm) as *mut c_void)
}

/// Allocate an opaque Java object in the Rust VM for a native entry point
/// which must be called with a VM-owned reference.
pub fn new_opaque_object(class_name: &str) -> Result<*mut c_void, String> {
    let vm = vm().ok_or("Rust JavaVM has not been created")?;
    let class = vm
        .find_or_define_class(class_name)
        .map_err(|error| error.to_string())?;
    let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    let object = vm
        .new_local_object(&env, class, crate::ObjectValue::Opaque)
        .map_err(|error| error.to_string())?;
    Ok(object.0 as usize as *mut c_void)
}

/// Allocate a Java string in the Rust VM for a native entry point.
pub fn new_string_ref(value: &str) -> Result<*mut c_void, String> {
    let vm = vm().ok_or("Rust JavaVM has not been created")?;
    let class = vm
        .find_or_define_class("java/lang/String")
        .map_err(|error| error.to_string())?;
    let env = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    let object = vm
        .new_local_object(
            &env,
            class,
            crate::ObjectValue::String(value.encode_utf16().collect()),
        )
        .map_err(|error| error.to_string())?;
    Ok(object.0 as usize as *mut c_void)
}

/// Invoke a mapped engine's `JNI_OnLoad` with the experimental JavaVM.
///
/// # Safety
/// `function` must point to the mapped engine's JNI_OnLoad export.
pub unsafe fn call_on_load(function: *mut c_void) -> Result<i32, String> {
    let vm = VM_HANDLE
        .get()
        .copied()
        .ok_or("Rust JavaVM has not been created")?;
    if function.is_null() {
        return Err("JNI_OnLoad pointer is null".into());
    }
    type OnLoad = unsafe extern "system" fn(*mut jni::JavaVM, *mut c_void) -> jni::jint;
    // SAFETY: caller guarantees this is the engine's JNI_OnLoad export.
    let on_load: OnLoad = unsafe { std::mem::transmute(function) };
    // SAFETY: handle was allocated as a live JavaVM table pointer above.
    Ok(unsafe { on_load(vm as *mut jni::JavaVM, ptr::null_mut()) })
}

fn thread_env(vm: &Vm) -> *mut jni::JNIEnv {
    THREAD_ENV.with(|slot| {
        let existing = slot.get();
        if existing != 0 {
            return existing as *mut jni::JNIEnv;
        }
        vm.attach_current_thread();
        let handle = Box::new(env_table() as jni::JNIEnv);
        let handle = Box::into_raw(handle);
        slot.set(handle as usize);
        handle
    })
}

unsafe extern "system" fn get_version(_env: *mut jni::JNIEnv) -> jni::jint {
    jni::JNI_VERSION_1_6
}

unsafe extern "system" fn find_class(_env: *mut jni::JNIEnv, name: *const c_char) -> jni::jclass {
    let (Some(vm), Some(name)) = (vm(), unsafe { c_string(name) }) else {
        return ptr::null_mut();
    };
    match vm.find_or_define_class(&name) {
        Ok(class) => class.0 as usize as jni::jclass,
        Err(error) => {
            eprintln!("[jnivm] FindClass({name}) failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn get_object_class(
    _env: *mut jni::JNIEnv,
    object: jni::jobject,
) -> jni::jclass {
    let Some(vm) = vm() else {
        return ptr::null_mut();
    };
    if vm.class_name(ClassId(object as usize as u64)).is_some() {
        return match vm.find_or_define_class("java/lang/Class") {
            Ok(class) => class.0 as usize as jni::jclass,
            Err(error) => {
                eprintln!("[jnivm] GetObjectClass(java class) failed: {error}");
                ptr::null_mut()
            }
        };
    }
    let object_id = object as usize as u64;
    let Some(name) = vm.object_class_name(object_id) else {
        if object.is_null() {
            // libjnivm deliberately maps GetObjectClass(null) to Invalid.
            // Roblox's startup parameter bridge relies on this behavior for
            // absent parameter objects, so distinguish it from a foreign or
            // otherwise untracked non-null handle in diagnostics.
            eprintln!("[jnivm] GetObjectClass(null) -> Invalid (libjnivm compatibility)");
        } else {
            eprintln!(
                "[jnivm] GetObjectClass on untracked reference {object_id:#x}; using libjnivm-compatible Invalid class"
            );
        }
        return match vm.find_or_define_class("Invalid") {
            Ok(class) => class.0 as usize as jni::jclass,
            Err(error) => {
                eprintln!("[jnivm] GetObjectClass fallback failed: {error}");
                ptr::null_mut()
            }
        };
    };
    match vm.find_or_define_class(&name) {
        Ok(class) => class.0 as usize as jni::jclass,
        Err(error) => {
            eprintln!("[jnivm] GetObjectClass failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn get_method_id(
    _env: *mut jni::JNIEnv,
    class: jni::jclass,
    name: *const c_char,
    signature: *const c_char,
) -> jni::jmethodID {
    // SAFETY: the JNI caller guarantees both names point to NUL-terminated
    // strings for this call.
    unsafe { resolve_method(class, name, signature, false) }
}

unsafe extern "system" fn get_static_method_id(
    _env: *mut jni::JNIEnv,
    class: jni::jclass,
    name: *const c_char,
    signature: *const c_char,
) -> jni::jmethodID {
    // SAFETY: the JNI caller guarantees both names point to NUL-terminated
    // strings for this call.
    unsafe { resolve_method(class, name, signature, true) }
}

unsafe fn resolve_method(
    class: jni::jclass,
    name: *const c_char,
    signature: *const c_char,
    is_static: bool,
) -> jni::jmethodID {
    let (Some(vm), Some(name), Some(signature)) = (vm(), unsafe { c_string(name) }, unsafe {
        c_string(signature)
    }) else {
        return ptr::null_mut();
    };
    let class = if class.is_null() {
        match vm.find_or_define_class("Invalid") {
            Ok(class) => class,
            Err(error) => {
                eprintln!("[jnivm] method ID resolution class fallback failed: {error}");
                return ptr::null_mut();
            }
        }
    } else {
        ClassId(class as usize as u64)
    };
    if !is_static && name == "<init>" {
        let Some(close_paren) = signature.find(')') else {
            eprintln!("[jnivm] constructor has an invalid method descriptor: {signature}");
            return ptr::null_mut();
        };
        let Some(class_name) = vm.class_name(class) else {
            eprintln!("[jnivm] constructor lookup used an unknown class ID");
            return ptr::null_mut();
        };
        let factory_signature = format!("{}L{class_name};", &signature[..=close_paren]);
        return match vm.resolve_method(class, &name, &factory_signature, true) {
            Ok(method) => method.0 as usize as jni::jmethodID,
            Err(error) => {
                eprintln!("[jnivm] constructor factory lookup failed: {error}");
                ptr::null_mut()
            }
        };
    }
    match vm.resolve_method(class, &name, &signature, is_static) {
        Ok(method) => method.0 as usize as jni::jmethodID,
        Err(error) => {
            eprintln!("[jnivm] method ID resolution failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn get_field_id(
    _env: *mut jni::JNIEnv,
    class: jni::jclass,
    name: *const c_char,
    signature: *const c_char,
) -> jni::jfieldID {
    // SAFETY: the JNI caller guarantees both names point to NUL-terminated
    // strings for this call.
    unsafe { resolve_field(class, name, signature, false) }
}

unsafe extern "system" fn get_static_field_id(
    _env: *mut jni::JNIEnv,
    class: jni::jclass,
    name: *const c_char,
    signature: *const c_char,
) -> jni::jfieldID {
    // SAFETY: the JNI caller guarantees both names point to NUL-terminated
    // strings for this call.
    unsafe { resolve_field(class, name, signature, true) }
}

unsafe fn resolve_field(
    class: jni::jclass,
    name: *const c_char,
    signature: *const c_char,
    is_static: bool,
) -> jni::jfieldID {
    let (Some(vm), Some(name), Some(signature)) = (vm(), unsafe { c_string(name) }, unsafe {
        c_string(signature)
    }) else {
        return ptr::null_mut();
    };
    let class = if class.is_null() {
        match vm.find_or_define_class("Invalid") {
            Ok(class) => class,
            Err(error) => {
                eprintln!("[jnivm] field ID resolution class fallback failed: {error}");
                return ptr::null_mut();
            }
        }
    } else {
        ClassId(class as usize as u64)
    };
    match vm.resolve_field(class, &name, &signature, is_static) {
        Ok(field) => field.0 as usize as jni::jfieldID,
        Err(error) => {
            eprintln!("[jnivm] field ID resolution failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn register_natives(
    _env: *mut jni::JNIEnv,
    class: jni::jclass,
    methods: *const jni::JNINativeMethod,
    count: jni::jint,
) -> jni::jint {
    let Some(vm) = vm() else { return jni::JNI_ERR };
    if count < 0 || (count > 0 && methods.is_null()) {
        eprintln!("[jnivm] RegisterNatives received an invalid method array");
        return jni::JNI_ERR;
    }
    let class = ClassId(class as usize as u64);
    // SAFETY: JNI specifies `methods` as an array of `count` entries.
    let methods = unsafe { std::slice::from_raw_parts(methods, count as usize) };
    for method in methods {
        let (Some(name), Some(signature)) = (unsafe { c_string(method.name) }, unsafe {
            c_string(method.signature)
        }) else {
            return jni::JNI_ERR;
        };
        if let Err(error) = vm.register_native(class, &name, &signature, method.fnPtr as usize) {
            eprintln!("[jnivm] RegisterNatives {name}{signature} failed: {error}");
            return jni::JNI_ERR;
        }
    }
    jni::JNI_OK
}

unsafe extern "system" fn new_local_ref(
    env: *mut jni::JNIEnv,
    object: jni::jobject,
) -> jni::jobject {
    if object.is_null() {
        return ptr::null_mut();
    }
    let Some(vm) = vm() else {
        return ptr::null_mut();
    };
    let Some(env) = env_token(vm, env) else {
        return ptr::null_mut();
    };
    if vm.class_name(ClassId(object as usize as u64)).is_some() {
        return object;
    }
    match vm.clone_local_ref(&env, object as usize as u64) {
        Ok(reference) => reference.0 as usize as jni::jobject,
        Err(error) => {
            eprintln!("[jnivm] NewLocalRef failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn delete_local_ref(env: *mut jni::JNIEnv, object: jni::jobject) {
    if object.is_null() {
        return;
    }
    let Some(vm) = vm() else { return };
    if vm.class_name(ClassId(object as usize as u64)).is_some() {
        return;
    }
    let Some(env) = env_token(vm, env) else {
        return;
    };
    if let Err(error) = vm.delete_local_ref(&env, crate::ObjectId(object as usize as u64)) {
        eprintln!("[jnivm] DeleteLocalRef failed: {error}");
    }
}

unsafe extern "system" fn new_global_ref(
    env: *mut jni::JNIEnv,
    object: jni::jobject,
) -> jni::jobject {
    if object.is_null() {
        return ptr::null_mut();
    }
    let Some(vm) = vm() else {
        return ptr::null_mut();
    };
    if vm.class_name(ClassId(object as usize as u64)).is_some() {
        return object;
    }
    let Some(env) = env_token(vm, env) else {
        return ptr::null_mut();
    };
    match vm.new_global_ref(&env, crate::ObjectId(object as usize as u64)) {
        Ok(reference) => reference.0 as usize as jni::jobject,
        Err(error) => {
            eprintln!("[jnivm] NewGlobalRef failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn delete_global_ref(_env: *mut jni::JNIEnv, object: jni::jobject) {
    if object.is_null() {
        return;
    }
    if let Some(vm) = vm() {
        if vm.class_name(ClassId(object as usize as u64)).is_some() {
            return;
        }
        if let Err(error) = vm.delete_global_ref(crate::ObjectId(object as usize as u64)) {
            eprintln!("[jnivm] DeleteGlobalRef failed: {error}");
        }
    }
}

unsafe extern "system" fn is_same_object(
    _env: *mut jni::JNIEnv,
    left: jni::jobject,
    right: jni::jobject,
) -> jni::jboolean {
    (left == right) as jni::jboolean
}

unsafe extern "system" fn exception_check(_env: *mut jni::JNIEnv) -> jni::jboolean {
    jni::JNI_FALSE
}

unsafe extern "system" fn exception_occurred(_env: *mut jni::JNIEnv) -> jni::jthrowable {
    ptr::null_mut()
}

unsafe extern "system" fn exception_describe(_env: *mut jni::JNIEnv) {
    eprintln!("[jnivm] ExceptionDescribe called with no pending exception");
}

unsafe extern "system" fn throw(_env: *mut jni::JNIEnv, _exception: jni::jthrowable) -> jni::jint {
    eprintln!("[jnivm] unimplemented JNI method: Throw");
    jni::JNI_ERR
}

unsafe extern "system" fn throw_new(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    _message: *const c_char,
) -> jni::jint {
    eprintln!("[jnivm] unimplemented JNI method: ThrowNew");
    jni::JNI_ERR
}

unsafe extern "system" fn exception_clear(_env: *mut jni::JNIEnv) {}

include!("abi/strings.rs");
include!("abi/fields.rs");

unsafe extern "system" fn get_java_vm(
    _env: *mut jni::JNIEnv,
    output: *mut *mut jni::JavaVM,
) -> jni::jint {
    let Some(vm) = VM_HANDLE.get().copied() else {
        return jni::JNI_ERR;
    };
    if output.is_null() {
        return jni::JNI_EINVAL;
    }
    // SAFETY: JNI specifies a writable output pointer on JNI_OK.
    unsafe { *output = vm as *mut jni::JavaVM };
    jni::JNI_OK
}

include!("abi/calls.rs");

fn env_token(vm: &Vm, env: *mut jni::JNIEnv) -> Option<crate::ThreadEnv> {
    let current = THREAD_ENV.with(Cell::get);
    if current == 0 || env as usize != current {
        return None;
    }
    vm.get_env()
}

unsafe fn c_string(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: JNI strings are NUL-terminated for the duration of the call.
    Some(
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned(),
    )
}

unsafe extern "system" fn destroy_vm(_vm: *mut jni::JavaVM) -> jni::jint {
    eprintln!("[jnivm] DestroyJavaVM is unsupported; keeping the process VM alive");
    jni::JNI_ERR
}

unsafe extern "system" fn attach_current_thread(
    _vm: *mut jni::JavaVM,
    output: *mut *mut c_void,
    _args: *mut c_void,
) -> jni::jint {
    let Some(env) = current_env() else {
        return jni::JNI_ERR;
    };
    if output.is_null() {
        return jni::JNI_EINVAL;
    }
    // SAFETY: JavaVM specifies a writable output pointer when Attach succeeds.
    unsafe { *output = env };
    jni::JNI_OK
}

unsafe extern "system" fn detach_current_thread(_vm: *mut jni::JavaVM) -> jni::jint {
    THREAD_ENV.with(|slot| {
        let raw = slot.replace(0);
        if raw == 0 {
            return jni::JNI_EDETACHED;
        }
        // SAFETY: this thread owns the boxed JNI table-pointer slot.
        unsafe { drop(Box::from_raw(raw as *mut jni::JNIEnv)) };
        if let Some(vm) = vm() {
            let _ = vm.detach_current_thread();
        }
        jni::JNI_OK
    })
}

unsafe extern "system" fn get_env(
    _vm: *mut jni::JavaVM,
    output: *mut *mut c_void,
    version: jni::jint,
) -> jni::jint {
    if output.is_null() {
        return jni::JNI_EINVAL;
    }
    if version != jni::JNI_VERSION_1_1
        && version != jni::JNI_VERSION_1_2
        && version != jni::JNI_VERSION_1_4
        && version != jni::JNI_VERSION_1_6
    {
        return jni::JNI_EVERSION;
    }
    let Some(env) = THREAD_ENV
        .with(Cell::get)
        .checked_sub(0)
        .filter(|value| *value != 0)
    else {
        return jni::JNI_EDETACHED;
    };
    // SAFETY: JavaVM specifies a writable output pointer on JNI_OK.
    unsafe { *output = env as *mut c_void };
    jni::JNI_OK
}
