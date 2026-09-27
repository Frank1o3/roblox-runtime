
    unsafe extern "C" {
        fn roblox_game_activity_init(
            f: *mut c_void,
            internal_path: *const c_char,
            obb_path: *const c_char,
            external_path: *const c_char,
            err: *mut c_char,
            err_len: usize,
        ) -> i64;
    }

    unsafe extern "C" {
        fn roblox_set_bootstrap(f: Option<extern "C" fn()>);
    }

    /// Install what `GameActivity.bootstrapTheApp()` runs.
    ///
    /// The engine calls that method from inside `initializeNativeCode` and reads
    /// its flags verdict on the next line, so this has to be installed before
    /// [`init`] rather than after it. Delivering the settings after
    /// `initializeNativeCode` returned is what Cordial did for months, and it is
    /// why the verdict was always `onFlagsFailed` no matter what the document
    /// contained: the engine had already asked and been told nothing.
    ///
    /// Passing `None` restores the previous behaviour, which is the control for
    /// any measurement of this.
    pub fn set_bootstrap(f: Option<extern "C" fn()>) {
        // SAFETY: stores a function pointer the C++ side only ever reads.
        unsafe { roblox_set_bootstrap(f) }
    }

    unsafe extern "C" {
        fn roblox_game_activity_start(
            handle: i64,
            width: c_int,
            height: c_int,
            format: c_int,
            err: *mut c_char,
            err_len: usize,
        ) -> c_int;
    }

    /// Drive the Activity lifecycle and hand the engine its surface.
    pub fn start(handle: i64, width: i32, height: i32, format: i32) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `handle` came from `initialize`; `err` is a live buffer.
        let rc = unsafe {
            roblox_game_activity_start(
                handle,
                width,
                height,
                format,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            let end = err.iter().position(|&b| b == 0).unwrap_or(err.len());
            Err(String::from_utf8_lossy(&err[..end]).into_owned())
        }
    }

    unsafe extern "C" {
        fn roblox_set_init_params(
            f: *mut c_void,
            assets: *const c_char,
            width: c_int,
            height: c_int,
            err: *mut c_char,
            err_len: usize,
        ) -> c_int;
    }

    /// `MainGameActivity.nativeAppBridgeSetInitParams` — where the service lives,
    /// what the device is, and what the viewport looks like. The engine renders
    /// its own app shell and draws nothing until it has these.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn set_init_params(
        native: *mut c_void,
        assets: &str,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        let a = CString::new(assets).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `a` outlives the call.
        let rc = unsafe {
            roblox_set_init_params(
                native,
                a.as_ptr(),
                width,
                height,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            let end = err.iter().position(|&b| b == 0).unwrap_or(err.len());
            Err(String::from_utf8_lossy(&err[..end]).into_owned())
        }
    }

    unsafe extern "C" {
        fn roblox_asset_manager_init(f: *mut c_void, err: *mut c_char, n: usize) -> c_int;
        fn roblox_storage_init(
            f: *mut c_void,
            a: *const c_char,
            b: *const c_char,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_call_bare(f: *mut c_void, err: *mut c_char, n: usize) -> c_int;
        fn roblox_init_flags(
            f: *mut c_void,
            settings: *const c_char,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_appbridge_init(
            f: *mut c_void,
            assets: *const c_char,
            w: c_int,
            h: c_int,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_appbridge_call_bare(f: *mut c_void, err: *mut c_char, n: usize) -> c_int;
        fn roblox_read_local_flags(f: *mut c_void, err: *mut c_char, n: usize) -> c_int;
        fn roblox_appbridge_call_bare_cls(
            f: *mut c_void,
            class_name: *const c_char,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_init_client_settings(
            f: *mut c_void,
            a: *const c_char,
            b: *const c_char,
            c: *const c_char,
            out_result: *mut c_int,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_init_client_settings_cached_compressed(
            f: *mut c_void,
            data: *const u8,
            len: usize,
            a: *const c_char,
            b: *const c_char,
            c: *const c_char,
            when: i64,
            flag: c_int,
            out_result: *mut c_int,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_set_display_size(width: c_int, height: c_int);
        fn roblox_set_display_physical_mm(width_mm: c_int, height_mm: c_int);
        fn roblox_set_ui_mode_night(night: c_int);
        fn roblox_get_fint(
            f: *mut c_void,
            name: *const c_char,
            fallback: c_int,
            out_result: *mut c_int,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_post_client_settings_loaded(f: *mut c_void, err: *mut c_char, n: usize)
        -> c_int;
        fn roblox_preload_flag_overrides(
            f: *mut c_void,
            json: *const c_char,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_call_static_strings(
            f: *mut c_void,
            class_name: *const c_char,
            args: *const *const c_char,
            n: usize,
            err: *mut c_char,
            n_err: usize,
        ) -> c_int;
        fn roblox_call_static_bool_string(
            f: *mut c_void,
            class_name: *const c_char,
            flag: c_int,
            text: *const c_char,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_set_device_info(
            f: *mut c_void,
            width: c_int,
            height: c_int,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_activity_lifecycle(
            f: *mut c_void,
            activity: *const c_char,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_appbridge_start_app(
            f: *mut c_void,
            assets: *const c_char,
            w: c_int,
            h: c_int,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_appbridge_update_surface_app(
            f: *mut c_void,
            assets: *const c_char,
            w: c_int,
            h: c_int,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_appbridge_update_surface_game(
            f: *mut c_void,
            assets: *const c_char,
            w: c_int,
            h: c_int,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_call_static_bare_bool(
            f: *mut c_void,
            class_name: *const c_char,
            out_result: *mut c_int,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_cookies_set_host_sink(sink: Option<unsafe extern "C" fn(*const c_char)>);
        fn roblox_cookies_register_handler(f: *mut c_void, err: *mut c_char, n: usize) -> c_int;
        fn roblox_identity_set_sinks(
            on_login: Option<unsafe extern "C" fn(*const c_char)>,
            on_logout: Option<extern "C" fn()>,
        );
        fn roblox_identity_publish(
            user_id: i64,
            username: *const c_char,
            display_name: *const c_char,
            membership_type: i64,
            is_under13: c_int,
            has_subscription: c_int,
        );
        fn roblox_identity_clear();
        fn roblox_cookies_get_for_domain(
            f: *mut c_void,
            class_name: *const c_char,
            domain: *const c_char,
            out: *mut c_char,
            out_len: usize,
            needed: *mut usize,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_pass_current_refresh_rate(
            f: *mut c_void,
            hz: f32,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_pass_supported_refresh_rates(
            f: *mut c_void,
            rates: *const f32,
            count: usize,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_deeplink_protocol_string(
            f: *mut c_void,
            class_name: *const c_char,
            out: *mut c_char,
            out_len: usize,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_deeplink_cold_start(
            f: *mut c_void,
            class_name: *const c_char,
            url: *const c_char,
            out_handled: *mut c_int,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_deeplink_protocol_init(
            f: *mut c_void,
            class_name: *const c_char,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_deeplink_two_strings_ret_string(
            f: *mut c_void,
            class_name: *const c_char,
            arg_a: *const c_char,
            arg_b: *const c_char,
            out: *mut c_char,
            out_len: usize,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_deeplink_string_ret_string(
            f: *mut c_void,
            class_name: *const c_char,
            arg: *const c_char,
            out: *mut c_char,
            out_len: usize,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_app_ready_set_sink(on_ready: Option<extern "C" fn(*const c_char)>);
        fn roblox_report_battery_state_changed(
            f: *mut c_void,
            status: c_int,
            plugged: c_int,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
        fn roblox_report_battery_status(
            f: *mut c_void,
            status: *const CordialBatteryStatus,
            err: *mut c_char,
            n: usize,
        ) -> c_int;
    }

    fn take_err(err: Vec<u8>) -> String {
        let end = err.iter().position(|&b| b == 0).unwrap_or(err.len());
        String::from_utf8_lossy(&err[..end]).into_owned()
    }

    /// `JNIAAssetManagerSetup.initNative` — hands the engine its asset manager.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn asset_manager_init(native: *mut c_void) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `err` is a live buffer.
        let rc = unsafe {
            roblox_asset_manager_init(native, err.as_mut_ptr() as *mut c_char, err.len())
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `LocalStorageManager.initStorageManagerNativeV3`.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn storage_init(native: *mut c_void, a: &str, b: &str) -> Result<(), String> {
        let ca = CString::new(a).map_err(|e| e.to_string())?;
        let cb = CString::new(b).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: as above; both paths outlive the call.
        let rc = unsafe {
            roblox_storage_init(
                native,
                ca.as_ptr(),
                cb.as_ptr(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// A static native on a named class taking up to three `String` arguments.
    ///
    /// `NativeSettingsInterface.nativeSetFilesDirectory` and friends are how the
    /// app tells the engine which directories it owns. Nothing here called them,
    /// so the engine resolved `appData`, `cache`, `http` and `sounds` against the
    /// working directory instead of absolute storage.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn call_static_strings(
        native: *mut c_void,
        class_name: &str,
        args: &[&str],
    ) -> Result<(), String> {
        let cls = CString::new(class_name).map_err(|e| e.to_string())?;
        let owned: Vec<CString> = args
            .iter()
            .map(|a| CString::new(*a).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        let ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; every buffer outlives the call.
        let rc = unsafe {
            roblox_call_static_strings(
                native,
                cls.as_ptr(),
                ptrs.as_ptr(),
                ptrs.len(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `LocalStorageManager.initStorageManagerNativeV3(AssetManager, String, String)`
    ///
    /// The engine's content store. See the C++ side for why this exists and what
    /// about the two paths is still unestablished.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn init_storage_manager(
        native: *mut c_void,
        a: &str,
        b: &str,
    ) -> Result<(), String> {
        let ca = CString::new(a).map_err(|e| e.to_string())?;
        let cb = CString::new(b).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; both buffers outlive the call.
        let rc = unsafe {
            roblox_init_storage_manager(
                native,
                ca.as_ptr(),
                cb.as_ptr(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// A static, zero-argument native returning `boolean`. Added purely to
    /// observe `NativeSettingsInterface.nativeIsLuaLoginEnabled()`'s own
    /// verdict for `docs/design/sign-in.md` — diagnostic-only, does not drive
    /// any UI or enter any credentials.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn call_static_bare_bool(
        native: *mut c_void,
        class_name: &str,
    ) -> Result<bool, String> {
        let cls = CString::new(class_name).map_err(|e| e.to_string())?;
        let mut out: c_int = -1;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; every buffer outlives the call.
        let rc = unsafe {
            roblox_call_static_bare_bool(
                native,
                cls.as_ptr(),
                &mut out as *mut c_int,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 {
            Ok(out != 0)
        } else {
            Err(take_err(err))
        }
    }

    /// `NativeGLInterface.nativePassCurrentDisplayRefreshRate(F)V`.
    ///
    /// Which rate to send when a window is on two outputs at once is decided in
    /// `roblox_runtime::refresh`, not here.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn pass_current_refresh_rate(native: *mut c_void, hz: f32) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; the buffer outlives the call.
        let rc = unsafe {
            roblox_pass_current_refresh_rate(
                native,
                hz,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeGLInterface.nativePassSupportedRefreshRates([F)V`.
    ///
    /// An empty slice is refused rather than sent. "Every rate this display
    /// supports, and there are none" is not a thing to tell a renderer, and the
    /// engine has been managing without the call at all — so saying nothing
    /// remains strictly better than saying that.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn pass_supported_refresh_rates(
        native: *mut c_void,
        rates: &[f32],
    ) -> Result<(), String> {
        if rates.is_empty() {
            return Err("no plausible refresh rates to report".into());
        }
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `rates` and the error
        // buffer both outlive the call.
        let rc = unsafe {
            roblox_pass_supported_refresh_rates(
                native,
                rates.as_ptr(),
                rates.len(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeGLInterface.reportBatteryStateChanged(II)V`. `status` and
    /// `plugged` are Android's own `BatteryManager` raw values — see
    /// `crates/cordial-runtime/src/battery.rs` for where they came from.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn report_battery_state_changed(
        native: *mut c_void,
        status: i32,
        plugged: i32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; the buffer outlives the call.
        let rc = unsafe {
            roblox_report_battery_state_changed(
                native,
                status,
                plugged,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// The Rust-friendly, `Option`-per-field shape of a `BatteryStatus`
    /// reading — this crate's own type, not borrowed from `cordial-runtime`
    /// (which depends on this crate, not the other way round; a shared type
    /// would need the dependency to point the wrong way). A caller ordinarily
    /// builds this by copying `roblox_runtime::battery::Reading`'s fields
    /// across one for one — a field-for-field copy, not a translation, for the
    /// same "no logic on the wrong side of a crate wall" reason
    /// `roblox_shell::refresh_watch`'s own `Output` type gives.
    ///
    /// `None` means the same thing here as it does in `battery.rs`: this
    /// machine's sysfs did not answer the question, so the field is left null
    /// on the Java side rather than sent as a guessed zero.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct BatteryStatusFields {
        pub present: Option<bool>,
        pub percentage: Option<i32>,
        pub status: Option<i32>,
        pub health: Option<i32>,
        pub voltage_mv: Option<i32>,
        pub current_now_ua: Option<i32>,
        pub current_avg_ua: Option<i32>,
        pub charge_counter_uah: Option<i32>,
        pub power_now_uw: Option<i32>,
        pub technology: Option<String>,
        pub temperature_c: Option<f32>,
        pub plugged: Option<i32>,
    }

    /// The `extern "C"` shape, mirroring `struct CordialBatteryStatus` in
    /// `native/battery.cpp` field for field — see that file for why each
    /// field carries its own `has_*` flag rather than a sentinel value.
    /// Private to this module: [`BatteryStatusFields`] is the public surface,
    /// and this is only the wire format `report_battery_status` builds on the
    /// way to the call.
    #[repr(C)]
    struct CordialBatteryStatus {
        has_present: i32,
        present: i32,
        has_percentage: i32,
        percentage: i32,
        has_status: i32,
        status: i32,
        has_health: i32,
        health: i32,
        has_voltage_mv: i32,
        voltage_mv: i32,
        has_current_now_ua: i32,
        current_now_ua: i32,
        has_current_avg_ua: i32,
        current_avg_ua: i32,
        has_charge_counter_uah: i32,
        charge_counter_uah: i32,
        has_power_now_uw: i32,
        power_now_uw: i32,
        has_technology: i32,
        technology: *const c_char,
        has_temperature_c: i32,
        temperature_c: f32,
        has_plugged: i32,
        plugged: i32,
    }

    impl Default for CordialBatteryStatus {
        fn default() -> Self {
            // Every `has_*` flag starts clear and every value starts zeroed —
            // the all-null `BatteryStatus` the engine gets if a caller sets
            // nothing, which is the honest reading for "nothing was measured"
            // rather than any particular zero being mistaken for a real one.
            unsafe { std::mem::zeroed() }
        }
    }

    /// `NativeGLInterface.reportBatteryStatus(Lcom/roblox/engine/jni/model/BatteryStatus;)V`.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn report_battery_status(
        native: *mut c_void,
        status: &BatteryStatusFields,
    ) -> Result<(), String> {
        let technology_c;
        let mut raw = CordialBatteryStatus::default();
        if let Some(v) = status.present {
            raw.has_present = 1;
            raw.present = v as i32;
        }
        if let Some(v) = status.percentage {
            raw.has_percentage = 1;
            raw.percentage = v;
        }
        if let Some(v) = status.status {
            raw.has_status = 1;
            raw.status = v;
        }
        if let Some(v) = status.health {
            raw.has_health = 1;
            raw.health = v;
        }
        if let Some(v) = status.voltage_mv {
            raw.has_voltage_mv = 1;
            raw.voltage_mv = v;
        }
        if let Some(v) = status.current_now_ua {
            raw.has_current_now_ua = 1;
            raw.current_now_ua = v;
        }
        if let Some(v) = status.current_avg_ua {
            raw.has_current_avg_ua = 1;
            raw.current_avg_ua = v;
        }
        if let Some(v) = status.charge_counter_uah {
            raw.has_charge_counter_uah = 1;
            raw.charge_counter_uah = v;
        }
        if let Some(v) = status.power_now_uw {
            raw.has_power_now_uw = 1;
            raw.power_now_uw = v;
        }
        if let Some(t) = &status.technology {
            technology_c = CString::new(t.as_str()).map_err(|e| e.to_string())?;
            raw.has_technology = 1;
            raw.technology = technology_c.as_ptr();
        }
        if let Some(v) = status.temperature_c {
            raw.has_temperature_c = 1;
            raw.temperature_c = v;
        }
        if let Some(v) = status.plugged {
            raw.has_plugged = 1;
            raw.plugged = v;
        }

        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `raw` and the `CString`
        // backing `raw.technology` both outlive this call.
        let rc = unsafe {
            roblox_report_battery_status(
                native,
                &raw as *const CordialBatteryStatus,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// A static, zero-argument native returning `String`.
    ///
    /// `JNILinkingProtocol`'s message and field names are read this way — see
    /// `native/deeplink.cpp`. Purely a read of a constant the engine already
    /// holds; nothing is passed in.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn call_static_ret_string(
        native: *mut c_void,
        class_name: &str,
    ) -> Result<String, String> {
        let cls = CString::new(class_name).map_err(|e| e.to_string())?;
        let mut out = vec![0u8; 512];
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; every buffer outlives the call.
        let rc = unsafe {
            roblox_deeplink_protocol_string(
                native,
                cls.as_ptr(),
                out.as_mut_ptr() as *mut c_char,
                out.len(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 {
            Ok(take_err(out))
        } else {
            Err(take_err(err))
        }
    }

    /// `maybeHandleColdStartProtocolLaunch(String) -> boolean`, on whichever of
    /// `JNIBaseUrlProtocol` / `JNIWebLoginProtocol` is named.
    ///
    /// The returned boolean is the engine's own answer to "did I take this
    /// URL", and it is the only honest signal Cordial has about a deep link.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn cold_start_protocol_launch(
        native: *mut c_void,
        class_name: &str,
        url: &str,
    ) -> Result<bool, String> {
        let cls = CString::new(class_name).map_err(|e| e.to_string())?;
        let u = CString::new(url).map_err(|e| e.to_string())?;
        let mut out: c_int = -1;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; every buffer outlives the call.
        let rc = unsafe {
            roblox_deeplink_cold_start(
                native,
                cls.as_ptr(),
                u.as_ptr(),
                &mut out as *mut c_int,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 {
            Ok(out != 0)
        } else {
            Err(take_err(err))
        }
    }

    /// `init(Context)` on one of the linking protocol classes.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn protocol_init(native: *mut c_void, class_name: &str) -> Result<(), String> {
        let cls = CString::new(class_name).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `cls`/`err` outlive the call.
        let rc = unsafe {
            roblox_deeplink_protocol_init(
                native,
                cls.as_ptr(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// A static native taking one `String` and returning one —
    /// `MessageBus.getLastRaw(String)`, which is how a publish is checked
    /// rather than assumed.
    /// A static native taking two `String`s and returning `String`.
    ///
    /// `MessageBus.getMessageId(protocolName, methodId)` composes a bus id this
    /// way. Asking the engine to compose it is the point: a subscriber that
    /// spelled the id itself would be guessing at a constant the engine owns,
    /// and would find out by never receiving anything.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn call_static_two_strings_ret_string(
        native: *mut c_void,
        class_name: &str,
        a: &str,
        b: &str,
    ) -> Result<String, String> {
        let cls = CString::new(class_name).map_err(|e| e.to_string())?;
        let ca = CString::new(a).map_err(|e| e.to_string())?;
        let cb = CString::new(b).map_err(|e| e.to_string())?;
        let mut out = vec![0u8; 512];
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; every buffer outlives the call.
        let rc = unsafe {
            roblox_deeplink_two_strings_ret_string(
                native,
                cls.as_ptr(),
                ca.as_ptr(),
                cb.as_ptr(),
                out.as_mut_ptr() as *mut c_char,
                out.len(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 {
            let n = out.iter().position(|b| *b == 0).unwrap_or(out.len());
            Ok(String::from_utf8_lossy(&out[..n]).into_owned())
        } else {
            Err(take_err(err))
        }
    }

    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn call_static_string_ret_string(
        native: *mut c_void,
        class_name: &str,
        arg: &str,
    ) -> Result<String, String> {
        let cls = CString::new(class_name).map_err(|e| e.to_string())?;
        let a = CString::new(arg).map_err(|e| e.to_string())?;
        // Generous, because a bus payload is JSON and truncation is reported as
        // an error rather than silently returning a prefix that still parses.
        let mut out = vec![0u8; 8192];
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; every buffer outlives the call.
        let rc = unsafe {
            roblox_deeplink_string_ret_string(
                native,
                cls.as_ptr(),
                a.as_ptr(),
                out.as_mut_ptr() as *mut c_char,
                out.len(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 {
            Ok(take_err(out))
        } else {
            Err(take_err(err))
        }
    }

    /// Install the sink `APP_READY` is reported to, or clear it with `None`.
    ///
    /// The payload is the app-shell state the engine reached — `Landing`,
    /// `Home` and so on. It is not personal data; the notification that is
    /// (`DID_LOG_IN`) goes to a different sink and is elided there.
    pub fn app_ready_set_sink(on_ready: Option<extern "C" fn(*const c_char)>) {
        // SAFETY: the far side stores the pointer in an atomic and calls it
        // from the engine's thread; a null clears it.
        unsafe { roblox_app_ready_set_sink(on_ready) }
    }

    /// Hand the framework layer the identity its Java mirrors answer from.
    ///
    /// `NativeUserJavaInterface` and `StartAppParams` both report who is signed
    /// in, and both used to report nobody. They live in `native/`, not in
    /// `libroblox.so`, so this call takes no engine symbol and has no ordering
    /// constraint against the engine — only against
    /// `nativeAppBridgeV2StartAppWithParams`, which copies four of these fields
    /// into the app-start parameters once and never asks again.
    ///
    /// A username identifies a person. It crosses this boundary as bytes and is
    /// never printed on either side; see `crate::identity` in `cordial-runtime`
    /// for the rest of that reasoning.
    pub fn identity_publish(
        user_id: i64,
        username: &str,
        display_name: &str,
        membership_type: i64,
        is_under13: bool,
        has_subscription: bool,
    ) {
        // A name carrying an interior nul is not something the engine produces,
        // and truncating one would publish a different account than the one that
        // signed in. Refused whole instead.
        let (Ok(user), Ok(display)) = (CString::new(username), CString::new(display_name)) else {
            return;
        };
        // SAFETY: both pointers are valid for the duration of the call, and the
        // C side copies out of them before returning.
        unsafe {
            roblox_identity_publish(
                user_id,
                user.as_ptr(),
                display.as_ptr(),
                membership_type,
                is_under13 as c_int,
                has_subscription as c_int,
            )
        }
    }

    /// Put the mirrors back to reporting nobody, on a logout.
    pub fn identity_clear() {
        // SAFETY: no arguments, and the C side takes its own lock.
        unsafe { roblox_identity_clear() }
    }

    /// Install the sinks the DataModel notification handler reports through.
    ///
    /// `on_login` receives a `DID_LOG_IN` payload; `on_logout` is called with
    /// nothing on a `DID_LOG_OUT`. Both are plain `extern "C"` functions with
    /// static lifetime, which is why the C side can hold them for the life of
    /// the process.
    pub fn identity_set_sinks(
        on_login: unsafe extern "C" fn(*const c_char),
        on_logout: extern "C" fn(),
    ) {
        // SAFETY: both are static function pointers with C ABI.
        unsafe { roblox_identity_set_sinks(Some(on_login), Some(on_logout)) }
    }

    /// A cookie jar on its way between the engine and the profile directory.
    ///
    /// A newtype rather than a `String` because the difference matters exactly
    /// once, in the diagnostic that gets added at three in the morning. The
    /// value is a live session: printing it to a log, a trace or a panic
    /// message hands somebody's account to whoever reads that log. `Debug`
    /// therefore reports the length and nothing else, so the careless thing to
    /// write is also the safe thing, and getting at the real bytes takes a
    /// deliberate call to [`Jar::expose`].
    pub struct Jar(String);

    impl Jar {
        /// The bytes, for handing back to the engine or writing to the profile.
        /// Every caller of this is a place to check for a leak.
        pub fn expose(&self) -> &str {
            &self.0
        }

        pub fn len(&self) -> usize {
            self.0.len()
        }

        pub fn is_empty(&self) -> bool {
            self.0.is_empty()
        }

        pub fn from_stored(value: String) -> Self {
            Jar(value)
        }
    }

    impl std::fmt::Debug for Jar {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Jar({} bytes)", self.0.len())
        }
    }

    /// `JNICookieProtocol.updateOnSetCookieHandler` — hand the engine an object
    /// to call when a response carries `Set-Cookie`.
    ///
    /// `sink` receives the *host* the cookies came from, never the cookies. See
    /// `native/cookies.cpp` for why that split is where it is: the host is
    /// extracted before anything else reads the URL, because the query string
    /// of a Roblox URL can carry a one-time authentication ticket.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn cookies_register_handler(
        native: *mut c_void,
        sink: unsafe extern "C" fn(*const c_char),
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `sink` is a plain
        // `extern "C"` fn with static lifetime, and `err` outlives the call.
        let rc = unsafe {
            roblox_cookies_set_host_sink(Some(sink));
            roblox_cookies_register_handler(native, err.as_mut_ptr() as *mut c_char, err.len())
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeSettingsInterface.nativeGetCookiesForDomain(String) -> String`.
    ///
    /// The buffer is large because the alternative is worse. A jar that does
    /// not fit is reported as an error naming its size rather than truncated:
    /// half a cookie still parses as a cookie, and the engine would accept it
    /// on the next launch and fail authentication for a reason with no visible
    /// relationship to a buffer.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn cookies_for_domain(
        native: *mut c_void,
        class_name: &str,
        domain: &str,
    ) -> Result<Jar, String> {
        let cls = CString::new(class_name).map_err(|e| e.to_string())?;
        let dom = CString::new(domain).map_err(|e| e.to_string())?;
        let mut out = vec![0u8; 256 * 1024];
        let mut needed: usize = 0;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; every buffer outlives
        // the call, and the C side nul-terminates within `out_len`.
        let rc = unsafe {
            roblox_cookies_get_for_domain(
                native,
                cls.as_ptr(),
                dom.as_ptr(),
                out.as_mut_ptr() as *mut c_char,
                out.len(),
                &mut needed as *mut usize,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc != 0 {
            return Err(take_err(err));
        }
        out.truncate(needed);
        String::from_utf8(out)
            .map(Jar)
            .map_err(|_| "the engine's cookie jar was not UTF-8".to_string())
    }

    /// A static native taking `(boolean, String)` — `setTaskSchedulerBackgroundMode`.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn call_static_bool_string(
        native: *mut c_void,
        class_name: &str,
        flag: bool,
        text: &str,
    ) -> Result<(), String> {
        let cls = CString::new(class_name).map_err(|e| e.to_string())?;
        let t = CString::new(text).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; every buffer outlives the call.
        let rc = unsafe {
            roblox_call_static_bool_string(
                native,
                cls.as_ptr(),
                if flag { 1 } else { 0 },
                t.as_ptr(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeSettingsInterface.nativeSetDeviceInfo(DeviceParams)`.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn set_device_info(
        native: *mut c_void,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `err` outlives the call.
        let rc = unsafe {
            roblox_set_device_info(
                native,
                width,
                height,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `FlagJniInterface.nativeInitializeNativeFlags` — what `bootstrapTheApp`
    /// exists to reach. Without it the engine reports `onFlagsFailed` and stops.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn init_flags(native: *mut c_void, settings_json: &str) -> Result<(), String> {
        let json = CString::new(settings_json).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; both buffers outlive the call.
        let rc = unsafe {
            roblox_init_flags(
                native,
                json.as_ptr(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeGLInterface.readLocalFlags()` — the offline counterpart to the
    /// network `ClientSettings` fetch. Not on the `ActivityNativeMain` chain
    /// Cordial drives (its only dex caller is a different startup path), so
    /// nothing else here calls it unless a caller in `load.rs` does.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn read_local_flags(native: *mut c_void) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `err` is a live buffer.
        let rc =
            unsafe { roblox_read_local_flags(native, err.as_mut_ptr() as *mut c_char, err.len()) };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// A no-argument native on a named class. `nativeAppBridgeAppStart` is on
    /// `NativeAppBridgeInterface`, not `NativeGLInterface`.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn call_bare_on(native: *mut c_void, class_name: &str) -> Result<(), String> {
        let cls = CString::new(class_name).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; buffers outlive the call.
        let rc = unsafe {
            roblox_appbridge_call_bare_cls(
                native,
                cls.as_ptr(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeGLInterface.nativeInitClientSettings(String, String, String)I` —
    /// what the real app calls after fetching client settings itself. Cordial
    /// *is* the host app in this architecture, so this is the legitimate
    /// interface, not a workaround. Returns the engine's own `int` result
    /// code, which is a better signal than anything printed to the log.
    ///
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn init_client_settings(
        native: *mut c_void,
        a: &str,
        b: &str,
        c: &str,
    ) -> Result<i32, String> {
        let ca = CString::new(a).map_err(|e| e.to_string())?;
        let cb = CString::new(b).map_err(|e| e.to_string())?;
        let cc = CString::new(c).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        let mut out: c_int = 0;
        // SAFETY: `native` is the exported JNI native; all buffers outlive the call.
        let rc = unsafe {
            roblox_init_client_settings(
                native,
                ca.as_ptr(),
                cb.as_ptr(),
                cc.as_ptr(),
                &mut out as *mut c_int,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(out) } else { Err(take_err(err)) }
    }

    /// `NativeGLInterface.nativeInitClientSettingsCachedCompressed(...)I` —
    /// hand the engine back the compressed flag cache it wrote itself.
    ///
    /// Cordial has only ever used the plain three-string form, so every launch
    /// has looked cold to the engine even with `flag_cache.dat` on disk beside
    /// it. Returns the engine's own `int`, on the same reasoning as
    /// [`init_client_settings`]: the result code is a better signal than the log.
    #[allow(clippy::too_many_arguments)]
    /// # Safety
    ///
    /// `native` must be a live pointer to the exported JNI native this call
    /// names, obtained via [`Library::symbol`] (or the module-level dlsym
    /// equivalent) against a `libroblox.so` Cordial has `dlopen`'d and never
    /// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
    /// native with from the process's own `JavaVM`, not from anything passed
    /// here -- so the one thing this call cannot check is that `native` really
    /// is that resolved export and not a stale, null-masked, or wrong pointer.
    pub unsafe fn init_client_settings_cached_compressed(
        native: *mut c_void,
        data: &[u8],
        a: &str,
        b: &str,
        c: &str,
        when: i64,
        flag: bool,
    ) -> Result<i32, String> {
        let ca = CString::new(a).map_err(|e| e.to_string())?;
        let cb = CString::new(b).map_err(|e| e.to_string())?;
        let cc = CString::new(c).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        let mut out: c_int = 0;
        // SAFETY: `native` is the exported JNI native; every buffer outlives the
        // call, and `data` is copied into a Java array on the other side.
        let rc = unsafe {
            roblox_init_client_settings_cached_compressed(
                native,
                data.as_ptr(),
                data.len(),
                ca.as_ptr(),
                cb.as_ptr(),
                cc.as_ptr(),
                when,
                c_int::from(flag),
                &mut out as *mut c_int,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(out) } else { Err(take_err(err)) }
    }

