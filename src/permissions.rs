//! Answers Roblox's asynchronous permission requests. Voice chat asks the
//! message bus for microphone permission; Android's checkSelfPermission shim
//! alone does not answer that protocol request.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

unsafe extern "C" {
    fn roblox_messagebus_set_request_handler_async(
        set_fn: *mut c_void,
        publish_fn: *mut c_void,
        respond_fn: *mut c_void,
        dual_response: c_int,
        protocol: *const c_char,
        method: *const c_char,
        sink: extern "C" fn(*const c_char, *mut c_char, usize) -> c_int,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
}

const PROTOCOL: &str = "PermissionsProtocol";
const MICROPHONE: &str = "MICROPHONE_ACCESS";
const SUPPORTED: [&str; 2] = [MICROPHONE, "LOCAL_NETWORK"];

extern "C" fn request(raw: *const c_char, out: *mut c_char, len: usize) -> c_int {
    answer("PermissionsRequest", raw, out, len)
}
extern "C" fn has(raw: *const c_char, out: *mut c_char, len: usize) -> c_int {
    answer("HasPermissions", raw, out, len)
}
extern "C" fn supports(raw: *const c_char, out: *mut c_char, len: usize) -> c_int {
    answer("SupportsPermissions", raw, out, len)
}
extern "C" fn upsell(raw: *const c_char, out: *mut c_char, len: usize) -> c_int {
    answer("ShouldShowPermissionUpsell", raw, out, len)
}
extern "C" fn rationale(raw: *const c_char, out: *mut c_char, len: usize) -> c_int {
    answer("ShouldShowRequestPermissionRationale", raw, out, len)
}

fn answer(method: &str, raw: *const c_char, out: *mut c_char, len: usize) -> c_int {
    if raw.is_null() || out.is_null() || len == 0 {
        return 0;
    }
    // SAFETY: the native request bridge passes a NUL-terminated request string
    // valid for the duration of this callback and an output buffer of `len`.
    let request = unsafe { CStr::from_ptr(raw) }.to_string_lossy();
    let object = serde_json::from_str::<serde_json::Value>(&request).ok();
    let permissions = object
        .as_ref()
        .and_then(|value| value.get("permissions"))
        .and_then(serde_json::Value::as_array);
    let names: Vec<&str> = permissions
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect();
    let response = match method {
        "SupportsPermissions" => {
            let supported: &[&str] = if object.is_some() { &SUPPORTED } else { &[] };
            serde_json::json!({ "permissions": supported })
        }
        "ShouldShowPermissionUpsell" | "ShouldShowRequestPermissionRationale" => {
            serde_json::json!({ "upsellStatus": "HIDE", "hiddenUpsellPermissions": names })
        }
        "HasPermissions" | "PermissionsRequest" => {
            let missing: Vec<&str> = names
                .iter()
                .copied()
                .filter(|name| !SUPPORTED.contains(name))
                .collect();
            let allowed = permissions.is_some() && missing.is_empty();
            serde_json::json!({
                "status": if allowed { "AUTHORIZED" } else { "DENIED" },
                "missingPermissions": missing
            })
        }
        _ => serde_json::json!({ "status": "DENIED", "missingPermissions": [MICROPHONE] }),
    };
    let Ok(response) = CString::new(response.to_string()) else { return 0 };
    if response.as_bytes_with_nul().len() > len {
        return 0;
    }
    // SAFETY: the response plus its terminator fits the caller-provided buffer.
    unsafe {
        std::ptr::copy_nonoverlapping(response.as_ptr(), out, response.as_bytes_with_nul().len());
    }
    1
}

/// Bind the handlers after GameActivity has brought the app message bus up.
pub fn arm(mut symbol: impl FnMut(&str) -> Option<*mut c_void>) {
    let (Some(set), Some(respond)) = (
        symbol("Java_com_roblox_universalapp_messagebus_MessageBus_setRequestHandlerAsyncRaw"),
        symbol("Java_com_roblox_universalapp_messagebus_MessageBus_callResponseHandlerRaw"),
    ) else {
        eprintln!("[runtime-permissions] message bus request natives are unavailable; voice permission requests cannot be answered");
        return;
    };
    let dual = std::env::var_os("RBX_RUNTIME_PERMISSIONS_DUAL_RESPONSE")
        .is_none_or(|value| value != "0");
    let publish = if dual {
        symbol("Java_com_roblox_universalapp_messagebus_MessageBus_publishProtocolMethodResponseRaw")
    } else {
        None
    };
    if dual && publish.is_none() {
        eprintln!("[runtime-permissions] protocol response native unavailable; permission handlers were not bound");
        return;
    }
    let protocol = CString::new(PROTOCOL).expect("literal");
    let handlers: [(&str, extern "C" fn(*const c_char, *mut c_char, usize) -> c_int); 5] = [
        ("PermissionsRequest", request),
        ("HasPermissions", has),
        ("SupportsPermissions", supports),
        ("ShouldShowPermissionUpsell", upsell),
        ("ShouldShowRequestPermissionRationale", rationale),
    ];
    for (method, callback) in handlers {
        let method_c = CString::new(method).expect("literal");
        let mut error = [0i8; 512];
        // SAFETY: native pointers come from this loaded engine's exports; the
        // C strings and error buffer remain valid for the duration of the call.
        let result = unsafe {
            roblox_messagebus_set_request_handler_async(
                set,
                publish.unwrap_or(std::ptr::null_mut()),
                respond,
                c_int::from(dual),
                protocol.as_ptr(),
                method_c.as_ptr(),
                callback,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if result == 0 {
            eprintln!("[runtime-permissions] {PROTOCOL}.{method} handler bound");
        } else {
            // SAFETY: the native bridge NUL-terminates the supplied error buffer.
            let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
            eprintln!("[runtime-permissions] could not bind {PROTOCOL}.{method}: {message}");
        }
    }
}
