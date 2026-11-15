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
mod unresolved;
mod vm;

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

pub(crate) fn cpp_fallback_enabled() -> bool {
    false
}
