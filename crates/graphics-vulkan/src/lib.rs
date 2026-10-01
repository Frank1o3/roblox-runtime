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

/// Whether this runtime can currently deliver captured swapchain pixels.
///
/// This remains false until an operational readback backend is implemented;
/// Vulkan format and swapchain eligibility probes do not make it true.
pub fn internal_frame_readback_supported() -> bool {
    INTERNAL_FRAME_READBACK_SUPPORTED.load(Ordering::Relaxed)
}

pub use loader::{LIBRARY_NAMES, available_for_surface, loader_symbol};
pub use platform::Surface;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferSourceIneligibilityReason {
    #[default]
    NotEvaluated,
    UnsupportedFormat,
    FormatTransferSourceUnsupported,
    SurfaceCapabilitiesUnavailable,
    SurfaceTransferSourceUnsupported,
    SwapchainTransferSourceUsageMissing,
    ZeroExtent,
}

/// Preliminary permission to copy from the swapchain image.
///
/// `Eligible` only means the Vulkan prerequisites are present; it does not
/// indicate that this runtime implements or performs pixel readback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferSourceEligibility {
    #[default]
    NotEvaluated,
    Eligible,
    Ineligible(TransferSourceIneligibilityReason),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameTapState {
    /// Raw swapchain handle whose last accepted presentation is described.
    pub swapchain: u64,
    /// Preliminary capability only; this is not operational pixel readback.
    pub transfer_source_eligibility: TransferSourceEligibility,
    pub last_present_index: u32,
    pub width: u32,
    pub height: u32,
    /// Raw Vulkan `VkFormat` value (`int32_t` in Vulkan).
    pub format: i32,
}

/// Return metadata for the last accepted present, if known.
///
/// This snapshot contains no pixels and must not be treated as a capture
/// backend or a signal that detector/aiming functionality is available.
pub fn frame_tap_state() -> Option<FrameTapState> {
    swapchain::frame_tap_state()
}

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
    use ash::vk;

    #[test]
    fn internal_frame_readback_is_unavailable_even_when_swapchain_is_eligible() {
        assert!(!internal_frame_readback_supported());
        let eligibility = swapchain::transfer_source_eligibility(
            vk::Format::B8G8R8A8_UNORM,
            true,
            Some(true),
            vk::ImageUsageFlags::TRANSFER_SRC,
            vk::Extent2D {
                width: 1280,
                height: 720,
            },
        );
        assert_eq!(eligibility, TransferSourceEligibility::Eligible);
        assert!(!internal_frame_readback_supported());
    }
}
