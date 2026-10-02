/// Deliver a synthesised mouse pointer event through `onTouchEventNative`.
///
/// `action` is an Android `MotionEvent.ACTION_*` constant. Returns
/// `Ok(Some(consumed))` on success; `Ok(None)` if `onTouchEventNative` has
/// not been registered yet, which happens for every call that arrives
/// before `initializeNativeCode` has finished — a normal race during
/// startup, not a failure. `x`/`y` are window-relative pixels, matching the
/// `dpiScale = 1.0` roblox-runtime reports in `PlatformParams`.
#[allow(clippy::too_many_arguments)]
pub fn touch(
    handle: i64,
    action: i32,
    x: f32,
    y: f32,
    button_state: i32,
    action_button: i32,
    event_time_ms: i64,
    down_time_ms: i64,
) -> Result<Option<bool>, String> {
    let mut err = vec![0u8; 512];
    let mut consumed: c_int = 0;
    // SAFETY: `handle` came from `initialize`; `err`/`consumed` are live.
    let rc = unsafe {
        roblox_game_activity_touch(
            handle,
            action,
            x,
            y,
            button_state,
            action_button,
            event_time_ms,
            down_time_ms,
            &mut consumed,
            err.as_mut_ptr() as *mut c_char,
            err.len(),
        )
    };
    match rc {
        0 => Ok(Some(consumed != 0)),
        -2 => Ok(None),
        _ => Err(take_err(err)),
    }
}

/// One finger on the glass, as the C side receives it.
///
/// Mirrors `RobloxRuntimeTouchContact` in `native/game_activity.cpp`; the two
/// definitions are the same three words in the same order and have to stay
/// that way. `id` is Android's stable *pointer id*, not the position of
/// this contact in the slice — the slice's order is the pointer *index*,
/// and the two stop agreeing the moment a finger that is not the last one
/// lifts.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchContact {
    pub id: i32,
    pub x: f32,
    pub y: f32,
}

/// Deliver a set of finger contacts through `onTouchEventNative`.
///
/// `contacts` is every contact still on the glass in pointer-index order,
/// including the one being lifted on an up action — Android reports a
/// departing pointer in the array of the event that says it left. `action`
/// is already packed with the pointer index for `ACTION_POINTER_DOWN`/`_UP`;
/// `android::input` owns that arithmetic. See `touch`'s doc comment for the
/// `Ok(None)` convention.
pub fn touch_multi(
    handle: i64,
    action: i32,
    contacts: &[TouchContact],
    event_time_ms: i64,
    down_time_ms: i64,
) -> Result<Option<bool>, String> {
    if contacts.is_empty() {
        return Err("a touch event with no contacts".into());
    }
    let mut err = vec![0u8; 512];
    let mut consumed: c_int = 0;
    // SAFETY: `handle` came from `initialize`; `contacts` is a live slice
    // borrowed for the duration of the call and the C side neither keeps
    // nor frees it; `err`/`consumed` are live.
    let rc = unsafe {
        roblox_game_activity_touch_multi(
            handle,
            action,
            contacts.as_ptr(),
            contacts.len() as c_int,
            event_time_ms,
            down_time_ms,
            &mut consumed,
            err.as_mut_ptr() as *mut c_char,
            err.len(),
        )
    };
    match rc {
        0 => Ok(Some(consumed != 0)),
        -2 => Ok(None),
        _ => Err(take_err(err)),
    }
}

/// Deliver a wheel movement through `onTouchEventNative` as ACTION_SCROLL.
///
/// `hscroll`/`vscroll` are detents, positive right and positive away from
/// the user, which is what `MotionEvent.AXIS_HSCROLL`/`AXIS_VSCROLL`
/// document. See `touch`'s doc comment for the `Ok(None)` convention.
pub fn scroll(
    handle: i64,
    x: f32,
    y: f32,
    hscroll: f32,
    vscroll: f32,
    event_time_ms: i64,
) -> Result<Option<bool>, String> {
    let mut err = vec![0u8; 512];
    let mut consumed: c_int = 0;
    // SAFETY: `handle` came from `initialize`; `err`/`consumed` are live.
    let rc = unsafe {
        roblox_game_activity_scroll(
            handle,
            x,
            y,
            hscroll,
            vscroll,
            event_time_ms,
            &mut consumed,
            err.as_mut_ptr() as *mut c_char,
            err.len(),
        )
    };
    match rc {
        0 => Ok(Some(consumed != 0)),
        -2 => Ok(None),
        _ => Err(take_err(err)),
    }
}

/// Deliver a synthesised key event through `onKeyDownNative`/`onKeyUpNative`.
///
/// See `touch`'s doc comment for the `Ok(None)` convention.
#[allow(clippy::too_many_arguments)]
pub fn key(
    handle: i64,
    down: bool,
    key_code: i32,
    scan_code: i32,
    meta_state: i32,
    repeat_count: i32,
    unicode_char: i32,
    event_time_ms: i64,
    down_time_ms: i64,
) -> Result<Option<bool>, String> {
    let mut err = vec![0u8; 512];
    let mut consumed: c_int = 0;
    // SAFETY: as above.
    let rc = unsafe {
        roblox_game_activity_key(
            handle,
            down as c_int,
            key_code,
            scan_code,
            meta_state,
            repeat_count,
            unicode_char,
            event_time_ms,
            down_time_ms,
            &mut consumed,
            err.as_mut_ptr() as *mut c_char,
            err.len(),
        )
    };
    match rc {
        0 => Ok(Some(consumed != 0)),
        -2 => Ok(None),
        _ => Err(take_err(err)),
    }
}

unsafe extern "C" {
    fn roblox_game_activity_lifecycle(
        handle: i64,
        native_name: *const c_char,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn roblox_game_activity_text_input(
        handle: i64,
        text: *const c_char,
        sel_start: c_int,
        sel_end: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_key_event(
        f: *mut c_void,
        down: c_int,
        key_code: c_int,
        modifiers: c_int,
        is_repeat: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_game_activity_surface_resized(
        handle: i64,
        format: c_int,
        width: c_int,
        height: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_update_keyboard_size(
        f: *mut c_void,
        visible: c_int,
        x: c_int,
        y: c_int,
        w: c_int,
        h: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_sync_textbox(
        f: *mut c_void,
        text: *const c_char,
        cursor: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_pass_text(
        f: *mut c_void,
        which: i64,
        text: *const c_char,
        flag: c_int,
        cursor: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_mouse_move(
        f: *mut c_void,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_mouse_button(
        f: *mut c_void,
        x: f32,
        y: f32,
        down: c_int,
        button: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_mouse_wheel(
        f: *mut c_void,
        x: f32,
        y: f32,
        delta: f32,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_pass_input(
        f: *mut c_void,
        pointer_id: c_int,
        x: f32,
        y: f32,
        action: c_int,
        width: c_int,
        height: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_set_touchscreen_present(present: c_int);
    fn roblox_input_gamepad_connect(
        f: *mut c_void,
        id: c_int,
        gamepad_type: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_gamepad_disconnect(
        f: *mut c_void,
        id: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_gamepad_button(
        f: *mut c_void,
        id: c_int,
        key_code: c_int,
        action: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_gamepad_axis(
        f: *mut c_void,
        id: c_int,
        axis: c_int,
        x: f32,
        y: f32,
        z: f32,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_gamepad_supported_key(
        f: *mut c_void,
        id: c_int,
        key_code: c_int,
        supported: c_int,
        gamepad_type: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_input_gamepad_supported_motion(
        f: *mut c_void,
        id: c_int,
        axis: c_int,
        source: c_int,
        supported: c_int,
        gamepad_type: c_int,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_textbox_handle() -> i64;
    fn roblox_textbox_generation() -> c_int;
    fn roblox_textbox_property_generation() -> c_int;
    fn roblox_textbox_text(buf: *mut c_char, n: c_int) -> c_int;
    fn roblox_registered_natives(class_name: *const c_char, out: *mut c_char, n: usize) -> c_int;
    fn roblox_textbox_info(out: *mut RawTextBoxInfo) -> c_int;
    fn roblox_textbox_info_now(
        f: *mut c_void,
        out: *mut RawTextBoxInfo,
        err: *mut c_char,
        n: usize,
    ) -> c_int;
    fn roblox_games_loaded() -> u32;
    fn roblox_last_place() -> i64;
    fn roblox_note_game_loaded(place_id: i64);
    fn roblox_game_activity_window_focus(
        handle: i64,
        focused: c_int,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn roblox_init_storage_manager(
        native: *mut c_void,
        a: *const c_char,
        b: *const c_char,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn roblox_game_activity_surface_redraw_needed(
        handle: i64,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn roblox_game_activity_set_input_connection(
        handle: i64,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn roblox_ime_state_generation() -> u32;
    fn roblox_ime_soft_keyboard_active() -> c_int;
    fn roblox_ime_state_text(buf: *mut c_char, n: c_int) -> c_int;
    fn roblox_ime_state_selection(start: *mut c_int, end: *mut c_int);
}

/// Record the game's NativeHelper callback in the runtime's shared join state.
pub fn note_game_loaded(place_id: i64) {
    // SAFETY: the runtime provides this atomic counter callback for the
    // process lifetime, alongside `roblox_games_loaded` above.
    unsafe { roblox_note_game_loaded(place_id) }
}
