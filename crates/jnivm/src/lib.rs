//! A small, pure Rust implementation of the JNI VM surface needed by the
//! Roblox runtime. The implementation is being built against the observed
//! surface in [`OBSERVED_SURFACE.md`](../OBSERVED_SURFACE.md).

macro_rules! eprintln {
    () => {{
        roblox_logging::emit(String::new());
    }};
    ($($arg:tt)*) => {{
        roblox_logging::emit(format!($($arg)*));
    }};
}

mod abi;
mod descriptor;
mod vm;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

static CPP_FALLBACK_ENABLED: AtomicBool = AtomicBool::new(true);
static SELECTED_FROM_ENV: OnceLock<bool> = OnceLock::new();

pub use abi::{
    call_on_load, class_ref, cpp_fallback_reference, cpp_fallback_same_object, create_vm,
    current_env, gc, new_configuration, new_opaque_object, new_string_array_ref, new_string_ref,
    register_method_handler, set_fmod_aaudio_support, set_game_loaded_callback,
    set_startup_bootstrap, string_object, vm_exists,
};
pub use descriptor::{
    DescriptorError, MethodDescriptor, Type, parse_field_descriptor, parse_method_descriptor,
};
pub use vm::{
    ClassId, FieldId, GcStats, JniError, JniValue, MethodHandler, MethodId, ObjectId, ObjectValue,
    ThreadEnv, Vm,
};

/// Whether the process explicitly selected the experimental pure Rust VM.
/// This is opt-in because the backend is still under development.
/// Result is cached after the first check to avoid redundant environment reads on hot paths.
pub fn selected_from_environment() -> bool {
    *SELECTED_FROM_ENV.get_or_init(|| {
        std::env::var("USE_EXPERIMENTAL_JNIVM")
            .is_ok_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1"))
    })
}

/// Explicitly configure the JNI backend selection, overriding the initial environment state.
pub fn set_selected_from_environment(selected: bool) {
    let _ = SELECTED_FROM_ENV.set(selected);
}

/// Enable or disable unresolved method and field dispatch through the
/// companion C++ compatibility VM.
pub fn set_cpp_fallback_enabled(enabled: bool) {
    CPP_FALLBACK_ENABLED.store(enabled, Ordering::Relaxed);
}

pub(crate) fn cpp_fallback_enabled() -> bool {
    CPP_FALLBACK_ENABLED.load(Ordering::Relaxed)
}
