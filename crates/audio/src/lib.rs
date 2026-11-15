//! Rust-native audio support for the Android audio APIs exposed by the runtime.

pub mod aaudio {
    pub mod backend;
    pub mod builder;
    pub mod ffi;
    pub mod stream;
}

pub mod opensles;

pub mod backend;
pub mod input_callback;
pub mod logline;
pub mod types;
