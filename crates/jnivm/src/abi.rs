//! JNI C ABI tables and process/thread handles.
//!
//! Only ABI entries with defined behavior are installed. Calls routed through
//! the implemented member dispatch log unresolved Java methods and return the
//! descriptor's zero/null default; the rest of the table remains to be filled
//! from runtime observations.

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

fn env_table() -> *const jni::JNINativeInterface_ {
    *ENV_TABLE.get_or_init(|| {
        // All-zero is a valid initial value for a C function table: null
        // function pointers mean an entry has not been implemented yet.
        let mut table: jni::JNINativeInterface_ = unsafe { std::mem::zeroed() };
        table.GetVersion = Some(get_version);
        table.FindClass = Some(find_class);
        table.GetObjectClass = Some(get_object_class);
        table.GetMethodID = Some(get_method_id);
        table.GetStaticMethodID = Some(get_static_method_id);
        table.GetFieldID = Some(get_field_id);
        table.GetStaticFieldID = Some(get_static_field_id);
        table.RegisterNatives = Some(register_natives);
        table.NewLocalRef = Some(new_local_ref);
        table.DeleteLocalRef = Some(delete_local_ref);
        table.NewGlobalRef = Some(new_global_ref);
        table.DeleteGlobalRef = Some(delete_global_ref);
        table.IsSameObject = Some(is_same_object);
        table.ExceptionCheck = Some(exception_check);
        table.ExceptionClear = Some(exception_clear);
        table.NewString = Some(new_string);
        table.NewStringUTF = Some(new_string_utf);
        table.GetStringLength = Some(get_string_length);
        table.GetStringUTFLength = Some(get_string_utf_length);
        table.GetStringChars = Some(get_string_chars);
        table.ReleaseStringChars = Some(release_string_chars);
        table.GetStringUTFChars = Some(get_string_utf_chars);
        table.ReleaseStringUTFChars = Some(release_string_utf_chars);
        table.GetJavaVM = Some(get_java_vm);
        table.CallObjectMethodV = Some(call_object_v);
        table.CallBooleanMethodV = Some(call_boolean_v);
        table.CallIntMethodV = Some(call_int_v);
        table.CallLongMethodV = Some(call_long_v);
        table.CallVoidMethodV = Some(call_void_v);
        table.CallStaticObjectMethodV = Some(call_static_object_v);
        table.CallStaticBooleanMethodV = Some(call_static_boolean_v);
        table.CallStaticIntMethodV = Some(call_static_int_v);
        table.CallStaticLongMethodV = Some(call_static_long_v);
        table.CallStaticVoidMethodV = Some(call_static_void_v);
        table.CallObjectMethodA = Some(call_object_a);
        table.CallBooleanMethodA = Some(call_boolean_a);
        table.CallIntMethodA = Some(call_int_a);
        table.CallLongMethodA = Some(call_long_a);
        table.CallVoidMethodA = Some(call_void_a);
        table.CallStaticObjectMethodA = Some(call_static_object_a);
        table.CallStaticBooleanMethodA = Some(call_static_boolean_a);
        table.CallStaticIntMethodA = Some(call_static_int_a);
        table.CallStaticLongMethodA = Some(call_static_long_a);
        table.CallStaticVoidMethodA = Some(call_static_void_a);
        Box::into_raw(Box::new(table)) as usize
    }) as *const jni::JNINativeInterface_
}

fn vm_table() -> *const jni::JNIInvokeInterface_ {
    *VM_TABLE.get_or_init(|| {
        let mut table: jni::JNIInvokeInterface_ = unsafe { std::mem::zeroed() };
        table.DestroyJavaVM = Some(destroy_vm);
        table.AttachCurrentThread = Some(attach_current_thread);
        table.AttachCurrentThreadAsDaemon = Some(attach_current_thread);
        table.DetachCurrentThread = Some(detach_current_thread);
        table.GetEnv = Some(get_env);
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
    let _ = env_table();
    let _ = vm_table();
    vm().expect("VM was just initialized")
        .attach_current_thread();
    let handle = Box::new(vm_table() as jni::JavaVM);
    let raw = Box::into_raw(handle) as usize;
    VM_HANDLE
        .set(raw)
        .map_err(|_| "a Rust JavaVM already exists")?;
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
    let Some(name) = vm.object_class_name(object as usize as u64) else {
        eprintln!("[jnivm] GetObjectClass on unknown object reference");
        return ptr::null_mut();
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
    let class = ClassId(class as usize as u64);
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
    match vm.resolve_field(ClassId(class as usize as u64), &name, &signature, is_static) {
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

unsafe extern "system" fn exception_clear(_env: *mut jni::JNIEnv) {}

include!("abi/strings.rs");

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
