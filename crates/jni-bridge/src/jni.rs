/// JNI entry points for the Rust-owned JavaVM and its temporary native bridge.
/// Rust hooks answer the migrated surface; native Android/GameActivity drivers
/// still use the compatibility VM while their hook implementations are ported.
pub mod jni {
    use std::ffi::{CString, c_int, c_void};

    unsafe extern "C" {
        fn roblox_jni_init_compat_bridge() -> c_int;
        fn roblox_jni_dump_classes(path: *const std::ffi::c_char) -> c_int;
    }

    /// Call Roblox's `JNI_OnLoad` with the Rust-owned JavaVM.
    ///
    /// # Safety
    /// `f` must point to the live `JNI_OnLoad` export of the mapped Roblox
    /// library.
    pub unsafe fn call_on_load(f: *mut c_void) -> Result<i32, String> {
        // SAFETY: the caller guarantees this is the mapped library's live
        // JNI_OnLoad export; jnivm contains panics at its JNI boundaries.
        unsafe { jnivm::call_on_load(f) }
    }

    /// Create the process JavaVM. Returns `None` when one already exists.
    pub fn create_vm() -> Option<*mut c_void> {
        jnivm::create_vm().ok()
    }

    /// Initialize the temporary C++ compatibility hooks.
    ///
    /// This remains until their observed callers have Rust implementations.
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

    /// The calling thread's Rust `JNIEnv*`.
    pub fn env() -> Option<*mut c_void> {
        jnivm::current_env()
    }

    /// Write the observed class/method inventory recorded by the compatibility
    /// VM. This diagnostic still uses the temporary C++ shim.
    pub fn dump_classes(path: &str) -> Result<(), String> {
        let c = CString::new(path).map_err(|e| e.to_string())?;
        // SAFETY: `c` is a valid NUL-terminated path for the duration of call.
        match unsafe { roblox_jni_dump_classes(c.as_ptr()) } {
            0 => Ok(()),
            -1 => Err("no JavaVM has been created".into()),
            -2 => Err("native compatibility diagnostics were built without JNI_DEBUG".into()),
            n => Err(format!("class dump failed ({n})")),
        }
    }
}
