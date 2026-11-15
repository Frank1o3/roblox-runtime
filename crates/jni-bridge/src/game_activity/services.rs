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
/// never printed on either side; see `crate::identity` in `roblox-runtime`
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
/// equivalent) against a `libroblox.so` roblox-runtime has `dlopen`'d and never
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
/// equivalent) against a `libroblox.so` roblox-runtime has `dlopen`'d and never
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
/// equivalent) against a `libroblox.so` roblox-runtime has `dlopen`'d and never
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
/// equivalent) against a `libroblox.so` roblox-runtime has `dlopen`'d and never
/// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
/// native with from the process's own `JavaVM`, not from anything passed
/// here -- so the one thing this call cannot check is that `native` really
/// is that resolved export and not a stale, null-masked, or wrong pointer.
pub unsafe fn set_device_info(native: *mut c_void, width: i32, height: i32) -> Result<(), String> {
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
/// equivalent) against a `libroblox.so` roblox-runtime has `dlopen`'d and never
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
/// roblox-runtime drives (its only dex caller is a different startup path), so
/// nothing else here calls it unless a caller in `load.rs` does.
///
/// # Safety
///
/// `native` must be a live pointer to the exported JNI native this call
/// names, obtained via [`Library::symbol`] (or the module-level dlsym
/// equivalent) against a `libroblox.so` roblox-runtime has `dlopen`'d and never
/// `dlclose`s. The C shim supplies the `JNIEnv`/`jobject` it invokes the
/// native with from the process's own `JavaVM`, not from anything passed
/// here -- so the one thing this call cannot check is that `native` really
/// is that resolved export and not a stale, null-masked, or wrong pointer.
pub unsafe fn read_local_flags(native: *mut c_void) -> Result<(), String> {
    let mut err = vec![0u8; 512];
    // SAFETY: `native` is the exported JNI native; `err` is a live buffer.
    let rc = unsafe { roblox_read_local_flags(native, err.as_mut_ptr() as *mut c_char, err.len()) };
    if rc == 0 { Ok(()) } else { Err(take_err(err)) }
}

/// A no-argument native on a named class. `nativeAppBridgeAppStart` is on
/// `NativeAppBridgeInterface`, not `NativeGLInterface`.
///
/// # Safety
///
/// `native` must be a live pointer to the exported JNI native this call
/// names, obtained via [`Library::symbol`] (or the module-level dlsym
/// equivalent) against a `libroblox.so` roblox-runtime has `dlopen`'d and never
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
/// what the real app calls after fetching client settings itself. roblox-runtime
/// *is* the host app in this architecture, so this is the legitimate
/// interface, not a workaround. Returns the engine's own `int` result
/// code, which is a better signal than anything printed to the log.
///
/// # Safety
///
/// `native` must be a live pointer to the exported JNI native this call
/// names, obtained via [`Library::symbol`] (or the module-level dlsym
/// equivalent) against a `libroblox.so` roblox-runtime has `dlopen`'d and never
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
/// roblox-runtime has only ever used the plain three-string form, so every launch
/// has looked cold to the engine even with `flag_cache.dat` on disk beside
/// it. Returns the engine's own `int`, on the same reasoning as
/// [`init_client_settings`]: the result code is a better signal than the log.
#[allow(clippy::too_many_arguments)]
/// # Safety
///
/// `native` must be a live pointer to the exported JNI native this call
/// names, obtained via [`Library::symbol`] (or the module-level dlsym
/// equivalent) against a `libroblox.so` roblox-runtime has `dlopen`'d and never
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
