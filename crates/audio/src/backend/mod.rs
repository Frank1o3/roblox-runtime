//! Host audio backend interfaces (PipeWire, PulseAudio, ALSA, fallback).

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

use cpal::traits::{DeviceTrait, HostTrait};

pub static VOICE_MUTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: u32,
    pub node_name: String,
    pub description: String,
    pub nick: String,
    pub object_path: String,
    pub is_source: bool,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostBackendKind {
    PipeWire,
    PulseAudio,
    Alsa,
    None,
}

impl HostBackendKind {
    pub fn name(&self) -> &'static str {
        match self {
            HostBackendKind::PipeWire => "pipewire",
            HostBackendKind::PulseAudio => "pulse",
            HostBackendKind::Alsa => "alsa",
            HostBackendKind::None => "none",
        }
    }
}

pub fn host_backend_kind() -> HostBackendKind {
    if let Ok(val) = std::env::var("RBX_RUNTIME_AUDIO_HOST") {
        match val.to_ascii_lowercase().as_str() {
            "pipewire" | "pw" => return HostBackendKind::PipeWire,
            "pulse" | "pulseaudio" => return HostBackendKind::PulseAudio,
            "alsa" => return HostBackendKind::Alsa,
            "none" | "off" => return HostBackendKind::None,
            _ => {}
        }
    }
    // Auto-detection preference
    if pipewire_available() {
        HostBackendKind::PipeWire
    } else if pulse_available() {
        HostBackendKind::PulseAudio
    } else if alsa_available() {
        HostBackendKind::Alsa
    } else {
        HostBackendKind::None
    }
}

pub fn pipewire_available() -> bool {
    // Check if PipeWire runtime socket exists or libpipewire can be opened
    if std::env::var_os("PIPEWIRE_RUNTIME_DIR").is_some()
        || std::env::var_os("XDG_RUNTIME_DIR").is_some()
    {
        unsafe {
            let lib = libc::dlopen(b"libpipewire-0.3.so.0\0".as_ptr().cast(), libc::RTLD_LAZY);
            if !lib.is_null() {
                libc::dlclose(lib);
                return true;
            }
        }
    }
    false
}

pub fn pulse_available() -> bool {
    unsafe {
        let lib = libc::dlopen(b"libpulse.so.0\0".as_ptr().cast(), libc::RTLD_LAZY);
        if !lib.is_null() {
            libc::dlclose(lib);
            return true;
        }
    }
    false
}

pub fn alsa_available() -> bool {
    unsafe {
        let lib = libc::dlopen(b"libasound.so.2\0".as_ptr().cast(), libc::RTLD_LAZY);
        if !lib.is_null() {
            libc::dlclose(lib);
            return true;
        }
    }
    false
}

pub fn host_backend_available() -> bool {
    host_backend_kind() != HostBackendKind::None
}

pub fn effective_backend_name() -> &'static str {
    host_backend_kind().name()
}

pub fn host_backend_name() -> String {
    std::env::var("RBX_RUNTIME_AUDIO_HOST").unwrap_or_else(|_| "auto".to_owned())
}

pub fn enumerate_devices() -> Result<Vec<DeviceInfo>, cpal::Error> {
    let host = cpal::default_host();
    let default_input_id = host
        .default_input_device()
        .map(|device| device.id())
        .transpose()?
        .map(|id| id.to_string());
    let default_output_id = host
        .default_output_device()
        .map(|device| device.id())
        .transpose()?
        .map(|id| id.to_string());

    let mut devices = Vec::new();
    let mut ids = HashSet::new();
    for device in host.devices()? {
        let device_id = device.id()?.to_string();
        let description = device.description()?.name().to_owned();

        if device.supports_output() {
            devices.push(device_info(
                &device_id,
                &description,
                false,
                default_output_id.as_deref() == Some(&device_id),
                &mut ids,
            ));
        }
        if device.supports_input() {
            devices.push(device_info(
                &device_id,
                &description,
                true,
                default_input_id.as_deref() == Some(&device_id),
                &mut ids,
            ));
        }
    }

    Ok(devices)
}

fn device_info(
    device_id: &str,
    description: &str,
    is_source: bool,
    is_default: bool,
    ids: &mut HashSet<u32>,
) -> DeviceInfo {
    let id = unique_device_id(device_id, is_source, ids);
    DeviceInfo {
        id,
        node_name: device_id.to_owned(),
        description: description.to_owned(),
        nick: description.to_owned(),
        object_path: String::new(),
        is_source,
        is_default,
    }
}

fn unique_device_id(device_id: &str, is_source: bool, ids: &mut HashSet<u32>) -> u32 {
    let direction: &[u8] = if is_source { b"input:" } else { b"output:" };
    let mut hash = 0x811c_9dc5_u32;
    for byte in direction.iter().copied().chain(device_id.bytes()) {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    hash = hash.max(1);

    while !ids.insert(hash) {
        hash = hash.wrapping_add(1).max(1);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::unique_device_id;
    use std::collections::HashSet;

    #[test]
    fn device_ids_are_stable_and_distinguish_directions() {
        let mut ids = HashSet::new();
        let output = unique_device_id("alsa:device-1", false, &mut ids);
        let input = unique_device_id("alsa:device-1", true, &mut ids);

        assert_ne!(output, input);
        assert_eq!(
            unique_device_id("alsa:device-1", false, &mut HashSet::new()),
            output
        );
    }
}
