use ash::vk::{self, Handle};
use std::sync::atomic::{AtomicI32, Ordering};

const MODE_UNCAPPED: i32 = -2;
const MODE_ENGINE: i32 = -3;
const MODE_AUTO: i32 = -4;
static PRESENT_MODE: AtomicI32 = AtomicI32::new(MODE_AUTO);

pub(super) fn set_present_mode(mode: Option<&str>, vsync: bool) {
    let selected = if !vsync {
        vk::PresentModeKHR::IMMEDIATE.as_raw()
    } else {
        match mode.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            None | Some("") | Some("auto") => MODE_AUTO,
            Some("uncapped") => MODE_UNCAPPED,
            Some("off") => vk::PresentModeKHR::IMMEDIATE.as_raw(),
            Some("engine") => MODE_ENGINE,
            Some("mailbox") => vk::PresentModeKHR::MAILBOX.as_raw(),
            Some("immediate") => vk::PresentModeKHR::IMMEDIATE.as_raw(),
            Some("fifo") => vk::PresentModeKHR::FIFO.as_raw(),
            Some("fifo-relaxed") | Some("fifo_relaxed") => {
                vk::PresentModeKHR::FIFO_RELAXED.as_raw()
            }
            Some(_) => MODE_AUTO,
        }
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

fn surface_usage(surface: vk::SurfaceKHR) -> Option<vk::ImageUsageFlags> {
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
    // SAFETY: this function pointer has Vulkan's surface-capabilities ABI.
    let function: vk::PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR =
        unsafe { std::mem::transmute(function) };
    let mut capabilities = vk::SurfaceCapabilitiesKHR::default();
    // SAFETY: output is a valid writable Vulkan capabilities struct.
    if unsafe { function(physical, surface, &mut capabilities) } != vk::Result::SUCCESS {
        return None;
    }
    Some(capabilities.supported_usage_flags)
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
    let mut rewritten = *info;
    let usage_supported = surface_usage(info.surface)
        .is_some_and(|usage| usage.contains(vk::ImageUsageFlags::TRANSFER_SRC));
    if usage_supported {
        rewritten.image_usage |= vk::ImageUsageFlags::TRANSFER_SRC;
    }

    let setting = PRESENT_MODE.load(Ordering::Relaxed);
    let present_rewrite = (setting != MODE_ENGINE)
        .then(|| supported_modes(info.surface))
        .flatten()
        .and_then(|modes| {
            let preferences = if setting == MODE_AUTO {
                vec![vk::PresentModeKHR::MAILBOX]
            } else if setting == MODE_UNCAPPED {
                vec![vk::PresentModeKHR::IMMEDIATE]
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
                    (chosen, image_count)
                })
        });
    if let Some((present_mode, image_count)) = present_rewrite {
        rewritten.present_mode = present_mode;
        rewritten.min_image_count = image_count;
    }
    // SAFETY: `rewritten` preserves the caller's struct and only adjusts
    // supported image usage and optional presentation settings. Chained
    // pointers remain valid for this synchronous Vulkan call.
    let result = unsafe { function(device, &rewritten, allocator, output) };
    if result == vk::Result::SUCCESS && !output.is_null() {
        // SAFETY: successful vkCreateSwapchainKHR wrote the created handle.
        super::capture::register_swapchain(
            device,
            unsafe { *output },
            info.image_format,
            info.image_extent,
            usage_supported,
        );
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
