use ash::vk::{self, Handle};
use std::ffi::{CStr, c_char};

const ANDROID_SURFACE: &[u8] = b"VK_KHR_android_surface";
const ANDROID_SURFACE_SPEC_VERSION: u32 = 6;

fn host_surface_extension() -> &'static CStr {
    match super::platform::current_surface() {
        Some(super::platform::Surface::Wayland { .. }) => c"VK_KHR_wayland_surface",
        _ => c"VK_KHR_xlib_surface",
    }
}

pub(crate) extern "system" fn create_instance(
    create_info: *const vk::InstanceCreateInfo<'_>,
    allocator: *const vk::AllocationCallbacks<'_>,
    output: *mut vk::Instance,
) -> vk::Result {
    let Some(entry) = super::loader::entry() else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    if create_info.is_null() {
        // SAFETY: Vulkan's loader reports null create-info as an initialization error.
        return unsafe { (entry.fp_v1_0().create_instance)(create_info, allocator, output) };
    }
    // SAFETY: non-null pointer comes from Vulkan and follows VkInstanceCreateInfo's ABI.
    let info = unsafe { &*create_info };
    let names = if info.enabled_extension_count == 0 || info.pp_enabled_extension_names.is_null() {
        &[][..]
    } else {
        // SAFETY: the paired count and pointer are supplied by the Vulkan caller.
        unsafe {
            std::slice::from_raw_parts(
                info.pp_enabled_extension_names,
                info.enabled_extension_count as usize,
            )
        }
    };
    let replacement = host_surface_extension();
    let rewritten: Vec<*const c_char> = names
        .iter()
        .map(|&name| {
            if name.is_null() {
                return name;
            }
            // SAFETY: enabled extension strings are NUL-terminated by Vulkan contract.
            if unsafe { CStr::from_ptr(name) }.to_bytes() == ANDROID_SURFACE {
                replacement.as_ptr()
            } else {
                name
            }
        })
        .collect();
    let mut patched = *info;
    if !rewritten.is_empty() {
        patched.pp_enabled_extension_names = rewritten.as_ptr();
    }
    let mut instance = vk::Instance::null();
    let instance_output = if output.is_null() {
        output
    } else {
        &mut instance
    };
    // SAFETY: `patched` preserves the caller's struct and only replaces the
    // Android WSI extension with its host equivalent for this live surface.
    let result = unsafe { (entry.fp_v1_0().create_instance)(&patched, allocator, instance_output) };
    if result == vk::Result::SUCCESS && !output.is_null() {
        super::dispatch::INSTANCE.store(
            instance.as_raw() as usize,
            std::sync::atomic::Ordering::Relaxed,
        );
        // SAFETY: Vulkan writes the created handle on success; the output is its paired pointer.
        unsafe {
            *output = instance;
        }
    }
    result
}

pub(crate) extern "system" fn enumerate_instance_extension_properties(
    layer_name: *const c_char,
    property_count: *mut u32,
    properties: *mut vk::ExtensionProperties,
) -> vk::Result {
    let Some(entry) = super::loader::entry() else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    if property_count.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    let layer = if layer_name.is_null() {
        None
    } else {
        // SAFETY: Vulkan layer names are NUL-terminated.
        Some(unsafe { CStr::from_ptr(layer_name) })
    };
    // SAFETY: Ash queries the loaded Vulkan loader with the supplied optional layer.
    let mut extensions = match unsafe { entry.enumerate_instance_extension_properties(layer) } {
        Ok(extensions) => extensions,
        Err(error) => return error,
    };
    let surface_extension = host_surface_extension();
    let has_host_surface = extensions.iter().any(|property| {
        // Vulkan specifies a NUL-terminated name in each fixed-size field.
        (unsafe { CStr::from_ptr(property.extension_name.as_ptr()) }) == surface_extension
    });
    let has_android_surface = extensions.iter().any(|property| {
        // SAFETY: same Vulkan fixed-size extension-name contract as above.
        unsafe { CStr::from_ptr(property.extension_name.as_ptr()) }.to_bytes() == ANDROID_SURFACE
    });
    if layer.is_none() && has_host_surface && !has_android_surface {
        let mut property = vk::ExtensionProperties::default();
        for (dst, src) in property
            .extension_name
            .iter_mut()
            .zip(ANDROID_SURFACE.iter().copied())
        {
            *dst = src as i8;
        }
        property.spec_version = ANDROID_SURFACE_SPEC_VERSION;
        extensions.push(property);
    }

    let total = extensions.len() as u32;
    // SAFETY: Vulkan's two-call enumeration contract supplies a valid count pointer.
    if properties.is_null() {
        unsafe {
            *property_count = total;
        }
        return vk::Result::SUCCESS;
    }
    // SAFETY: caller provides capacity in *property_count and output storage of that size.
    let capacity = unsafe { *property_count };
    let written = capacity.min(total);
    // SAFETY: `written` is bounded by both output capacity and vector length.
    unsafe {
        std::ptr::copy_nonoverlapping(extensions.as_ptr(), properties, written as usize);
        *property_count = written;
    }
    if written < total {
        vk::Result::INCOMPLETE
    } else {
        vk::Result::SUCCESS
    }
}
