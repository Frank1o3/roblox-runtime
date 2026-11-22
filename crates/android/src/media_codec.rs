//! Unsupported Android NDK MediaCodec entry points used by optional video.
//!
//! The runtime reports no hardware codecs through its Java compatibility
//! hooks, and cannot create Android MediaCodec objects on the Linux host.
//! `createCodecByName` therefore follows the NDK failure contract and returns
//! null so video playback can be skipped cleanly.

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

static REPORTED: AtomicBool = AtomicBool::new(false);

extern "C" fn codec_create_by_name(name: *const c_char) -> *mut c_void {
    if !REPORTED.swap(true, Ordering::Relaxed) {
        let label = if name.is_null() {
            "(null)"
        } else {
            // SAFETY: the NDK contract supplies a NUL-terminated codec name.
            unsafe { std::ffi::CStr::from_ptr(name) }
                .to_str()
                .unwrap_or("(non-UTF-8)")
        };
        eprintln!(
            "[media] AMediaCodec_createCodecByName({label}) refused: MediaCodec is unsupported"
        );
    }
    std::ptr::null_mut()
}

/// Register the unsupported NDK codec factory with its exact C ABI.
pub fn overrides() -> Vec<(&'static str, *mut c_void)> {
    vec![(
        "AMediaCodec_createCodecByName",
        codec_create_by_name as *const () as *mut c_void,
    )]
}
