/// Drive AGDK `GameActivity` bring-up.
///
/// The module keeps lifecycle, settings, input and surface calls separate in
/// source files while preserving one shared FFI state and public API.
pub mod game_activity {
    use std::ffi::{CString, c_char, c_int, c_void};

    include!("bootstrap.rs");
    include!("display.rs");
    include!("input.rs");
    include!("lifecycle_text.rs");
    include!("surface.rs");
}
