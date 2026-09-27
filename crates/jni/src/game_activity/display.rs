    /// Tell the framework layer the real window size.
    ///
    /// The C++ setter behind this had no `extern "C"` and therefore no caller,
    /// so `DisplayMetrics`, the User-Agent resolution fields and the
    /// `AConfiguration` screen size have all been reporting the compiled
    /// 1280x720 regardless of the window. Call it as soon as the window's
    /// geometry is known, and again whenever it changes.
    /// The desktop's dark/light preference, as Android's `uiMode` night field.
    ///
    /// Cordial hardcoded "night: no" and Roblox believed it, which is why the
    /// client stayed light however the desktop was set. Anything above zero
    /// reports night mode on; `-1` means nobody said and leaves the old
    /// behaviour, because a runtime started without the shell has no better
    /// answer and guessing dark would be as wrong as guessing light.
    pub fn set_ui_mode_night(night: i32) {
        // SAFETY: stores an int into an atomic on the C++ side; no ownership.
        unsafe { roblox_set_ui_mode_night(night as c_int) }
    }

    pub fn set_display_size(width: i32, height: i32) {
        // SAFETY: writes two ints behind a mutex-free but single-threaded
        // startup path, the same one `set_init_params` already runs on.
        unsafe { roblox_set_display_size(width as c_int, height as c_int) }
    }

    /// The display's physical size in millimetres, for
    /// `DeviceUtils.getScreenPhysicalSizeInMillimeters`.
    ///
    /// Zero means "not known", which is a state the engine has its own branch
    /// for -- see the class in `native/init_params.cpp`. Passing zero is
    /// therefore correct rather than a failure to call this.
    pub fn set_display_physical_mm(width_mm: i32, height_mm: i32) {
        // SAFETY: two ints into a setter that only stores them.
        unsafe { roblox_set_display_physical_mm(width_mm as c_int, height_mm as c_int) }
    }

    /// `FlagJniInterface.nativeGetFInt(String, int)I` — ask the engine what a
    /// flag actually holds, rather than inferring it from behaviour.
    ///
    /// `fallback` comes back when the name is not a registered flag, so a
    /// sentinel separates "the engine has this set to 0" from "the engine has
    /// never heard of it". Cordial spent a session unable to tell those apart
    /// for `FLogNativeDM`; see docs/analysis/flag-init.md §22.
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
    pub unsafe fn get_fint(native: *mut c_void, name: &str, fallback: i32) -> Result<i32, String> {
        let cn = CString::new(name).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        let mut out: c_int = 0;
        // SAFETY: `native` is the exported JNI native; all buffers outlive the call.
        let rc = unsafe {
            roblox_get_fint(
                native,
                cn.as_ptr(),
                fallback as c_int,
                &mut out as *mut c_int,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(out) } else { Err(take_err(err)) }
    }

    /// `NativeGLInterface.nativePostClientSettingsLoadedInitialization3(List)V`
    /// — the finishing step of the client-settings handshake, called with an
    /// empty `ArrayList`.
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
    pub unsafe fn post_client_settings_loaded(native: *mut c_void) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe {
            roblox_post_client_settings_loaded(native, err.as_mut_ptr() as *mut c_char, err.len())
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `MainGameActivity.nativePreloadFlagOverrides(String)V` — takes whatever
    /// JSON text is given and hands it straight through, so candidate shapes
    /// can be compared by their effect on the flags verdict / JNI trace.
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
    pub unsafe fn preload_flag_overrides(native: *mut c_void, json: &str) -> Result<(), String> {
        let cs = CString::new(json).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `cs`/`err` outlive the call.
        let rc = unsafe {
            roblox_preload_flag_overrides(
                native,
                cs.as_ptr(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeGLInterface.nativeAppBridgeV2InitWithParams` — the real app-bridge
    /// entry. The launcher Activity targets `ActivityNativeMain`, whose chain runs
    /// through here rather than through AGDK's `MainGameActivity`.
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
    pub unsafe fn appbridge_init(
        native: *mut c_void,
        assets: &str,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        let a = CString::new(assets).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `a` outlives the call.
        let rc = unsafe {
            roblox_appbridge_init(
                native,
                a.as_ptr(),
                width,
                height,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// A `NativeGLInterface` native taking no arguments — `nativeAppBridgeStartLuaAppDM`.
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
    pub unsafe fn appbridge_call_bare(native: *mut c_void) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe {
            roblox_appbridge_call_bare(native, err.as_mut_ptr() as *mut c_char, err.len())
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `nativeAppBridgeV2StartAppWithParams` — the call that hands the engine
    /// its window. Everything before it is setup.
    /// `nativeAppBridgeV2UpdateSurfaceApp/GameWithPlatformParams`.
    ///
    /// Two calls Sober makes and Cordial did not — see `update_surface` in
    /// `native/init_params.cpp` for the measurement. `game` selects the
    /// three-argument form, which takes an Activity as well.
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
    pub unsafe fn appbridge_update_surface(
        native: *mut c_void,
        assets: &str,
        width: i32,
        height: i32,
        game: bool,
    ) -> Result<(), String> {
        let a = CString::new(assets).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `a` outlives the call.
        let rc = unsafe {
            let f = if game {
                roblox_appbridge_update_surface_game
            } else {
                roblox_appbridge_update_surface_app
            };
            f(
                native,
                a.as_ptr(),
                width,
                height,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
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
    pub unsafe fn appbridge_start_app(
        native: *mut c_void,
        assets: &str,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        let a = CString::new(assets).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `a` outlives the call.
        let rc = unsafe {
            roblox_appbridge_start_app(
                native,
                a.as_ptr(),
                width,
                height,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// One of `JNIActivityLifecycleCallbacks`' natives. The engine stores
    /// per-Activity context — including the JNI environment it later reaches
    /// through — as these fire.
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
    pub unsafe fn activity_lifecycle(native: *mut c_void, activity: &str) -> Result<(), String> {
        let a = CString::new(activity).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `a` outlives the call.
        let rc = unsafe {
            roblox_activity_lifecycle(
                native,
                a.as_ptr(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// A native taking nothing but the JNI pair — `nativeRetryInit`.
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
    pub unsafe fn call_bare(native: *mut c_void) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe { roblox_call_bare(native, err.as_mut_ptr() as *mut c_char, err.len()) };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
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
    pub unsafe fn initialize(
        native: *mut c_void,
        internal_path: &str,
        obb_path: &str,
        external_path: &str,
    ) -> Result<i64, String> {
        let internal = CString::new(internal_path).map_err(|e| e.to_string())?;
        let obb = CString::new(obb_path).map_err(|e| e.to_string())?;
        let external = CString::new(external_path).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];

        // SAFETY: `native` is libroblox's initializeNativeCode export; the paths
        // outlive the call. The shim takes the JNI environment from the VM
        // itself — Rust cannot name `jnivm::ENV` and must not pretend to.
        let handle = unsafe {
            roblox_game_activity_init(
                native,
                internal.as_ptr(),
                obb.as_ptr(),
                external.as_ptr(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };

        if handle == 0 {
            let end = err.iter().position(|&b| b == 0).unwrap_or(err.len());
            let msg = String::from_utf8_lossy(&err[..end]).into_owned();
            Err(if msg.is_empty() {
                "initializeNativeCode returned a null handle".into()
            } else {
                msg
            })
        } else {
            Ok(handle)
        }
    }

    unsafe extern "C" {
        fn roblox_game_activity_touch(
            handle: i64,
            action: c_int,
            x: f32,
            y: f32,
            button_state: c_int,
            action_button: c_int,
            event_time_ms: i64,
            down_time_ms: i64,
            consumed: *mut c_int,
            err: *mut c_char,
            err_len: usize,
        ) -> c_int;
        fn roblox_game_activity_touch_multi(
            handle: i64,
            action: c_int,
            contacts: *const TouchContact,
            count: c_int,
            event_time_ms: i64,
            down_time_ms: i64,
            consumed: *mut c_int,
            err: *mut c_char,
            err_len: usize,
        ) -> c_int;
        fn roblox_game_activity_scroll(
            handle: i64,
            x: f32,
            y: f32,
            hscroll: f32,
            vscroll: f32,
            event_time_ms: i64,
            consumed: *mut c_int,
            err: *mut c_char,
            err_len: usize,
        ) -> c_int;
        fn roblox_game_activity_key(
            handle: i64,
            down: c_int,
            key_code: c_int,
            scan_code: c_int,
            meta_state: c_int,
            repeat_count: c_int,
            unicode_char: c_int,
            event_time_ms: i64,
            down_time_ms: i64,
            consumed: *mut c_int,
            err: *mut c_char,
            err_len: usize,
        ) -> c_int;
    }

