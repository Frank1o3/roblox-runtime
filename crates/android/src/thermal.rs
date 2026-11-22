//! Android NDK thermal APIs on a desktop host.
//!
//! The Linux host has no Android thermal service. Report a neutral status and
//! zero normalized headroom, while providing a stable opaque manager handle so
//! clients can safely query/release it. This surface intentionally does not
//! emulate thermal listeners or Android device thresholds.

use std::ffi::c_void;

#[repr(C)]
struct ThermalHeadroomThreshold {
    thermal_status: i32,
    headroom: f32,
}

static MANAGER_TOKEN: u8 = 1;

fn manager_handle() -> *mut c_void {
    (&MANAGER_TOKEN as *const u8).cast_mut().cast()
}

fn valid_manager(manager: *mut c_void) -> bool {
    manager == manager_handle()
}

extern "C" fn acquire_manager() -> *mut c_void {
    manager_handle()
}

extern "C" fn release_manager(_manager: *mut c_void) {}

extern "C" fn get_current_thermal_status(manager: *mut c_void) -> i32 {
    if valid_manager(manager) { 0 } else { -1 }
}

extern "C" fn get_thermal_headroom(manager: *mut c_void, _forecast_seconds: i32) -> f32 {
    if valid_manager(manager) { 0.0 } else { -1.0 }
}

extern "C" fn get_thermal_headroom_thresholds(
    manager: *mut c_void,
    out_thresholds: *mut *const ThermalHeadroomThreshold,
    count: *mut usize,
) -> i32 {
    if count.is_null() {
        return -1;
    }
    // SAFETY: the NDK API requires a writable count pointer.
    unsafe { count.write(0) };
    if !out_thresholds.is_null() {
        // SAFETY: the caller supplied this optional writable output pointer.
        unsafe { out_thresholds.write(std::ptr::null()) };
    }
    if valid_manager(manager) { 0 } else { -1 }
}

/// Export the Android NDK functions observed through `dlsym` by the current APK.
pub fn overrides() -> Vec<(&'static str, *mut c_void)> {
    vec![
        (
            "AThermal_acquireManager",
            acquire_manager as *const () as *mut c_void,
        ),
        (
            "AThermal_releaseManager",
            release_manager as *const () as *mut c_void,
        ),
        (
            "AThermal_getCurrentThermalStatus",
            get_current_thermal_status as *const () as *mut c_void,
        ),
        (
            "AThermal_getThermalHeadroom",
            get_thermal_headroom as *const () as *mut c_void,
        ),
        (
            "AThermal_getThermalHeadroomThresholds",
            get_thermal_headroom_thresholds as *const () as *mut c_void,
        ),
    ]
}
