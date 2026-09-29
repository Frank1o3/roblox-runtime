//! A small, pure Rust implementation of the JNI VM surface needed by the
//! Roblox runtime. The implementation is being built against the observed
//! surface in [`OBSERVED_SURFACE.md`](../OBSERVED_SURFACE.md).

mod abi;
mod descriptor;
mod vm;

pub use abi::{
    call_on_load, create_vm, current_env, new_configuration, new_opaque_object,
    new_string_array_ref, new_string_ref,
    set_fmod_aaudio_support, set_game_loaded_callback, set_startup_bootstrap, vm_exists,
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
