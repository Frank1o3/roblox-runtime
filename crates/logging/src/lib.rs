//! Small process-wide logging policy shared by the launcher and runtime.

/// Emit a message when its category is enabled in RUSTY_BLOX_LOG_CATEGORIES.
pub fn emit(message: String) {
    let has_category = message.starts_with('[') && message.contains(']');
    if category_enabled(&message) || (!has_category && looks_like_error(&message)) {
        std::eprintln!("{message}");
    }
}

fn category_enabled(message: &str) -> bool {
    let category = message
        .strip_prefix('[')
        .and_then(|value| value.split(']').next())
        .unwrap_or("base");
    let category = if category == "input" {
        "input"
    } else if category.starts_with("jnivm") || category == "jni" {
        "jnivm"
    } else if category.starts_with("linker") {
        "linker"
    } else if category.starts_with("stub") {
        "stubs"
    } else if category.starts_with("runtime-webview") || category == "webview" {
        "webview"
    } else if category == "session" {
        "session"
    } else if category == "roblox" {
        "base"
    } else if category.starts_with("runtime") || category == "android" {
        "runtime"
    } else {
        "base"
    };
    std::env::var("RUSTY_BLOX_LOG_CATEGORIES")
        .unwrap_or_else(|_| "base,runtime".to_owned())
        .split(',')
        .any(|enabled| enabled.trim() == category)
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
