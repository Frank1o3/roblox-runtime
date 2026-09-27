    /// Re-drive `onSurfaceChangedNative` after the host window is resized.
    pub fn surface_resized(
        handle: i64,
        format: i32,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `err` outlives the call.
        let rc = unsafe {
            roblox_game_activity_surface_resized(
                handle,
                format,
                width,
                height,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeGLInterface.updateKeyboardSize` — tells the engine an editor is
    /// up. Without it the engine focuses a box but never starts capturing.
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
    pub unsafe fn update_keyboard_size(
        native: *mut c_void,
        visible: bool,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `err` outlives the call.
        let rc = unsafe {
            roblox_input_update_keyboard_size(
                native,
                visible as c_int,
                x,
                y,
                w,
                h,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeGLInterface.syncTextboxTextAndCursorPosition2` — the per-keystroke
    /// text update. Takes no box handle: it applies to whatever has focus.
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
    pub unsafe fn sync_textbox(native: *mut c_void, text: &str, cursor: i32) -> Result<(), String> {
        let t = CString::new(text).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `t` and `err` outlive the call.
        let rc = unsafe {
            roblox_input_sync_textbox(
                native,
                t.as_ptr(),
                cursor,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeGLInterface.nativePassText` — text entered into a focused box.
    ///
    /// `which` is the handle from `showKeyboard`, which is how the engine knows
    /// which box the text belongs to.
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
    pub unsafe fn pass_text(
        native: *mut c_void,
        which: i64,
        text: &str,
        flag: bool,
        cursor: i32,
    ) -> Result<(), String> {
        let t = CString::new(text).map_err(|e| e.to_string())?;
        let mut err = vec![0u8; 512];
        // SAFETY: `t` and `err` outlive the call.
        let rc = unsafe {
            roblox_input_pass_text(
                native,
                which,
                t.as_ptr(),
                flag as c_int,
                cursor,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeInputInterface.nativePassMouseMove` — the path Roblox's interface
    /// actually reads, as distinct from AGDK's `onTouchEventNative`.
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
    pub unsafe fn pass_mouse_move(
        native: *mut c_void,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `err` outlives the call.
        let rc = unsafe {
            roblox_input_mouse_move(
                native,
                x,
                y,
                dx,
                dy,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeInputInterface.nativePassMouseButton`.
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
    pub unsafe fn pass_mouse_button(
        native: *mut c_void,
        x: f32,
        y: f32,
        down: bool,
        button: i32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe {
            roblox_input_mouse_button(
                native,
                x,
                y,
                if down { 1 } else { 0 },
                button,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `NativeInputInterface.nativePassMouseWheel(F,F,F)` — the wheel's
    /// equivalent of [`pass_mouse_button`], and the call Cordial had never
    /// made. `delta` is in detents, positive away from the user.
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
    pub unsafe fn pass_mouse_wheel(
        native: *mut c_void,
        x: f32,
        y: f32,
        delta: f32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe {
            roblox_input_mouse_wheel(
                native,
                x,
                y,
                delta,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// Tell `PlatformParams`/`Configuration` whether this host has a
    /// touchscreen, before the engine is initialised and asks.
    ///
    /// `android::input::report_touchscreen` is the only caller and owns the
    /// policy — the seat's answer, and what `RBX_RUNTIME_INPUT_TOUCH` and
    /// `RBX_RUNTIME_NO_TOUCH` do to it. This is only the wire. Nothing on the C
    /// side reads it after startup; see `roblox_set_touchscreen_present` in
    /// `native/init_params.cpp` for why that is ordering rather than policy.
    pub fn set_touchscreen_present(present: bool) {
        // SAFETY: a plain store into a C++ `std::atomic<int>`; no pointers, no
        // allocation, and safe to call from any thread at any time.
        unsafe { roblox_set_touchscreen_present(if present { 1 } else { 0 }) }
    }

    /// `NativeInputInterface.nativePassInput(I,F,F,I,I,I)` — one finger, one
    /// call.
    ///
    /// The descriptor is read out of this build's dex; the three `action`
    /// values are `INFERRED` from mocktail. See `roblox_input_pass_input` in
    /// `native/game_activity.cpp` for both, and `android::input::TOUCH_*` for
    /// the constants themselves.
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
    pub unsafe fn pass_input(
        native: *mut c_void,
        pointer_id: i32,
        x: f32,
        y: f32,
        action: i32,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe {
            roblox_input_pass_input(
                native,
                pointer_id,
                x,
                y,
                action,
                width,
                height,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    // `NativeInputInterface`'s six gamepad natives. Descriptors read from the
    // shipping APK's dex, not guessed; what the integer arguments *mean* is
    // INFERRED and is argued out in `native/game_activity.cpp` beside each
    // trampoline, and in `roblox_runtime::android::gamepad`. Callers should go
    // through that module rather than these directly -- it is what holds the
    // all-or-nothing resolution gate and the off switch.

    /// `nativeGamepadConnectEventWithGamepadType(I id, I gamepadType)`.
    ///
    /// **This build ships no type-less connect**, so there is no way to announce
    /// a pad without naming a type, and no evidence available here says what the
    /// ordinals are. See `android::gamepad::gamepad_type`.
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
    pub unsafe fn gamepad_connect(
        native: *mut c_void,
        id: i32,
        gamepad_type: i32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `native` is the exported JNI native; `err` outlives the call.
        let rc = unsafe {
            roblox_input_gamepad_connect(
                native,
                id,
                gamepad_type,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `nativeGamepadDisconnectEvent(I id)` — no type, because the engine kept
    /// the one it was given at connect.
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
    pub unsafe fn gamepad_disconnect(native: *mut c_void, id: i32) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe {
            roblox_input_gamepad_disconnect(native, id, err.as_mut_ptr() as *mut c_char, err.len())
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `nativeGamepadButtonEvent(I id, I keyCode, I action)`.
    ///
    /// `key_code` is read as an Android `KeyEvent.KEYCODE_BUTTON_*` and `action`
    /// as `ACTION_DOWN`/`ACTION_UP`. INFERRED from the Android platform contract
    /// the Java caller would have been working to; nothing observed it.
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
    pub unsafe fn gamepad_button(
        native: *mut c_void,
        id: i32,
        key_code: i32,
        action: i32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe {
            roblox_input_gamepad_button(
                native,
                id,
                key_code,
                action,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `nativeGamepadAxisEvent(I id, I axis, F x, F y, F z)`.
    ///
    /// Three floats read as a `Vector3`, which is what Roblox's Lua
    /// `InputObject.Position` is for a thumbstick. INFERRED with no control
    /// behind it — the TV-remote family has no axis method to difference against.
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
    pub unsafe fn gamepad_axis(
        native: *mut c_void,
        id: i32,
        axis: i32,
        x: f32,
        y: f32,
        z: f32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe {
            roblox_input_gamepad_axis(
                native,
                id,
                axis,
                x,
                y,
                z,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `nativeSetGamepadSupportedKeyWithGamepadType(I id, I keyCode, Z supported, I gamepadType)`
    /// — one call per button, before any button event.
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
    pub unsafe fn gamepad_supported_key(
        native: *mut c_void,
        id: i32,
        key_code: i32,
        supported: bool,
        gamepad_type: i32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe {
            roblox_input_gamepad_supported_key(
                native,
                id,
                key_code,
                if supported { 1 } else { 0 },
                gamepad_type,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    /// `nativeSetGamepadSupportedMotionWithGamepadType(I id, I axis, I source, Z supported, I gamepadType)`.
    ///
    /// The middle pair is read as Android's `(axis, source)` motion-range key.
    /// The least established of the six: `(IIIZI)` has one more `int` than the
    /// key variant and nothing to difference it against.
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
    pub unsafe fn gamepad_supported_motion(
        native: *mut c_void,
        id: i32,
        axis: i32,
        source: i32,
        supported: bool,
        gamepad_type: i32,
    ) -> Result<(), String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe {
            roblox_input_gamepad_supported_motion(
                native,
                id,
                axis,
                source,
                if supported { 1 } else { 0 },
                gamepad_type,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(take_err(err)) }
    }

    pub fn window_focus(handle: i64, focused: bool) -> Result<Option<()>, String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `handle` came from `initialize`; `err` is a live buffer.
        let rc = unsafe {
            roblox_game_activity_window_focus(
                handle,
                if focused { 1 } else { 0 },
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        match rc {
            0 => Ok(Some(())),
            -2 => Ok(None),
            _ => Err(take_err(err)),
        }
    }

    /// `onSurfaceRedrawNeededNative` — the "repaint now" nudge, driven from
    /// X11 `Expose`.
    pub fn surface_redraw_needed(handle: i64) -> Result<Option<()>, String> {
        let mut err = vec![0u8; 512];
        // SAFETY: as above.
        let rc = unsafe {
            roblox_game_activity_surface_redraw_needed(
                handle,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        match rc {
            0 => Ok(Some(())),
            -2 => Ok(None),
            _ => Err(take_err(err)),
        }
    }

    /// `GameActivity.setInputConnectionNative` — hands the engine the
    /// `InputConnection` it will later call `setState`/`setSoftKeyboardActive`/
    /// `restartInput` on. Meant to be driven once, early (see the call site in
    /// `load.rs`), not per frame — a second call would construct and register a
    /// second `InputConnection` C++ side, but the engine keeps calling back on
    /// whichever one it saw first, so nothing after the first call would ever
    /// be reached anyway.
    pub fn set_input_connection(handle: i64) -> Result<Option<()>, String> {
        let mut err = vec![0u8; 512];
        // SAFETY: `err` outlives the call.
        let rc = unsafe {
            roblox_game_activity_set_input_connection(
                handle,
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        match rc {
            0 => Ok(Some(())),
            -2 => Ok(None),
            _ => Err(take_err(err)),
        }
    }

    /// Bumped on every `InputConnection.setState`/`restartInput` the engine has
    /// made — the outbound half of the IME contract, as distinct from
    /// [`textbox_generation`]'s focus-change counter driven by `showKeyboard`.
    pub fn ime_state_generation() -> u32 {
        // SAFETY: a plain atomic load on the C++ side.
        unsafe { roblox_ime_state_generation() }
    }

    /// Whether the engine last asked for a soft keyboard via
    /// `InputConnection.setSoftKeyboardActive`. Not currently wired to
    /// anything — `updateKeyboardSize` remains the outbound acknowledgement
    /// path (see `android::input`'s `keyboard_report_enabled` doc) — kept
    /// available for whichever future change replaces it, so that decision
    /// does not also have to rediscover how to read this flag.
    pub fn ime_soft_keyboard_active() -> bool {
        // SAFETY: a plain atomic load on the C++ side.
        unsafe { roblox_ime_soft_keyboard_active() != 0 }
    }

    /// The text `InputConnection.setState` last reported, i.e. what the engine
    /// itself currently believes the focused field contains — the "real"
    /// contents `android::input`'s reseed should prefer over `showKeyboard`'s
    /// one-shot byte array once a `setState` has actually been observed for
    /// the current focus.
    pub fn ime_state_text() -> String {
        let mut buf = vec![0u8; 4096];
        // SAFETY: `buf` is writable for its full length and outlives the call.
        let n =
            unsafe { roblox_ime_state_text(buf.as_mut_ptr() as *mut c_char, buf.len() as c_int) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf8_lossy(&buf[..n as usize]).into_owned()
    }

    /// The selection `InputConnection.setState` last reported, as a
    /// `(start, end)` char-offset pair. Roblox's own text boxes are
    /// single-caret in practice, so `android::input` collapses this to
    /// `end` for its own caret; the pair is returned as-is in case a future
    /// caller needs a real selection range.
    pub fn ime_state_selection() -> (i32, i32) {
        let mut start: c_int = 0;
        let mut end: c_int = 0;
        // SAFETY: `start`/`end` are live out-parameters for the call.
        unsafe { roblox_ime_state_selection(&mut start, &mut end) };
        (start, end)
    }

    unsafe extern "C" {
        fn roblox_textbox_test_focus(
            handle: i64,
            text: *const c_char,
            f0: f32,
            f1: f32,
            f2: f32,
            f3: f32,
            f4: f32,
            multiline: c_int,
            x_alignment: c_int,
            y_alignment: c_int,
            text_color: c_int,
            font: c_int,
            text_input_type: c_int,
            return_key_type: c_int,
            manual_focus_release: c_int,
            text_wrapped: c_int,
            z14: c_int,
        );
        fn roblox_textbox_blurred();
    }

    /// Synthesise a focused `NativeTextBoxInfo` exactly as the engine's own
    /// `showKeyboard`/`<init>` hook would -- for devctl's `fakefocus` verb,
    /// added alongside the multi-line `gtk::TextView` overlay so its GTK side
    /// can be exercised without a running game.
    ///
    /// **Test-only, and the engine never calls this.**
    /// `native/android_classes.cpp` built `roblox_textbox_test_focus` for
    /// this module's own `textbox_info_arrives_slot_for_slot` test below, to
    /// prove the fifteen-slot layout survives the FFI boundary intact. This is
    /// the same round trip, made reachable from outside the crate so devctl
    /// can drive `WaylandWindow::sync_text_overlay`'s real placement and
    /// styling code with a `multiline=1` box on demand. A reading taken
    /// through it establishes that the GTK side does what the spec says --
    /// it establishes nothing about what shape of spec a real multi-line
    /// Roblox `TextBox` sends, which no capture in this project has yet held.
    /// Say so plainly wherever a reading taken this way is reported.
    pub fn test_focus_textbox(handle: i64, text: &str, info: RawTextBoxInfo) {
        let text = CString::new(text.replace('\0', "")).unwrap_or_default();
        // SAFETY: `text` outlives the call, which only reads it, same as
        // `textbox_info_arrives_slot_for_slot` below.
        unsafe {
            roblox_textbox_test_focus(
                handle,
                text.as_ptr(),
                info.x,
                info.y,
                info.width,
                info.height,
                info.font_size,
                info.multiline,
                info.x_alignment,
                info.y_alignment,
                info.text_color,
                info.font,
                info.text_input_type,
                info.return_key_type,
                info.manual_focus_release,
                info.text_wrapped,
                info.z14,
            )
        }
    }

    /// Clear whatever [`test_focus_textbox`] focused -- the same
    /// `roblox_textbox_blurred` call `hideKeyboard` makes, declared again in
    /// this `extern` block so the binding exists outside `#[cfg(test)]` too.
    pub fn test_blur_textbox() {
        // SAFETY: no arguments.
        unsafe { roblox_textbox_blurred() }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        unsafe extern "C" {
            fn roblox_textbox_focused(
                handle: i64,
                text: *const c_char,
                info: *const RawTextBoxInfo,
            );
            fn roblox_textbox_blurred();
            #[allow(clippy::too_many_arguments)]
            fn roblox_textbox_test_focus(
                handle: i64,
                text: *const c_char,
                f0: f32,
                f1: f32,
                f2: f32,
                f3: f32,
                f4: f32,
                multiline: c_int,
                x_alignment: c_int,
                y_alignment: c_int,
                text_color: c_int,
                font: c_int,
                text_input_type: c_int,
                return_key_type: c_int,
                manual_focus_release: c_int,
                text_wrapped: c_int,
                z14: c_int,
            );
        }

        /// The fifteen values arriving in the slots they were sent to, with
        /// C++ naming the members on its side and Rust naming them again on
        /// this one. That is what makes the two layouts have to agree: a test
        /// that handed a Rust-built struct to `roblox_textbox_focused` and
        /// read it back passes with the mirror shifted by a field, because
        /// nothing between the two ever looks inside.
        ///
        /// Every value is distinct so a drift of one slot cannot land on an
        /// equal number and go unseen.
        ///
        /// This says nothing about which slot means what — that is the engine's
        /// to answer, not this crate's.
        ///
        /// One test rather than several: the focused box is process-wide state,
        /// and the test harness runs tests on threads, so two of these would
        /// race each other into an intermittent failure of exactly the kind
        /// this repository has already spent time chasing.
        #[test]
        fn textbox_info_arrives_slot_for_slot() {
            let text = CString::new("hello").expect("no interior NUL");
            // SAFETY: `text` outlives the call, which only reads it.
            unsafe {
                roblox_textbox_test_focus(
                    42,
                    text.as_ptr(),
                    1.5,
                    2.5,
                    3.5,
                    4.5,
                    5.5,
                    1,
                    6,
                    7,
                    8,
                    9,
                    10,
                    11,
                    0,
                    1,
                    1,
                )
            };

            assert_eq!(focused_textbox(), Some(42));
            assert_eq!(
                focused_textbox_info(),
                Some(RawTextBoxInfo {
                    x: 1.5,
                    y: 2.5,
                    width: 3.5,
                    height: 4.5,
                    font_size: 5.5,
                    multiline: 1,
                    x_alignment: 6,
                    y_alignment: 7,
                    text_color: 8,
                    font: 9,
                    text_input_type: 10,
                    return_key_type: 11,
                    manual_focus_release: 0,
                    text_wrapped: 1,
                    z14: 1,
                })
            );

            // SAFETY: no arguments; clears the global the assertions above set.
            unsafe { roblox_textbox_blurred() };
            assert_eq!(focused_textbox(), None);
            // Blur has to drop the spec too, or a caller keeps an editor up
            // over a box that no longer has focus.
            assert_eq!(focused_textbox_info(), None);

            // A focus the engine supplied no spec for reports none rather than
            // a zeroed box, and does not resurrect the previous box's numbers.
            // Nothing has run `NativeTextBoxInfo.<init>` in this process, so
            // the last-built fallback is empty too.
            // SAFETY: `text` outlives the call; a null spec is the documented
            // "the engine gave us nothing" case.
            unsafe { roblox_textbox_focused(7, text.as_ptr(), std::ptr::null()) };
            assert_eq!(focused_textbox(), Some(7));
            assert_eq!(focused_textbox_info(), None);

            // SAFETY: no arguments.
            unsafe { roblox_textbox_blurred() };
        }
    }
