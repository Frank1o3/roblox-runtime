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

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::{Mutex, OnceLock};

/// A Roblox request to show an in-experience page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebViewRequest {
    pub url: String,
    pub title: String,
}

/// A request from either the legacy JNI callback or the newer message bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebViewEvent {
    Open(WebViewRequest),
    Close,
}

const WEBVIEW_PROTOCOL: &str = "com/roblox/protocols/webview/WebViewProtocol";
const MESSAGE_BUS: &str = "com/roblox/universalapp/messagebus/MessageBus";

#[derive(Clone)]
struct OpenWindowKeys {
    url: String,
    title: String,
}

static EVENT: OnceLock<Mutex<Option<WebViewEvent>>> = OnceLock::new();
static OPEN_WINDOW_KEYS: OnceLock<OpenWindowKeys> = OnceLock::new();
type MessageSink = extern "C" fn(*const c_char);
static RUST_SINKS: OnceLock<Mutex<HashMap<u64, MessageSink>>> = OnceLock::new();

fn event_slot() -> &'static Mutex<Option<WebViewEvent>> {
    EVENT.get_or_init(|| Mutex::new(None))
}

/// Take the latest pending request, if Roblox has asked to open a page.
///
/// Call this from the host's UI thread. A newer request replaces an older
/// request that the host has not consumed yet, so a stalled UI cannot grow an
/// unbounded queue or open a backlog of stale pages.
pub fn take_request() -> Option<WebViewRequest> {
    match take_event()? {
        WebViewEvent::Open(request) => Some(request),
        WebViewEvent::Close => None,
    }
}

/// Take the latest pending open or close command from Roblox.
pub fn take_event() -> Option<WebViewEvent> {
    event_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

/// Discard an unconsumed request during host shutdown.
pub fn clear_pending_request() {
    event_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
}

/// Subscribe to Roblox's current `WebViewProtocol` transport.
///
/// The protocol name, method identifiers and JSON keys are read from the
/// loaded engine instead of guessed. Call this after the app bridge has
/// started, since `MessageBus.doSubscribeRaw` needs a live bus.
pub fn arm(engine: &crate::LoadedEngine) {
    if let Some(native) = engine.symbol(
        "Java_com_roblox_protocols_webview_WebViewProtocol_initializeAndroidWebViewProtocol",
    ) {
        // SAFETY: this is the protocol's exported zero-argument initializer
        // from the loaded engine, called after App Bridge startup.
        match unsafe { crate::jni::game_activity::protocol_init(native, WEBVIEW_PROTOCOL) } {
            Ok(()) => eprintln!("[runtime-webview] WebViewProtocol initialized"),
            Err(error) => {
                eprintln!("[runtime-webview] WebViewProtocol initialization failed: {error}")
            }
        }
    } else {
        eprintln!(
            "[runtime-webview] WebViewProtocol initializer is unavailable in this engine build"
        );
    }

    let getter = |name: &str| -> Option<String> {
        let symbol = format!("Java_com_roblox_protocols_webview_WebViewProtocol_{name}");
        let native = engine.symbol(&symbol)?;
        // SAFETY: this is the matching zero-argument String getter exported by
        // the loaded libroblox.so and the JNI VM is initialized by this point.
        unsafe { crate::jni::game_activity::call_static_ret_string(native, WEBVIEW_PROTOCOL).ok() }
    };

    let Some(protocol) = getter("getProtocolName") else {
        eprintln!("[runtime-webview] WebViewProtocol.getProtocolName is unavailable");
        return;
    };
    let Some(open_id) = getter("getOpenWindowId") else {
        eprintln!("[runtime-webview] WebViewProtocol.getOpenWindowId is unavailable");
        return;
    };
    let Some(url_key) = getter("getUrlKey") else {
        eprintln!("[runtime-webview] WebViewProtocol.getUrlKey is unavailable");
        return;
    };
    let title_key = getter("getTitleKey").unwrap_or_else(|| "title".to_owned());
    let _ = OPEN_WINDOW_KEYS.set(OpenWindowKeys {
        url: url_key,
        title: title_key,
    });

    let get_message_id =
        engine.symbol("Java_com_roblox_universalapp_messagebus_MessageBus_getMessageId");
    let subscribe =
        engine.symbol("Java_com_roblox_universalapp_messagebus_MessageBus_doSubscribeRaw");
    let (Some(get_message_id), Some(subscribe)) = (get_message_id, subscribe) else {
        eprintln!(
            "[runtime-webview] MessageBus natives are unavailable; no WebView requests will arrive"
        );
        return;
    };
    // SAFETY: getMessageId is a static `(String, String) -> String` native
    // exported by this engine; protocol and method are the values returned by
    // that same engine's WebViewProtocol getters.
    let open_bus_id = match unsafe {
        crate::jni::game_activity::call_static_two_strings_ret_string(
            get_message_id,
            MESSAGE_BUS,
            &protocol,
            &open_id,
        )
    } {
        Ok(id) if !id.is_empty() => id,
        Ok(_) => {
            eprintln!("[runtime-webview] MessageBus returned an empty WebView.openWindow id");
            return;
        }
        Err(error) => {
            eprintln!("[runtime-webview] could not compose WebView.openWindow id: {error}");
            return;
        }
    };
    let subscribe_result = if jnivm::selected_from_environment() {
        eprintln!("[runtime-webview] using RustVM-owned MessageBus callback objects");
        subscribe_raw_rust(subscribe, &open_bus_id, on_open_window)
    } else {
        eprintln!("[runtime-webview] using libjnivm MessageBus callback objects");
        subscribe_raw(subscribe, &open_bus_id, on_open_window)
    };
    if let Err(error) = subscribe_result {
        eprintln!("[runtime-webview] could not subscribe to WebView.openWindow: {error}");
        return;
    }
    eprintln!("[runtime-webview] subscribed to {protocol}.{open_id} ({open_bus_id})");

    if let Some(close_id) = getter("getCloseWindowId") {
        // SAFETY: same getter pairing and JNI signature as above.
        match unsafe {
            crate::jni::game_activity::call_static_two_strings_ret_string(
                get_message_id,
                MESSAGE_BUS,
                &protocol,
                &close_id,
            )
        } {
            Ok(id) if !id.is_empty() => match if jnivm::selected_from_environment() {
                subscribe_raw_rust(subscribe, &id, on_close_window)
            } else {
                subscribe_raw(subscribe, &id, on_close_window)
            } {
                Ok(()) => eprintln!("[runtime-webview] subscribed to {protocol}.{close_id} ({id})"),
                Err(error) => eprintln!(
                    "[runtime-webview] could not subscribe to WebView.closeWindow: {error}"
                ),
            },
            Ok(_) => {
                eprintln!("[runtime-webview] MessageBus returned an empty WebView.closeWindow id")
            }
            Err(error) => {
                eprintln!("[runtime-webview] could not compose WebView.closeWindow id: {error}")
            }
        }
    }
}

unsafe extern "C" {
    fn roblox_messagebus_subscribe(
        native: *mut c_void,
        message_id: *const c_char,
        sink: Option<MessageSink>,
        error: *mut c_char,
        error_len: usize,
    ) -> c_int;
}

fn subscribe_raw_rust(
    native: *mut c_void,
    message_id: &str,
    sink: MessageSink,
) -> Result<(), String> {
    const RAW_CALLBACK: &str = "com/roblox/universalapp/messagebus/RawCallback";
    jnivm::register_method_handler(
        RAW_CALLBACK,
        "run",
        "(Ljava/lang/String;)V",
        raw_callback_run,
    )?;
    let env = jnivm::current_env().ok_or("Rust JavaVM is not available")?;
    let class = jnivm::class_ref(MESSAGE_BUS)?;
    let message_id_ref = jnivm::new_string_ref(message_id)?;
    let callback = jnivm::new_opaque_object(RAW_CALLBACK)?;
    let callback_id = callback as usize as u64;
    RUST_SINKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(callback_id, sink);

    type Subscribe = unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u8,
    ) -> *mut c_void;
    // SAFETY: `native` is the loaded engine's static JNI doSubscribeRaw
    // export. These class, String and callback references were all allocated
    // by the active Rust VM and are passed with that VM's JNIEnv.
    let subscribe: Subscribe = unsafe { std::mem::transmute(native) };
    let connection = unsafe { subscribe(env, class, message_id_ref, callback, 0) };
    if connection.is_null() {
        Err("doSubscribeRaw returned null".into())
    } else {
        Ok(())
    }
}

fn raw_callback_run(
    _vm: &jnivm::Vm,
    receiver: Option<jnivm::ObjectId>,
    args: &[jnivm::JniValue],
) -> jnivm::JniValue {
    let Some(receiver) = receiver else {
        eprintln!("[runtime-webview] MessageBus callback has no receiver");
        return jnivm::JniValue::Void;
    };
    let Some(jnivm::JniValue::Object(Some(payload))) = args.first() else {
        eprintln!("[runtime-webview] MessageBus callback has no payload");
        return jnivm::JniValue::Void;
    };
    let Ok(payload) = jnivm::string_object(*payload) else {
        eprintln!("[runtime-webview] MessageBus callback payload is not a Rust VM String");
        return jnivm::JniValue::Void;
    };
    let Some(sink) = RUST_SINKS
        .get()
        .and_then(|sinks| sinks.lock().ok()?.get(&receiver.raw()).copied())
    else {
        eprintln!("[runtime-webview] MessageBus callback has no registered sink");
        return jnivm::JniValue::Void;
    };
    let Ok(payload) = CString::new(payload) else {
        eprintln!("[runtime-webview] MessageBus callback payload contains NUL");
        return jnivm::JniValue::Void;
    };
    sink(payload.as_ptr());
    jnivm::JniValue::Void
}

fn subscribe_raw(native: *mut c_void, message_id: &str, sink: MessageSink) -> Result<(), String> {
    let message_id = CString::new(message_id).map_err(|error| error.to_string())?;
    let mut error = vec![0u8; 512];
    // SAFETY: native is the loaded engine's doSubscribeRaw export; the string,
    // sink and error buffer all outlive the synchronous registration call.
    let result = unsafe {
        roblox_messagebus_subscribe(
            native,
            message_id.as_ptr(),
            Some(sink),
            error.as_mut_ptr().cast(),
            error.len(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        let end = error
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(error.len());
        Err(String::from_utf8_lossy(&error[..end]).into_owned())
    }
}

extern "C" fn on_open_window(json: *const c_char) {
    if json.is_null() {
        eprintln!("[runtime-webview] WebView.openWindow arrived with a null payload");
        return;
    }
    // SAFETY: MessageBusRawCallback passes a live NUL-terminated string for
    // this synchronous callback.
    let bytes = unsafe { CStr::from_ptr(json) }.to_bytes();
    let body = String::from_utf8_lossy(bytes);
    let Some(keys) = OPEN_WINDOW_KEYS.get() else {
        eprintln!("[runtime-webview] WebView.openWindow arrived before its keys were initialized");
        return;
    };
    let parsed = serde_json::from_str::<serde_json::Value>(&body);
    let Ok(value) = parsed else {
        eprintln!(
            "[runtime-webview] WebView.openWindow payload is not valid JSON ({} bytes)",
            bytes.len()
        );
        return;
    };
    let Some(object) = value.as_object() else {
        eprintln!("[runtime-webview] WebView.openWindow payload is not an object");
        return;
    };
    let Some(url) = object
        .get(&keys.url)
        .and_then(serde_json::Value::as_str)
        .filter(|url| !url.is_empty())
    else {
        eprintln!(
            "[runtime-webview] WebView.openWindow payload has no URL field {:?}",
            keys.url
        );
        return;
    };
    let title = object
        .get(&keys.title)
        .and_then(serde_json::Value::as_str)
        .filter(|title| !title.is_empty())
        .unwrap_or("Roblox")
        .to_owned();
    *event_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(WebViewEvent::Open(WebViewRequest {
            url: url.to_owned(),
            title,
        }));
    eprintln!(
        "[runtime-webview] WebView.openWindow request queued ({} bytes)",
        bytes.len()
    );
}

extern "C" fn on_close_window(_json: *const c_char) {
    *event_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(WebViewEvent::Close);
    eprintln!("[runtime-webview] WebView.closeWindow request queued");
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

    *event_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(WebViewEvent::Open(WebViewRequest { url, title }));
    eprintln!("[roblox] web view request queued for the host UI");
}

// Keep the C entry point in the final Rust library even when the host only
// consumes requests through `take_request`.
#[used]
static WEBVIEW_OPEN_ENTRY: extern "C" fn(*const c_char, *const c_char) =
    roblox_runtime_webview_open;
