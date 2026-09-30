//! Host handoff for Roblox's native web view requests.
//!
//! Roblox reaches this API through `NativeGLJavaInterface.openNativeOverlay`.
//! The JNI call can arrive on an engine worker, so this module only copies the
//! request into a bounded, thread-safe slot. The window host takes it on its UI
//! thread and presents it with the browser backend it chooses.
//!
//! Keeping the browser engine out of this crate lets embedders use their
//! platform browser and keeps GTK/WebKit out of runtime-only consumers. On
//! Linux, WebKitGTK is a practical system dependency for a host that needs a
//! full browser engine; it should render in a host-owned UI widget, because the
//! Wayland text overlay surface is a shared-memory painter, not a browser
//! render target.

use std::ffi::{CStr, c_char};
use std::sync::{Mutex, OnceLock};

/// A Roblox request to show an in-experience page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebViewRequest {
    pub url: String,
    pub title: String,
}

static REQUEST: OnceLock<Mutex<Option<WebViewRequest>>> = OnceLock::new();

fn request_slot() -> &'static Mutex<Option<WebViewRequest>> {
    REQUEST.get_or_init(|| Mutex::new(None))
}

/// Take the latest pending request, if Roblox has asked to open a page.
///
/// Call this from the host's UI thread. A newer request replaces an older
/// request that the host has not consumed yet, so a stalled UI cannot grow an
/// unbounded queue or open a backlog of stale pages.
pub fn take_request() -> Option<WebViewRequest> {
    request_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

/// Discard an unconsumed request during host shutdown.
pub fn clear_pending_request() {
    request_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
}

const MAX_URL_BYTES: usize = 16 * 1024;
const MAX_TITLE_BYTES: usize = 1024;

/// Called by the JNI compatibility shim after Roblox invokes
/// `openNativeOverlay(String, String)`.
///
/// The arguments are borrowed NUL-terminated UTF-8-compatible strings owned by
/// the shim and remain valid for the duration of this call. Query strings are
/// intentionally neither logged nor normalized here; the host must apply its
/// browser navigation policy before loading the URL.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn roblox_runtime_webview_open(url: *const c_char, title: *const c_char) {
    if url.is_null() || title.is_null() {
        eprintln!("[runtime] Roblox requested a web view with a missing URL or title");
        return;
    }

    // SAFETY: the C++ shim passes `std::string::c_str()` values for this call;
    // both are NUL terminated and live until the function returns.
    let (url, title) = unsafe { (CStr::from_ptr(url), CStr::from_ptr(title)) };
    let url = String::from_utf8_lossy(url.to_bytes()).into_owned();
    let title = String::from_utf8_lossy(title.to_bytes()).into_owned();
    if url.is_empty() || url.len() > MAX_URL_BYTES || title.len() > MAX_TITLE_BYTES {
        eprintln!("[runtime] Roblox web view request rejected (empty or oversized field)");
        return;
    }

    *request_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(WebViewRequest { url, title });
}

// Keep the C entry point in the final Rust library even when the host only
// consumes requests through `take_request`.
#[used]
static WEBVIEW_OPEN_ENTRY: extern "C" fn(*const c_char, *const c_char) = roblox_runtime_webview_open;
