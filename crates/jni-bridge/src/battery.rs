//! JNI representation of Android battery readings for Roblox.
//!
//! The operating-system scan and Android unit/enum interpretation live in
//! `roblox-android::battery`. This module only shapes those values as the
//! boxed Java fields expected by Roblox's `BatteryStatus` object.

use jnivm::{JniValue, ObjectId, ObjectValue, ThreadEnv, Vm};
use std::ffi::c_void;

const STATUS_CLASS: &str = "com/roblox/engine/jni/model/BatteryStatus";
const NATIVE_GL_CLASS: &str = "com/roblox/engine/jni/NativeGLInterface";

/// Nullable fields accepted by Roblox's `BatteryStatus` hook.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BatteryStatusFields {
    pub present: Option<bool>,
    pub percentage: Option<i32>,
    pub status: Option<i32>,
    pub health: Option<i32>,
    pub voltage_mv: Option<i32>,
    pub current_now_ua: Option<i32>,
    pub current_avg_ua: Option<i32>,
    pub charge_counter_uah: Option<i32>,
    pub power_now_uw: Option<i32>,
    pub technology: Option<String>,
    pub temperature_c: Option<f32>,
    pub plugged: Option<i32>,
}

impl From<&roblox_android::battery::Reading> for BatteryStatusFields {
    fn from(reading: &roblox_android::battery::Reading) -> Self {
        let Some(battery) = reading.battery.as_ref() else {
            return Self {
                plugged: reading.plugged,
                ..Self::default()
            };
        };
        Self {
            present: Some(battery.present),
            percentage: battery.percentage.map(i32::from),
            status: Some(battery.status),
            health: battery.health,
            voltage_mv: battery.voltage_mv,
            current_now_ua: battery.current_now_ua,
            current_avg_ua: battery.current_avg_ua,
            charge_counter_uah: battery.charge_counter_uah,
            power_now_uw: battery.power_now_uw,
            technology: battery.technology.clone(),
            temperature_c: battery
                .temperature_tenths_c
                .map(|value| value as f32 / 10.0),
            plugged: reading.plugged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roblox_android::battery::{Battery, Reading};

    #[test]
    fn reading_maps_only_observed_fields_and_converts_temperature() {
        let reading = Reading {
            battery: Some(Battery {
                present: true,
                percentage: Some(61),
                status: roblox_android::battery::status::CHARGING,
                health: None,
                voltage_mv: Some(4100),
                current_now_ua: None,
                current_avg_ua: Some(200),
                charge_counter_uah: None,
                power_now_uw: Some(820),
                technology: Some("Li-ion".into()),
                temperature_tenths_c: Some(237),
            }),
            plugged: Some(roblox_android::battery::plugged::USB),
        };
        let fields = BatteryStatusFields::from(&reading);
        assert_eq!(fields.present, Some(true));
        assert_eq!(fields.percentage, Some(61));
        assert_eq!(fields.temperature_c, Some(23.7));
        assert_eq!(fields.plugged, Some(roblox_android::battery::plugged::USB));
        assert_eq!(fields.current_now_ua, None);
        assert_eq!(fields.charge_counter_uah, None);
    }

    #[test]
    fn no_battery_keeps_the_independent_plugged_reading() {
        let reading = Reading {
            battery: None,
            plugged: Some(roblox_android::battery::plugged::AC),
        };
        assert_eq!(
            BatteryStatusFields::from(&reading).plugged,
            Some(roblox_android::battery::plugged::AC)
        );
        assert_eq!(state_changed_args(&reading), None);
    }
}

/// The two-int callback has no representation for a missing battery, so it is
/// skipped when no present battery can provide a status.
pub fn state_changed_args(reading: &roblox_android::battery::Reading) -> Option<(i32, i32)> {
    roblox_android::battery::state_changed_args(reading)
}

/// Register only the boxed values and `BatteryStatus` fields used by the
/// reporting callback. Unknown fields retain the VM's ordinary null/default
/// behavior.
pub(crate) fn register_hooks(vm: &Vm) -> Result<(), String> {
    for (class_name, descriptor, method, result) in [
        ("java/lang/Integer", "I", "intValue", "()I"),
        ("java/lang/Boolean", "Z", "booleanValue", "()Z"),
        ("java/lang/Long", "J", "longValue", "()J"),
        ("java/lang/Float", "F", "floatValue", "()F"),
    ] {
        let class = vm.register_class(class_name).map_err(|e| e.to_string())?;
        vm.register_field(class, "value", descriptor, false)
            .map_err(|e| e.to_string())?;
        let method = vm
            .register_method(class, method, result, false)
            .map_err(|e| e.to_string())?;
        vm.install_method_handler(method, boxed_value)
            .map_err(|e| e.to_string())?;
    }

    let status = vm.register_class(STATUS_CLASS).map_err(|e| e.to_string())?;
    for (name, descriptor) in [
        ("present", "Ljava/lang/Boolean;"),
        ("batteryPercentage", "Ljava/lang/Integer;"),
        ("batterySaverMode", "Ljava/lang/Boolean;"),
        ("chargeCounter", "Ljava/lang/Integer;"),
        ("currentAverage", "Ljava/lang/Integer;"),
        ("currentNow", "Ljava/lang/Integer;"),
        ("energyCounter", "Ljava/lang/Long;"),
        ("health", "Ljava/lang/Integer;"),
        ("plugged", "Ljava/lang/Integer;"),
        ("power", "Ljava/lang/Integer;"),
        ("batteryLow", "Ljava/lang/Boolean;"),
        ("status", "Ljava/lang/Integer;"),
        ("technology", "Ljava/lang/String;"),
        ("temperature", "Ljava/lang/Float;"),
        ("voltage", "Ljava/lang/Integer;"),
    ] {
        vm.register_field(status, name, descriptor, false)
            .map_err(|e| e.to_string())?;
    }
    vm.register_class(NATIVE_GL_CLASS)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn boxed_value(vm: &Vm, receiver: Option<ObjectId>, _args: &[JniValue]) -> JniValue {
    let Some(receiver) = receiver else {
        return JniValue::Void;
    };
    let Some(class_name) = vm.object_class_name(receiver.raw()) else {
        return JniValue::Void;
    };
    let Some(descriptor) = (match class_name.as_str() {
        "java/lang/Integer" => Some("I"),
        "java/lang/Boolean" => Some("Z"),
        "java/lang/Long" => Some("J"),
        "java/lang/Float" => Some("F"),
        _ => None,
    }) else {
        return JniValue::Void;
    };
    let Ok(class) = vm.find_or_define_class(&class_name) else {
        return JniValue::Void;
    };
    let Ok(field) = vm.find_field(class, "value", descriptor, false) else {
        return JniValue::Void;
    };
    vm.field_value(field, Some(receiver))
        .unwrap_or(JniValue::Void)
}

/// Call Roblox's exported state-change JNI native with Rust VM handles.
///
/// # Safety
/// `native` must be the live `reportBatteryStateChanged(II)V` export from the
/// mapped Roblox library and must remain mapped for this call.
pub unsafe fn report_battery_state_changed(
    native: *mut c_void,
    status: i32,
    plugged: i32,
) -> Result<(), String> {
    if native.is_null() {
        return Err("reportBatteryStateChanged native is null".into());
    }
    let vm = jnivm::process_vm().ok_or("Rust JavaVM has not been created")?;
    let env = jnivm::current_env().ok_or("current JNIEnv is unavailable")?;
    let class = vm
        .find_or_define_class(NATIVE_GL_CLASS)
        .map_err(|e| e.to_string())?;
    type Native = unsafe extern "system" fn(*mut c_void, *mut c_void, i32, i32);
    // SAFETY: the caller supplies the matching live JNI native export.
    unsafe {
        std::mem::transmute::<*mut c_void, Native>(native)(
            env,
            class.raw() as usize as *mut c_void,
            status,
            plugged,
        )
    };
    Ok(())
}

/// Call Roblox's exported `reportBatteryStatus` JNI native with an object
/// assembled from the shared Android reading data. Each unsupported sysfs
/// value remains Java null.
///
/// # Safety
/// `native` must be the live `reportBatteryStatus(BatteryStatus)V` export from
/// the mapped Roblox library and must remain mapped for this call.
pub unsafe fn report_battery_status(
    native: *mut c_void,
    fields: &BatteryStatusFields,
) -> Result<(), String> {
    if native.is_null() {
        return Err("reportBatteryStatus native is null".into());
    }
    let vm = jnivm::process_vm().ok_or("Rust JavaVM has not been created")?;
    let thread = vm.get_env().unwrap_or_else(|| vm.attach_current_thread());
    vm.push_local_frame(&thread, 32)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let status_class = vm
            .find_or_define_class(STATUS_CLASS)
            .map_err(|e| e.to_string())?;
        let native_class = vm
            .find_or_define_class(NATIVE_GL_CLASS)
            .map_err(|e| e.to_string())?;
        let status = vm
            .new_local_object(&thread, status_class, ObjectValue::Opaque)
            .map_err(|e| e.to_string())?;

        set_optional_boxed(
            vm,
            &thread,
            status,
            "present",
            fields
                .present
                .map(|v| ("java/lang/Boolean", JniValue::Boolean(v))),
        )?;
        set_optional_boxed(
            vm,
            &thread,
            status,
            "batteryPercentage",
            fields
                .percentage
                .map(|v| ("java/lang/Integer", JniValue::Int(v))),
        )?;
        set_optional_boxed(
            vm,
            &thread,
            status,
            "status",
            fields
                .status
                .map(|v| ("java/lang/Integer", JniValue::Int(v))),
        )?;
        set_optional_boxed(
            vm,
            &thread,
            status,
            "health",
            fields
                .health
                .map(|v| ("java/lang/Integer", JniValue::Int(v))),
        )?;
        set_optional_boxed(
            vm,
            &thread,
            status,
            "voltage",
            fields
                .voltage_mv
                .map(|v| ("java/lang/Integer", JniValue::Int(v))),
        )?;
        set_optional_boxed(
            vm,
            &thread,
            status,
            "currentNow",
            fields
                .current_now_ua
                .map(|v| ("java/lang/Integer", JniValue::Int(v))),
        )?;
        set_optional_boxed(
            vm,
            &thread,
            status,
            "currentAverage",
            fields
                .current_avg_ua
                .map(|v| ("java/lang/Integer", JniValue::Int(v))),
        )?;
        set_optional_boxed(
            vm,
            &thread,
            status,
            "chargeCounter",
            fields
                .charge_counter_uah
                .map(|v| ("java/lang/Integer", JniValue::Int(v))),
        )?;
        set_optional_boxed(
            vm,
            &thread,
            status,
            "power",
            fields
                .power_now_uw
                .map(|v| ("java/lang/Integer", JniValue::Int(v))),
        )?;
        set_optional_boxed(
            vm,
            &thread,
            status,
            "plugged",
            fields
                .plugged
                .map(|v| ("java/lang/Integer", JniValue::Int(v))),
        )?;
        set_optional_boxed(
            vm,
            &thread,
            status,
            "temperature",
            fields
                .temperature_c
                .map(|v| ("java/lang/Float", JniValue::Float(v))),
        )?;

        if let Some(text) = &fields.technology {
            let string_class = vm
                .find_or_define_class("java/lang/String")
                .map_err(|e| e.to_string())?;
            let string = vm
                .new_local_object(
                    &thread,
                    string_class,
                    ObjectValue::String(text.encode_utf16().collect()),
                )
                .map_err(|e| e.to_string())?;
            set_object_field(vm, status, "technology", "Ljava/lang/String;", string)?;
        }

        type Native = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void);
        // SAFETY: the caller supplies the matching live JNI native export; all
        // arguments are references owned by this VM and this local frame.
        unsafe {
            std::mem::transmute::<*mut c_void, Native>(native)(
                jnivm::current_env().ok_or("current JNIEnv is unavailable")?,
                native_class.raw() as usize as *mut c_void,
                status.raw() as usize as *mut c_void,
            )
        };
        Ok(())
    })();
    let pop = vm.pop_local_frame(&thread, None).map_err(|e| e.to_string());
    match (result, pop) {
        (Err(error), _) => Err(error),
        (Ok(()), Ok(_)) => Ok(()),
        (Ok(()), Err(error)) => Err(error),
    }
}

fn set_optional_boxed(
    vm: &Vm,
    thread: &ThreadEnv,
    receiver: ObjectId,
    field_name: &str,
    value: Option<(&str, JniValue)>,
) -> Result<(), String> {
    if let Some((class_name, value)) = value {
        let descriptor = match class_name {
            "java/lang/Boolean" => "Z",
            "java/lang/Integer" => "I",
            "java/lang/Long" => "J",
            "java/lang/Float" => "F",
            _ => return Err(format!("unsupported boxed battery type {class_name}")),
        };
        let boxed_class = vm
            .find_or_define_class(class_name)
            .map_err(|e| e.to_string())?;
        let boxed = vm
            .new_local_object(thread, boxed_class, ObjectValue::Opaque)
            .map_err(|e| e.to_string())?;
        let value_field = vm
            .find_field(boxed_class, "value", descriptor, false)
            .map_err(|e| e.to_string())?;
        vm.set_field_value(value_field, Some(boxed), value)
            .map_err(|e| e.to_string())?;
        let field_descriptor = format!("L{class_name};");
        set_object_field(vm, receiver, field_name, &field_descriptor, boxed)?;
    }
    Ok(())
}

fn set_object_field(
    vm: &Vm,
    receiver: ObjectId,
    name: &str,
    descriptor: &str,
    value: ObjectId,
) -> Result<(), String> {
    let class = vm
        .find_or_define_class(STATUS_CLASS)
        .map_err(|e| e.to_string())?;
    let field = vm
        .find_field(class, name, descriptor, false)
        .map_err(|e| e.to_string())?;
    vm.set_field_value(field, Some(receiver), JniValue::Object(Some(value)))
        .map_err(|e| e.to_string())
}
