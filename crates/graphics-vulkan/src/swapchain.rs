use ash::vk::{self, Handle};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

const MODE_OFF: i32 = -1;
const MODE_UNCAPPED: i32 = -2;
const MODE_AUTO: i32 = 1;
static PRESENT_MODE: AtomicI32 = AtomicI32::new(MODE_AUTO);

#[allow(dead_code)]
#[derive(Clone, Copy, Default)]
struct SwapchainMetadata {
    min_image_count: u32,
    format: vk::Format,
    extent: vk::Extent2D,
    last_present_index: u32,
    transfer_source_eligibility: super::TransferSourceEligibility,
}

static SWAPCHAIN_METADATA: OnceLock<Mutex<HashMap<u64, SwapchainMetadata>>> = OnceLock::new();
static LAST_FRAME_TAP_STATE: OnceLock<Mutex<Option<super::FrameTapState>>> = OnceLock::new();

fn swapchain_metadata() -> &'static Mutex<HashMap<u64, SwapchainMetadata>> {
    SWAPCHAIN_METADATA.get_or_init(|| Mutex::new(HashMap::new()))
}

fn frame_tap_state_store() -> &'static Mutex<Option<super::FrameTapState>> {
    LAST_FRAME_TAP_STATE.get_or_init(|| Mutex::new(None))
}

pub(super) fn frame_tap_state() -> Option<super::FrameTapState> {
    *frame_tap_state_store()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn record_frame_tap_state(
    metadata: &SwapchainMetadata,
    swapchain: u64,
    present_index: u32,
) -> super::FrameTapState {
    super::FrameTapState {
        swapchain,
        transfer_source_eligibility: metadata.transfer_source_eligibility,
        last_present_index: present_index,
        width: metadata.extent.width,
        height: metadata.extent.height,
        format: metadata.format.as_raw(),
    }
}

fn device_supports_transfer_source(physical: vk::PhysicalDevice, format: vk::Format) -> bool {
    let instance =
        vk::Instance::from_raw(super::dispatch::INSTANCE.load(Ordering::Relaxed) as u64 as _);
    if instance.is_null() || physical.is_null() {
        return false;
    }
    let function = super::loader::host_proc(instance, c"vkGetPhysicalDeviceFormatProperties");
    let Some(function) = function else {
        return false;
    };
    let function: vk::PFN_vkGetPhysicalDeviceFormatProperties =
        unsafe { std::mem::transmute(function) };
    let mut properties = vk::FormatProperties::default();
    unsafe { function(physical, format, &mut properties) };
    properties
        .optimal_tiling_features
        .contains(vk::FormatFeatureFlags::TRANSFER_SRC)
}

fn surface_supports_transfer_source(
    physical: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
) -> Option<bool> {
    let instance =
        vk::Instance::from_raw(super::dispatch::INSTANCE.load(Ordering::Relaxed) as u64 as _);
    if instance.is_null() || physical.is_null() {
        return None;
    }
    let function =
        super::loader::host_proc(instance, c"vkGetPhysicalDeviceSurfaceCapabilitiesKHR")?;
    let function: vk::PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR =
        unsafe { std::mem::transmute(function) };
    let mut capabilities = vk::SurfaceCapabilitiesKHR::default();
    let result = unsafe { function(physical, surface, &mut capabilities) };
    (result == vk::Result::SUCCESS).then(|| {
        capabilities
            .supported_usage_flags
            .contains(vk::ImageUsageFlags::TRANSFER_SRC)
    })
}

pub(super) fn transfer_source_eligibility(
    format: vk::Format,
    format_transfer_source_supported: bool,
    surface_transfer_source_supported: Option<bool>,
    image_usage: vk::ImageUsageFlags,
    extent: vk::Extent2D,
) -> super::TransferSourceEligibility {
    use super::{
        TransferSourceEligibility as Eligibility, TransferSourceIneligibilityReason as Reason,
    };

    if !matches!(
        format,
        vk::Format::B8G8R8A8_SRGB | vk::Format::B8G8R8A8_UNORM
    ) {
        return Eligibility::Ineligible(Reason::UnsupportedFormat);
    }
    if !format_transfer_source_supported {
        return Eligibility::Ineligible(Reason::FormatTransferSourceUnsupported);
    }
    match surface_transfer_source_supported {
        None => return Eligibility::Ineligible(Reason::SurfaceCapabilitiesUnavailable),
        Some(false) => return Eligibility::Ineligible(Reason::SurfaceTransferSourceUnsupported),
        Some(true) => {}
    }
    if !image_usage.contains(vk::ImageUsageFlags::TRANSFER_SRC) {
        return Eligibility::Ineligible(Reason::SwapchainTransferSourceUsageMissing);
    }
    if extent.width == 0 || extent.height == 0 {
        return Eligibility::Ineligible(Reason::ZeroExtent);
    }
    Eligibility::Eligible
}

fn evaluate_transfer_source_eligibility(
    physical: vk::PhysicalDevice,
    info: &vk::SwapchainCreateInfoKHR<'_>,
) -> super::TransferSourceEligibility {
    let format_transfer_source_supported =
        !physical.is_null() && device_supports_transfer_source(physical, info.image_format);
    let surface_transfer_source_supported =
        surface_supports_transfer_source(physical, info.surface);
    transfer_source_eligibility(
        info.image_format,
        format_transfer_source_supported,
        surface_transfer_source_supported,
        info.image_usage,
        info.image_extent,
    )
}

fn is_accepted_present_result(result: vk::Result) -> bool {
    matches!(result, vk::Result::SUCCESS | vk::Result::SUBOPTIMAL_KHR)
}

fn clear_snapshot_for(snapshot: &mut Option<super::FrameTapState>, swapchain: u64) {
    if snapshot.is_some_and(|state| state.swapchain == swapchain) {
        *snapshot = None;
    }
}

fn insert_swapchain_metadata(
    metadata_by_handle: &mut HashMap<u64, SwapchainMetadata>,
    handle: u64,
    metadata: SwapchainMetadata,
) {
    metadata_by_handle.insert(handle, metadata);
}

fn remove_swapchain_metadata(
    metadata_by_handle: &mut HashMap<u64, SwapchainMetadata>,
    handle: u64,
) {
    metadata_by_handle.remove(&handle);
}

pub(super) fn set_present_mode(mode: Option<&str>) {
    let selected = match mode.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("") | Some("auto") | Some("mailbox") => MODE_AUTO,
        Some("uncapped") => MODE_UNCAPPED,
        Some("off") | Some("engine") => MODE_OFF,
        Some("immediate") => vk::PresentModeKHR::IMMEDIATE.as_raw(),
        Some("fifo") => vk::PresentModeKHR::FIFO.as_raw(),
        Some("fifo-relaxed") | Some("fifo_relaxed") => vk::PresentModeKHR::FIFO_RELAXED.as_raw(),
        Some(_) => MODE_AUTO,
    };
    PRESENT_MODE.store(selected, Ordering::Relaxed);
}

fn supported_modes(surface: vk::SurfaceKHR) -> Option<Vec<vk::PresentModeKHR>> {
    let instance =
        vk::Instance::from_raw(super::dispatch::INSTANCE.load(Ordering::Relaxed) as u64 as _);
    let physical = vk::PhysicalDevice::from_raw(
        super::dispatch::PHYSICAL_DEVICE.load(Ordering::Relaxed) as u64 as _,
    );
    if instance.is_null() || physical.is_null() {
        return None;
    }
    let function =
        super::loader::host_proc(instance, c"vkGetPhysicalDeviceSurfacePresentModesKHR")?;
    // SAFETY: this pointer was returned for vkGetPhysicalDeviceSurfacePresentModesKHR.
    let function: vk::PFN_vkGetPhysicalDeviceSurfacePresentModesKHR =
        unsafe { std::mem::transmute(function) };
    let mut count = 0;
    // SAFETY: the two-call Vulkan enumeration idiom supplies a valid count pointer.
    let result = unsafe { function(physical, surface, &mut count, std::ptr::null_mut()) };
    if result != vk::Result::SUCCESS {
        return None;
    }
    let mut modes = vec![vk::PresentModeKHR::IMMEDIATE; count as usize];
    if count > 0 {
        // SAFETY: `modes` has `count` entries as reported by the driver above.
        let result = unsafe { function(physical, surface, &mut count, modes.as_mut_ptr()) };
        if result != vk::Result::SUCCESS {
            return None;
        }
        modes.truncate(count as usize);
    }
    Some(modes)
}

fn image_count_limits(surface: vk::SurfaceKHR) -> Option<(u32, u32)> {
    let instance =
        vk::Instance::from_raw(super::dispatch::INSTANCE.load(Ordering::Relaxed) as u64 as _);
    let physical = vk::PhysicalDevice::from_raw(
        super::dispatch::PHYSICAL_DEVICE.load(Ordering::Relaxed) as u64 as _,
    );
    if instance.is_null() || physical.is_null() {
        return None;
    }
    let function =
        super::loader::host_proc(instance, c"vkGetPhysicalDeviceSurfaceCapabilitiesKHR")?;
    // SAFETY: this pointer has the Vulkan surface-capabilities signature.
    let function: vk::PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR =
        unsafe { std::mem::transmute(function) };
    let mut capabilities = vk::SurfaceCapabilitiesKHR::default();
    // SAFETY: output is a valid writable Vulkan capabilities struct.
    if unsafe { function(physical, surface, &mut capabilities) } != vk::Result::SUCCESS {
        return None;
    }
    Some((capabilities.min_image_count, capabilities.max_image_count))
}

pub(crate) extern "system" fn create_swapchain(
    device: vk::Device,
    create_info: *const vk::SwapchainCreateInfoKHR<'_>,
    allocator: *const vk::AllocationCallbacks<'_>,
    output: *mut vk::SwapchainKHR,
) -> vk::Result {
    let address = super::dispatch::HOST_CREATE_SWAPCHAIN.load(Ordering::Relaxed);
    if address == 0 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: this is the host loader's vkCreateSwapchainKHR obtained through GIPA/GDPA.
    let function: vk::PFN_vkCreateSwapchainKHR = unsafe { std::mem::transmute(address) };
    // Null and otherwise invalid inputs are forwarded for the driver to report.
    // SAFETY: non-null create-info points to the caller's Vulkan struct.
    let Some(info) = (unsafe { create_info.as_ref() }) else {
        // SAFETY: forwarding all caller arguments unchanged preserves Vulkan's validation.
        return unsafe { function(device, create_info, allocator, output) };
    };
    let setting = PRESENT_MODE.load(Ordering::Relaxed);
    let rewritten = if setting == MODE_OFF {
        None
    } else {
        supported_modes(info.surface).and_then(|modes| {
            let preferences = if setting == MODE_AUTO {
                vec![vk::PresentModeKHR::MAILBOX]
            } else if setting == MODE_UNCAPPED {
                vec![vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::IMMEDIATE]
            } else {
                vec![vk::PresentModeKHR::from_raw(setting)]
            };
            preferences
                .into_iter()
                .find(|mode| modes.contains(mode) && *mode != info.present_mode)
                .map(|chosen| {
                    let mut image_count = info.min_image_count;
                    if matches!(
                        chosen,
                        vk::PresentModeKHR::MAILBOX | vk::PresentModeKHR::IMMEDIATE
                    ) && image_count < 3
                    {
                        if let Some((_, max)) = image_count_limits(info.surface) {
                            if max == 0 || max >= 3 {
                                image_count = 3;
                            }
                        }
                    }
                    vk::SwapchainCreateInfoKHR {
                        present_mode: chosen,
                        min_image_count: image_count,
                        ..*info
                    }
                })
        })
    };
    let effective_info = rewritten.as_ref().unwrap_or(info);
    // SAFETY: `effective_info` is either the caller's structure or a shallow
    // copy with only the present mode/image count adjusted; chained pointers
    // remain valid for this synchronous Vulkan call.
    let result = unsafe { function(device, effective_info, allocator, output) };
    if result == vk::Result::SUCCESS && !output.is_null() {
        let created = unsafe { *output };
        if !created.is_null() {
            let handle = created.as_raw() as u64;
            let physical = vk::PhysicalDevice::from_raw(
                super::dispatch::PHYSICAL_DEVICE.load(Ordering::Relaxed) as u64 as _,
            );
            let metadata = SwapchainMetadata {
                min_image_count: effective_info.min_image_count,
                format: effective_info.image_format,
                extent: effective_info.image_extent,
                last_present_index: 0,
                transfer_source_eligibility: evaluate_transfer_source_eligibility(
                    physical,
                    effective_info,
                ),
            };
            {
                let mut metadata_by_handle = swapchain_metadata()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                insert_swapchain_metadata(&mut metadata_by_handle, handle, metadata);
            }
            let old_handle = info.old_swapchain.as_raw() as u64;
            let mut snapshot = frame_tap_state_store()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            clear_snapshot_for(&mut snapshot, handle);
            if old_handle != 0 {
                clear_snapshot_for(&mut snapshot, old_handle);
            }
        }
    }
    result
}

pub(crate) extern "system" fn get_surface_capabilities(
    physical: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
    output: *mut vk::SurfaceCapabilitiesKHR,
) -> vk::Result {
    let address = super::dispatch::HOST_GET_SURFACE_CAPABILITIES.load(Ordering::Relaxed);
    if address == 0 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: this pointer was resolved from the host Vulkan loader.
    let function: vk::PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR =
        unsafe { std::mem::transmute(address) };
    // SAFETY: forward caller arguments and preserve the driver's result.
    let result = unsafe { function(physical, surface, output) };
    if result == vk::Result::SUCCESS && !output.is_null() {
        if let Some(super::platform::Surface::Wayland { width, height, .. }) =
            super::platform::current_surface()
        {
            // SAFETY: successful Vulkan call filled the writable capabilities struct.
            let caps = unsafe { &mut *output };
            if caps.current_extent.width == u32::MAX {
                caps.current_extent = vk::Extent2D { width, height };
            }
        }
    }
    result
}

#[allow(dead_code)]
pub(crate) extern "system" fn queue_present(
    queue: vk::Queue,
    info: *const vk::PresentInfoKHR<'_>,
) -> vk::Result {
    let address = super::dispatch::HOST_QUEUE_PRESENT.load(Ordering::Relaxed);
    if address == 0 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    type Function =
        unsafe extern "system" fn(vk::Queue, *const vk::PresentInfoKHR<'_>) -> vk::Result;
    let function: Function = unsafe { std::mem::transmute(address) };
    let result = unsafe { function(queue, info) };
    if !info.is_null() {
        let info = unsafe { &*info };
        if info.swapchain_count > 0
            && !info.p_swapchains.is_null()
            && !info.p_image_indices.is_null()
        {
            let mut last_snapshot = None;
            {
                let mut metadata_by_handle = swapchain_metadata()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                for index in 0..info.swapchain_count as usize {
                    let present_result = if info.p_results.is_null() {
                        result
                    } else {
                        unsafe { *info.p_results.add(index) }
                    };
                    if !is_accepted_present_result(present_result) {
                        continue;
                    }
                    let swapchain = unsafe { *info.p_swapchains.add(index) };
                    let image_index = unsafe { *info.p_image_indices.add(index) };
                    if let Some(metadata) = metadata_by_handle.get_mut(&(swapchain.as_raw() as u64))
                    {
                        metadata.last_present_index = image_index;
                        last_snapshot = Some(record_frame_tap_state(
                            metadata,
                            swapchain.as_raw() as u64,
                            image_index,
                        ));
                    }
                }
            }
            if let Some(snapshot) = last_snapshot {
                *frame_tap_state_store()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(snapshot);
            }
        }
    }
    result
}

pub(crate) extern "system" fn destroy_swapchain(
    device: vk::Device,
    swapchain: vk::SwapchainKHR,
    allocator: *const vk::AllocationCallbacks<'_>,
) {
    let address = super::dispatch::HOST_DESTROY_SWAPCHAIN.load(Ordering::Relaxed);
    if address != 0 {
        let function: vk::PFN_vkDestroySwapchainKHR = unsafe { std::mem::transmute(address) };
        // SAFETY: the host function pointer was resolved as vkDestroySwapchainKHR.
        unsafe { function(device, swapchain, allocator) };
    }
    let handle = swapchain.as_raw() as u64;
    {
        let mut metadata_by_handle = swapchain_metadata()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        remove_swapchain_metadata(&mut metadata_by_handle, handle);
    }
    let mut snapshot = frame_tap_state_store()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    clear_snapshot_for(&mut snapshot, handle);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        TransferSourceEligibility as Eligibility, TransferSourceIneligibilityReason as Reason,
    };

    fn eligible(usage: vk::ImageUsageFlags, extent: vk::Extent2D) -> Eligibility {
        transfer_source_eligibility(vk::Format::B8G8R8A8_UNORM, true, Some(true), usage, extent)
    }

    #[test]
    fn eligibility_requires_every_transfer_source_prerequisite() {
        let extent = vk::Extent2D {
            width: 640,
            height: 480,
        };
        assert_eq!(
            eligible(vk::ImageUsageFlags::TRANSFER_SRC, extent),
            Eligibility::Eligible
        );
        assert_eq!(
            transfer_source_eligibility(
                vk::Format::R8G8B8A8_UNORM,
                true,
                Some(true),
                vk::ImageUsageFlags::TRANSFER_SRC,
                extent,
            ),
            Eligibility::Ineligible(Reason::UnsupportedFormat)
        );
        assert_eq!(
            transfer_source_eligibility(
                vk::Format::B8G8R8A8_UNORM,
                false,
                Some(true),
                vk::ImageUsageFlags::TRANSFER_SRC,
                extent,
            ),
            Eligibility::Ineligible(Reason::FormatTransferSourceUnsupported)
        );
        assert_eq!(
            transfer_source_eligibility(
                vk::Format::B8G8R8A8_UNORM,
                true,
                None,
                vk::ImageUsageFlags::TRANSFER_SRC,
                extent,
            ),
            Eligibility::Ineligible(Reason::SurfaceCapabilitiesUnavailable)
        );
        assert_eq!(
            transfer_source_eligibility(
                vk::Format::B8G8R8A8_UNORM,
                true,
                Some(false),
                vk::ImageUsageFlags::TRANSFER_SRC,
                extent,
            ),
            Eligibility::Ineligible(Reason::SurfaceTransferSourceUnsupported)
        );
        assert_eq!(
            eligible(vk::ImageUsageFlags::COLOR_ATTACHMENT, extent),
            Eligibility::Ineligible(Reason::SwapchainTransferSourceUsageMissing)
        );
        assert_eq!(
            eligible(
                vk::ImageUsageFlags::TRANSFER_SRC,
                vk::Extent2D {
                    width: 0,
                    height: 480
                },
            ),
            Eligibility::Ineligible(Reason::ZeroExtent)
        );
    }

    #[test]
    fn metadata_lifecycle_clears_destroyed_and_replaced_snapshots() {
        let mut metadata_by_handle = HashMap::new();
        let metadata = SwapchainMetadata {
            transfer_source_eligibility: Eligibility::Eligible,
            ..SwapchainMetadata::default()
        };
        insert_swapchain_metadata(&mut metadata_by_handle, 11, metadata);
        assert!(metadata_by_handle.contains_key(&11));

        let mut snapshot = Some(super::super::FrameTapState {
            swapchain: 11,
            ..super::super::FrameTapState::default()
        });
        clear_snapshot_for(&mut snapshot, 11);
        assert!(snapshot.is_none());

        insert_swapchain_metadata(&mut metadata_by_handle, 12, metadata);
        remove_swapchain_metadata(&mut metadata_by_handle, 12);
        assert!(!metadata_by_handle.contains_key(&12));
    }

    #[test]
    fn successful_and_suboptimal_presents_are_accepted() {
        assert!(is_accepted_present_result(vk::Result::SUCCESS));
        assert!(is_accepted_present_result(vk::Result::SUBOPTIMAL_KHR));
        assert!(!is_accepted_present_result(
            vk::Result::ERROR_OUT_OF_DATE_KHR
        ));
    }
}
