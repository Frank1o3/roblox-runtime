//! Unsupported NDK image-reader APIs used by Android video decoding.
//!
//! Roblox 2.746 imports these symbols before startup. The runtime has no
//! `AImageReader` implementation, so creation must fail explicitly instead of
//! returning success with an uninitialised reader pointer.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

const AMEDIA_ERROR_BASE: i32 = -10_000;
const AMEDIA_ERROR_UNSUPPORTED: i32 = AMEDIA_ERROR_BASE - 2;
const AMEDIA_ERROR_INVALID_OBJECT: i32 = AMEDIA_ERROR_BASE - 3;

static REPORTED: AtomicBool = AtomicBool::new(false);

extern "C" fn reader_new_with_usage(
    width: i32,
    height: i32,
    format: i32,
    usage: u64,
    max_images: i32,
    reader: *mut *mut c_void,
) -> i32 {
    if !reader.is_null() {
        // SAFETY: the NDK contract supplies a writable reader out-pointer.
        unsafe { *reader = std::ptr::null_mut() };
    }
    if !REPORTED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "[media] AImageReader_newWithUsage({width}x{height}, format {format:#x}, usage {usage:#x}, {max_images} images) refused: image readers are unsupported"
        );
    }
    AMEDIA_ERROR_UNSUPPORTED
}

extern "C" fn reader_delete(_reader: *mut c_void) {}

extern "C" fn reader_get_window(_reader: *mut c_void, window: *mut *mut c_void) -> i32 {
    if !window.is_null() {
        // SAFETY: the NDK contract supplies a writable window out-pointer.
        unsafe { *window = std::ptr::null_mut() };
    }
    AMEDIA_ERROR_INVALID_OBJECT
}

extern "C" fn reader_acquire_next_image(_reader: *mut c_void, image: *mut *mut c_void) -> i32 {
    if !image.is_null() {
        // SAFETY: the NDK contract supplies a writable image out-pointer.
        unsafe { *image = std::ptr::null_mut() };
    }
    AMEDIA_ERROR_INVALID_OBJECT
}

extern "C" fn image_delete(_image: *mut c_void) {}

extern "C" fn image_get_hardware_buffer(_image: *const c_void, buffer: *mut *mut c_void) -> i32 {
    if !buffer.is_null() {
        // SAFETY: the NDK contract supplies a writable buffer out-pointer.
        unsafe { *buffer = std::ptr::null_mut() };
    }
    AMEDIA_ERROR_INVALID_OBJECT
}

extern "C" fn image_get_timestamp(_image: *const c_void, _timestamp: *mut i64) -> i32 {
    AMEDIA_ERROR_INVALID_OBJECT
}

extern "C" fn hardware_buffer_acquire(_buffer: *mut c_void) {}
extern "C" fn hardware_buffer_release(_buffer: *mut c_void) {}

/// Export the NDK symbols Roblox imports while keeping video support explicitly
/// unavailable until the runtime can produce real image-reader objects.
pub fn overrides() -> Vec<(&'static str, *mut c_void)> {
    macro_rules! f {
        ($name:literal, $function:expr) => {
            ($name, $function as *const () as *mut c_void)
        };
    }

    vec![
        f!("AImageReader_newWithUsage", reader_new_with_usage),
        f!("AImageReader_delete", reader_delete),
        f!("AImageReader_getWindow", reader_get_window),
        f!("AImageReader_acquireNextImage", reader_acquire_next_image),
        f!("AImage_delete", image_delete),
        f!("AImage_getHardwareBuffer", image_get_hardware_buffer),
        f!("AImage_getTimestamp", image_get_timestamp),
        f!("AHardwareBuffer_acquire", hardware_buffer_acquire),
        f!("AHardwareBuffer_release", hardware_buffer_release),
    ]
}
