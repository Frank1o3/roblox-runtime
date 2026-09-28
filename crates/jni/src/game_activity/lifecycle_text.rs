/// One `GameActivity` native shaped `(J)V` — `onPauseNative`,
/// `onStopNative`, `onSurfaceDestroyedNative`, and `terminateNativeCode`
/// at teardown. `Ok(None)` when `native_name` was never registered —
/// treated as "did not happen" rather than an error, matching
/// `touch`/`key`'s convention.
pub fn lifecycle(handle: i64, native_name: &str) -> Result<Option<()>, String> {
    let name = CString::new(native_name).map_err(|e| e.to_string())?;
    let mut err = vec![0u8; 512];
    // SAFETY: `handle` came from `initialize`; `name`/`err` outlive the call.
    let rc = unsafe {
        roblox_game_activity_lifecycle(
            handle,
            name.as_ptr(),
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

/// `onWindowFocusChangedNative(hasFocus)`. `start` already drives the
/// `true` case inline at bring-up; this is for the `false` case Android
/// sends immediately before `onPauseNative` when a run ends.
/// `GameActivity.onTextInputEventNative` — the whole field contents.
///
/// Android text fields receive state, not keystrokes, which is why keys
/// alone left the login form's boxes empty.
pub fn text_input(handle: i64, text: &str, sel_start: i32, sel_end: i32) -> Result<(), String> {
    let t = CString::new(text).map_err(|e| e.to_string())?;
    let mut err = vec![0u8; 512];
    // SAFETY: `t` and `err` outlive the call.
    let rc = unsafe {
        roblox_game_activity_text_input(
            handle,
            t.as_ptr(),
            sel_start,
            sel_end,
            err.as_mut_ptr() as *mut c_char,
            err.len(),
        )
    };
    if rc == 0 { Ok(()) } else { Err(take_err(err)) }
}

/// `NativeGLInterface.nativePassKeyEvent` — Roblox's own keyboard path.
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
pub unsafe fn pass_key_event(
    native: *mut c_void,
    down: bool,
    key_code: i32,
    modifiers: i32,
    is_repeat: bool,
) -> Result<(), String> {
    let mut err = vec![0u8; 512];
    // SAFETY: `err` outlives the call.
    let rc = unsafe {
        roblox_input_key_event(
            native,
            down as c_int,
            key_code,
            modifiers,
            is_repeat as c_int,
            err.as_mut_ptr() as *mut c_char,
            err.len(),
        )
    };
    if rc == 0 { Ok(()) } else { Err(take_err(err)) }
}

/// Which text box the engine has focus in, learned from `showKeyboard`.
/// `None` when nothing is focused — text must then not be sent at all,
/// rather than sent to handle 0, which is what left the login form empty.
pub fn focused_textbox() -> Option<i64> {
    // SAFETY: a plain atomic load on the C++ side.
    match unsafe { roblox_textbox_handle() } {
        0 => None,
        h => Some(h),
    }
}

/// Bumped on every focus change. Editing state keyed on this reseeds when
/// focus moves, without comparing handles — a handle can be reused once a
/// box is destroyed, so equal handles do not imply the same box.
pub fn textbox_generation() -> u32 {
    // SAFETY: a plain atomic load on the C++ side.
    unsafe { roblox_textbox_generation() as u32 }
}

/// Bumped by `onLuaTextBoxPropertyChangedCallback`, deliberately a
/// separate counter from [`textbox_generation`] — see the comment on
/// `g_textbox_property_generation` in `native/android_classes.cpp` for
/// why folding the two together would be wrong, not just redundant.
pub fn textbox_property_generation() -> u32 {
    // SAFETY: a plain atomic load on the C++ side.
    unsafe { roblox_textbox_property_generation() as u32 }
}

/// The spec for the editor that has to be drawn over the focused text box,
/// as the engine handed it to `showKeyboard`.
///
/// Layout must match `RobloxRuntimeTextBoxInfo` in `native/android_classes.cpp`
/// field-for-field — this is a `#[repr(C)]` mirror, not a coincidence.
///
/// The fourteen values are `NativeTextBoxInfo`'s constructor arguments,
/// `(FFFFFZIIIIIIZZ)`. Slots that have been identified by watching real
/// boxes on the Login screen carry a name; the rest keep their slot number,
/// because the APK's declarations give the class's field list sorted by
/// name and never say which order the constructor takes them in. The long
/// comment on `RobloxRuntimeTextBoxInfo` in `native/android_classes.cpp` has the
/// captured numbers and the argument from each of them; read it before
/// renaming anything here.
///
/// Do not assume a numbered slot means what the field list's reading order
/// would suggest. That guess put `textColor` at slot 7 and it is at slot 8.
///
/// **`x_alignment`/`y_alignment` (slots 6/7) are confirmed, not
/// `INFERRED`, as of 2026-08-30** -- corroborated rather than guessed, by
/// mocktail's `NativeTextBoxInfo` constructor
/// (`~/Projects/mocktail/src/jnivm/jnivm.cc:4016-4024`, Apache-2.0), which
/// declares the same six ints in the order `xAlignment, yAlignment,
/// textColor, font, textInputType, returnKeyType`. That order is a fact
/// about Roblox's platform API and is taken as one; the values this
/// struct actually carries were captured from Cordial's own boxes and are
/// not mocktail's. See the long comment on `RobloxRuntimeTextBoxInfo` in
/// `native/android_classes.cpp` for the rest of the reasoning, and for
/// what these names are evidence of: two positional readings of the same
/// constructor agreeing, not a reflection of the real Java class.
///
/// Every slot but the fifteenth carries a name as of 2026-09-04.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawTextBoxInfo {
    /// Left edge in surface pixels. Observed 470 for two boxes on a
    /// 1280-wide surface, one of them 340 wide, and 470 = (1280-340)/2.
    pub x: f32,
    /// Top edge in surface pixels; the coordinate that differed between two
    /// vertically stacked login fields.
    pub y: f32,
    pub width: f32,
    /// `INFERRED`. Observed 22 against a `font_size` of 16, and a 22px line
    /// box holding 16pt text is a text field where the reverse is not
    /// anything. Slots 3 and 4 are the two floats no observation has yet
    /// told apart.
    pub height: f32,
    /// `INFERRED`; see [`RawTextBoxInfo::height`].
    pub font_size: f32,
    /// Roblox's `TextBox.MultiLine`. Slot 5, settled 2026-09-04 by
    /// mocktail's constructor placing `textWrapped` at slot 13 -- which
    /// leaves 5 as the only candidate this project's own captures could
    /// not rule out. See `RobloxRuntimeTextBoxInfo` for both halves.
    pub multiline: i32,
    /// Roblox's `Enum.TextXAlignment`: `Left` = 0, `Right` = 1,
    /// `Center` = 2. Confirmed as slot 6 by mocktail's constructor field
    /// order; see this struct's own doc comment.
    ///
    /// **The ordinal-to-name mapping was wrong here until 2026-09-26.**
    /// This comment used to read `Left = 0, Center = 1, Right = 2` --
    /// alphabetical order, not Roblox's -- and nothing in this project had
    /// ever caught it: every real box captured so far reports `0`
    /// (`create.roblox.com/docs/reference/engine/enums/TextXAlignment`
    /// and `robloxapi.github.io/ref/enum/TextXAlignment.html`, checked
    /// against each other, both give `Left=0, Right=1, Center=2` -- the
    /// enum was reindexed at some point in Roblox's history, swapping
    /// `Right` and `Center`, and this struct's comment had the pre-reindex
    /// order). Fixed alongside `gtk_xalign` in
    /// `crates/cordial-shell/src/host_window.rs`, which is where the wrong
    /// ordinal actually reached a pixel: it drew `Right`-styled boxes
    /// centred and `Center`-styled boxes flush right.
    pub x_alignment: i32,
    /// Roblox's `Enum.TextYAlignment`: `Top` = 0, `Center` = 1,
    /// `Bottom` = 2. Confirmed as slot 7 alongside `x_alignment`.
    pub y_alignment: i32,
    /// Packed ARGB. Observed `0xffd5d5dd` on both login boxes, which is
    /// what identified this slot: nothing else in the class is a colour.
    pub text_color: i32,
    /// Roblox's font id for this box. Slot 9, which is what
    /// `editor_font::font_slot` already defaulted to.
    pub font: i32,
    /// Roblox's own input-type enum, not Android's `InputType`: the two
    /// login boxes reported 5 and 7, which are far too small for Android's
    /// packed class-plus-variation words. Slot 10.
    pub text_input_type: i32,
    /// Which action key the box asks for -- the two login boxes differed
    /// here, Next against Done. Slot 11.
    pub return_key_type: i32,
    /// Observed 1 on two single-line login fields, so this is *not*
    /// `multiline` — that is slot 5 or slot 13. Settling which wants a box
    /// that genuinely differs, such as an in-experience chat entry.
    /// Slot 12. Both login boxes reported 1.
    pub manual_focus_release: i32,
    /// Slot 13, which is what settles [`RawTextBoxInfo::multiline`].
    pub text_wrapped: i32,
    /// The fifteenth constructor slot. The dex signature is
    /// `(FFFFFZIIIIIIZZZ)` -- three trailing booleans, not two -- and
    /// omitting this one made the whole hook fail to match. Unnamed
    /// because nothing has established what it means, only that it exists.
    pub z14: i32,
}

/// The focused box's spec, or `None` when nothing is focused or the engine
/// gave Cordial no `NativeTextBoxInfo` for it.
///
/// `None` is not a zeroed box. A caller must not fall back to drawing an
/// editor at the origin: an editor in the wrong place reads as a layout bug
/// and hides the fact that the value never arrived.
/// How many times the engine has reported a place finished loading.
///
/// Both `gameLoadedCallback` and `onGameLoaded` bump it, because either
/// means the join completed and different builds have been seen to call
/// different ones. The join watchdog waits for this to move; it is a count
/// rather than a flag so a second join in the same session is visible.
pub fn games_loaded() -> u32 {
    // SAFETY: a plain atomic load on the C++ side.
    unsafe { roblox_games_loaded() }
}

/// The place id of the most recent load, or 0 if none has been reported.
pub fn last_place() -> i64 {
    // SAFETY: a plain atomic load on the C++ side.
    unsafe { roblox_last_place() }
}

/// `NativeGLInterface.nativeGetTextBoxInfo()` — the focused box's geometry
/// asked for now, rather than remembered from `showKeyboard`.
///
/// **Not a replacement for [`focused_textbox_info`], a second opinion when
/// that one is unusable.** The engine volunteers a spec at focus time and
/// sometimes volunteers it too early: Roblox's search modal is focused with
/// `w=0 h=0` and this call returns real geometry for the same box about a
/// second later. It is also null for the whole of the sign-in page, so
/// `Ok(None)` is an ordinary answer and not a failure.
///
/// `native` is `Java_com_roblox_engine_jni_NativeGLInterface_nativeGetTextBoxInfo`,
/// resolved by the loader. Calling this costs a JNI call and a Java object,
/// so the caller owns the decision about how often — see `sync_text_overlay`.
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
pub unsafe fn textbox_info_now(native: *mut c_void) -> Result<Option<RawTextBoxInfo>, String> {
    let mut info = RawTextBoxInfo::default();
    let mut err = vec![0u8; 512];
    // SAFETY: `info` is a live, fully initialised mirror of the C++ struct
    // and `err` outlives the call; both are only written into.
    let rc = unsafe {
        roblox_textbox_info_now(
            native,
            &mut info as *mut RawTextBoxInfo,
            err.as_mut_ptr() as *mut c_char,
            err.len(),
        )
    };
    match rc {
        1 => Ok(Some(info)),
        0 => Ok(None),
        _ => Err(take_err(err)),
    }
}

pub fn focused_textbox_info() -> Option<RawTextBoxInfo> {
    let mut info = RawTextBoxInfo::default();
    // SAFETY: `info` is a live, fully initialised mirror of the C++ struct
    // and outlives the call, which only copies into it.
    let known = unsafe { roblox_textbox_info(&mut info as *mut RawTextBoxInfo) };
    if known == 0 { None } else { Some(info) }
}

/// Every native the engine has registered on `class_name`, and where each
/// points, or an empty list when it has registered none.
///
/// **This answers a question that has been argued rather than looked up.**
/// A native registered through `RegisterNatives` never appears in `nm -D`,
/// so an exported-symbol table says nothing about whether the engine drives
/// a class -- and `docs/HANDOVER.md` concluded for weeks that voice chat's
/// downlink "cannot be written" from exactly that absence. Cordial has
/// depended on the distinction since `terminateNativeCode` (see
/// `native/game_activity.cpp`) without being able to see it.
///
/// The pointer is reported beside the name because "registered" and
/// "registered to something real" are different claims.
pub fn registered_natives(class_name: &str) -> String {
    let Ok(c) = CString::new(class_name) else {
        return String::new();
    };
    let mut buf = vec![0u8; 8192];
    // SAFETY: both buffers outlive the call and their lengths are passed.
    let n = unsafe {
        roblox_registered_natives(c.as_ptr(), buf.as_mut_ptr() as *mut c_char, buf.len())
    };
    if n < 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// The focused box's contents, as the engine reported them at focus time.
pub fn textbox_text() -> String {
    let mut buf = vec![0u8; 4096];
    // SAFETY: `buf` is writable for its full length and outlives the call.
    let n = unsafe { roblox_textbox_text(buf.as_mut_ptr() as *mut c_char, buf.len() as c_int) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf8_lossy(&buf[..n as usize]).into_owned()
}
