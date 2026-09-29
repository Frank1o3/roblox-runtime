//! A small, pure Rust implementation of the JNI VM surface needed by the
//! Roblox runtime. The implementation is being built against the observed
//! surface in [`OBSERVED_SURFACE.md`](../OBSERVED_SURFACE.md).

mod abi;
mod descriptor;
mod vm;

use std::sync::atomic::{AtomicBool, Ordering};

static CPP_FALLBACK_ENABLED: AtomicBool = AtomicBool::new(true);

pub use abi::{
    call_on_load, create_vm, current_env, new_configuration, new_opaque_object,
    new_string_array_ref, new_string_ref, set_fmod_aaudio_support, set_game_loaded_callback,
    set_startup_bootstrap, vm_exists,
};
pub use descriptor::{
    DescriptorError, MethodDescriptor, Type, parse_field_descriptor, parse_method_descriptor,
};
pub use vm::{
    ClassId, FieldId, JniError, JniValue, MethodHandler, MethodId, ObjectId, ObjectValue,
    ThreadEnv, Vm,
};

/// Whether the process explicitly selected the experimental pure Rust VM.
/// This is opt-in because the backend is still under development.
pub fn selected_from_environment() -> bool {
    std::env::var("USE_EXPERIMENTAL_JNIVM")
        .is_ok_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1"))
}

/// Enable or disable unresolved method and field dispatch through the
/// companion C++ compatibility VM.
pub fn set_cpp_fallback_enabled(enabled: bool) {
    CPP_FALLBACK_ENABLED.store(enabled, Ordering::Relaxed);
}

pub(crate) fn cpp_fallback_enabled() -> bool {
    CPP_FALLBACK_ENABLED.load(Ordering::Relaxed)
}
