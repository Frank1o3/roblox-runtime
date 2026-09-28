//! Android framework compatibility provided to the Roblox Android client.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

pub mod asset;
pub mod battery;
pub mod config;
pub mod local_storage;
pub mod native_window;
pub mod system;

pub use native_window::{
    HostSurface, SurfaceError, clear, current, install, is_installed, wayland_display,
};

static TRACE: AtomicBool = AtomicBool::new(false);

/// Enable diagnostic output for Android compatibility calls.
pub fn set_trace(enabled: bool) {
    TRACE.store(enabled, Ordering::Relaxed);
}

pub(crate) fn trace(args: std::fmt::Arguments<'_>) {
    if TRACE.load(Ordering::Relaxed) {
        eprintln!("[android] {args}");
    }
}

/// Native Android symbols implemented by this crate.
pub fn overrides() -> Vec<(&'static str, *mut c_void)> {
    local_storage::link_symbols();
    let mut symbols = asset::overrides();
    symbols.extend(config::overrides());
    symbols.extend(native_window::overrides());
    symbols
}
