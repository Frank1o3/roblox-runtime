//! Audio log lines in the Android `P/Tag message` shape, with a priority
//! floor read from the environment.
//!
//! The engine's own log output is the single most useful diagnostic when a
//! stream misbehaves, so audio narration keeps the shape a reader is used to
//! from Android logs rather than an ad-hoc prefix. The floor policy mirrors
//! the runtime's liblog policy (own re-derivation): INFO by default — a
//! default that silently hides a whole severity has already cost this project
//! one investigation by turning an absence into false evidence — VERBOSE
//! behind an explicit opt-in.

use std::sync::OnceLock;

/// The tag every audio line carries, so the audio narration reads as one
/// column-aligned block in the merged log.
pub const AUDIO_TAG: &str = "RobloxRuntime-Audio";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Verbose,
    Debug,
    Info,
    Warn,
    Error,
}

impl Priority {
    fn letter(self) -> char {
        match self {
            Priority::Verbose => 'V',
            Priority::Debug => 'D',
            Priority::Info => 'I',
            Priority::Warn => 'W',
            Priority::Error => 'E',
        }
    }
}

/// Minimum priority actually printed. `RUSTY_BLOX_LOG_LEVEL` (and its alias
/// `RBX_RUNTIME_LOG_LEVEL`) select 1=WARN, 2=INFO, 3=DEBUG, 4=VERBOSE;
/// anything else means the default, INFO.
fn minimum_priority() -> Priority {
    static FLOOR: OnceLock<Priority> = OnceLock::new();
    *FLOOR.get_or_init(|| {
        let raw = std::env::var("RUSTY_BLOX_LOG_LEVEL")
            .or_else(|_| std::env::var("RBX_RUNTIME_LOG_LEVEL"))
            .unwrap_or_default();
        match raw.as_str() {
            "1" => Priority::Warn,
            "3" => Priority::Debug,
            "4" => Priority::Verbose,
            _ => Priority::Info,
        }
    })
}

/// Emit one android-shaped line if the priority passes the floor.
pub fn log(prio: Priority, tag: &str, message: &str) {
    if prio < minimum_priority() {
        return;
    }
    // Tag width 30 matches the shape the runtime's audio narration has used;
    // the tag column stays aligned across all audio lines.
    std::eprintln!("{}/{:<30} {message}", prio.letter(), tag);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_and_ordering_match_android_priorities() {
        assert_eq!(Priority::Verbose.letter(), 'V');
        assert_eq!(Priority::Error.letter(), 'E');
        assert!(Priority::Error > Priority::Warn);
        assert!(Priority::Verbose < Priority::Debug);
    }
}
