//! Feed the VM's invented answers into the runtime's gap inventory.
//!
//! The C++ path bridges libjnivm's `Constructed Unresolved symbol` log line to
//! `roblox_unimplemented_record` through `crates/native-sys/native/jnivm_logger/log.cpp`. This
//! module is the Rust VM's equivalent, without the message-matching fragility:
//! the call sites record at the exact moment the VM invents a registration,
//! with libjnivm's wording preserved so the report's rows stay comparable
//! across backends.
//!
//! `roblox_unimplemented_record` is resolved through the dynamic loader rather
//! than an extern declaration: this crate stays linkable on its own (its tests
//! do not link the runtime), while the shipping binary — which always links
//! `roblox-abi` — resolves the real recorder. `Kind::Jni` is code 0, matching
//! `roblox-abi`'s `Kind::from_code`.

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::OnceLock;

type RecordFn = unsafe extern "C" fn(kind: u32, detail: *const c_char);

static RECORD: OnceLock<Option<RecordFn>> = OnceLock::new();

/// The code `roblox-abi`'s `Kind::from_code` maps to `Kind::Jni`.
const KIND_JNI: u32 = 0;

fn record() -> Option<&'static RecordFn> {
    RECORD
        .get_or_init(|| {
            let name = CString::new("roblox_unimplemented_record").ok()?;
            // SAFETY: `name` is a live NUL-terminated string for this call; the
            // result is only transmuted when it is non-null, and the recorder is
            // an `unsafe extern "C" fn(u32, *const c_char)` exactly as declared.
            let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
            if address.is_null() {
                return None;
            }
            Some(unsafe { std::mem::transmute::<*mut std::ffi::c_void, RecordFn>(address) })
        })
        .as_ref()
}

/// libjnivm's exact marker wording, kept so a Rust-VM report reads the same
/// as a C++-VM report side by side.
const MARKER: &str = "Constructed Unresolved symbol";

fn record_detail(detail: String) {
    let Some(record) = record() else {
        return;
    };
    let Ok(text) = CString::new(detail) else {
        return;
    };
    // SAFETY: `text` is a live NUL-terminated string for the duration of the
    // synchronous call; the recorder copies it before returning.
    unsafe { record(KIND_JNI, text.as_ptr()) };
}

/// Record a method the engine looked up and the VM had to invent.
pub fn record_unresolved_method(class: &str, name: &str, descriptor: &str, is_static: bool) {
    let prefix = if is_static { "Static" } else { "" };
    record_detail(format!(
        "{MARKER}, Class=`{class}`, {prefix}Method=`{name}`, Signature=`{descriptor}`"
    ));
}

/// Record a field the engine looked up and the VM had to invent.
pub fn record_unresolved_field(class: &str, name: &str, descriptor: &str, is_static: bool) {
    let prefix = if is_static { "Static" } else { "" };
    record_detail(format!(
        "{MARKER}, Class=`{class}`, {prefix}Field=`{name}`, Signature=`{descriptor}`"
    ));
}
