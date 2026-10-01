//! Small process-wide logging policy shared by the launcher and runtime.

/// Emit an existing log message when its category is enabled by the selected
/// level. Messages with a recognized prefix are assigned to their tier;
/// unclassified diagnostics are shown at level 4.
pub fn emit(message: String) {
    let required = if message.starts_with("[input]") {
        4
    } else if message.starts_with("[jnivm") {
        3
    } else if message.starts_with("[runtime") || message.starts_with("[android]") {
        2
    } else if message.starts_with("[roblox]") || message.starts_with("[session]") {
        1
    } else {
        4
    };

    if current_level() >= required || looks_like_error(&message) {
        std::eprintln!("{message}");
    }
}

/// Read dynamically so the settings-selected value is visible to runtime
/// threads and libraries initialized after the launcher starts.
fn current_level() -> u8 {
    std::env::var("RUSTY_BLOX_LOG_LEVEL")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|level| (1..=4).contains(level))
        .unwrap_or(1)
}

fn looks_like_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        " failed",
        "failure",
        "error:",
        "could not",
        "unavailable",
        "rejected",
        "blocked",
        "not found",
        "missing",
        "not implemented",
        "abort()",
        "stack protector",
        "log-assert",
        "panic",
    ]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Emit a startup or runtime failure independently of the selected verbosity.
pub fn emit_error(message: String) {
    std::eprintln!("{message}");
}

#[macro_export]
macro_rules! eprintln {
    () => {{
        $crate::emit(String::new());
    }};
    ($($arg:tt)*) => {{
        $crate::emit(format!($($arg)*));
    }};
}
