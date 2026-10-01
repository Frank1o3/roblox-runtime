use ash::vk::{self, Handle};
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) static INSTANCE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static PHYSICAL_DEVICE: AtomicUsize = AtomicUsize::new(0);
static HOST_GET_DEVICE_PROC_ADDR: AtomicUsize = AtomicUsize::new(0);
pub(crate) static HOST_CREATE_SWAPCHAIN: AtomicUsize = AtomicUsize::new(0);
pub(crate) static HOST_GET_SURFACE_CAPABILITIES: AtomicUsize = AtomicUsize::new(0);
pub(crate) static HOST_QUEUE_PRESENT: AtomicUsize = AtomicUsize::new(0);
static HOST_CREATE_DEVICE: AtomicUsize = AtomicUsize::new(0);

fn raw(function: vk::PFN_vkVoidFunction) -> *mut c_void {
    super::loader::function_pointer(function)
}

pub(crate) extern "system" fn get_instance_proc_addr(
    instance: vk::Instance,
    name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    if !instance.is_null() {
        INSTANCE.store(instance.as_raw() as usize, Ordering::Relaxed);
    }
    if name.is_null() {
        return None;
    }
    // SAFETY: Vulkan requires a NUL-terminated function name.
    let bytes = unsafe { CStr::from_ptr(name) }.to_bytes();
    let wrapper = match bytes {
        b"vkGetInstanceProcAddr" => Some(get_instance_proc_addr as *const () as *mut c_void),
        b"vkCreateInstance" => Some(super::instance::create_instance as *const () as *mut c_void),
        b"vkEnumerateInstanceExtensionProperties" => Some(
            super::instance::enumerate_instance_extension_properties as *const () as *mut c_void,
        ),
        b"vkCreateAndroidSurfaceKHR" => {
            Some(super::surface::create_android_surface as *const () as *mut c_void)
        }
        b"vkGetDeviceProcAddr" => {
            let function = super::loader::host_proc(instance, c"vkGetDeviceProcAddr");
            HOST_GET_DEVICE_PROC_ADDR.store(raw(function) as usize, Ordering::Relaxed);
            Some(get_device_proc_addr as *const () as *mut c_void)
        }
        b"vkCreateDevice" => {
            let function = super::loader::host_proc(instance, c"vkCreateDevice");
            HOST_CREATE_DEVICE.store(raw(function) as usize, Ordering::Relaxed);
            Some(create_device as *const () as *mut c_void)
        }
        b"vkCreateSwapchainKHR" => {
            let function = super::loader::host_proc(instance, c"vkCreateSwapchainKHR");
            HOST_CREATE_SWAPCHAIN.store(raw(function) as usize, Ordering::Relaxed);
            Some(super::swapchain::create_swapchain as *const () as *mut c_void)
        }
        b"vkQueuePresentKHR" => {
            let function = super::loader::host_proc(instance, c"vkQueuePresentKHR");
            HOST_QUEUE_PRESENT.store(raw(function) as usize, Ordering::Relaxed);
            Some(super::swapchain::queue_present as *const () as *mut c_void)
        }
        b"vkGetPhysicalDeviceSurfaceCapabilitiesKHR" => {
            let function =
                super::loader::host_proc(instance, c"vkGetPhysicalDeviceSurfaceCapabilitiesKHR");
            HOST_GET_SURFACE_CAPABILITIES.store(raw(function) as usize, Ordering::Relaxed);
            Some(super::swapchain::get_surface_capabilities as *const () as *mut c_void)
        }
        _ => return super::loader::host_proc(instance, unsafe { CStr::from_ptr(name) }),
    };
    // SAFETY: every address in `wrapper` is a Vulkan system-ABI trampoline.
    wrapper.map(|address| unsafe { std::mem::transmute(address) })
}

extern "system" fn get_device_proc_addr(
    device: vk::Device,
    name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    let address = HOST_GET_DEVICE_PROC_ADDR.load(Ordering::Relaxed);
    if address == 0 || name.is_null() {
        return None;
    }
    type Function = unsafe extern "system" fn(vk::Device, *const c_char) -> vk::PFN_vkVoidFunction;
    // SAFETY: the pointer is the host loader's vkGetDeviceProcAddr.
    let function: Function = unsafe { std::mem::transmute(address) };
    // SAFETY: Vulkan requires a NUL-terminated function name.
    match unsafe { CStr::from_ptr(name) }.to_bytes() {
        b"vkCreateSwapchainKHR" => {
            HOST_CREATE_SWAPCHAIN.store(
                raw(unsafe { function(device, name) }) as usize,
                Ordering::Relaxed,
            );
            Some(unsafe { std::mem::transmute(super::swapchain::create_swapchain as *const ()) })
        }
        b"vkQueuePresentKHR" => {
            HOST_QUEUE_PRESENT.store(
                raw(unsafe { function(device, name) }) as usize,
                Ordering::Relaxed,
            );
            Some(unsafe { std::mem::transmute(super::swapchain::queue_present as *const ()) })
        }
        _ => unsafe { function(device, name) },
    }
}

extern "system" fn create_device(
    physical_device: vk::PhysicalDevice,
    info: *const vk::DeviceCreateInfo<'_>,
    allocator: *const vk::AllocationCallbacks<'_>,
    output: *mut vk::Device,
) -> vk::Result {
    let address = HOST_CREATE_DEVICE.load(Ordering::Relaxed);
    if address == 0 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    type Function = unsafe extern "system" fn(
        vk::PhysicalDevice,
        *const vk::DeviceCreateInfo<'_>,
        *const vk::AllocationCallbacks<'_>,
        *mut vk::Device,
    ) -> vk::Result;
    // SAFETY: the pointer was resolved from the host loader as vkCreateDevice.
    let function: Function = unsafe { std::mem::transmute(address) };
    // SAFETY: the arguments are forwarded unmodified under Vulkan's contract.
    let result = unsafe { function(physical_device, info, allocator, output) };
    if result == vk::Result::SUCCESS {
        PHYSICAL_DEVICE.store(physical_device.as_raw() as usize, Ordering::Relaxed);
    }
    result
}

extern "system" fn queue_present(
    queue: vk::Queue,
    info: *const vk::PresentInfoKHR<'_>,
) -> vk::Result {
    let address = HOST_QUEUE_PRESENT.load(Ordering::Relaxed);
    if address == 0 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    type Function =
        unsafe extern "system" fn(vk::Queue, *const vk::PresentInfoKHR<'_>) -> vk::Result;
    // SAFETY: the pointer was resolved from the host loader as vkQueuePresentKHR.
    let function: Function = unsafe { std::mem::transmute(address) };
    // SAFETY: the arguments are forwarded under Vulkan's contract.
    unsafe { function(queue, info) }
}
