//! Public integration surface for the Roblox Android runtime.
//!
//! Client-owned files and configuration enter through [`RuntimeConfig`]. This
//! crate deliberately does not discover or download an APK.

use std::path::{Path, PathBuf};

pub use roblox_android as android;
pub mod graphics;

/// Paths and options supplied by the embedding client.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    /// APK or application bundle supplied by the caller.
    pub apk: PathBuf,
    /// Directory containing extracted Android native libraries.
    pub native_lib_dir: PathBuf,
    /// Android-visible writable data directory.
    pub data_dir: PathBuf,
    /// Runtime cache directory.
    pub cache_dir: PathBuf,
    /// Fast Flags and other client configuration, already loaded by the caller.
    pub fast_flags: serde_json::Value,
    /// Runtime-specific options supplied by the caller.
    pub options: RuntimeOptions,
}

/// Options whose interpretation belongs to the runtime.
#[derive(Clone, Debug, Default)]
pub struct RuntimeOptions {
    /// Optional client settings document supplied by the caller.
    pub client_settings: Option<PathBuf>,
    /// Renderer preference. The runtime resolves `Automatic` after inspecting
    /// the supplied host surface and available graphics loaders.
    pub graphics_backend: graphics::BackendPreference,
}

impl RuntimeConfig {
    /// Reject missing paths before starting native compatibility code.
    pub fn validate_paths(&self) -> Result<(), ConfigError> {
        for (name, path) in [
            ("APK", self.apk.as_path()),
            ("native library directory", self.native_lib_dir.as_path()),
            ("data directory", self.data_dir.as_path()),
            ("cache directory", self.cache_dir.as_path()),
        ] {
            if path.as_os_str().is_empty() {
                return Err(ConfigError::EmptyPath(name));
            }
        }
        Ok(())
    }

    /// Set up Android-visible paths using only locations supplied by the
    /// embedding client. Returns the host-backed `/system` tree.
    pub fn prepare_android_environment(&self) -> Result<PathBuf, ConfigError> {
        self.validate_paths()?;
        android::system::set_files_dir(&self.data_dir);
        Ok(android::system::install(&self.cache_dir))
    }
}

/// Configuration validation error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    EmptyPath(&'static str),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath(name) => write!(f, "{name} path is empty"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Whether a supplied path exists, without resolving it through user-specific
/// discovery rules.
pub fn supplied_path_exists(path: &Path) -> bool {
    path.exists()
}
