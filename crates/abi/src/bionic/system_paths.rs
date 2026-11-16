//! Rust path redirections for fixed-arity libc calls under Android `/system`.
//!
//! Variadic `open`/`__open_2` and structure-sensitive `stat`/`realpath`/
//! `statvfs` remain in `native-sys`. The fixed-arity operations here preserve
//! libc behavior while rewriting paths beneath `/system`.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

const PATH_MAX: usize = 4096;
static TRACE_PATHS: AtomicBool = AtomicBool::new(false);

pub fn set_trace(enabled: bool) {
    TRACE_PATHS.store(enabled, Ordering::Release);
}

fn remapped_path(path: &CStr) -> Option<CString> {
    let root = roblox_android::system::system_root()?;
    let root = root.to_string_lossy();
    remap_with_root(path, root.as_bytes())
}

unsafe fn remapped_path_arg(path: *const c_char) -> Option<CString> {
    if path.is_null() {
        return None;
    }
    // SAFETY: the libc caller must provide a valid C string for this call.
    let path = unsafe { CStr::from_ptr(path) };
    remapped_path(path)
}

fn remap_with_root(path: &CStr, root: &[u8]) -> Option<CString> {
    let bytes = path.to_bytes();
    if !bytes.starts_with(b"/system/") {
        return None;
    }
    let mapped_len = root.len().checked_add(bytes.len() - b"/system".len())?;
    if mapped_len + 1 > PATH_MAX {
        return None;
    }
    let mut mapped = Vec::with_capacity(root.len() + bytes.len() - b"/system".len());
    mapped.extend_from_slice(root);
    mapped.extend_from_slice(&bytes[b"/system".len()..]);
    CString::new(mapped).ok()
}

fn trace_path(call: &str, path: *const c_char, result: impl std::fmt::Display) {
    if TRACE_PATHS.load(Ordering::Acquire) {
        let tid = unsafe { libc::syscall(libc::SYS_gettid) };
        let shown = if path.is_null() {
            "(null)".into()
        } else {
            // SAFETY: the path was passed to libc and remains valid through this call.
            unsafe { CStr::from_ptr(path) }
                .to_string_lossy()
                .into_owned()
        };
        std::eprintln!("[paths] tid={tid} {call}(\"{shown}\") = {result}");
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system_mkdir(path: *const c_char, mode: libc::mode_t) -> c_int {
    // SAFETY: `path` follows mkdir's C-string contract.
    let mapped = unsafe { remapped_path_arg(path) };
    let actual = mapped.as_ref().map_or(path, |p| p.as_ptr());
    // SAFETY: actual is either the caller's original pointer or a live CString.
    let result = unsafe { libc::mkdir(actual, mode) };
    trace_path("mkdir", actual, result);
    result
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system_unlink(path: *const c_char) -> c_int {
    // SAFETY: `path` follows unlink's C-string contract.
    let mapped = unsafe { remapped_path_arg(path) };
    let actual = mapped.as_ref().map_or(path, |p| p.as_ptr());
    // SAFETY: actual is either the caller's original pointer or a live CString.
    let result = unsafe { libc::unlink(actual) };
    trace_path("unlink", actual, result);
    result
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system_access(path: *const c_char, mode: c_int) -> c_int {
    // SAFETY: `path` follows access's C-string contract.
    let mapped = unsafe { remapped_path_arg(path) };
    let actual = mapped.as_ref().map_or(path, |p| p.as_ptr());
    // SAFETY: actual is either the caller's original pointer or a live CString.
    let result = unsafe { libc::access(actual, mode) };
    trace_path("access", actual, result);
    result
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system_readlink(
    path: *const c_char,
    buffer: *mut c_char,
    size: usize,
) -> libc::ssize_t {
    // SAFETY: `path` follows readlink's C-string contract.
    let mapped = unsafe { remapped_path_arg(path) };
    let actual = mapped.as_ref().map_or(path, |p| p.as_ptr());
    // SAFETY: buffer/size follow readlink's caller-provided output contract.
    let result = unsafe { libc::readlink(actual, buffer, size) };
    trace_path("readlink", actual, result);
    result
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system_fopen(path: *const c_char, mode: *const c_char) -> *mut libc::FILE {
    // SAFETY: `path` follows fopen's C-string contract.
    let mapped = unsafe { remapped_path_arg(path) };
    let actual = mapped.as_ref().map_or(path, |p| p.as_ptr());
    // SAFETY: actual and mode follow fopen's C-string contract.
    let result = unsafe { libc::fopen(actual, mode) };
    trace_path(
        "fopen",
        actual,
        if result.is_null() { "null" } else { "ok" },
    );
    result
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system_rename(old: *const c_char, new: *const c_char) -> c_int {
    // SAFETY: both paths follow rename's C-string contract.
    let old_mapped = unsafe { remapped_path_arg(old) };
    // SAFETY: both paths follow rename's C-string contract.
    let new_mapped = unsafe { remapped_path_arg(new) };
    let old_actual = old_mapped.as_ref().map_or(old, |p| p.as_ptr());
    let new_actual = new_mapped.as_ref().map_or(new, |p| p.as_ptr());
    // SAFETY: pointers remain valid through the libc call.
    let result = unsafe { libc::rename(old_actual, new_actual) };
    trace_path("rename", old_actual, result);
    result
}

pub fn overrides() -> Vec<(&'static str, *mut c_void)> {
    vec![
        ("access", system_access as *const () as *mut c_void),
        ("readlink", system_readlink as *const () as *mut c_void),
        ("fopen", system_fopen as *const () as *mut c_void),
        ("mkdir", system_mkdir as *const () as *mut c_void),
        ("unlink", system_unlink as *const () as *mut c_void),
        ("rename", system_rename as *const () as *mut c_void),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaps_only_paths_beneath_system_root() {
        let path = CString::new("/system/fonts/Roboto.ttf").unwrap();
        let mapped = remap_with_root(path.as_c_str(), b"/runtime/system").unwrap();
        assert_eq!(mapped.to_bytes(), b"/runtime/system/fonts/Roboto.ttf");

        let other = CString::new("/tmp/file").unwrap();
        assert!(remap_with_root(other.as_c_str(), b"/runtime/system").is_none());
        let prefix = CString::new("/systematic/file").unwrap();
        assert!(remap_with_root(prefix.as_c_str(), b"/runtime/system").is_none());
    }

    #[test]
    fn registers_the_fixed_arity_exports() {
        let entries = overrides();
        assert_eq!(
            entries.iter().map(|e| e.0).collect::<Vec<_>>(),
            ["access", "readlink", "fopen", "mkdir", "unlink", "rename"]
        );
        assert!(entries.iter().all(|e| !e.1.is_null()));
    }
}
