use ash::vk;

pub(crate) extern "system" fn create_android_surface(
    instance: vk::Instance,
    _create_info: *const vk::AndroidSurfaceCreateInfoKHR<'_>,
    allocator: *const vk::AllocationCallbacks<'_>,
    output: *mut vk::SurfaceKHR,
) -> vk::Result {
    let host_instance = instance;
    match super::platform::current_surface() {
        Some(super::platform::Surface::Wayland {
            display, surface, ..
        }) => {
            let Some(function) =
                super::loader::host_proc(host_instance, c"vkCreateWaylandSurfaceKHR")
            else {
                return vk::Result::ERROR_EXTENSION_NOT_PRESENT;
            };
            type Function = unsafe extern "system" fn(
                vk::Instance,
                *const vk::WaylandSurfaceCreateInfoKHR<'_>,
                *const vk::AllocationCallbacks<'_>,
                *mut vk::SurfaceKHR,
            ) -> vk::Result;
            // SAFETY: Vulkan returned this pointer for vkCreateWaylandSurfaceKHR.
            let function: Function = unsafe { std::mem::transmute(function) };
            let info = vk::WaylandSurfaceCreateInfoKHR::default()
                .display((display as *mut std::ffi::c_void).cast())
                .surface((surface as *mut std::ffi::c_void).cast());
            // SAFETY: the handles are the live client window objects installed by the host.
            unsafe { function(instance, &info, allocator, output) }
        }
        Some(super::platform::Surface::Xlib {
            display, window, ..
        }) => {
            let Some(function) = super::loader::host_proc(host_instance, c"vkCreateXlibSurfaceKHR")
            else {
                return vk::Result::ERROR_EXTENSION_NOT_PRESENT;
            };
            type Function = unsafe extern "system" fn(
                vk::Instance,
                *const vk::XlibSurfaceCreateInfoKHR<'_>,
                *const vk::AllocationCallbacks<'_>,
                *mut vk::SurfaceKHR,
            ) -> vk::Result;
            // SAFETY: Vulkan returned this pointer for vkCreateXlibSurfaceKHR.
            let function: Function = unsafe { std::mem::transmute(function) };
            let info = vk::XlibSurfaceCreateInfoKHR::default()
                .dpy((display as *mut std::ffi::c_void).cast())
                .window(window as vk::Window);
            // SAFETY: the handles are the live client window objects installed by the host.
            unsafe { function(instance, &info, allocator, output) }
        }
        None => vk::Result::ERROR_INITIALIZATION_FAILED,
    }
}
