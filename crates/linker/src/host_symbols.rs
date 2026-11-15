//! Host dynamic symbol lookup used by the Rust ELF loader and runtime.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::{Mutex, OnceLock, RwLock};

const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;

/// Look up a symbol in the host process, then in host libc.
pub fn runtime_symbol(symbol: &str) -> Option<*mut c_void> {
    let name = CString::new(symbol).ok()?;
    // SAFETY: null is RTLD_DEFAULT on supported Linux hosts; `name` is NUL-terminated.
    let address = unsafe { host_ffi::dlsym(std::ptr::null_mut(), name.as_ptr()) };
    if !address.is_null() {
        Some(address)
    } else {
        host_symbol("libc.so.6", symbol)
    }
}

/// Look up a symbol defined by the selected host library.
///
/// Handles stay open because callers may retain addresses in guest symbol tables.
pub fn host_symbol(library: &str, symbol: &str) -> Option<*mut c_void> {
    static LIBRARIES: OnceLock<Mutex<BTreeMap<String, usize>>> = OnceLock::new();
    static SYMBOLS: OnceLock<RwLock<BTreeMap<String, BTreeMap<String, Option<usize>>>>> =
        OnceLock::new();
    let symbols = SYMBOLS.get_or_init(|| RwLock::new(BTreeMap::new()));
    if let Some(address) = symbols
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(library)
        .and_then(|entries| entries.get(symbol))
        .copied()
    {
        return address.map(|address| address as *mut c_void);
    }

    let library_name = CString::new(library).ok()?;
    let symbol_name = CString::new(symbol).ok()?;
    let mut libraries = LIBRARIES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let handle = if let Some(handle) = libraries.get(library) {
        *handle as *mut c_void
    } else {
        // SAFETY: `library_name` is NUL-terminated; the handle is process-lived.
        let handle = unsafe { host_ffi::dlopen(library_name.as_ptr(), RTLD_NOW | RTLD_GLOBAL) };
        if handle.is_null() {
            std::ptr::null_mut()
        } else {
            libraries.insert(library.to_owned(), handle as usize);
            handle
        }
    };
    let result = if handle.is_null() {
        None
    } else {
        // SAFETY: handle remains live and the symbol name is NUL-terminated.
        let address = unsafe { host_ffi::dlsym(handle, symbol_name.as_ptr()) };
        if address.is_null() {
            None
        } else {
            let mut info = host_ffi::DlInfo::default();
            // SAFETY: `address` was returned by the host loader and `info` is writable.
            if unsafe { host_ffi::dladdr(address, &mut info) } == 0 || info.filename.is_null() {
                None
            } else {
                // SAFETY: successful dladdr returns a live, NUL-terminated filename.
                let filename = unsafe { CStr::from_ptr(info.filename) }.to_string_lossy();
                let basename = filename.rsplit('/').next().unwrap_or(&filename);
                let stem = library
                    .split_once(".so")
                    .map_or(library, |(stem, _)| &library[..stem.len() + 3]);
                let is_vdso_libc =
                    library.starts_with("libc.") && basename.starts_with("linux-vdso");
                (basename.starts_with(stem) || is_vdso_libc).then_some(address)
            }
        }
    };
    symbols
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(library.to_owned())
        .or_default()
        .insert(symbol.to_owned(), result.map(|address| address as usize));
    result
}

mod host_ffi {
    use super::{c_char, c_int, c_void};

    unsafe extern "C" {
        #[link_name = "dlopen"]
        pub fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        #[link_name = "dlsym"]
        pub fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        pub fn dladdr(address: *mut c_void, info: *mut DlInfo) -> c_int;
    }

    #[repr(C)]
    pub struct DlInfo {
        pub filename: *const c_char,
        pub base: *mut c_void,
        pub symbol_name: *const c_char,
        pub symbol_address: *mut c_void,
    }

    impl Default for DlInfo {
        fn default() -> Self {
            Self {
                filename: std::ptr::null(),
                base: std::ptr::null_mut(),
                symbol_name: std::ptr::null(),
                symbol_address: std::ptr::null_mut(),
            }
        }
    }
}
