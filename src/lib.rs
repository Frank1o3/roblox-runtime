//! Public integration surface for the Roblox Android runtime.
//!
//! Client-owned files and configuration enter through [`RuntimeConfig`]. This
//! crate deliberately does not discover or download an APK.

use std::path::{Path, PathBuf};

pub use roblox_android as android;
pub use roblox_linker::elf::{Binding as ImportBinding, Imports as EngineImports};
pub mod graphics;
mod symbols;

/// Paths and options supplied by the embedding client.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    /// Base APK and split APKs supplied by the caller.
    pub apk_paths: Vec<PathBuf>,
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
        if self.apk_paths.is_empty() {
            return Err(ConfigError::NoApks);
        }
        for path in &self.apk_paths {
            if path.as_os_str().is_empty() {
                return Err(ConfigError::EmptyPath("APK"));
            }
        }
        for (name, path) in [
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
        android::asset::set_apks(&self.apk_paths).map_err(ConfigError::AssetSetup)?;
        android::system::set_files_dir(&self.data_dir);
        Ok(android::system::install(&self.cache_dir))
    }

    /// Extract APK filesystem assets into the caller-provided runtime cache.
    pub fn prepare_asset_tree(&self) -> Result<PathBuf, ConfigError> {
        self.validate_paths()?;
        android::asset::extract_to(&self.apk_paths, &self.cache_dir.join("android-assets"))
            .map_err(ConfigError::AssetExtraction)
    }

    /// Inspect the supplied engine library's required and optional imports.
    /// This does not load the object or resolve any symbols.
    pub fn engine_imports(&self) -> Result<EngineImports, ConfigError> {
        self.validate_paths()?;
        roblox_linker::elf::undefined_symbols(&self.native_lib_dir.join("libroblox.so"))
            .map_err(|error| ConfigError::ElfInspection(error.to_string()))
    }

    /// Resolve imports and load `libroblox.so` through the bionic linker.
    /// Loading maps and relocates the engine but defers its ELF constructors;
    /// it does not call Roblox's GameActivity bootstrap entry points.
    pub fn load_engine(&self) -> Result<LoadedEngine, LoadError> {
        self.validate_paths().map_err(LoadError::Config)?;
        let imports = self.engine_imports().map_err(LoadError::Config)?;
        let tables = symbols::build(&imports);
        if !tables.missing.is_empty() {
            return Err(LoadError::UnresolvedImports(
                tables
                    .missing
                    .into_iter()
                    .map(|import| import.name)
                    .collect(),
            ));
        }
        if !roblox_linker::constructor_deferral_available() {
            return Err(LoadError::ConstructorDeferralUnavailable);
        }

        roblox_linker::init();
        roblox_linker::set_library_path(
            self.native_lib_dir
                .to_str()
                .ok_or(LoadError::NonUtf8Path("native library directory"))?,
        )
        .map_err(|error| LoadError::Linker(error.to_string()))?;
        for (name, symbols) in tables.libraries {
            roblox_linker::register(name, &symbols)
                .map_err(|error| LoadError::Linker(error.to_string()))?;
        }
        let library_path = self.native_lib_dir.join("libroblox.so");
        let library_name = library_path
            .to_str()
            .ok_or(LoadError::NonUtf8Path("libroblox.so"))?;
        roblox_linker::defer_next_ctors(true);
        let load_result = roblox_linker::dlopen(library_name, roblox_linker::RTLD_NOW);
        roblox_linker::defer_next_ctors(false);
        let library = load_result.map_err(|error| LoadError::Linker(error.to_string()))?;
        Ok(LoadedEngine {
            library,
            constructors_pending: true,
        })
    }
}

/// A mapped engine library. Roblox bootstrap is not invoked automatically.
#[derive(Debug)]
pub struct LoadedEngine {
    library: roblox_linker::Library,
    constructors_pending: bool,
}

impl LoadedEngine {
    /// Address of the mapped engine's ELF base.
    pub fn base(&self) -> usize {
        self.library.base()
    }

    /// Address and length of the engine's executable segment.
    pub fn code_region(&self) -> (usize, usize) {
        self.library.code_region()
    }

    /// Look up an exported engine function by name.
    pub fn symbol(&self, name: &str) -> Option<*mut std::ffi::c_void> {
        self.library.symbol(name)
    }

    /// Run the engine's deferred ELF constructors once the caller has finished
    /// preparing Android services and graphics state. This executes engine
    /// code and can reach compatibility APIs that are not implemented yet.
    pub fn run_constructors(&mut self) {
        if self.constructors_pending {
            roblox_linker::run_deferred_ctors(self.library);
            self.constructors_pending = false;
        }
    }
}

/// Failure while resolving or loading the engine.
#[derive(Debug)]
pub enum LoadError {
    Config(ConfigError),
    NonUtf8Path(&'static str),
    ConstructorDeferralUnavailable,
    UnresolvedImports(Vec<String>),
    Linker(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(error) => write!(f, "{error}"),
            Self::NonUtf8Path(name) => write!(f, "{name} path is not valid UTF-8"),
            Self::ConstructorDeferralUnavailable => {
                f.write_str("native linker cannot defer engine constructors")
            }
            Self::UnresolvedImports(imports) => {
                write!(
                    f,
                    "{} required engine import(s) are unresolved",
                    imports.len()
                )?;
                for import in imports.iter().take(12) {
                    write!(f, "; {import}")?;
                }
                if imports.len() > 12 {
                    write!(f, "; and {} more", imports.len() - 12)?;
                }
                Ok(())
            }
            Self::Linker(message) => write!(f, "engine linker failed: {message}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Configuration validation error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    EmptyPath(&'static str),
    NoApks,
    AssetSetup(String),
    AssetExtraction(String),
    ElfInspection(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath(name) => write!(f, "{name} path is empty"),
            Self::NoApks => f.write_str("at least one APK path is required"),
            Self::AssetSetup(message) => write!(f, "could not configure APK assets: {message}"),
            Self::AssetExtraction(message) => {
                write!(f, "could not extract APK filesystem assets: {message}")
            }
            Self::ElfInspection(message) => write!(f, "could not inspect libroblox.so: {message}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Whether a supplied path exists, without resolving it through user-specific
/// discovery rules.
pub fn supplied_path_exists(path: &Path) -> bool {
    path.exists()
}
