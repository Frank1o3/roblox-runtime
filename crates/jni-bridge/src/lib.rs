//! JNI and Android Java compatibility bindings for the Roblox runtime.

#![allow(unsafe_code)]

#[path = "accessibility.rs"]
pub mod accessibility;
pub mod battery;
#[path = "game_activity/mod.rs"]
mod game_activity_impl;
pub mod hooks;
#[path = "jni.rs"]
mod jni_impl;

pub use game_activity_impl::game_activity;
pub use jni_impl::jni;

/// Register Rust-owned compatibility hooks after the Rust VM is created and
/// before Roblox's `JNI_OnLoad` can call them.
pub fn register_rust_hooks() -> Result<(), String> {
    let vm = jnivm::process_vm().ok_or("Rust JavaVM has not been created")?;
    hooks::register(vm)?;
    battery::register_hooks(vm)?;
    accessibility::register_hooks()
}
