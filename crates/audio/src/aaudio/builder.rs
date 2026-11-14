//! AAudioStreamBuilder: the values the engine can set through the builder
//! surface.
//!
//! There is no `setSampleRate` and no `setChannelCount` among the 25 symbols
//! the engine looks up, so there are no such fields here either: rate and
//! channel count are chosen by the host (output) or declared by the input
//! path, never asked. Defaults mirror what a fresh builder holds before any
//! setter runs.

use std::ffi::c_void;
use crate::types::*;

pub type AAudioStreamDataCallback =
    Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, i32) -> AAudioDataCallbackResult>;
pub type AAudioStreamErrorCallback =
    Option<unsafe extern "C" fn(*mut c_void, *mut c_void, AAudioResult)>;

#[derive(Debug, Clone, Copy)]
pub struct StreamBuilder {
    pub direction: AAudioDirection,
    pub format: AAudioFormat,
    pub buffer_capacity_in_frames: i32,
    pub performance_mode: AAudioPerformanceMode,
    pub usage: AAudioUsage,
    pub input_preset: AAudioInputPreset,
    pub data_callback: AAudioStreamDataCallback,
    pub error_callback: AAudioStreamErrorCallback,
    /// The engine's `user_data` for each callback, stored to be handed back
    /// verbatim and never dereferenced here.
    pub(crate) data_user: *mut c_void,
    pub(crate) error_user: *mut c_void,
}

impl Default for StreamBuilder {
    fn default() -> Self {
        Self {
            direction: AAUDIO_DIRECTION_OUTPUT,
            format: AAUDIO_FORMAT_UNSPECIFIED,
            buffer_capacity_in_frames: 0,
            performance_mode: 0,
            usage: 0,
            input_preset: 0,
            data_callback: None,
            error_callback: None,
            data_user: std::ptr::null_mut(),
            error_user: std::ptr::null_mut(),
        }
    }
}

// SAFETY: the two raw pointers are the engine's `user_data`, stored only to
// be handed back verbatim to the engine's own callbacks. Nothing here
// dereferences them on its own authority.
unsafe impl Send for StreamBuilder {}
