//! Public integration surface for the Roblox Android runtime.
//!
//! Client-owned files and configuration enter through [`RuntimeConfig`]. This
//! crate deliberately does not discover or download an APK.

macro_rules! eprintln {
    () => {{
        roblox_logging::emit(String::new());
    }};
    ($($arg:tt)*) => {{
        roblox_logging::emit(format!($($arg)*));
    }};
}

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

pub use roblox_abi as abi;
pub use roblox_android as android;
pub use roblox_jni as jni;
pub use roblox_linker::elf::{Binding as ImportBinding, Imports as EngineImports};
pub mod graphics;
pub mod session;
mod symbols;
pub mod webview;

struct StartupBootstrap {
    preload_native: usize,
    settings_native: usize,
    flags_native: usize,
    settings: String,
    fast_flags: String,
    delivered: AtomicBool,
}

static STARTUP_BOOTSTRAP: OnceLock<StartupBootstrap> = OnceLock::new();

/// Configure whether the Rust JNI VM may dispatch unresolved members to the
/// companion C++ compatibility VM.
pub fn set_jnivm_cpp_fallback(enabled: bool) {
    jnivm::set_cpp_fallback_enabled(enabled);
}

#[allow(unsafe_code)]
extern "C" fn run_startup_bootstrap() {
    let Some(plan) = STARTUP_BOOTSTRAP.get() else {
        eprintln!("[runtime] GameActivity bootstrap has no startup plan");
        return;
    };
    if plan.delivered.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // GameActivity invokes this on its native startup thread. Deliver the
        // host-supplied settings from inside that callback, before Roblox
        // evaluates its flags verdict.
        if plan.preload_native != 0 && plan.fast_flags != "{}" && plan.fast_flags != "null" {
            // SAFETY: this symbol is from the mapped engine; JSON remains alive
            // for the duration of the JNI call.
            match unsafe {
                roblox_jni::game_activity::preload_flag_overrides(
                    plan.preload_native as *mut std::ffi::c_void,
                    &plan.fast_flags,
                )
            } {
                Ok(()) => eprintln!("[runtime] nativePreloadFlagOverrides completed"),
                Err(error) => eprintln!("[runtime] nativePreloadFlagOverrides failed: {error}"),
            }
        }
        if plan.settings_native != 0 {
            // SAFETY: this symbol is from the mapped engine, which remains
            // loaded for the process lifetime; JNI is active before callback.
            match unsafe {
                roblox_jni::game_activity::init_client_settings(
                    plan.settings_native as *mut std::ffi::c_void,
                    &plan.settings,
                    "",
                    "",
                )
            } {
                Ok(code) => eprintln!("[runtime] nativeInitClientSettings -> {code}"),
                Err(error) => eprintln!("[runtime] nativeInitClientSettings failed: {error}"),
            }
        }
        if plan.flags_native != 0 {
            // Match the name cache registered by Roblox's Android client;
            // values still come from the client settings and overrides above.
            // SAFETY: same mapped-library and live-JNI guarantees as above.
            match unsafe {
                roblox_jni::game_activity::init_flags(
                    plan.flags_native as *mut std::ffi::c_void,
                    roblox_jni::game_activity::NATIVE_FLAG_NAMES,
                )
            } {
                Ok(()) => eprintln!("[runtime] nativeInitializeNativeFlags completed"),
                Err(error) => eprintln!("[runtime] nativeInitializeNativeFlags failed: {error}"),
            }
        }
    }));
}

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
    /// Optional named login profile. With no profile the runtime starts logged out.
    pub session: Option<session::Session>,
}

/// Options whose interpretation belongs to the runtime.
#[derive(Clone, Debug)]
pub struct RuntimeOptions {
    /// Optional client settings document supplied by the caller.
    pub client_settings: Option<PathBuf>,
    /// Renderer preference. The runtime resolves `Automatic` after inspecting
    /// the supplied host surface and available graphics loaders.
    pub graphics_backend: graphics::BackendPreference,
    /// Requested Vulkan presentation mode (`fifo`, `fifo-relaxed`, `mailbox`, `immediate`, `auto`).
    pub present_mode: Option<String>,
    /// Override GLES swap intervals to zero when VSync is disabled.
    pub vsync: bool,
    /// EGL swap interval when OpenGL ES is selected: -1 adaptive, 0 off, 1 on.
    pub opengl_swap_interval: i32,
    /// Resolve otherwise-unhandled libc symbols from glibc for diagnostics.
    /// This mirrors roblox-runtime's `--host-libc` and is ABI-unsafe; keep it off for
    /// ordinary runtime execution until each required interface is ported.
    pub host_libc: bool,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            client_settings: None,
            graphics_backend: graphics::BackendPreference::default(),
            present_mode: None,
            vsync: true,
            opengl_swap_interval: 1,
            host_libc: false,
        }
    }
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
        let files_dir = self.data_dir.join("files");
        let external_dir = self.data_dir.join("external");
        for path in [&files_dir, &self.cache_dir, &external_dir] {
            std::fs::create_dir_all(path).map_err(|error| {
                ConfigError::DirectorySetup(format!("{}: {error}", path.display()))
            })?;
        }
        android::system::set_files_dir(&files_dir);
        android::local_storage::set_store_dir(&files_dir);
        Ok(android::system::install(&self.cache_dir))
    }

    /// Extract APK filesystem assets into the caller-provided runtime cache.
    pub fn prepare_asset_tree(&self) -> Result<PathBuf, ConfigError> {
        self.validate_paths()?;
        android::asset::extract_to(&self.apk_paths, &self.cache_dir.join("android-assets"))
            .map_err(ConfigError::AssetExtraction)
    }

    /// Give Roblox its private working directory and install the APK's
    /// certificate bundle at the relative locations its HTTP stack uses.
    pub fn prepare_engine_working_directory(
        &self,
        asset_root: &Path,
    ) -> Result<PathBuf, ConfigError> {
        let run_dir = self.data_dir.join("run");
        let run_exe = run_dir.join("exe");
        let files_exe = self.data_dir.join("files/exe");
        for dir in [&run_exe, &files_exe] {
            std::fs::create_dir_all(dir).map_err(|error| {
                ConfigError::DirectorySetup(format!("{}: {error}", dir.display()))
            })?;
        }

        let ca_bundle = asset_root.join("ssl/cacert.pem");
        if !ca_bundle.is_file() {
            return Err(ConfigError::DirectorySetup(format!(
                "APK CA bundle is missing: {}",
                ca_bundle.display()
            )));
        }
        for destination in [run_exe.join("cacert.pem"), files_exe.join("cacert.pem")] {
            std::fs::copy(&ca_bundle, &destination).map_err(|error| {
                ConfigError::DirectorySetup(format!("{}: {error}", destination.display()))
            })?;
        }

        std::fs::create_dir_all(&run_dir).map_err(|error| {
            ConfigError::DirectorySetup(format!("{}: {error}", run_dir.display()))
        })?;
        let run_dir = std::fs::canonicalize(run_dir).map_err(|error| {
            ConfigError::DirectorySetup(format!("canonicalize Roblox run directory: {error}"))
        })?;
        std::env::set_current_dir(&run_dir).map_err(|error| {
            ConfigError::DirectorySetup(format!("enter {}: {error}", run_dir.display()))
        })?;
        Ok(run_dir)
    }

    /// Check that the client surface and requested renderer are ready before
    /// engine constructors run. Install the client-owned surface first.
    pub fn prepare_graphics(&self) -> Result<graphics::Backend, graphics::BackendUnavailable> {
        graphics::prepare_with_swap_interval(
            self.options.graphics_backend,
            self.options.present_mode.as_deref(),
            self.options.vsync,
            self.options.opengl_swap_interval,
        )
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
        let tables = symbols::build(&imports, self.options.host_libc);
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
        roblox_abi::bionic::announce_audio_backend();
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
        if let Some(address) = roblox_graphics_vulkan::loader_symbol() {
            let symbols = [("vkGetInstanceProcAddr".to_owned(), address)];
            for name in roblox_graphics_vulkan::LIBRARY_NAMES {
                roblox_linker::register(name, &symbols)
                    .map_err(|error| LoadError::Linker(error.to_string()))?;
            }
        }
        // FMOD reaches AAudio with dlopen("libaaudio.so"), so none of its
        // exports appear in libroblox.so's DT_NEEDED import table. Register
        // the virtual library explicitly whenever the FMOD Java bridge will
        // advertise AAudio support; otherwise FMOD fails System::init with
        // FMOD_ERR_OUTPUT_INIT before it can try the Java AudioDevice path.
        if roblox_abi::bionic::aaudio_selected() {
            let aaudio: Vec<_> = roblox_abi::bionic::aaudio_overrides()
                .into_iter()
                .map(|(name, address)| (name.to_owned(), address))
                .collect();
            roblox_linker::register("libaaudio.so", &aaudio)
                .map_err(|error| LoadError::Linker(error.to_string()))?;
        }
        let library_path = self.native_lib_dir.join("libroblox.so");
        let exported_symbols = roblox_linker::elf::exported_symbols(&library_path)
            .map_err(|error| LoadError::Config(ConfigError::ElfInspection(error.to_string())))?;
        let library_name = library_path
            .to_str()
            .ok_or(LoadError::NonUtf8Path("libroblox.so"))?;
        roblox_linker::defer_next_ctors(true);
        let load_result = roblox_linker::dlopen(library_name, roblox_linker::RTLD_NOW);
        roblox_linker::defer_next_ctors(false);
        let library = load_result.map_err(|error| LoadError::Linker(error.to_string()))?;
        Ok(LoadedEngine {
            library,
            exported_symbols,
            constructors_pending: true,
            constructors_ready: false,
            vm_initialized: false,
            jni_initialized: false,
            files_dir: self.data_dir.join("files"),
            cache_dir: self.cache_dir.clone(),
            external_dir: self.data_dir.join("external"),
        })
    }
}

/// A mapped engine library. Roblox bootstrap is not invoked automatically.
#[derive(Debug)]
pub struct LoadedEngine {
    library: roblox_linker::Library,
    exported_symbols: std::collections::BTreeMap<String, u64>,
    constructors_pending: bool,
    constructors_ready: bool,
    vm_initialized: bool,
    jni_initialized: bool,
    files_dir: PathBuf,
    cache_dir: PathBuf,
    external_dir: PathBuf,
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
        self.library.symbol(name).or_else(|| {
            let value = *self.exported_symbols.get(name)?;
            let value = usize::try_from(value).ok()?;
            let address = self.library.base().checked_add(value)?;
            eprintln!(
                "[runtime] linker dlsym missed exported symbol {name}; using its ELF address"
            );
            Some(address as *mut std::ffi::c_void)
        })
    }

    /// Run the engine's deferred ELF constructors once the caller has finished
    /// preparing Android services and graphics state. This executes engine
    /// code and can reach compatibility APIs that are not implemented yet.
    pub fn run_constructors(&mut self) -> Result<(), JniError> {
        if self.constructors_pending {
            if !self.constructors_ready {
                return Err(JniError::PreConstructorSetupRequired);
            }
            roblox_linker::run_deferred_ctors(self.library);
            self.constructors_pending = false;
        }
        Ok(())
    }

    /// Create the JavaVM and prepare the engine-owned storage directories.
    /// This must run before [`Self::run_constructors`]. Directory setters are
    /// delivered later, after constructors and before native app startup.
    #[allow(unsafe_code)]
    pub fn prepare_before_constructors(&mut self) -> Result<(), JniError> {
        if !self.constructors_pending {
            return Err(JniError::ConstructorsAlreadyRun);
        }
        let data_dir = self.data_dir_for_storage();
        let directories = [
            self.files_dir.clone(),
            self.cache_dir.clone(),
            self.external_dir.clone(),
            data_dir,
            self.files_dir.join("appData/LocalStorage"),
            self.files_dir.join("appData/rbx-storage"),
            self.files_dir.join("appData/ClientSettings"),
            self.cache_dir.join("ContentProvider_2"),
            self.cache_dir.join("rbx-storage"),
            self.cache_dir.join("sounds"),
        ];
        for path in &directories {
            std::fs::create_dir_all(path).map_err(|error| {
                JniError::DirectorySetup(format!("{}: {error}", path.display()))
            })?;
        }
        if !self.vm_initialized {
            create_java_vm()?;
            self.vm_initialized = true;
        }
        self.constructors_ready = true;
        Ok(())
    }

    /// Deliver Android storage paths in the startup window roblox-runtime measured:
    /// after ELF constructors, before GameActivity initializes native app code.
    #[allow(unsafe_code)]
    pub fn prepare_startup_directories(&self) -> Result<(), JniError> {
        if self.constructors_pending {
            return Err(JniError::ConstructorsDeferred);
        }
        if !self.vm_initialized {
            return Err(JniError::VmNotInitialized);
        }
        const SETTINGS_CLASS: &str = "com/roblox/engine/jni/NativeSettingsInterface";
        let files = self.files_dir.to_string_lossy().into_owned();
        let cache = self.cache_dir.to_string_lossy().into_owned();
        let external = self.external_dir.to_string_lossy().into_owned();
        let setters: [(&str, Vec<&str>); 4] = [
            (
                "Java_com_roblox_engine_jni_NativeSettingsInterface_nativeSetFilesDirectory",
                vec![files.as_str()],
            ),
            (
                "Java_com_roblox_engine_jni_NativeSettingsInterface_nativeSetCacheDirectory",
                vec![cache.as_str()],
            ),
            (
                "Java_com_roblox_engine_jni_NativeSettingsInterface_nativeSetExternalDirectory",
                vec![external.as_str()],
            ),
            (
                "Java_com_roblox_engine_jni_NativeSettingsInterface_nativeSetBaseDataDirectories",
                vec![files.as_str(), cache.as_str()],
            ),
        ];
        for (name, args) in setters {
            let native = self
                .symbol(name)
                .ok_or(JniError::MissingStartupNative(name))?;
            // SAFETY: constructors have run, the VM is live, and this export
            // belongs to the engine mapped for this RuntimeConfig.
            unsafe {
                roblox_jni::game_activity::call_static_strings(native, SETTINGS_CLASS, &args)
            }
            .map_err(|error| JniError::StartupNative(format!("{name}: {error}")))?;
        }
        Ok(())
    }

    /// Install the host's settings delivery before GameActivity starts.
    /// Android normally invokes this work from `GameActivity.bootstrapTheApp`;
    /// a desktop embedding has no framework Activity to do it for us.
    pub fn install_startup_bootstrap(
        &self,
        settings: String,
        fast_flags: String,
    ) -> Result<(), JniError> {
        let preload_native = self
            .symbol("Java_com_roblox_client_startup_MainGameActivity_nativePreloadFlagOverrides")
            .map_or(0, |address| address as usize);
        let settings_native = self
            .symbol("Java_com_roblox_engine_jni_NativeGLInterface_nativeInitClientSettings")
            .ok_or(JniError::MissingBootstrapNative("nativeInitClientSettings"))?;
        let flags_native = self
            .symbol("Java_com_roblox_client_flags_FlagJniInterface_nativeInitializeNativeFlags")
            .ok_or(JniError::MissingBootstrapNative(
                "nativeInitializeNativeFlags",
            ))?;
        STARTUP_BOOTSTRAP
            .set(StartupBootstrap {
                preload_native,
                settings_native: settings_native as usize,
                flags_native: flags_native as usize,
                settings,
                fast_flags,
                delivered: AtomicBool::new(false),
            })
            .map_err(|_| JniError::BootstrapAlreadyInstalled)?;
        if jnivm::selected_from_environment() {
            jnivm::set_startup_bootstrap(run_startup_bootstrap)
                .map_err(|_| JniError::BootstrapAlreadyInstalled)?;
            jnivm::set_game_loaded_callback(report_rust_game_loaded)
                .map_err(|_| JniError::BootstrapAlreadyInstalled)?;
        }
        roblox_jni::game_activity::set_bootstrap(Some(run_startup_bootstrap));
        // This host does not have Android's framework Activity to guarantee
        // that GameActivity.bootstrapTheApp is dispatched before the engine's
        // flags verdict. Deliver synchronously now, after JNI_OnLoad and before
        // initializeNativeCode; the installed hook is a duplicate-safe fallback.
        run_startup_bootstrap();
        Ok(())
    }

    fn data_dir_for_storage(&self) -> PathBuf {
        self.files_dir
            .parent()
            .unwrap_or(self.files_dir.as_path())
            .to_path_buf()
    }

    /// Create libjnivm's JavaVM and call Roblox's `JNI_OnLoad` export.
    /// Constructors must have run first.
    #[allow(unsafe_code)]
    pub fn initialize_jni(&mut self) -> Result<i32, JniError> {
        if self.constructors_pending {
            return Err(JniError::ConstructorsDeferred);
        }
        let on_load = self.symbol("JNI_OnLoad").ok_or(JniError::MissingOnLoad)?;
        if !self.vm_initialized {
            create_java_vm()?;
            self.vm_initialized = true;
        }
        // SAFETY: the function pointer is this live library's JNI_OnLoad;
        // the native shim contains exceptions at the FFI boundary.
        let version = if jnivm::selected_from_environment() {
            // SAFETY: this is the live JNI_OnLoad export from the mapped engine.
            unsafe { jnivm::call_on_load(on_load) }.map_err(JniError::OnLoad)?
        } else {
            // SAFETY: this is the live JNI_OnLoad export from the mapped engine.
            unsafe { roblox_jni::jni::call_on_load(on_load) }.map_err(JniError::OnLoad)?
        };
        self.jni_initialized = true;
        Ok(version)
    }

    /// Call AGDK's `initializeNativeCode` through the selected JNI VM.
    /// A JavaVM must exist, and constructors must have run first.
    #[allow(unsafe_code)]
    pub fn initialize_game_activity(
        &self,
        internal_path: &str,
        obb_path: &str,
        external_path: &str,
        width: u32,
        height: u32,
    ) -> Result<i64, JniError> {
        if self.constructors_pending {
            return Err(JniError::ConstructorsDeferred);
        }
        if !self.jni_initialized || current_jni_env().is_none() {
            return Err(JniError::JniNotInitialized);
        }
        let native = self
            .symbol("Java_com_google_androidgamesdk_GameActivity_initializeNativeCode")
            .ok_or(JniError::MissingGameActivityInit)?;
        if jnivm::selected_from_environment() {
            let env = jnivm::current_env().ok_or(JniError::JniNotInitialized)?;
            let activity = jnivm::new_opaque_object("com/google/androidgamesdk/GameActivity")
                .map_err(JniError::ExperimentalJniVmUnavailable)?;
            let internal = jnivm::new_string_ref(internal_path)
                .map_err(JniError::ExperimentalJniVmUnavailable)?;
            let obb =
                jnivm::new_string_ref(obb_path).map_err(JniError::ExperimentalJniVmUnavailable)?;
            let external = jnivm::new_string_ref(external_path)
                .map_err(JniError::ExperimentalJniVmUnavailable)?;
            let assets = jnivm::new_opaque_object("android/content/res/AssetManager")
                .map_err(JniError::ExperimentalJniVmUnavailable)?;
            let width = i32::try_from(width).map_err(|_| {
                JniError::ExperimentalJniVmUnavailable(
                    "configuration width exceeds Android limits".into(),
                )
            })?;
            let height = i32::try_from(height).map_err(|_| {
                JniError::ExperimentalJniVmUnavailable(
                    "configuration height exceeds Android limits".into(),
                )
            })?;
            let configuration = jnivm::new_configuration(width, height)
                .map_err(JniError::ExperimentalJniVmUnavailable)?;
            // SAFETY: all JNI references and the native export belong to the
            // mapped engine and selected Rust VM, and remain live during call.
            unsafe {
                roblox_jni::game_activity::initialize_with_vm_refs(
                    native,
                    env,
                    activity,
                    internal,
                    obb,
                    external,
                    assets,
                    configuration,
                )
            }
            .map_err(JniError::GameActivity)
        } else {
            // SAFETY: the symbol belongs to this mapped engine and the process
            // VM was checked above.
            unsafe {
                roblox_jni::game_activity::initialize(
                    native,
                    internal_path,
                    obb_path,
                    external_path,
                )
            }
            .map_err(JniError::GameActivity)
        }
    }

    /// Deliver the initial client surface through GameActivity's native
    /// lifecycle callbacks after `initialize_game_activity` has succeeded.
    pub fn start_game_activity(
        &self,
        handle: i64,
        width: u32,
        height: u32,
        format: i32,
    ) -> Result<(), JniError> {
        if self.constructors_pending {
            return Err(JniError::ConstructorsDeferred);
        }
        if !self.jni_initialized || current_jni_env().is_none() {
            return Err(JniError::JniNotInitialized);
        }
        let width = i32::try_from(width)
            .map_err(|_| JniError::GameActivity("surface width exceeds Android limits".into()))?;
        let height = i32::try_from(height)
            .map_err(|_| JniError::GameActivity("surface height exceeds Android limits".into()))?;
        roblox_jni::game_activity::start(handle, width, height, format)
            .map_err(JniError::GameActivity)
    }

    /// Deliver a host-window resize to Roblox's app bridge and GameActivity.
    ///
    /// `assets` is the extracted Android asset directory. Resize the host
    /// window (including its `wl_egl_window`) before calling this method.
    #[allow(unsafe_code)]
    pub fn resize_surface(
        &self,
        game_activity: i64,
        assets: &str,
        format: i32,
        width: u32,
        height: u32,
    ) -> Result<(), JniError> {
        if self.constructors_pending {
            return Err(JniError::ConstructorsDeferred);
        }
        if !self.jni_initialized || current_jni_env().is_none() {
            return Err(JniError::JniNotInitialized);
        }
        let width_i32 = i32::try_from(width)
            .map_err(|_| JniError::SurfaceUpdate("width exceeds Android's integer range".into()))?;
        let height_i32 = i32::try_from(height).map_err(|_| {
            JniError::SurfaceUpdate("height exceeds Android's integer range".into())
        })?;
        let app = self
            .symbol("Java_com_roblox_engine_jni_NativeGLInterface_nativeAppBridgeV2UpdateSurfaceAppWithPlatformParams")
            .ok_or(JniError::MissingSurfaceUpdateNative("app"))?;
        let game = self
            .symbol("Java_com_roblox_engine_jni_NativeGLInterface_nativeAppBridgeV2UpdateSurfaceGameWithPlatformParams")
            .ok_or(JniError::MissingSurfaceUpdateNative("game"))?;

        graphics::resize_surface(width, height)
            .map_err(|error| JniError::SurfaceUpdate(error.to_string()))?;
        // SAFETY: both addresses are exports from this mapped library and the
        // live JavaVM was checked above. `assets` remains valid for each call.
        unsafe {
            roblox_jni::game_activity::appbridge_update_surface(
                app, assets, width_i32, height_i32, false,
            )
        }
        .map_err(JniError::SurfaceUpdate)?;
        // SAFETY: same conditions as the app surface update above.
        unsafe {
            roblox_jni::game_activity::appbridge_update_surface(
                game, assets, width_i32, height_i32, true,
            )
        }
        .map_err(JniError::SurfaceUpdate)?;
        roblox_jni::game_activity::surface_resized(game_activity, format, width_i32, height_i32)
            .map_err(JniError::SurfaceUpdate)
    }
}

extern "C" fn report_rust_game_loaded(place_id: i64) {
    roblox_jni::game_activity::note_game_loaded(place_id);
}

fn create_java_vm() -> Result<(), JniError> {
    if jnivm::selected_from_environment() {
        // FMOD can query its static Java capabilities during JNI_OnLoad, before
        // the later startup-bootstrap installation point.
        jnivm::set_fmod_aaudio_support(|| {
            roblox_abi::bionic::aaudio_selected()
                && roblox_abi::bionic::audio_host_backend_available()
        })
        .map_err(|_| JniError::BootstrapAlreadyInstalled)?;
        jnivm::create_vm()
            .map(|_| ())
            .map_err(JniError::ExperimentalJniVmUnavailable)?;
        roblox_jni::jni::init_compat_bridge().map_err(JniError::ExperimentalJniVmUnavailable)
    } else {
        roblox_jni::jni::create_vm()
            .map(|_| ())
            .ok_or(JniError::VmAlreadyExists)
    }
}

fn current_jni_env() -> Option<*mut std::ffi::c_void> {
    if jnivm::selected_from_environment() {
        jnivm::current_env()
    } else {
        roblox_jni::jni::env()
    }
}

/// Failure during JavaVM or AGDK GameActivity initialisation.
#[derive(Debug)]
pub enum JniError {
    ConstructorsDeferred,
    ConstructorsAlreadyRun,
    PreConstructorSetupRequired,
    VmNotInitialized,
    MissingStartupNative(&'static str),
    StartupNative(String),
    VmAlreadyExists,
    ExperimentalJniVmUnavailable(String),
    JniNotInitialized,
    MissingOnLoad,
    MissingGameActivityInit,
    MissingBootstrapNative(&'static str),
    BootstrapAlreadyInstalled,
    MissingPreConstructorNative(&'static str),
    MissingSurfaceUpdateNative(&'static str),
    DirectorySetup(String),
    PreConstructorNative(String),
    SurfaceUpdate(String),
    OnLoad(String),
    GameActivity(String),
}

impl std::fmt::Display for JniError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConstructorsDeferred => f.write_str("engine constructors have not run"),
            Self::ConstructorsAlreadyRun => {
                f.write_str("pre-constructor setup must run before engine constructors")
            }
            Self::PreConstructorSetupRequired => {
                f.write_str("prepare the JavaVM and engine directories before running constructors")
            }
            Self::VmNotInitialized => {
                f.write_str("create the JavaVM before setting startup directories")
            }
            Self::MissingStartupNative(name) => {
                write!(f, "required startup native is not exported: {name}")
            }
            Self::StartupNative(message) => write!(f, "startup directory setter failed: {message}"),
            Self::VmAlreadyExists => f.write_str("a JavaVM already exists in this process"),
            Self::ExperimentalJniVmUnavailable(error) => {
                write!(f, "create experimental Rust JavaVM: {error}")
            }
            Self::JniNotInitialized => {
                f.write_str("complete JNI_OnLoad before GameActivity initialization")
            }
            Self::MissingOnLoad => f.write_str("libroblox.so does not export JNI_OnLoad"),
            Self::MissingGameActivityInit => {
                f.write_str("GameActivity.initializeNativeCode is not exported")
            }
            Self::MissingBootstrapNative(name) => {
                write!(f, "required startup native is not exported: {name}")
            }
            Self::BootstrapAlreadyInstalled => {
                f.write_str("GameActivity startup bootstrap is already installed")
            }
            Self::MissingPreConstructorNative(name) => {
                write!(f, "required pre-constructor native is not exported: {name}")
            }
            Self::MissingSurfaceUpdateNative(which) => {
                write!(
                    f,
                    "app bridge does not export the {which} surface update native"
                )
            }
            Self::DirectorySetup(message) => {
                write!(f, "cannot prepare engine directories: {message}")
            }
            Self::PreConstructorNative(message) => {
                write!(f, "pre-constructor directory setter failed: {message}")
            }
            Self::SurfaceUpdate(message) => write!(f, "surface resize failed: {message}"),
            Self::OnLoad(message) => write!(f, "JNI_OnLoad failed: {message}"),
            Self::GameActivity(message) => {
                write!(f, "GameActivity initialization failed: {message}")
            }
        }
    }
}

impl std::error::Error for JniError {}

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
    DirectorySetup(String),
    AssetSetup(String),
    AssetExtraction(String),
    ElfInspection(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath(name) => write!(f, "{name} path is empty"),
            Self::NoApks => f.write_str("at least one APK path is required"),
            Self::DirectorySetup(message) => {
                write!(f, "could not prepare Android directories: {message}")
            }
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
