/// Drive AGDK `GameActivity` bring-up.
///
/// The module keeps lifecycle, settings, input and surface calls separate in
/// source files while preserving one shared FFI state and public API.
pub mod game_activity {
    use std::ffi::{CString, c_char, c_int, c_void};

    /// Android's client registers these native flag names during startup.
    /// Captured from the matching APK's startup trace; absent names are
    /// harmlessly reported as unknown by the engine.
    pub const NATIVE_FLAG_NAMES: &str = include_str!("../../data/native-flag-names.txt");

    include!("bootstrap.rs");
    include!("services.rs");
    include!("display.rs");
    include!("input.rs");
    include!("lifecycle_text.rs");
    include!("surface.rs");
}
