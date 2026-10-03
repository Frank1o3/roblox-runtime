use ash::vk::{self, Handle};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

const MODE_UNCAPPED: i32 = -2;
const MODE_ENGINE: i32 = -3;
const MODE_AUTO: i32 = -4;
static PRESENT_MODE: AtomicI32 = AtomicI32::new(MODE_AUTO);
static LAST_PRESENT_REPORT: AtomicU64 = AtomicU64::new(u64::MAX);
static NVIDIA_RETRY_ANNOUNCED: AtomicBool = AtomicBool::new(false);

/// NVIDIA hybrid systems have been reported to fail the first surface-mode
/// query after boot. The retry is INFERRED to help: it is only a bounded second
/// ask, gated to NVIDIA and the reported transient errors. Other vendors retain
/// the driver's original single-call behaviour.
pub(crate) extern "system" fn get_surface_present_modes(
    physical: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
    count: *mut u32,
    modes: *mut vk::PresentModeKHR,
) -> vk::Result {
    let instance =
        vk::Instance::from_raw(super::dispatch::INSTANCE.load(Ordering::Relaxed) as u64 as _);
    let Some(address) =
        super::loader::host_proc(instance, c"vkGetPhysicalDeviceSurfacePresentModesKHR")
    else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    let function: vk::PFN_vkGetPhysicalDeviceSurfacePresentModesKHR =
        unsafe { std::mem::transmute(address) };

    if !is_nvidia_device(instance, physical) {
        // SAFETY: the host address was resolved for this Vulkan command and all
        // pointers are forwarded unchanged under Vulkan's caller contract.
        return unsafe { function(physical, surface, count, modes) };
    }

    let initial_count = if count.is_null() {
        0
    } else {
        // SAFETY: Vulkan requires a valid count pointer for this command.
        unsafe { *count }
    };
    // SAFETY: the host address was resolved for this Vulkan command; restoring
    // count before each attempt preserves the caller's original input capacity.
    let mut result = unsafe { function(physical, surface, count, modes) };
    let mut attempts = 1;
    for delay_ms in [100, 250, 500, 1000] {
        if !matches!(
            result,
            vk::Result::ERROR_UNKNOWN
                | vk::Result::ERROR_INITIALIZATION_FAILED
                | vk::Result::ERROR_SURFACE_LOST_KHR
        ) {
            break;
        }
        if !NVIDIA_RETRY_ANNOUNCED.swap(true, Ordering::Relaxed) {
            roblox_logging::emit(
                "[runtime] NVIDIA surface present-mode query failed; retrying transient driver errors"
                    .to_owned(),
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        if !count.is_null() {
            // SAFETY: same caller-owned count pointer used for the initial call.
            unsafe { *count = initial_count };
        }
        // SAFETY: same resolved host command and live caller pointers.
        result = unsafe { function(physical, surface, count, modes) };
        attempts += 1;
    }
    if attempts > 1 {
        let outcome = if result == vk::Result::SUCCESS {
            "recovered"
        } else {
            "still failed"
        };
        roblox_logging::emit(format!(
            "[runtime] NVIDIA present-mode query {outcome} after {attempts} attempts ({result:?})"
        ));
    }
    result
}

fn is_nvidia_device(instance: vk::Instance, physical: vk::PhysicalDevice) -> bool {
    let Some(address) = super::loader::host_proc(instance, c"vkGetPhysicalDeviceProperties") else {
        return false;
    };
    let function: vk::PFN_vkGetPhysicalDeviceProperties = unsafe { std::mem::transmute(address) };
    let mut properties = vk::PhysicalDeviceProperties::default();
    // SAFETY: the host address and output struct use Vulkan's declared ABI.
    unsafe { function(physical, &mut properties) };
    properties.vendor_id == 0x10DE
}

fn present_mode_name(raw: i32) -> String {
    if raw == vk::PresentModeKHR::IMMEDIATE.as_raw() {
        "immediate".to_owned()
    } else if raw == vk::PresentModeKHR::MAILBOX.as_raw() {
        "mailbox".to_owned()
    } else if raw == vk::PresentModeKHR::FIFO.as_raw() {
        "fifo".to_owned()
    } else if raw == vk::PresentModeKHR::FIFO_RELAXED.as_raw() {
        "fifo-relaxed".to_owned()
    } else {
        format!("unknown ({raw})")
    }
}

fn requested_mode_name(raw: i32) -> String {
    match raw {
        MODE_UNCAPPED => "uncapped (immediate)".to_owned(),
        MODE_ENGINE => "engine default".to_owned(),
        MODE_AUTO => "auto (mailbox preferred)".to_owned(),
        _ => present_mode_name(raw),
    }
}

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
    let mut count = 0;
    let result = get_surface_present_modes(physical, surface, &mut count, std::ptr::null_mut());
    if result != vk::Result::SUCCESS {
        return None;
    }
    let mut modes = vec![vk::PresentModeKHR::IMMEDIATE; count as usize];
    if count > 0 {
        let result = get_surface_present_modes(physical, surface, &mut count, modes.as_mut_ptr());
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
    let modes = (setting != MODE_ENGINE)
        .then(|| supported_modes(info.surface))
        .flatten();
    let preferred_mode = if setting == MODE_AUTO {
        vk::PresentModeKHR::MAILBOX
    } else if setting == MODE_UNCAPPED {
        vk::PresentModeKHR::IMMEDIATE
    } else if setting == MODE_ENGINE {
        info.present_mode
    } else {
        vk::PresentModeKHR::from_raw(setting)
    };
    let chosen_mode = if setting == MODE_ENGINE
        || modes
            .as_ref()
            .is_some_and(|available| available.contains(&preferred_mode))
    {
        preferred_mode
    } else {
        info.present_mode
    };

    let requested_name = requested_mode_name(setting);
    let effective_name = present_mode_name(chosen_mode.as_raw());
    let report_key = ((setting as u32 as u64) << 32) | chosen_mode.as_raw() as u32 as u64;
    if LAST_PRESENT_REPORT.swap(report_key, Ordering::Relaxed) != report_key {
        let fallback = if chosen_mode == preferred_mode {
            ""
        } else if setting == MODE_AUTO {
            " (mailbox unavailable; keeping engine mode)"
        } else if modes.is_none() {
            " (could not query surface modes; keeping engine mode)"
        } else {
            " (requested mode is unsupported or unavailable; keeping engine mode)"
        };
        roblox_logging::emit(format!(
            "[runtime] Vulkan requested presentation mode {requested_name}; effective mode {effective_name}{fallback}"
        ));
    }

    if chosen_mode != info.present_mode {
        let mut image_count = info.min_image_count;
        if matches!(
            chosen_mode,
            vk::PresentModeKHR::MAILBOX | vk::PresentModeKHR::IMMEDIATE
        ) && image_count < 3
        {
            if let Some((_, max)) = image_count_limits(info.surface) {
                if max == 0 || max >= 3 {
                    image_count = 3;
                }
            }
        }
        rewritten.present_mode = chosen_mode;
        rewritten.min_image_count = image_count;
    }
    // SAFETY: `rewritten` preserves the caller's struct and only adjusts
    // supported image usage and optional presentation settings. Chained
    // pointers remain valid for this synchronous Vulkan call.
    let result = unsafe { function(device, &rewritten, allocator, output) };
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
