//! AAudio C ABI adapter. The engine sees AAudio's opaque pointer handles;
//! `stream` owns and validates those handles in Rust.

use std::ffi::c_void;
use std::time::{Duration, Instant};

use super::stream;
use crate::aaudio::builder::{AAudioStreamDataCallback, AAudioStreamErrorCallback};
use crate::types::*;

fn handle(pointer: *mut c_void) -> Option<usize> {
    (!pointer.is_null()).then_some(pointer as usize)
}

unsafe extern "C" fn create_stream_builder(out: *mut *mut c_void) -> AAudioResult {
    if out.is_null() {
        return AAUDIO_ERROR_NULL;
    }
    let builder = stream::create_stream_builder();
    // SAFETY: AAudio's caller supplies writable storage for the output pointer.
    unsafe { out.write(builder as *mut c_void) };
    AAUDIO_OK
}

unsafe extern "C" fn delete_stream_builder(builder: *mut c_void) -> AAudioResult {
    handle(builder).map_or(AAUDIO_ERROR_NULL, stream::delete_stream_builder)
}

unsafe extern "C" fn builder_set_direction(builder: *mut c_void, direction: AAudioDirection) {
    if let Some(builder) = handle(builder) {
        stream::builder_set_direction(builder, direction);
    }
}

unsafe extern "C" fn builder_set_format(builder: *mut c_void, format: AAudioFormat) {
    if let Some(builder) = handle(builder) {
        stream::builder_set_format(builder, format);
    }
}

unsafe extern "C" fn builder_set_buffer_capacity(builder: *mut c_void, frames: i32) {
    if let Some(builder) = handle(builder) {
        stream::builder_set_buffer_capacity_in_frames(builder, frames);
    }
}

unsafe extern "C" fn builder_set_performance_mode(builder: *mut c_void, mode: i32) {
    if let Some(builder) = handle(builder) {
        stream::builder_set_performance_mode(builder, mode);
    }
}

unsafe extern "C" fn builder_set_usage(builder: *mut c_void, usage: i32) {
    if let Some(builder) = handle(builder) {
        stream::builder_set_usage(builder, usage);
    }
}

unsafe extern "C" fn builder_set_input_preset(builder: *mut c_void, preset: i32) {
    if let Some(builder) = handle(builder) {
        stream::builder_set_input_preset(builder, preset);
    }
}

unsafe extern "C" fn builder_set_data_callback(
    builder: *mut c_void,
    callback: AAudioStreamDataCallback,
    user: *mut c_void,
) {
    if let Some(builder) = handle(builder) {
        stream::builder_set_data_callback(builder, callback, user);
    }
}

unsafe extern "C" fn builder_set_error_callback(
    builder: *mut c_void,
    callback: AAudioStreamErrorCallback,
    user: *mut c_void,
) {
    if let Some(builder) = handle(builder) {
        stream::builder_set_error_callback(builder, callback, user);
    }
}

unsafe extern "C" fn builder_open_stream(
    builder: *mut c_void,
    out: *mut *mut c_void,
) -> AAudioResult {
    let (Some(builder), false) = (handle(builder), out.is_null()) else {
        return AAUDIO_ERROR_NULL;
    };
    // SAFETY: the caller passed a writable AAudioStream* output slot.
    unsafe { out.write(std::ptr::null_mut()) };
    let mut stream_handle = None;
    let result = stream::open_stream(builder, Some(&mut stream_handle));
    if result == AAUDIO_OK {
        if let Some(stream_handle) = stream_handle {
            // SAFETY: same live output slot as above; the pointer is an opaque
            // registry key and is only interpreted by this AAudio adapter.
            unsafe { out.write(stream_handle as *mut c_void) };
        }
    }
    result
}

unsafe extern "C" fn stream_close(pointer: *mut c_void) -> AAudioResult {
    handle(pointer).map_or(AAUDIO_ERROR_NULL, stream::close_stream)
}

unsafe extern "C" fn stream_request_start(pointer: *mut c_void) -> AAudioResult {
    handle(pointer).map_or(AAUDIO_ERROR_NULL, stream::request_start)
}

unsafe extern "C" fn stream_request_pause(pointer: *mut c_void) -> AAudioResult {
    handle(pointer).map_or(AAUDIO_ERROR_NULL, stream::request_pause)
}

unsafe extern "C" fn stream_request_stop(pointer: *mut c_void) -> AAudioResult {
    handle(pointer).map_or(AAUDIO_ERROR_NULL, stream::request_stop)
}

unsafe extern "C" fn stream_get_state(pointer: *mut c_void) -> AAudioStreamState {
    handle(pointer).map_or(AAUDIO_STREAM_STATE_CLOSED, stream::get_state)
}

unsafe extern "C" fn stream_wait_for_state_change(
    pointer: *mut c_void,
    input_state: AAudioStreamState,
    next_state: *mut AAudioStreamState,
    timeout_nanoseconds: i64,
) -> AAudioResult {
    let Some(handle) = handle(pointer) else {
        return AAUDIO_ERROR_NULL;
    };
    if timeout_nanoseconds < 0 {
        return AAUDIO_ERROR_ILLEGAL_ARGUMENT;
    }
    let start = Instant::now();
    let timeout = Duration::from_nanos(timeout_nanoseconds as u64);
    loop {
        let current = stream::get_state(handle);
        if current != input_state {
            if !next_state.is_null() {
                // SAFETY: AAudio declares this optional output writable.
                unsafe { next_state.write(current) };
            }
            return AAUDIO_OK;
        }
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            if !next_state.is_null() {
                // SAFETY: AAudio declares this optional output writable.
                unsafe { next_state.write(current) };
            }
            return AAUDIO_ERROR_TIMEOUT;
        }
        std::thread::sleep((timeout - elapsed).min(Duration::from_millis(1)));
    }
}

unsafe extern "C" fn stream_get_sample_rate(pointer: *mut c_void) -> i32 {
    handle(pointer).map_or(0, stream::get_sample_rate)
}

unsafe extern "C" fn stream_get_channel_count(pointer: *mut c_void) -> i32 {
    handle(pointer).map_or(0, stream::get_channel_count)
}

unsafe extern "C" fn stream_get_format(pointer: *mut c_void) -> AAudioFormat {
    handle(pointer).map_or(AAUDIO_FORMAT_INVALID, stream::get_format)
}

unsafe extern "C" fn stream_get_frames_per_burst(pointer: *mut c_void) -> i32 {
    handle(pointer).map_or(0, stream::get_frames_per_burst)
}

unsafe extern "C" fn stream_get_buffer_capacity(pointer: *mut c_void) -> i32 {
    handle(pointer).map_or(0, stream::get_buffer_capacity_in_frames)
}

unsafe extern "C" fn stream_get_buffer_size(pointer: *mut c_void) -> i32 {
    handle(pointer).map_or(0, stream::get_buffer_size_in_frames)
}

unsafe extern "C" fn stream_set_buffer_size(pointer: *mut c_void, frames: i32) -> AAudioResult {
    handle(pointer).map_or(AAUDIO_ERROR_NULL, |handle| {
        stream::set_buffer_size_in_frames(handle, frames)
    })
}

unsafe extern "C" fn stream_get_xrun_count(pointer: *mut c_void) -> i32 {
    handle(pointer).map_or(0, stream::get_xrun_count)
}

unsafe extern "C" fn stream_read(
    pointer: *mut c_void,
    buffer: *mut c_void,
    frames: i32,
    timeout_ns: i64,
) -> AAudioResult {
    handle(pointer).map_or(AAUDIO_ERROR_NULL, |handle| {
        stream::read_stream(handle, buffer, frames, timeout_ns)
    })
}

/// The guest's virtual `libaaudio.so` symbol table.
pub fn symbols() -> Vec<(&'static str, *mut c_void)> {
    [
        ("AAudio_createStreamBuilder", create_stream_builder as *const () as *mut c_void),
        ("AAudioStreamBuilder_delete", delete_stream_builder as *const () as *mut c_void),
        ("AAudioStreamBuilder_openStream", builder_open_stream as *const () as *mut c_void),
        ("AAudioStreamBuilder_setBufferCapacityInFrames", builder_set_buffer_capacity as *const () as *mut c_void),
        ("AAudioStreamBuilder_setDataCallback", builder_set_data_callback as *const () as *mut c_void),
        ("AAudioStreamBuilder_setDirection", builder_set_direction as *const () as *mut c_void),
        ("AAudioStreamBuilder_setErrorCallback", builder_set_error_callback as *const () as *mut c_void),
        ("AAudioStreamBuilder_setFormat", builder_set_format as *const () as *mut c_void),
        ("AAudioStreamBuilder_setInputPreset", builder_set_input_preset as *const () as *mut c_void),
        ("AAudioStreamBuilder_setPerformanceMode", builder_set_performance_mode as *const () as *mut c_void),
        ("AAudioStreamBuilder_setUsage", builder_set_usage as *const () as *mut c_void),
        ("AAudioStream_close", stream_close as *const () as *mut c_void),
        ("AAudioStream_getBufferCapacityInFrames", stream_get_buffer_capacity as *const () as *mut c_void),
        ("AAudioStream_getBufferSizeInFrames", stream_get_buffer_size as *const () as *mut c_void),
        ("AAudioStream_getChannelCount", stream_get_channel_count as *const () as *mut c_void),
        ("AAudioStream_getFormat", stream_get_format as *const () as *mut c_void),
        ("AAudioStream_getFramesPerBurst", stream_get_frames_per_burst as *const () as *mut c_void),
        ("AAudioStream_getSampleRate", stream_get_sample_rate as *const () as *mut c_void),
        ("AAudioStream_getState", stream_get_state as *const () as *mut c_void),
        ("AAudioStream_waitForStateChange", stream_wait_for_state_change as *const () as *mut c_void),
        ("AAudioStream_getXRunCount", stream_get_xrun_count as *const () as *mut c_void),
        ("AAudioStream_read", stream_read as *const () as *mut c_void),
        ("AAudioStream_requestPause", stream_request_pause as *const () as *mut c_void),
        ("AAudioStream_requestStart", stream_request_start as *const () as *mut c_void),
        ("AAudioStream_requestStop", stream_request_stop as *const () as *mut c_void),
        ("AAudioStream_setBufferSizeInFrames", stream_set_buffer_size as *const () as *mut c_void),
    ]
    .into_iter()
    .collect()
}

/// Native JNI hooks query the same API-backend choice through this C ABI
/// bridge until those remaining hooks move into Rust.
#[unsafe(no_mangle)]
pub extern "C" fn roblox_audio_backend_is_aaudio() -> i32 {
    i32::from(stream::aaudio_selected())
}

/// Native JNI hooks use the same host availability decision as the Rust
/// AAudio stream surface until those hooks are ported.
#[unsafe(no_mangle)]
pub extern "C" fn roblox_audio_host_backend_available() -> i32 {
    i32::from(stream::audio_host_backend_available())
}
