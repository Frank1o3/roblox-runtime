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
static INTERNAL_FRAME_READBACK_SUPPORTED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_internal_frame_readback_supported(enabled: bool) {
    INTERNAL_FRAME_READBACK_SUPPORTED.store(enabled, Ordering::Relaxed);
}

pub fn internal_frame_readback_supported() -> bool {
    INTERNAL_FRAME_READBACK_SUPPORTED.load(Ordering::Relaxed)
}

pub use loader::{LIBRARY_NAMES, available_for_surface, loader_symbol};
pub use platform::Surface;

/// Set the preferred present mode for this process (`auto`, `mailbox`,
/// `immediate`, `fifo`, `fifo-relaxed`, or `off`).
pub fn set_present_mode(mode: Option<&str>) {
    swapchain::set_present_mode(mode);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_frame_readback_support_defaults_to_disabled() {
        set_internal_frame_readback_supported(false);
        assert!(!internal_frame_readback_supported());

        set_internal_frame_readback_supported(true);
        assert!(internal_frame_readback_supported());

        set_internal_frame_readback_supported(false);
    }
}
