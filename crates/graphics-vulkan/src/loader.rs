use ash::{Entry, vk};
use std::ffi::{CStr, c_void};
use std::sync::OnceLock;

static ENTRY: OnceLock<Option<Entry>> = OnceLock::new();

pub(crate) fn entry() -> Option<&'static Entry> {
    ENTRY
        // SAFETY: Ash loads the system Vulkan library and keeps it alive in
        // `Entry`; the process-global value is never dropped while pointers are used.
        .get_or_init(|| unsafe {
            Entry::load_from("libvulkan.so.1")
                .or_else(|_| Entry::load_from("libvulkan.so"))
                .ok()
        })
        .as_ref()
}

pub(crate) fn function_pointer(function: vk::PFN_vkVoidFunction) -> *mut c_void {
    function.map_or(std::ptr::null_mut(), |function| {
        function as *const () as *mut c_void
    })
}

pub fn available_for_surface() -> bool {
    let Some(entry) = entry() else { return false };
    let Some(surface) = super::platform::current_surface() else {
        return false;
    };
    let required = match surface {
        super::platform::Surface::Xlib { .. } => c"VK_KHR_xlib_surface",
        super::platform::Surface::Wayland { .. } => c"VK_KHR_wayland_surface",
    };
    // SAFETY: Ash owns a live host Vulkan loader for the process lifetime.
    let Ok(extensions) = (unsafe { entry.enumerate_instance_extension_properties(None) }) else {
        return false;
    };
    extensions.iter().any(|extension| {
        // Vulkan guarantees this fixed-size extension name is NUL terminated.
        let name = unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) };
        name == required
    })
}

pub fn loader_symbol() -> Option<*mut c_void> {
    (super::enabled() && available_for_surface())
        .then_some(super::dispatch::get_instance_proc_addr as *const () as *mut c_void)
}

pub const LIBRARY_NAMES: [&str; 2] = ["libvulkan.so", "libvulkan.so.1"];

pub(crate) fn host_proc(instance: vk::Instance, name: &CStr) -> vk::PFN_vkVoidFunction {
    let Some(entry) = entry() else { return None };
    // SAFETY: `name` is NUL terminated; `instance` is null or a host Vulkan handle.
    unsafe { entry.get_instance_proc_addr(instance, name.as_ptr()) }
}
