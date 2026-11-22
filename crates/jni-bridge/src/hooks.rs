//! Rust implementations of the Android and Roblox Java hooks required by the
//! observed compatibility surface.

use jnivm::{JniValue, ObjectValue, ThreadEnv, Vm};
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};

fn attached_thread_env(vm: &Vm) -> ThreadEnv {
    vm.get_env().unwrap_or_else(|| vm.attach_current_thread())
}

include!("hooks/builtins.rs");
include!("hooks/flags.rs");
include!("hooks/platform.rs");
include!("hooks/audio.rs");
include!("hooks/native_helper.rs");

/// Register the runtime-owned Android and Roblox Java methods in the Rust VM.
pub fn register(vm: &Vm) -> Result<(), String> {
    install_builtin_methods(vm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_android_and_roblox_hooks_register_in_the_bridge() {
        let vm = Vm::new();
        let _env = vm.attach_current_thread();
        register(&vm).unwrap();

        let locale = vm
            .find_class("com/roblox/engine/jni/locale/NativeLocaleJavaInterface")
            .unwrap();
        let get_locale = vm
            .find_method(locale, "getLocale", "()Ljava/lang/String;", true)
            .unwrap();
        assert!(vm.method_info(get_locale).unwrap().4);

        let activity = vm.find_class("android/app/ActivityThread").unwrap();
        let current = vm
            .find_method(
                activity,
                "currentActivityThread",
                "()Landroid/app/ActivityThread;",
                true,
            )
            .unwrap();
        assert!(vm.method_info(current).unwrap().4);
    }
}
