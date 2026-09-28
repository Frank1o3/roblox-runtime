//! Android `ANativeWindow` callbacks backed by a surface owned by the client.

use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

/// Platform handles for one client-owned native window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostSurface {
    /// Xlib display and XID. The X connection and window must outlive this
    /// descriptor and every engine surface created from it.
    Xlib {
        display: usize,
        window: u64,
        width: u32,
        height: u32,
    },
    /// A same-connection Wayland display/surface and its `wl_egl_window`.
    /// All three objects must outlive this descriptor and the engine surface.
    Wayland {
        display: usize,
        surface: usize,
        egl_window: usize,
        width: u32,
        height: u32,
    },
}

impl HostSurface {
    /// Construct an Xlib surface descriptor.
    ///
    /// # Safety
    ///
    /// `display` must be a live Xlib `Display*`, and `window` must name a live
    /// X window on it until the runtime releases the surface.
    pub unsafe fn xlib(
        display: *mut c_void,
        window: u64,
        width: u32,
        height: u32,
    ) -> Result<Self, SurfaceError> {
        if display.is_null() || window == 0 {
            return Err(SurfaceError::InvalidHandle);
        }
        if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
            return Err(SurfaceError::InvalidDimensions);
        }
        Ok(Self::Xlib {
            display: display as usize,
            window,
            width,
            height,
        })
    }

    /// Construct a Wayland descriptor using the client's existing connection.
    ///
    /// # Safety
    ///
    /// These must be live `wl_display*`, `wl_surface*` and `wl_egl_window*`
    /// objects, with the EGL window created for the supplied surface on the
    /// same connection. They must remain live until the runtime releases them.
    pub unsafe fn wayland(
        display: *mut c_void,
        surface: *mut c_void,
        egl_window: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<Self, SurfaceError> {
        if display.is_null() || surface.is_null() || egl_window.is_null() {
            return Err(SurfaceError::InvalidHandle);
        }
        if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
            return Err(SurfaceError::InvalidDimensions);
        }
        Ok(Self::Wayland {
            display: display as usize,
            surface: surface as usize,
            egl_window: egl_window as usize,
            width,
            height,
        })
    }

    pub fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Xlib { width, height, .. } | Self::Wayland { width, height, .. } => {
                (width, height)
            }
        }
    }

    pub fn egl_native_window(self) -> usize {
        match self {
            Self::Xlib { window, .. } => window as usize,
            Self::Wayland { egl_window, .. } => egl_window,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceError {
    InvalidHandle,
    InvalidDimensions,
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHandle => f.write_str("host surface contains a null native handle"),
            Self::InvalidDimensions => f.write_str("host surface dimensions must be nonzero"),
        }
    }
}

impl std::error::Error for SurfaceError {}

static CURRENT: OnceLock<Mutex<Option<HostSurface>>> = OnceLock::new();
static WIDTH: AtomicI32 = AtomicI32::new(0);
static HEIGHT: AtomicI32 = AtomicI32::new(0);
static FORMAT: AtomicI32 = AtomicI32::new(1); // HAL_PIXEL_FORMAT_RGBA_8888
static WINDOW_TOKEN: u8 = 0;

fn current_lock() -> &'static Mutex<Option<HostSurface>> {
    CURRENT.get_or_init(|| Mutex::new(None))
}

pub fn install(surface: HostSurface) {
    let (width, height) = surface.dimensions();
    *current_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(surface);
    WIDTH.store(width as i32, Ordering::Release);
    HEIGHT.store(height as i32, Ordering::Release);
    FORMAT.store(1, Ordering::Release);
}

/// Update the host-owned window dimensions after a resize event.
pub fn resize(width: u32, height: u32) -> Result<(), SurfaceError> {
    if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err(SurfaceError::InvalidDimensions);
    }
    let mut current = current_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(surface) = current.as_mut() else {
        return Err(SurfaceError::InvalidHandle);
    };
    match surface {
        HostSurface::Xlib {
            width: current_width,
            height: current_height,
            ..
        }
        | HostSurface::Wayland {
            width: current_width,
            height: current_height,
            ..
        } => {
            *current_width = width;
            *current_height = height;
        }
    }
    WIDTH.store(width as i32, Ordering::Release);
    HEIGHT.store(height as i32, Ordering::Release);
    Ok(())
}

pub fn clear() {
    *current_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    WIDTH.store(0, Ordering::Release);
    HEIGHT.store(0, Ordering::Release);
}

pub fn is_installed() -> bool {
    current_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_some()
}

pub fn current() -> Option<HostSurface> {
    *current_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn wayland_display() -> Option<*mut c_void> {
    match current()? {
        HostSurface::Wayland { display, .. } => Some(display as *mut c_void),
        HostSurface::Xlib { .. } => None,
    }
}

fn handle() -> *mut c_void {
    if is_installed() {
        std::ptr::addr_of!(WINDOW_TOKEN).cast_mut().cast()
    } else {
        std::ptr::null_mut()
    }
}

fn valid(window: *mut c_void) -> bool {
    !window.is_null() && window == std::ptr::addr_of!(WINDOW_TOKEN).cast_mut().cast()
}

extern "C" fn from_surface(_env: *mut c_void, _surface: *mut c_void) -> *mut c_void {
    handle()
}

extern "C" fn acquire(_window: *mut c_void) {}
extern "C" fn release(_window: *mut c_void) {}

extern "C" fn get_width(window: *mut c_void) -> c_int {
    if valid(window) {
        WIDTH.load(Ordering::Acquire)
    } else {
        0
    }
}

extern "C" fn get_height(window: *mut c_void) -> c_int {
    if valid(window) {
        HEIGHT.load(Ordering::Acquire)
    } else {
        0
    }
}

extern "C" fn get_format(window: *mut c_void) -> c_int {
    if valid(window) {
        FORMAT.load(Ordering::Acquire)
    } else {
        0
    }
}

extern "C" fn set_buffers_geometry(
    window: *mut c_void,
    width: c_int,
    height: c_int,
    format: c_int,
) -> c_int {
    if !valid(window) {
        return -22;
    }
    if width > 0 {
        WIDTH.store(width, Ordering::Release);
    }
    if height > 0 {
        HEIGHT.store(height, Ordering::Release);
    }
    if format > 0 {
        FORMAT.store(format, Ordering::Release);
    }
    0
}

extern "C" fn lock(_window: *mut c_void, _buffer: *mut c_void, _dirty: *mut c_void) -> c_int {
    -38 // -ENOSYS; the supplied surface is rendered through EGL/Vulkan.
}

extern "C" fn unlock_and_post(_window: *mut c_void) -> c_int {
    -38
}

pub fn overrides() -> Vec<(&'static str, *mut c_void)> {
    macro_rules! f {
        ($name:literal, $function:expr) => {
            ($name, $function as *const () as *mut c_void)
        };
    }
    vec![
        f!("ANativeWindow_fromSurface", from_surface),
        f!("ANativeWindow_acquire", acquire),
        f!("ANativeWindow_release", release),
        f!("ANativeWindow_getWidth", get_width),
        f!("ANativeWindow_getHeight", get_height),
        f!("ANativeWindow_getFormat", get_format),
        f!("ANativeWindow_setBuffersGeometry", set_buffers_geometry),
        f!("ANativeWindow_lock", lock),
        f!("ANativeWindow_unlockAndPost", unlock_and_post),
    ]
}
