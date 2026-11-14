//! AAudioStreamBuilder implementation.

use std::ffi::c_void;
use crate::types::*;

pub type AAudioStreamDataCallback =
    Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, i32) -> AAudioDataCallbackResult>;
pub type AAudioStreamErrorCallback =
    Option<unsafe extern "C" fn(*mut c_void, *mut c_void, AAudioResult)>;

#[derive(Debug, Clone)]
pub struct StreamBuilder {
    pub direction: AAudioDirection,
    pub format: AAudioFormat,
    pub sample_rate: i32,
    pub channel_count: i32,
    pub buffer_capacity_in_frames: i32,
    pub performance_mode: AAudioPerformanceMode,
    pub usage: AAudioUsage,
    pub input_preset: AAudioInputPreset,
    pub data_callback: AAudioStreamDataCallback,
    pub error_callback: AAudioStreamErrorCallback,
    pub user_data: *mut c_void,
}

impl Default for StreamBuilder {
    fn default() -> Self {
        Self {
            direction: AAUDIO_DIRECTION_OUTPUT,
            format: AAUDIO_FORMAT_UNSPECIFIED,
            sample_rate: 48000,
            channel_count: 2,
            buffer_capacity_in_frames: 4096,
            performance_mode: AAUDIO_PERFORMANCE_MODE_LOW_LATENCY,
            usage: AAUDIO_USAGE_MEDIA,
            input_preset: AAUDIO_INPUT_PRESET_GENERIC,
            data_callback: None,
            error_callback: None,
            user_data: std::ptr::null_mut(),
        }
    }
}
