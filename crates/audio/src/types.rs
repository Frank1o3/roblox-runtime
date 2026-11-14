//! Common types and constants for Android Audio APIs (AAudio and OpenSL ES).

use std::os::raw::c_int;

// ----------------------------------------------------------------------------
// AAudio Constants & Types
// ----------------------------------------------------------------------------

pub type AAudioResult = i32;
pub type AAudioStreamState = i32;
pub type AAudioDirection = i32;
pub type AAudioFormat = i32;
pub type AAudioDataCallbackResult = i32;
pub type AAudioPerformanceMode = i32;
pub type AAudioUsage = i32;
pub type AAudioInputPreset = i32;

pub const AAUDIO_OK: AAudioResult = 0;
pub const AAUDIO_ERROR_DISCONNECTED: AAudioResult = -899;
pub const AAUDIO_ERROR_ILLEGAL_ARGUMENT: AAudioResult = -898;
pub const AAUDIO_ERROR_INVALID_STATE: AAudioResult = -895;
pub const AAUDIO_ERROR_UNIMPLEMENTED: AAudioResult = -890;
pub const AAUDIO_ERROR_UNAVAILABLE: AAudioResult = -889;
pub const AAUDIO_ERROR_NO_MEMORY: AAudioResult = -887;
pub const AAUDIO_ERROR_NULL: AAudioResult = -886;

pub const AAUDIO_DIRECTION_OUTPUT: AAudioDirection = 0;
pub const AAUDIO_DIRECTION_INPUT: AAudioDirection = 1;

pub const AAUDIO_FORMAT_INVALID: AAudioFormat = -1;
pub const AAUDIO_FORMAT_UNSPECIFIED: AAudioFormat = 0;
pub const AAUDIO_FORMAT_PCM_I16: AAudioFormat = 1;
pub const AAUDIO_FORMAT_PCM_FLOAT: AAudioFormat = 2;
pub const AAUDIO_FORMAT_PCM_I24_PACKED: AAudioFormat = 3;
pub const AAUDIO_FORMAT_PCM_I32: AAudioFormat = 4;

pub const AAUDIO_STREAM_STATE_UNINITIALIZED: AAudioStreamState = 0;
pub const AAUDIO_STREAM_STATE_UNKNOWN: AAudioStreamState = 1;
pub const AAUDIO_STREAM_STATE_OPEN: AAudioStreamState = 2;
pub const AAUDIO_STREAM_STATE_STARTING: AAudioStreamState = 3;
pub const AAUDIO_STREAM_STATE_STARTED: AAudioStreamState = 4;
pub const AAUDIO_STREAM_STATE_PAUSING: AAudioStreamState = 5;
pub const AAUDIO_STREAM_STATE_PAUSED: AAudioStreamState = 6;
pub const AAUDIO_STREAM_STATE_FLUSHING: AAudioStreamState = 7;
pub const AAUDIO_STREAM_STATE_FLUSHED: AAudioStreamState = 8;
pub const AAUDIO_STREAM_STATE_STOPPING: AAudioStreamState = 9;
pub const AAUDIO_STREAM_STATE_STOPPED: AAudioStreamState = 10;
pub const AAUDIO_STREAM_STATE_CLOSING: AAudioStreamState = 11;
pub const AAUDIO_STREAM_STATE_CLOSED: AAudioStreamState = 12;
pub const AAUDIO_STREAM_STATE_DISCONNECTED: AAudioStreamState = 13;

pub const AAUDIO_CALLBACK_RESULT_CONTINUE: AAudioDataCallbackResult = 0;
pub const AAUDIO_CALLBACK_RESULT_STOP: AAudioDataCallbackResult = 1;

pub const AAUDIO_PERFORMANCE_MODE_NONE: AAudioPerformanceMode = 10;
pub const AAUDIO_PERFORMANCE_MODE_LOW_LATENCY: AAudioPerformanceMode = 11;
pub const AAUDIO_PERFORMANCE_MODE_POWER_SAVING: AAudioPerformanceMode = 12;

pub const AAUDIO_USAGE_MEDIA: AAudioUsage = 1;
pub const AAUDIO_INPUT_PRESET_GENERIC: AAudioInputPreset = 1;
pub const AAUDIO_INPUT_PRESET_VOICE_COMMUNICATION: AAudioInputPreset = 7;

// ----------------------------------------------------------------------------
// OpenSL ES Constants & Types
// ----------------------------------------------------------------------------

pub type SLresult = u32;
pub type SLuint32 = u32;
pub type SLint32 = i32;
pub type SLmillibel = i16;
pub type SLboolean = u32;

pub const SL_RESULT_SUCCESS: SLresult = 0;
pub const SL_RESULT_PRECONDITIONS_VIOLATED: SLresult = 1;
pub const SL_RESULT_PARAMETER_INVALID: SLresult = 2;
pub const SL_RESULT_MEMORY_FAILURE: SLresult = 3;
pub const SL_RESULT_RESOURCE_ERROR: SLresult = 4;
pub const SL_RESULT_RESOURCE_LOST: SLresult = 5;
pub const SL_RESULT_IO_ERROR: SLresult = 6;
pub const SL_RESULT_BUFFER_INSUFFICIENT: SLresult = 7;
pub const SL_RESULT_UNKNOWN_ERROR: SLresult = 8;
pub const SL_RESULT_SERVICE_UNAVAILABLE: SLresult = 9;
pub const SL_RESULT_CONTROL_LOST: SLresult = 10;
pub const SL_RESULT_FEATURE_UNSUPPORTED: SLresult = 12;

pub const SL_BOOLEAN_FALSE: SLboolean = 0;
pub const SL_BOOLEAN_TRUE: SLboolean = 1;

pub const SL_PLAYSTATE_STOPPED: SLuint32 = 1;
pub const SL_PLAYSTATE_PAUSED: SLuint32 = 2;
pub const SL_PLAYSTATE_PLAYING: SLuint32 = 3;

pub const SL_RECORDSTATE_STOPPED: SLuint32 = 1;
pub const SL_RECORDSTATE_PAUSED: SLuint32 = 2;
pub const SL_RECORDSTATE_RECORDING: SLuint32 = 3;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SLInterfaceID {
    pub time_low: u32,
    pub time_mid: u16,
    pub time_hi_and_version: u16,
    pub clock_seq: u16,
    pub node: [u8; 6],
}
