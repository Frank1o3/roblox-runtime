//! Host audio backend interfaces.
//!
//! Backend selection lives in [`host`]: the cpal-probe selector (a host counts
//! only if it constructs and has a default output device, because
//! `supportsAAudio()` is a one-way door). The one-implementation-per-need rule:
//! PipeWire, PulseAudio and ALSA are all served by [`cpal_output`]; OSS keeps
//! its own libc implementation in [`oss`].
pub mod capture;
pub mod cpal_output;
pub mod host;
pub mod oss;
pub mod output;
pub mod pcm;

pub use host::{effective_backend_name, host_backend_available, host_backend_name, make_output_stream};

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

use cpal::traits::{DeviceTrait, HostTrait};

/// Set by the voice-mute path (`AppRtcDeviceWrapper.wrapSetCommunicationMute`
/// and friends). The capture stream keeps recording while muted only if a
/// consumer says so — muting is an engine-side mixing decision, not a
/// microphone-teardown decision.
pub static VOICE_MUTED: AtomicBool = AtomicBool::new(false);

/// One audio endpoint as the engine's device queries see it.
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

/// Enumerate host devices through cpal. **This opens no stream of any kind** —
/// not a capture stream, not an output stream: the whole privacy gate rests on
/// enumeration being side-effect-free.
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

/// Stable FNV-style hash distinguishing direction, so the same device can be
/// both a source and a sink with two different ids.
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
