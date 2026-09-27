//! JNI virtual machine API used by the Android compatibility runtime.
//!
//! The native libjnivm implementation currently shares a CMake build with the
//! linker. This crate is the independently named Rust dependency boundary;
//! separating the native build targets can follow without changing callers.

pub use roblox_linker::jni::*;
