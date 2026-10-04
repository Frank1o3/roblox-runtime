//! Android `AAssetManager` compatibility backed by caller-supplied APKs.
//!
//! Assets stay in their APK archives and are inflated only when requested.
//! Base APKs are searched before split APKs, matching the order supplied by
//! the embedding client.

use std::collections::{HashMap, VecDeque};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

struct Asset {
    bytes: Arc<Vec<u8>>,
}

const MAX_ASSET_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHED_ASSET_COUNT: usize = 1024;

#[derive(Default)]
struct AssetCache {
    entries: HashMap<String, Arc<Vec<u8>>>,
    insertion_order: VecDeque<String>,
    bytes: usize,
}

impl AssetCache {
    fn insert(&mut self, name: String, bytes: Arc<Vec<u8>>) {
        if bytes.len() > MAX_ASSET_CACHE_BYTES || self.entries.contains_key(&name) {
            return;
        }
        while self.entries.len() >= MAX_CACHED_ASSET_COUNT
            || self.bytes.saturating_add(bytes.len()) > MAX_ASSET_CACHE_BYTES
        {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(evicted.len());
            }
        }
        self.bytes += bytes.len();
        self.insertion_order.push_back(name.clone());
        self.entries.insert(name, bytes);
    }
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
    // Keep the central directory in place and serialize access to it. Cloning
    // ZipArchive per asset request duplicates its entry index on cache misses.
    archive: OnceLock<Option<Mutex<zip::ZipArchive<ApkReader>>>>,
}

impl Apk {
    fn archive(&self) -> Option<&Mutex<zip::ZipArchive<ApkReader>>> {
        self.archive
            .get_or_init(|| {
                let reader = ApkReader::open(&self.path).ok()?;
                Some(Mutex::new(zip::ZipArchive::new(reader).ok()?))
            })
            .as_ref()
    }
}

struct Manager {
    apks: Vec<Apk>,
    cache: Mutex<AssetCache>,
}

impl Manager {
    fn read(&self, name: &str) -> Option<Arc<Vec<u8>>> {
        if let Some(bytes) = self.cache.lock().ok()?.entries.get(name).cloned() {
            return Some(bytes);
        }

        let entry_name = format!("assets/{name}");
        for apk in &self.apks {
            let Some(archive) = apk.archive() else {
                continue;
            };
            let Ok(mut archive) = archive.lock() else {
                continue;
            };
            let Ok(mut entry) = archive.by_name(&entry_name) else {
                continue;
            };
            let size = usize::try_from(entry.size()).ok()?;
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(size).ok()?;
            if entry.read_to_end(&mut bytes).is_err() {
                continue;
            }
            let bytes = Arc::new(bytes);
            if let Ok(mut cache) = self.cache.lock() {
                if let Some(existing) = cache.entries.get(name) {
                    return Some(existing.clone());
                }
                cache.insert(name.to_owned(), bytes.clone());
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
            cache: Mutex::new(AssetCache::default()),
        })
        .map_err(|_| "APK assets are already configured".into())
}

/// Read one `assets/` entry. Cached bytes are bounded; open handles keep their
/// own shared reference alive until Android closes them.
pub fn read_asset(name: &str) -> Option<Arc<Vec<u8>>> {
    MANAGER.get()?.read(name)
}

/// Extract the `assets/` trees from the supplied APK set into a managed cache.
///
/// A completion stamp keys the tree to APK file metadata. Extraction occurs in
/// a sibling directory and is published only after every entry was written;
/// replacing an APK set also removes files that were present only in an older
/// set.
pub fn extract_to(apks: &[PathBuf], destination: &Path) -> Result<PathBuf, String> {
    if apks.is_empty() {
        return Err("at least one APK is required".into());
    }
    let stamp = apk_set_stamp(apks)?;
    let marker = destination.join(".roblox-runtime-apk-set");
    if fs::read_to_string(&marker).is_ok_and(|existing| existing == stamp) {
        return Ok(destination.to_path_buf());
    }

    let parent = destination
        .parent()
        .ok_or_else(|| format!("asset cache path has no parent: {}", destination.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    let name = destination
        .file_name()
        .and_then(|part| part.to_str())
        .ok_or_else(|| format!("invalid asset cache path: {}", destination.display()))?;
    let staging = destination.with_file_name(format!(".{name}.staging-{}", std::process::id()));
    let backup = destination.with_file_name(format!(".{name}.previous-{}", std::process::id()));
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
    }
    fs::create_dir(&staging).map_err(|error| error.to_string())?;

    let extracted = extract_apk_assets(apks, &staging, &stamp);
    if let Err(error) = extracted {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    if backup.exists() {
        fs::remove_dir_all(&backup).map_err(|error| error.to_string())?;
    }
    let had_previous = destination.exists();
    if had_previous {
        fs::rename(destination, &backup).map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(&staging, destination) {
        if had_previous {
            let _ = fs::rename(&backup, destination);
        }
        return Err(error.to_string());
    }
    if had_previous {
        fs::remove_dir_all(backup).map_err(|error| error.to_string())?;
    }
    Ok(destination.to_path_buf())
}

fn apk_set_stamp(apks: &[PathBuf]) -> Result<String, String> {
    use std::os::unix::fs::MetadataExt;

    let mut stamp = String::new();
    for apk in apks {
        let metadata = fs::metadata(apk).map_err(|error| format!("{}: {error}", apk.display()))?;
        stamp.push_str(&format!(
            "{:?}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            apk,
            metadata.dev(),
            metadata.ino(),
            metadata.size(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec()
        ));
    }
    Ok(stamp)
}

fn extract_apk_assets(apks: &[PathBuf], destination: &Path, stamp: &str) -> Result<(), String> {
    for apk in apks {
        let file = File::open(apk).map_err(|error| format!("{}: {error}", apk.display()))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
            let Some(path) = entry.enclosed_name() else {
                continue;
            };
            let Ok(relative) = path.strip_prefix("assets") else {
                continue;
            };
            if relative.as_os_str().is_empty()
                || relative
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
                || entry.is_dir()
            {
                continue;
            }
            let output = destination.join(relative);
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            let temporary = output.with_extension(format!("runtime-{}", std::process::id()));
            let mut file = File::create(&temporary).map_err(|error| error.to_string())?;
            std::io::copy(&mut entry, &mut file).map_err(|error| error.to_string())?;
            fs::rename(&temporary, &output).map_err(|error| error.to_string())?;
        }
    }
    fs::write(destination.join(".roblox-runtime-apk-set"), stamp).map_err(|error| error.to_string())
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
    use super::{Apk, Manager, extract_to};
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

        assert_eq!(
            manager.read("shared.txt").as_deref().map(Vec::as_slice),
            Some(&b"base"[..])
        );
        assert_eq!(
            manager.read("split.txt").as_deref().map(Vec::as_slice),
            Some(&b"only-split"[..])
        );
        assert_eq!(manager.read("missing.txt"), None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn extraction_replaces_stale_files_when_the_apk_set_changes() {
        let _lock = fixture_lock();
        let root = std::env::temp_dir().join(format!(
            "roblox-runtime-asset-extract-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let base_path = root.join("base.apk");
        let split_path = root.join("split_config.x86_64.apk");
        let destination = root.join("extracted");
        write_apk(
            &base_path,
            &[
                ("assets/keep.txt", b"old"),
                ("assets/removed.txt", b"stale"),
            ],
        );
        write_apk(&split_path, &[("assets/override.txt", b"split-v1")]);

        extract_to(&[base_path.clone(), split_path.clone()], &destination).unwrap();
        assert_eq!(fs::read(destination.join("keep.txt")).unwrap(), b"old");
        assert_eq!(
            fs::read(destination.join("override.txt")).unwrap(),
            b"split-v1"
        );

        write_apk(&base_path, &[("assets/keep.txt", b"new-value")]);
        write_apk(&split_path, &[("assets/override.txt", b"split-v2")]);
        extract_to(&[base_path, split_path], &destination).unwrap();

        assert_eq!(
            fs::read(destination.join("keep.txt")).unwrap(),
            b"new-value"
        );
        assert_eq!(
            fs::read(destination.join("override.txt")).unwrap(),
            b"split-v2"
        );
        assert!(!destination.join("removed.txt").exists());
        let _ = fs::remove_dir_all(root);
    }
}
