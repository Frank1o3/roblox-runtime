//! Renderer preference owned by the runtime.
//!
//! The embedding client owns the host window and supplies its renderable
//! surface. Renderer policy stays here because the Android graphics APIs
//! exposed to Roblox, and the decision to offer Vulkan or GLES, are runtime
//! responsibilities.

use std::ffi::{CStr, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

static VSYNC: AtomicBool = AtomicBool::new(true);
static OPENGL_SWAP_INTERVAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);
const EGL_TRUE: c_int = 1;
const EGL_SUCCESS: c_int = 0x3000;
const EGL_BAD_PARAMETER: c_int = 0x300C;
const EGL_EXTENSIONS: c_int = 0x3055;

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
    roblox_jni_bridge::game_activity::set_display_size(width as i32, height as i32);
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
    roblox_jni_bridge::game_activity::set_display_size(width as i32, height as i32);
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
    prepare_with_swap_interval(preference, present_mode, vsync, 1, None)
}

/// Prepare the selected renderer with an explicit EGL swap interval. Valid
/// OpenGL values are -1 (adaptive, when advertised by the active EGL display),
/// 0 (off), and 1 (on).
pub fn prepare_with_swap_interval(
    preference: BackendPreference,
    present_mode: Option<&str>,
    vsync: bool,
    opengl_swap_interval: i32,
    vulkan_fps_limit: Option<u32>,
) -> Result<Backend, BackendUnavailable> {
    if !has_surface() {
        return Err(BackendUnavailable::NoSurface);
    }
    let vulkan_available = roblox_graphics_vulkan::available_for_surface();
    let selected = preference.select(vulkan_available)?;
    roblox_graphics_vulkan::set_enabled(selected == Backend::Vulkan);
    VSYNC.store(vsync, Ordering::Relaxed);
    OPENGL_SWAP_INTERVAL.store(
        normalize_swap_interval(opengl_swap_interval),
        Ordering::Relaxed,
    );
    roblox_graphics_vulkan::set_present_mode(present_mode, vsync, vulkan_fps_limit);
    if selected == Backend::OpenGlEs && !host_egl_available() {
        return Err(BackendUnavailable::OpenGlEs);
    }
    Ok(selected)
}

/// Apply the user's VSync setting to the engine's EGL swap interval request.
#[allow(unsafe_code)]
extern "C" fn egl_swap_interval(display: *mut c_void, interval: c_int) -> c_int {
    let Some(address) = roblox_linker::host_symbol("libEGL.so.1", "eglSwapInterval") else {
        eprintln!("[graphics] eglSwapInterval unavailable; requested interval={interval}");
        return 0;
    };
    type Function = extern "C" fn(*mut c_void, c_int) -> c_int;
    // SAFETY: this host address has EGL's exact swap interval ABI.
    let function: Function = unsafe { std::mem::transmute(address) };
    let configured = OPENGL_SWAP_INTERVAL.load(Ordering::Relaxed);
    let requested = effective_swap_interval(VSYNC.load(Ordering::Relaxed), configured);
    let adaptive_supported =
        requested != -1 || has_egl_extension(display, "EGL_EXT_swap_control_tear");
    let interval_to_apply = select_swap_interval(
        VSYNC.load(Ordering::Relaxed),
        configured,
        adaptive_supported,
    );
    let get_error = roblox_linker::host_symbol("libEGL.so.1", "eglGetError").map(|address| {
        // SAFETY: this host address has EGL's exact error-query ABI.
        unsafe { std::mem::transmute::<*mut c_void, extern "C" fn() -> c_int>(address) }
    });
    let result = apply_swap_interval(
        interval_to_apply,
        |interval| function(display, interval),
        || get_error.map(|get_error| get_error()),
    );
    if let Some(error) = result.previous_error.filter(|error| *error != EGL_SUCCESS) {
        eprintln!(
            "[graphics] eglSwapInterval found a pending EGL error before the call: 0x{:04x}",
            error
        );
    }
    eprintln!(
        "[graphics] eglSwapInterval engine_requested={interval} configured={configured} vsync={} adaptive_supported={} applied={:?} success={} egl_error={:?} adaptive_fallback={:?}",
        VSYNC.load(Ordering::Relaxed),
        adaptive_supported,
        result.applied,
        result.success,
        result.error.map(|error| format!("0x{error:04x}")),
        result.fallback_error.map(|error| format!("0x{error:04x}")),
    );
    if !result.success && get_error.is_none() {
        eprintln!("[graphics] eglGetError is unavailable; EGL failure code could not be read");
    }
    if result.success { EGL_TRUE } else { 0 }
}

#[allow(unsafe_code)]
fn has_egl_extension(display: *mut c_void, extension: &str) -> bool {
    let Some(address) = roblox_linker::host_symbol("libEGL.so.1", "eglQueryString") else {
        return false;
    };
    type Function = extern "C" fn(*mut c_void, c_int) -> *const std::ffi::c_char;
    // SAFETY: this host address has EGL's exact eglQueryString ABI.
    let function: Function = unsafe { std::mem::transmute(address) };
    let extensions = function(display, EGL_EXTENSIONS);
    if extensions.is_null() {
        return false;
    }
    // SAFETY: EGL returns a NUL-terminated extension string owned by the EGL
    // implementation and valid until the display is terminated.
    extension_list_contains(unsafe { CStr::from_ptr(extensions) }.to_bytes(), extension)
}

fn normalize_swap_interval(interval: i32) -> i32 {
    interval.clamp(-1, 1)
}

fn effective_swap_interval(vsync: bool, configured: i32) -> i32 {
    if vsync {
        normalize_swap_interval(configured)
    } else {
        0
    }
}

fn select_swap_interval(vsync: bool, configured: i32, adaptive_supported: bool) -> i32 {
    let requested = effective_swap_interval(vsync, configured);
    if requested == -1 && !adaptive_supported {
        1
    } else {
        requested
    }
}

fn extension_list_contains(extensions: &[u8], requested: &str) -> bool {
    extensions
        .split(u8::is_ascii_whitespace)
        .any(|extension| extension == requested.as_bytes())
}

struct SwapIntervalResult {
    applied: Option<i32>,
    success: bool,
    error: Option<c_int>,
    fallback_error: Option<c_int>,
    previous_error: Option<c_int>,
}

fn apply_swap_interval(
    requested: i32,
    mut swap: impl FnMut(i32) -> c_int,
    mut get_error: impl FnMut() -> Option<c_int>,
) -> SwapIntervalResult {
    let previous_error = get_error();
    if swap(requested) == EGL_TRUE {
        return SwapIntervalResult {
            applied: Some(requested),
            success: true,
            error: None,
            fallback_error: None,
            previous_error,
        };
    }

    let error = get_error();
    if requested == -1 && error == Some(EGL_BAD_PARAMETER) {
        if swap(1) == EGL_TRUE {
            return SwapIntervalResult {
                applied: Some(1),
                success: true,
                error: None,
                fallback_error: error,
                previous_error,
            };
        }
        return SwapIntervalResult {
            applied: None,
            success: false,
            error: get_error(),
            fallback_error: error,
            previous_error,
        };
    }
    SwapIntervalResult {
        applied: None,
        success: false,
        error,
        fallback_error: None,
        previous_error,
    }
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
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::{
        Backend, BackendPreference, EGL_BAD_PARAMETER, EGL_SUCCESS, apply_swap_interval,
        effective_swap_interval, extension_list_contains, normalize_swap_interval,
        select_swap_interval,
    };

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

    #[test]
    fn global_vsync_setting_selects_configured_opengl_interval() {
        assert_eq!(effective_swap_interval(false, 1), 0);
        assert_eq!(effective_swap_interval(true, 0), 0);
        assert_eq!(effective_swap_interval(true, 1), 1);
        assert_eq!(effective_swap_interval(true, -1), -1);
    }

    #[test]
    fn swap_interval_is_limited_to_supported_setting_values() {
        assert_eq!(normalize_swap_interval(-9), -1);
        assert_eq!(normalize_swap_interval(-1), -1);
        assert_eq!(normalize_swap_interval(0), 0);
        assert_eq!(normalize_swap_interval(1), 1);
        assert_eq!(normalize_swap_interval(9), 1);
    }

    #[test]
    fn adaptive_interval_requires_an_advertised_display_extension() {
        assert_eq!(select_swap_interval(true, -1, false), 1);
        assert_eq!(select_swap_interval(true, -1, true), -1);
        assert_eq!(select_swap_interval(false, -1, false), 0);
        assert!(extension_list_contains(
            b"EGL_EXT_other EGL_EXT_swap_control_tear EGL_VENDOR_extension",
            "EGL_EXT_swap_control_tear"
        ));
        assert!(!extension_list_contains(
            b"EGL_EXT_swap_control_tearing EGL_VENDOR_extension",
            "EGL_EXT_swap_control_tear"
        ));
    }

    #[test]
    fn adaptive_interval_falls_back_to_vsync_when_driver_rejects_it() {
        let calls = RefCell::new(Vec::new());
        let errors = RefCell::new(VecDeque::from([EGL_SUCCESS, EGL_BAD_PARAMETER]));
        let result = apply_swap_interval(
            -1,
            |interval| {
                calls.borrow_mut().push(interval);
                if interval == -1 { 0 } else { 1 }
            },
            || errors.borrow_mut().pop_front(),
        );
        assert_eq!(result.applied, Some(1));
        assert!(result.success);
        assert_eq!(result.fallback_error, Some(EGL_BAD_PARAMETER));
        assert_eq!(*calls.borrow(), [-1, 1]);
        assert_eq!(result.previous_error, Some(EGL_SUCCESS));
    }

    #[test]
    fn egl_failure_is_returned_and_is_not_silently_ignored() {
        let result = apply_swap_interval(0, |_| 0, || Some(EGL_BAD_PARAMETER));
        assert!(!result.success);
        assert_eq!(result.applied, None);
        assert_eq!(result.error, Some(EGL_BAD_PARAMETER));
        assert_eq!(result.fallback_error, None);
    }
}
