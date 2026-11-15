//! Shared state and services used by the Android compatibility layers.

use std::ffi::{CString, c_char};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

#[derive(Default)]
struct RuntimePaths {
    files_dir: Option<PathBuf>,
    system_root: Option<PathBuf>,
}

static PATHS: RwLock<RuntimePaths> = RwLock::new(RuntimePaths {
    files_dir: None,
    system_root: None,
});

/// Set the active profile's Android app-private files directory.
pub fn set_files_dir(path: &Path) {
    PATHS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .files_dir = Some(path.to_path_buf());
}

/// Return the files directory using the runtime's established precedence:
/// environment override, configured profile directory, then legacy fallback.
pub fn files_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("RBX_RUNTIME_FILES_DIR") {
        return PathBuf::from(path);
    }
    if let Some(path) = PATHS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .files_dir
        .clone()
    {
        return path;
    }

    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local/share"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let path = base.join("roblox-runtime/instances/default/data");
    // Keep the pre-profile fallback usable for native calls made before the
    // runtime configures its active profile directory.
    let _ = std::fs::create_dir_all(&path);
    path
}

/// Set the directory that serves Android's virtual `/system` tree.
pub fn set_system_root(path: &Path) {
    PATHS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .system_root = Some(path.to_path_buf());
}

/// Copy the configured virtual `/system` root into a C caller's buffer.
/// Returns the number of bytes required, excluding the trailing NUL.
#[unsafe(no_mangle)]
pub extern "C" fn roblox_framework_copy_system_root(output: *mut c_char, capacity: usize) -> usize {
    let Some(path) = PATHS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .system_root
        .clone()
    else {
        if !output.is_null() && capacity > 0 {
            // SAFETY: a non-null output with positive capacity names at least
            // one writable byte, as required by this C ABI.
            unsafe { output.write(0) };
        }
        return 0;
    };
    let value = path.to_string_lossy();
    let bytes = value.as_bytes();
    if !output.is_null() && capacity > 0 {
        let copy_len = bytes.len().min(capacity - 1);
        // SAFETY: caller provides `capacity` writable bytes; source and target
        // do not overlap because the source is owned by this Rust string.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), output.cast(), copy_len);
            output.add(copy_len).write(0);
        }
    }
    bytes.len()
}

/// Stable C string used by the native Java compatibility callbacks.
///
/// The first read freezes the effective path for process lifetime, matching
/// the former C++ `files_dir()` cache. The runtime sets its profile path before
/// JNI callbacks can ask for it.
#[unsafe(no_mangle)]
pub extern "C" fn roblox_framework_files_dir() -> *const c_char {
    static FILES_DIR: OnceLock<CString> = OnceLock::new();
    FILES_DIR
        .get_or_init(|| CString::new(files_dir().to_string_lossy().as_bytes()).unwrap_or_default())
        .as_ptr()
}

#[cfg(test)]
mod tests {
    use super::{files_dir, roblox_framework_copy_system_root, roblox_framework_files_dir};
    use std::ffi::{CStr, c_char};
    use std::path::PathBuf;

    #[test]
    fn configured_paths_are_shared_with_native_framework_callbacks() {
        let root = PathBuf::from(format!("/tmp/rusty-blox-framework-{}", std::process::id()));
        let files = root.join("files");
        let system = root.join("system");
        super::set_files_dir(&files);
        super::set_system_root(&system);

        assert_eq!(files_dir(), files);
        // SAFETY: the framework getter returns a process-lifetime NUL-terminated string.
        let native_files = unsafe { CStr::from_ptr(roblox_framework_files_dir()) };
        assert_eq!(native_files.to_str().unwrap(), files.to_str().unwrap());

        let mut output = [0 as c_char; 512];
        let required = roblox_framework_copy_system_root(output.as_mut_ptr(), output.len());
        assert_eq!(required, system.to_string_lossy().len());
        // SAFETY: the copy API writes a trailing NUL when the supplied buffer fits.
        let native_system = unsafe { CStr::from_ptr(output.as_ptr()) };
        assert_eq!(native_system.to_str().unwrap(), system.to_str().unwrap());
    }
}
