//! JNI and Android Java compatibility bindings for the Roblox runtime.

#![allow(unsafe_code)]

#[path = "accessibility.rs"]
mod accessibility_impl;
#[path = "game_activity/mod.rs"]
mod game_activity_impl;
#[path = "jni.rs"]
mod jni_impl;

pub use accessibility_impl::accessibility;
pub use game_activity_impl::game_activity;
pub use jni_impl::jni;
