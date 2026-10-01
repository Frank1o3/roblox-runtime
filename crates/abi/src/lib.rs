//! Bionic and ABI compatibility functions used by the Android runtime.

#![allow(unsafe_code)]

macro_rules! eprintln {
    () => {{
        roblox_logging::emit(String::new());
    }};
    ($($arg:tt)*) => {{
        roblox_logging::emit(format!($($arg)*));
    }};
}

pub mod bionic;
pub mod stubs;
pub mod unimplemented;
