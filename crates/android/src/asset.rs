//! Android `AAssetManager` compatibility backed by caller-supplied APKs.
//!
//! Assets stay in their APK archives and are inflated only when requested.
//! Base APKs are searched before split APKs, matching the order supplied by
//! the embedding client.

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

struct Asset {
    bytes: &'static [u8],
}

#[derive(Clone)]
struct ApkReader {
    file: Arc<File>,
    position: u64,
    length: u64,
}

impl ApkReader {
    fn open(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let length = file.metadata()?.len();
        Ok(Self {
            file: Arc::new(file),
            position: 0,
            length,
        })
    }
}

impl Read for ApkReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.file.read_at(buffer, self.position)?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for ApkReader {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::End(offset) => i128::from(self.length) + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
        };
        if !(0..=i128::from(u64::MAX)).contains(&position) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek outside APK file",
            ));
        }
        self.position = position as u64;
        Ok(self.position)
    }
}

struct Apk {
    path: PathBuf,
    archive: OnceLock<Option<zip::ZipArchive<ApkReader>>>,
}

impl Apk {
    fn archive(&self) -> Option<zip::ZipArchive<ApkReader>> {
        self.archive
            .get_or_init(|| {
                let reader = ApkReader::open(&self.path).ok()?;
                zip::ZipArchive::new(reader).ok()
            })
            .clone()
    }
}

struct Manager {
    apks: Vec<Apk>,
    cache: Mutex<HashMap<String, &'static [u8]>>,
}

impl Manager {
    fn read(&self, name: &str) -> Option<&'static [u8]> {
        if let Some(bytes) = self.cache.lock().ok()?.get(name).copied() {
            return Some(bytes);
        }

        for apk in &self.apks {
            let Some(mut archive) = apk.archive() else {
                continue;
            };
            let Ok(mut entry) = archive.by_name(&format!("assets/{name}")) else {
                continue;
            };
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            if entry.read_to_end(&mut bytes).is_err() {
                continue;
            }
            let bytes: &'static [u8] = Vec::leak(bytes);
            if let Ok(mut cache) = self.cache.lock() {
                cache.insert(name.to_owned(), bytes);
            }
            return Some(bytes);
        }
        None
    }
}

static MANAGER: OnceLock<Manager> = OnceLock::new();

/// Configure the process asset manager from paths owned by the client.
pub fn set_apks(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("at least one APK is required".into());
    }
    let mut apks = Vec::with_capacity(paths.len());
    for path in paths {
        File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
        apks.push(Apk {
            path: path.clone(),
            archive: OnceLock::new(),
        });
    }
    MANAGER
        .set(Manager {
            apks,
            cache: Mutex::new(HashMap::new()),
        })
        .map_err(|_| "APK assets are already configured".into())
}

/// Read one `assets/` entry, retaining its buffer for the process lifetime.
pub fn read_asset(name: &str) -> Option<&'static [u8]> {
    MANAGER.get()?.read(name)
}

unsafe fn c_string(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: the Android API requires a NUL-terminated filename pointer.
    Some(
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned(),
    )
}

extern "C" fn asset_manager_from_java(_env: *mut c_void, _object: *mut c_void) -> *mut c_void {
    MANAGER.get().map_or(std::ptr::null_mut(), |manager| {
        manager as *const _ as *mut c_void
    })
}

extern "C" fn asset_manager_open(
    _manager: *mut c_void,
    filename: *const c_char,
    _mode: c_int,
) -> *mut c_void {
    // SAFETY: the Android API supplies a NUL-terminated asset name.
    let Some(name) = (unsafe { c_string(filename) }) else {
        return std::ptr::null_mut();
    };
    let Some(bytes) = read_asset(&name) else {
        return std::ptr::null_mut();
    };
    Box::into_raw(Box::new(Asset { bytes })).cast()
}

extern "C" fn asset_get_buffer(asset: *mut c_void) -> *const c_void {
    if asset.is_null() {
        return std::ptr::null();
    }
    // SAFETY: handles are allocated by `asset_manager_open` and released by
    // `asset_close`; the caller must keep the handle open while using it.
    unsafe { (*(asset.cast::<Asset>())).bytes.as_ptr().cast() }
}

extern "C" fn asset_get_length(asset: *mut c_void) -> i64 {
    if asset.is_null() {
        return 0;
    }
    // SAFETY: the handle has the provenance described by `asset_get_buffer`.
    unsafe { (&*asset.cast::<Asset>()).bytes.len() as i64 }
}

extern "C" fn asset_close(asset: *mut c_void) {
    if !asset.is_null() {
        // SAFETY: Android closes each handle returned by `asset_manager_open`
        // once, returning ownership of its box here.
        drop(unsafe { Box::from_raw(asset.cast::<Asset>()) });
    }
}

extern "C" fn asset_open_file_descriptor(
    asset: *mut c_void,
    out_start: *mut i64,
    out_length: *mut i64,
) -> c_int {
    if asset.is_null() {
        return -1;
    }
    // SAFETY: the handle has the provenance described by `asset_get_buffer`.
    let bytes = unsafe { &(*(asset.cast::<Asset>())).bytes };
    unsafe extern "C" {
        fn memfd_create(name: *const c_char, flags: u32) -> c_int;
        fn write(fd: c_int, bytes: *const c_void, count: usize) -> isize;
        fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
        fn close(fd: c_int) -> c_int;
    }

    // SAFETY: the name is static and NUL-terminated; no flags are requested.
    let fd = unsafe { memfd_create(c"roblox-runtime-asset".as_ptr(), 0) };
    if fd < 0 {
        return -1;
    }
    let mut written = 0usize;
    while written < bytes.len() {
        // SAFETY: `bytes[written..]` is readable for its reported length and
        // `fd` is a newly-created writable memory file.
        let count = unsafe { write(fd, bytes[written..].as_ptr().cast(), bytes.len() - written) };
        if count <= 0 {
            // SAFETY: `fd` remains owned by this function on this error path.
            unsafe { close(fd) };
            return -1;
        }
        written += count as usize;
    }
    // SAFETY: rewind the memory file before returning it to Android.
    if unsafe { lseek(fd, 0, 0) } < 0 {
        // SAFETY: `fd` remains owned by this function on this error path.
        unsafe { close(fd) };
        return -1;
    }
    if !out_start.is_null() {
        // SAFETY: non-null output pointers are writable per the Android API.
        unsafe { *out_start = 0 };
    }
    if !out_length.is_null() {
        // SAFETY: non-null output pointers are writable per the Android API.
        unsafe { *out_length = bytes.len() as i64 };
    }
    fd
}

/// Android symbols implemented by this module.
pub fn overrides() -> Vec<(&'static str, *mut c_void)> {
    vec![
        (
            "AAssetManager_fromJava",
            asset_manager_from_java as *const () as *mut c_void,
        ),
        (
            "AAssetManager_open",
            asset_manager_open as *const () as *mut c_void,
        ),
        (
            "AAsset_getBuffer",
            asset_get_buffer as *const () as *mut c_void,
        ),
        (
            "AAsset_getLength",
            asset_get_length as *const () as *mut c_void,
        ),
        ("AAsset_close", asset_close as *const () as *mut c_void),
        (
            "AAsset_openFileDescriptor",
            asset_open_file_descriptor as *const () as *mut c_void,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::{Apk, Manager};
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    fn fixture_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn write_apk(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        for (name, bytes) in entries {
            archive
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap();
    }

    #[test]
    fn reads_assets_from_base_and_split_apks_in_client_order() {
        let _lock = fixture_lock();
        let root =
            std::env::temp_dir().join(format!("roblox-runtime-asset-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let base_path = root.join("base.apk");
        let split_path = root.join("split_config.x86_64.apk");
        write_apk(
            &base_path,
            &[
                ("assets/shared.txt", b"base"),
                ("assets/base.txt", b"only-base"),
            ],
        );
        write_apk(
            &split_path,
            &[
                ("assets/shared.txt", b"split"),
                ("assets/split.txt", b"only-split"),
            ],
        );

        let manager = Manager {
            apks: [base_path, split_path]
                .into_iter()
                .map(|path| Apk {
                    path,
                    archive: OnceLock::new(),
                })
                .collect(),
            cache: Mutex::new(Default::default()),
        };

        assert_eq!(manager.read("shared.txt"), Some(&b"base"[..]));
        assert_eq!(manager.read("split.txt"), Some(&b"only-split"[..]));
        assert_eq!(manager.read("missing.txt"), None);
        let _ = fs::remove_dir_all(root);
    }
}
