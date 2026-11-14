//! Which host backend this run uses. Probes are real: a host counts only if it
//! constructs and has a default output device, because `supportsAAudio()` is a
//! one-way door (FMOD does not fall back once it has been told AAudio exists).

use std::sync::OnceLock;

use cpal::traits::HostTrait;

use super::cpal_output::CpalOutputStream;
use super::oss::{make_oss_stream, oss_available};
use super::output::OutputStream;

const ORDER: [&str; 3] = ["pipewire", "pulseaudio", "alsa"];

fn normalize(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => Some("auto"),
        "pipewire" | "pw" => Some("pipewire"),
        "pulse" | "pulseaudio" => Some("pulseaudio"),
        "alsa" => Some("alsa"),
        "oss" => Some("oss"),
        "none" | "off" => Some("none"),
        _ => None,
    }
}

/// What `RBX_RUNTIME_AUDIO_HOST` asked for. Read once; "auto" when unset.
pub fn host_backend_name() -> &'static str {
    static NAME: OnceLock<&'static str> = OnceLock::new();
    NAME.get_or_init(|| {
        let raw = std::env::var("RBX_RUNTIME_AUDIO_HOST").unwrap_or_default();
        normalize(&raw).unwrap_or_else(|| {
            eprintln!(
                "W/RobloxRuntime-Audio           RBX_RUNTIME_AUDIO_HOST={raw} is not a backend this \
                 build has (pipewire, pulse, alsa, oss, none); detecting instead."
            );
            "auto"
        })
    })
}

/// Matches on the host id's Debug name, so this compiles whichever hosts the
/// enabled cpal features provide.
pub fn host_by_name(name: &str) -> Option<cpal::Host> {
    cpal::available_hosts()
        .into_iter()
        .find(|id| format!("{id:?}").eq_ignore_ascii_case(name))
        .and_then(|id| cpal::host_from_id(id).ok())
}

fn probe(name: &str) -> bool {
    host_by_name(name).is_some_and(|h| h.default_output_device().is_some())
}

/// What is actually there, resolved once. Order: PipeWire, PulseAudio, ALSA, OSS.
pub fn effective_backend_name() -> &'static str {
    static CHOSEN: OnceLock<&'static str> = OnceLock::new();
    CHOSEN.get_or_init(|| {
        let asked = host_backend_name();
        match asked {
            "none" => return "none",
            "oss" => {
                if oss_available() {
                    return "oss";
                }
                eprintln!("W/RobloxRuntime-Audio           RBX_RUNTIME_AUDIO_HOST=oss, but the device would not open; detecting instead.");
            }
            "auto" => {}
            name => {
                if probe(name) {
                    return name;
                }
                eprintln!("W/RobloxRuntime-Audio           RBX_RUNTIME_AUDIO_HOST={name}, but that host is not available or has no output device; detecting instead.");
            }
        }
        for name in ORDER {
            if probe(name) {
                return name;
            }
        }
        if oss_available() { "oss" } else { "none" }
    })
}

pub fn host_backend_available() -> bool {
    effective_backend_name() != "none"
}

/// Never fails: a host that cannot work returns a stream whose `open` fails.
pub fn make_output_stream() -> Box<dyn OutputStream> {
    match effective_backend_name() {
        "oss" => make_oss_stream(),
        name => Box::new(CpalOutputStream::new(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn names_normalize_and_unknowns_are_rejected() {
        assert_eq!(normalize(""), Some("auto"));
        assert_eq!(normalize("PulseAudio"), Some("pulseaudio"));
        assert_eq!(normalize("pulse"), Some("pulseaudio"));
        assert_eq!(normalize("PW"), Some("pipewire"));
        assert_eq!(normalize("OSS"), Some("oss"));
        assert_eq!(normalize("jack"), None);
    }
}