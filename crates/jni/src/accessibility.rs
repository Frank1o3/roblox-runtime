/// The accessibility mirror `native/accessibility.cpp` builds from whatever
/// `AccessibilityNodeInfo`/`AccessibilityManager`/`AccessibilityEvent` calls
/// Roblox's engine makes over JNI. Kept as its own top-level module rather
/// than folded into [`game_activity`]: unlike everything else there, nothing
/// here is on the render/input critical path, and
/// `crates/cordial-runtime/src/android/accessibility.rs` is the only caller,
/// so a clean boundary keeps that file's own header comment about what is and
/// is not verified from getting lost among touch/key/IME plumbing.
pub mod accessibility {
    use std::ffi::{CString, c_char, c_int};

    /// Layout must match `RobloxRuntimeA11yNode` in `native/accessibility.cpp`
    /// field-for-field — this is a `#[repr(C)]` mirror, not a coincidence.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct RawNode {
        pub id: u32,
        pub class_name: [c_char; 128],
        pub text: [c_char; 256],
        pub content_description: [c_char; 256],
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
        pub state: u32,
        pub actions: [i32; 16],
        pub action_count: u32,
    }

    impl Default for RawNode {
        fn default() -> Self {
            // SAFETY: an all-zero `RawNode` is a valid value for every field
            // — the char arrays are NUL already, every integer's zero is a
            // legitimate zero, not an uninitialised trap representation.
            unsafe { std::mem::zeroed() }
        }
    }

    fn cstr_field(buf: &[c_char]) -> String {
        // SAFETY: `buf` is a NUL-terminated `snprintf` target on the C++
        // side, always at least one byte, so a NUL is guaranteed to exist
        // within `buf`'s own length.
        let bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len()) };
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }

    /// A [`RawNode`], decoded into owned Rust types. What
    /// `crates/cordial-runtime/src/android/accessibility.rs` actually works
    /// with — the raw struct exists only to cross the FFI boundary cheaply.
    #[derive(Clone, Debug, Default)]
    pub struct Node {
        pub id: u32,
        pub class_name: String,
        pub text: String,
        pub content_description: String,
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
        pub state: u32,
        pub actions: Vec<i32>,
    }

    impl From<RawNode> for Node {
        fn from(r: RawNode) -> Self {
            let n = r.action_count as usize;
            let n = n.min(r.actions.len());
            Node {
                id: r.id,
                class_name: cstr_field(&r.class_name),
                text: cstr_field(&r.text),
                content_description: cstr_field(&r.content_description),
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
                state: r.state,
                actions: r.actions[..n].to_vec(),
            }
        }
    }

    /// State bits, mirrored from the `NodeStateBit` enum in
    /// `native/accessibility.cpp` — this file's own bit layout, translated
    /// into real AT-SPI `StateType` ordinals on the `accessibility.rs` side.
    pub mod state_bit {
        pub const CHECKABLE: u32 = 1 << 0;
        pub const CHECKED: u32 = 1 << 1;
        pub const CLICKABLE: u32 = 1 << 2;
        pub const ENABLED: u32 = 1 << 3;
        pub const FOCUSABLE: u32 = 1 << 4;
        pub const FOCUSED: u32 = 1 << 5;
        pub const LONG_CLICKABLE: u32 = 1 << 6;
        pub const PASSWORD: u32 = 1 << 7;
        pub const SCROLLABLE: u32 = 1 << 8;
        pub const SELECTED: u32 = 1 << 9;
        pub const VISIBLE_TO_USER: u32 = 1 << 10;
    }

    unsafe extern "C" {
        fn roblox_accessibility_set_bridge_connected(connected: c_int);
        fn roblox_accessibility_snapshot(out: *mut RawNode, max: usize) -> usize;
        fn roblox_accessibility_node_count() -> usize;
        fn roblox_accessibility_generation() -> u32;
        fn roblox_accessibility_next_event(
            event_type: *mut c_int,
            class_name_buf: *mut c_char,
            cn_len: c_int,
            text_buf: *mut c_char,
            text_len: c_int,
        ) -> c_int;
        fn roblox_accessibility_test_seed_node(
            class_name: *const c_char,
            text: *const c_char,
            content_description: *const c_char,
            left: c_int,
            top: c_int,
            right: c_int,
            bottom: c_int,
            state: u32,
        ) -> u32;
        fn roblox_accessibility_test_clear();
    }

    /// Tell `AccessibilityManager.isEnabled()` whether to answer true.
    /// `accessibility.rs` calls this once, after it knows whether it managed
    /// to attach to the AT-SPI bus — see that file for why the gate lives on
    /// this side of the boundary rather than the engine blocking a JNI call
    /// on a D-Bus round-trip.
    pub fn set_bridge_connected(connected: bool) {
        // SAFETY: a plain atomic store on the C++ side, no aliasing concerns.
        unsafe { roblox_accessibility_set_bridge_connected(connected as c_int) };
    }

    /// Every node currently in the mirror. Not cheap — copies the whole
    /// registry — so callers should gate this on [`generation`] having
    /// changed rather than call it on a tight poll loop.
    pub fn snapshot() -> Vec<Node> {
        // SAFETY: a first call establishes how many nodes exist; a second,
        // sized to match, copies them. A node added between the two calls is
        // simply missed until the next poll, which is fine for a bridge whose
        // whole design is "poll and diff", not "exactly once".
        let count = unsafe { roblox_accessibility_node_count() };
        if count == 0 {
            return Vec::new();
        }
        let mut buf = vec![RawNode::default(); count];
        let written = unsafe { roblox_accessibility_snapshot(buf.as_mut_ptr(), buf.len()) };
        buf.truncate(written);
        buf.into_iter().map(Node::from).collect()
    }

    /// Bumped on every node add/change/recycle. Cheap to poll — a plain
    /// atomic load on the C++ side — which is the point: a caller can spin a
    /// loop on this without the cost `snapshot()` would carry.
    pub fn generation() -> u32 {
        // SAFETY: a plain atomic load on the C++ side.
        unsafe { roblox_accessibility_generation() }
    }

    /// One pending `AccessibilityManager.sendAccessibilityEvent` call, if any
    /// is queued. `(event_type, class_name, text)`. Callers should drain in a
    /// loop until this returns `None` — the engine can queue events faster
    /// than a poll loop drains them.
    pub fn next_event() -> Option<(i32, String, String)> {
        let mut event_type: c_int = 0;
        let mut cn = vec![0u8; 256];
        let mut text = vec![0u8; 512];
        // SAFETY: `cn`/`text` are writable for their full length and outlive
        // the call; `event_type` is a live out-parameter.
        let got = unsafe {
            roblox_accessibility_next_event(
                &mut event_type,
                cn.as_mut_ptr() as *mut c_char,
                cn.len() as c_int,
                text.as_mut_ptr() as *mut c_char,
                text.len() as c_int,
            )
        };
        if got == 0 {
            return None;
        }
        let cn_end = cn.iter().position(|&b| b == 0).unwrap_or(cn.len());
        let text_end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
        Some((
            event_type as i32,
            String::from_utf8_lossy(&cn[..cn_end]).into_owned(),
            String::from_utf8_lossy(&text[..text_end]).into_owned(),
        ))
    }

    /// Inject one synthetic node, bypassing JNI entirely. Test-only — see
    /// `roblox_accessibility_test_seed_node`'s own doc comment in
    /// `native/accessibility.cpp` for why this exists and why it is not a
    /// second way for Roblox to reach the registry. Returns the assigned id.
    pub fn test_seed_node(
        class_name: &str,
        text: &str,
        content_description: &str,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        state: u32,
    ) -> u32 {
        let class_name = CString::new(class_name).unwrap_or_default();
        let text = CString::new(text).unwrap_or_default();
        let content_description = CString::new(content_description).unwrap_or_default();
        // SAFETY: all three `CString`s outlive the call.
        unsafe {
            roblox_accessibility_test_seed_node(
                class_name.as_ptr(),
                text.as_ptr(),
                content_description.as_ptr(),
                left,
                top,
                right,
                bottom,
                state,
            )
        }
    }

    /// Drop every node, seeded or (in principle) real. Test-only.
    pub fn test_clear() {
        // SAFETY: no arguments, no aliasing concerns.
        unsafe { roblox_accessibility_test_clear() };
    }
}
