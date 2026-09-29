/// The JNI virtual machine Roblox's native code calls back into.
///
/// Roblox registers 518 natives statically, but the traffic that matters runs the
/// other way: native code reaching for Java classes it expects Android to
/// provide. libjnivm answers those calls and records what was asked for, which is
/// how the framework-API backlog stops being a guess.
pub mod jni {
    use std::ffi::{CString, c_char, c_int, c_void};

    unsafe extern "C" {
        fn roblox_jni_create_vm() -> *mut c_void;
        fn roblox_jni_init_compat_bridge() -> c_int;
        fn roblox_jni_env() -> *mut c_void;
        fn roblox_jni_dump_classes(path: *const c_char) -> c_int;
        fn roblox_jni_call_onload(f: *mut c_void, err: *mut c_char, err_len: usize) -> c_int;
    }

    /// Call Roblox's `JNI_OnLoad` with the process JavaVM.
    ///
    /// Any C++ exception is caught on the far side: letting one cross the FFI
    /// boundary gives a core dump and no explanation.
    ///
    /// # Safety
    ///
    /// `f` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `f` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn call_on_load(f: *mut c_void) -> Result<i32, String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `f` is libroblox's JNI_OnLoad export; `err` is a live buffer of
        // the length passed alongside it.
        let rc = unsafe { roblox_jni_call_onload(f, err.as_mut_ptr() as *mut c_char, err.len()) };
        match rc {
            -1 => Err("no JavaVM, or JNI_OnLoad not found".into()),
            -2 | -3 => {
                let end = err.iter().position(|&b| b == 0).unwrap_or(err.len());
                Err(String::from_utf8_lossy(&err[..end]).into_owned())
            }
            v => Ok(v),
        }
    }

    /// Create the process's `JavaVM`. Returns `None` if one already exists.
    pub fn create_vm() -> Option<*mut c_void> {
        // SAFETY: the VM is process-global and owned by the shim.
        let vm = unsafe { roblox_jni_create_vm() };
        (!vm.is_null()).then_some(vm)
    }

    /// Initialize the C++ Java compatibility classes used by runtime-owned
    /// direct calls into Roblox JNI exports. In experimental mode Roblox still
    /// receives the Rust JavaVM; this companion VM only backs existing C++
    /// bridge hooks until those hooks have Rust implementations.
    pub fn init_compat_bridge() -> Result<(), String> {
        // SAFETY: idempotent process-global initialization inside the native
        // shim; the returned status contains any initialization failure.
        match unsafe { roblox_jni_init_compat_bridge() } {
            0 => Ok(()),
            code => Err(format!(
                "C++ Java compatibility bridge initialization failed ({code})"
            )),
        }
    }

    /// The calling thread's `JNIEnv*`.
    pub fn env() -> Option<*mut c_void> {
        // SAFETY: returns null when no VM exists, which is checked.
        let env = unsafe { roblox_jni_env() };
        (!env.is_null()).then_some(env)
    }

    /// Write C++ stubs for every Java class and method the native code reached
    /// for. This is the observed Phase 2 backlog.
    pub fn dump_classes(path: &str) -> Result<(), String> {
        let c = CString::new(path).map_err(|e| e.to_string())?;
        // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
        match unsafe { roblox_jni_dump_classes(c.as_ptr()) } {
            0 => Ok(()),
            -1 => Err("no JavaVM has been created".into()),
            -2 => Err("libjnivm was built without JNI_DEBUG".into()),
            n => Err(format!("class dump failed ({n})")),
        }
    }
}
