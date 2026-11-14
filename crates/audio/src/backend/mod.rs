//! Host audio backend interfaces (PipeWire, PulseAudio, ALSA, fallback).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
    if std::env::var_os("PIPEWIRE_RUNTIME_DIR").is_some() || std::env::var_os("XDG_RUNTIME_DIR").is_some() {
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

pub fn enumerate_devices() -> Vec<DeviceInfo> {
    // Returns default mock or discovered devices
    vec![
        DeviceInfo {
            id: 1,
            node_name: "default_playback".into(),
            description: "Default Audio Output".into(),
            nick: "Speaker".into(),
            object_path: "alsa:default".into(),
            is_source: false,
            is_default: true,
        },
        DeviceInfo {
            id: 2,
            node_name: "default_capture".into(),
            description: "Default Audio Input".into(),
            nick: "Microphone".into(),
            object_path: "alsa:capture".into(),
            is_source: true,
            is_default: true,
        },
    ]
}
