//! Host Vulkan loader bridge and Android-to-desktop WSI translation.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

mod dispatch;
mod instance;
mod loader;
mod platform;
mod surface;
mod swapchain;

static ENABLED: AtomicBool = AtomicBool::new(false);

pub use loader::{LIBRARY_NAMES, available_for_surface, loader_symbol};
pub use platform::Surface;

/// Enable or disable exposing the host Vulkan loader to Roblox.
///
/// The runtime selects the backend before loading the engine. Keeping Vulkan
/// disabled by default prevents a caller that has not prepared graphics from
/// accidentally advertising a partially configured Vulkan implementation.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Set the preferred present mode for this process (`auto`, `mailbox`,
/// `immediate`, `fifo`, `fifo-relaxed`, or `off`). Disabling VSync requests
/// immediate presentation. Unsupported modes are left to the engine's mode.
pub fn set_present_mode(mode: Option<&str>, vsync: bool) {
    swapchain::set_present_mode(mode, vsync);
}

pub fn set_surface(surface: Option<Surface>) {
    platform::set_surface(surface);
}

pub fn resize_surface(width: u32, height: u32) {
    platform::resize_surface(width, height);
}

/// Export table for a virtual `libvulkan.so` loaded by the guest bionic linker.
pub fn library_symbols() -> Vec<(String, *mut c_void)> {
    vec![(
        "vkGetInstanceProcAddr".to_owned(),
        loader_symbol().unwrap_or(std::ptr::null_mut()),
    )]
}
