//! Private per-user values used by the engine's platform local-storage bridge.
//!
//! The Java compatibility layer calls these C ABI functions from native code.
//! Values are scoped by account id and kept in one private JSON document so a
//! partial write cannot replace a valid credential with truncated data.

use std::collections::BTreeMap;
use std::ffi::{CStr, c_char, c_int, c_longlong};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

type UserValues = BTreeMap<String, String>;
type Values = BTreeMap<String, UserValues>;

static STORE_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
static STORE_LOCK: Mutex<()> = Mutex::new(());

fn store_dir() -> &'static Mutex<Option<PathBuf>> {
    STORE_DIR.get_or_init(|| Mutex::new(None))
}

/// Set the client-owned directory for private local-storage values.
pub fn set_store_dir(path: &Path) {
    *store_dir()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path.to_path_buf());
}

/// Keep the native bridge's Rust implementations in the final executable.
pub fn link_symbols() {
    std::hint::black_box(roblox_local_storage_get as *const ());
    std::hint::black_box(roblox_local_storage_set as *const ());
    std::hint::black_box(roblox_local_storage_delete as *const ());
    std::hint::black_box(roblox_local_storage_delete_user as *const ());
}

fn path() -> Option<PathBuf> {
    store_dir()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .map(|directory| directory.join("local-storage-secrets.json"))
}

fn load(path: &Path) -> Result<Values, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("stored values are invalid JSON: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(format!("cannot read private values: {error}")),
    }
}

fn save(path: &Path, values: &Values) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "private values path has no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create private directory: {error}"))?;
    let temporary = path.with_extension("json.new");
    let bytes = serde_json::to_vec(values).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| format!("cannot create private values file: {error}"))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("cannot restrict private values file: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("cannot write private values: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("cannot flush private values: {error}"))?;
    drop(file);
    fs::rename(&temporary, path).map_err(|error| format!("cannot publish private values: {error}"))
}

fn key<'a>(key: *const c_char) -> Result<&'a str, ()> {
    if key.is_null() {
        return Err(());
    }
    // SAFETY: the C++ caller passes a NUL-terminated key valid through the call.
    unsafe { CStr::from_ptr(key) }.to_str().map_err(|_| ())
}

fn log_failure(operation: &str, error: &str) {
    eprintln!("[roblox-runtime/local-storage] {operation} failed: {error}");
}

/// Read one value. Returns zero for a completed lookup, with `found` and
/// `out_len` carrying the result; a nonzero status means the request failed.
#[unsafe(no_mangle)]
pub extern "C" fn roblox_local_storage_get(
    user_id: c_longlong,
    name: *const c_char,
    out: *mut c_char,
    out_capacity: usize,
    found: *mut c_int,
    out_len: *mut usize,
) -> c_int {
    let Ok(name) = key(name) else { return -1 };
    if out.is_null() || found.is_null() || out_len.is_null() {
        return -1;
    }
    let Some(path) = path() else { return -1 };
    let _guard = STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let values = match load(&path) {
        Ok(values) => values,
        Err(error) => {
            log_failure("read", &error);
            return -1;
        }
    };
    let value = values
        .get(&user_id.to_string())
        .and_then(|user| user.get(name));
    // SAFETY: the C++ caller provides writable out-parameters and a buffer of
    // exactly `out_capacity` bytes for this synchronous call.
    unsafe {
        match value {
            Some(value) if value.len() < out_capacity => {
                std::ptr::copy_nonoverlapping(value.as_ptr(), out.cast::<u8>(), value.len());
                *out.add(value.len()) = 0;
                *found = 1;
                *out_len = value.len();
            }
            Some(value) => {
                *found = 0;
                *out_len = value.len();
                return -1;
            }
            None => {
                *found = 0;
                *out_len = 0;
            }
        }
    }
    0
}

/// Store a UTF-8 value, replacing any previous value for this user and key.
#[unsafe(no_mangle)]
pub extern "C" fn roblox_local_storage_set(
    user_id: c_longlong,
    name: *const c_char,
    value: *const c_char,
    value_len: usize,
) -> c_int {
    let Ok(name) = key(name) else { return -1 };
    if value.is_null() {
        return -1;
    }
    // SAFETY: the C++ caller owns `value_len` readable bytes for this call.
    let bytes = unsafe { std::slice::from_raw_parts(value.cast::<u8>(), value_len) };
    let Ok(value) = std::str::from_utf8(bytes) else {
        return -1;
    };
    let Some(path) = path() else { return -1 };
    let _guard = STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut values = match load(&path) {
        Ok(values) => values,
        Err(error) => {
            log_failure("write", &error);
            return -1;
        }
    };
    values
        .entry(user_id.to_string())
        .or_default()
        .insert(name.to_owned(), value.to_owned());
    match save(&path, &values) {
        Ok(()) => 0,
        Err(error) => {
            log_failure("write", &error);
            -1
        }
    }
}

/// Delete one value. Deleting an absent key is a successful no-op.
#[unsafe(no_mangle)]
pub extern "C" fn roblox_local_storage_delete(user_id: c_longlong, name: *const c_char) -> c_int {
    let Ok(name) = key(name) else { return -1 };
    update("delete", |values| {
        if let Some(user) = values.get_mut(&user_id.to_string()) {
            user.remove(name);
            if user.is_empty() {
                values.remove(&user_id.to_string());
            }
        }
    })
}

/// Delete all values for one user, leaving other users' entries intact.
#[unsafe(no_mangle)]
pub extern "C" fn roblox_local_storage_delete_user(user_id: c_longlong) -> c_int {
    update("delete user", |values| {
        values.remove(&user_id.to_string());
    })
}

fn update(operation: &str, change: impl FnOnce(&mut Values)) -> c_int {
    let Some(path) = path() else { return -1 };
    let _guard = STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut values = match load(&path) {
        Ok(values) => values,
        Err(error) => {
            log_failure(operation, &error);
            return -1;
        }
    };
    change(&mut values);
    match save(&path, &values) {
        Ok(()) => 0,
        Err(error) => {
            log_failure(operation, &error);
            -1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_user_scoped_and_file_is_private() {
        let directory = std::env::temp_dir().join(format!(
            "roblox-runtime-local-storage-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        set_store_dir(&directory);
        let name = std::ffi::CString::new("token").unwrap();
        let value = b"secret-value";
        assert_eq!(
            roblox_local_storage_set(12, name.as_ptr(), value.as_ptr().cast(), value.len()),
            0
        );
        assert_eq!(
            roblox_local_storage_set(13, name.as_ptr(), b"other".as_ptr().cast(), 5),
            0
        );
        let mut output = [0i8; 32];
        let (mut found, mut len) = (0, 0);
        assert_eq!(
            roblox_local_storage_get(
                12,
                name.as_ptr(),
                output.as_mut_ptr(),
                output.len(),
                &mut found,
                &mut len,
            ),
            0
        );
        assert_eq!(found, 1);
        assert_eq!(
            &output[..len]
                .iter()
                .map(|byte| *byte as u8)
                .collect::<Vec<_>>(),
            value
        );
        let metadata = fs::metadata(directory.join("local-storage-secrets.json")).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        let _ = fs::remove_dir_all(&directory);
    }
}
