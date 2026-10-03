//! Renderer preference owned by the runtime.
//!
//! The embedding client owns the host window and supplies its renderable
//! surface. Renderer policy stays here because the Android graphics APIs
//! exposed to Roblox, and the decision to offer Vulkan or GLES, are runtime
//! responsibilities.

use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

static VSYNC: AtomicBool = AtomicBool::new(true);
static OPENGL_SWAP_INTERVAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

pub use roblox_android::native_window::{HostSurface, SurfaceError};

/// Requested graphics backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackendPreference {
    /// Prefer Vulkan when the supplied surface and host loader support it;
    /// otherwise use OpenGL ES.
    #[default]
    Automatic,
    /// Require Vulkan.
    Vulkan,
    /// Require OpenGL ES.
    OpenGlEs,
}

/// Backend selected after checking host support.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Vulkan,
    OpenGlEs,
}

impl BackendPreference {
    /// Resolve this preference against Vulkan support on the supplied surface.
    pub fn select(self, vulkan_available: bool) -> Result<Backend, BackendUnavailable> {
        match self {
            Self::Automatic if vulkan_available => Ok(Backend::Vulkan),
            Self::Automatic | Self::OpenGlEs => Ok(Backend::OpenGlEs),
            Self::Vulkan if vulkan_available => Ok(Backend::Vulkan),
            Self::Vulkan => Err(BackendUnavailable::Vulkan),
        }
    }
}

/// Install the native window supplied by the client. Its X11/Wayland objects
/// must stay alive until [`clear_surface`] is called after engine shutdown.
pub fn install_surface(surface: HostSurface) {
    let (width, height) = surface.dimensions();
    let vulkan_surface = match surface {
        HostSurface::Xlib {
            display,
            window,
            width,
            height,
        } => roblox_graphics_vulkan::Surface::Xlib {
            display,
            window,
            width,
            height,
        },
        HostSurface::Wayland {
            display,
            surface,
            width,
            height,
            ..
        } => roblox_graphics_vulkan::Surface::Wayland {
            display,
            surface,
            width,
            height,
        },
    };
    roblox_graphics_vulkan::set_surface(Some(vulkan_surface));
    roblox_android::native_window::install(surface);
    roblox_jni::game_activity::set_display_size(width as i32, height as i32);
}

/// Release the process-wide host-surface descriptor after the engine stops.
pub fn clear_surface() {
    roblox_graphics_vulkan::set_surface(None);
    roblox_android::native_window::clear();
    crate::webview::clear_pending_request();
}

/// Update the installed client surface after its host window is resized.
pub fn resize_surface(width: u32, height: u32) -> Result<(), SurfaceError> {
    roblox_android::native_window::resize(width, height)?;
    roblox_graphics_vulkan::resize_surface(width, height);
    roblox_jni::game_activity::set_display_size(width as i32, height as i32);
    Ok(())
}

/// Whether the client has installed a renderable surface.
pub fn has_surface() -> bool {
    roblox_android::native_window::is_installed()
}

/// Whether the host loaders needed by the initial EGL path can be found.
/// This checks loader symbols only; it does not create a context or present.
pub fn host_egl_available() -> bool {
    roblox_linker::host_symbol("libEGL.so.1", "eglGetDisplay").is_some()
        && roblox_linker::host_symbol("libEGL.so.1", "eglCreateWindowSurface").is_some()
        && roblox_linker::host_symbol("libGLESv2.so.2", "glGetString").is_some()
}

/// Check the installed host surface and renderer support before constructors.
/// Vulkan is offered only when Ash can load the host loader and the matching
/// Xlib or Wayland WSI extension is available. Roblox then renders through the
/// host Vulkan implementation; this crate translates Android WSI requests.
pub fn prepare(
    preference: BackendPreference,
    present_mode: Option<&str>,
    vsync: bool,
) -> Result<Backend, BackendUnavailable> {
    prepare_with_swap_interval(preference, present_mode, vsync, 1)
}

/// Prepare the selected renderer with an explicit EGL swap interval. Valid
/// OpenGL values are -1 (adaptive), 0 (off), and 1 (on).
pub fn prepare_with_swap_interval(
    preference: BackendPreference,
    present_mode: Option<&str>,
    vsync: bool,
    opengl_swap_interval: i32,
) -> Result<Backend, BackendUnavailable> {
    if !has_surface() {
        return Err(BackendUnavailable::NoSurface);
    }
    let vulkan_available = roblox_graphics_vulkan::available_for_surface();
    let selected = preference.select(vulkan_available)?;
    roblox_graphics_vulkan::set_enabled(selected == Backend::Vulkan);
    VSYNC.store(vsync, Ordering::Relaxed);
    OPENGL_SWAP_INTERVAL.store(opengl_swap_interval.clamp(-1, 1), Ordering::Relaxed);
    roblox_graphics_vulkan::set_present_mode(present_mode, vsync);
    if selected == Backend::OpenGlEs && !host_egl_available() {
        return Err(BackendUnavailable::OpenGlEs);
    }
    Ok(selected)
}

/// Apply the user's VSync setting to the engine's EGL swap interval request.
#[allow(unsafe_code)]
extern "C" fn egl_swap_interval(display: *mut c_void, interval: c_int) -> c_int {
    let Some(address) = roblox_linker::host_symbol("libEGL.so.1", "eglSwapInterval") else {
        return 0;
    };
    type Function = extern "C" fn(*mut c_void, c_int) -> c_int;
    // SAFETY: this host address has EGL's exact swap interval ABI.
    let function: Function = unsafe { std::mem::transmute(address) };
    function(
        display,
        if VSYNC.load(Ordering::Relaxed) {
            // `interval` is the engine's request. The saved preference chooses
            // the EGL swap-control mode independently of that request.
            let _ = interval;
            OPENGL_SWAP_INTERVAL.load(Ordering::Relaxed)
        } else {
            0
        },
    )
}

/// Present the supplied Android surface through the host EGL window type.
#[allow(unsafe_code)]
extern "C" fn egl_create_window_surface(
    display: *mut c_void,
    config: *mut c_void,
    _android_window: *mut c_void,
    attributes: *const c_int,
) -> *mut c_void {
    let Some(surface) = roblox_android::native_window::current() else {
        return std::ptr::null_mut();
    };
    let Some(address) = roblox_linker::host_symbol("libEGL.so.1", "eglCreateWindowSurface") else {
        return std::ptr::null_mut();
    };
    type Function =
        extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *const c_int) -> *mut c_void;
    // SAFETY: this address is the host EGL implementation of this exact
    // Khronos function; the translated native window matches its platform ABI.
    let function: Function = unsafe { std::mem::transmute(address) };
    function(
        display,
        config,
        surface.egl_native_window() as *mut c_void,
        attributes,
    )
}

/// Bind EGL on Wayland to the same `wl_display` that owns the supplied surface.
#[allow(unsafe_code)]
extern "C" fn egl_get_display(native_display: *mut c_void) -> *mut c_void {
    if let Some(display) = roblox_android::native_window::wayland_display() {
        for name in ["eglGetPlatformDisplay", "eglGetPlatformDisplayEXT"] {
            if let Some(address) = roblox_linker::host_symbol("libEGL.so.1", name) {
                type Function = extern "C" fn(u32, *mut c_void, *const c_int) -> *mut c_void;
                // SAFETY: the selected host export has the EGL platform-display
                // signature, and the platform enum identifies Wayland.
                let function: Function = unsafe { std::mem::transmute(address) };
                let result = function(0x31D8, display, std::ptr::null());
                if !result.is_null() {
                    return result;
                }
            }
        }
        return std::ptr::null_mut();
    }

    let Some(address) = roblox_linker::host_symbol("libEGL.so.1", "eglGetDisplay") else {
        return std::ptr::null_mut();
    };
    type Function = extern "C" fn(*mut c_void) -> *mut c_void;
    // SAFETY: this is the host EGL implementation of the named function.
    let function: Function = unsafe { std::mem::transmute(address) };
    function(native_display)
}

/// EGL adaptations included in the Android symbol table.
#[allow(unsafe_code)]
pub(crate) fn function_overrides() -> Vec<(&'static str, *mut c_void)> {
    vec![
        (
            "eglCreateWindowSurface",
            egl_create_window_surface as *const () as *mut c_void,
        ),
        ("eglGetDisplay", egl_get_display as *const () as *mut c_void),
        (
            "eglSwapInterval",
            egl_swap_interval as *const () as *mut c_void,
        ),
    ]
}

/// A requested backend cannot be supplied by the host surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendUnavailable {
    NoSurface,
    OpenGlEs,
    Vulkan,
}

impl std::fmt::Display for BackendUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSurface => f.write_str("a host renderable surface has not been installed"),
            Self::OpenGlEs => f.write_str("the host EGL/GLES libraries are unavailable"),
            Self::Vulkan => {
                f.write_str("the host Vulkan loader or matching WSI extension is unavailable")
            }
        }
    }
}

impl std::error::Error for BackendUnavailable {}

#[cfg(test)]
mod tests {
    use super::{Backend, BackendPreference};

    #[test]
    fn automatic_prefers_vulkan_and_falls_back_to_gles() {
        assert_eq!(
            BackendPreference::Automatic.select(true),
            Ok(Backend::Vulkan)
        );
        assert_eq!(
            BackendPreference::Automatic.select(false),
            Ok(Backend::OpenGlEs)
        );
    }

    #[test]
    fn explicit_backend_is_respected() {
        assert_eq!(
            BackendPreference::OpenGlEs.select(true),
            Ok(Backend::OpenGlEs)
        );
        assert_eq!(BackendPreference::Vulkan.select(true), Ok(Backend::Vulkan));
        assert!(BackendPreference::Vulkan.select(false).is_err());
    }
}
