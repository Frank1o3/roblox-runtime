//! Answers Roblox's asynchronous permission requests. Voice chat asks the
//! message bus for microphone permission; Android's checkSelfPermission shim
//! alone does not answer that protocol request.

use std::ffi::c_void;

const PROTOCOL: &str = "PermissionsProtocol";
const MICROPHONE: &str = "MICROPHONE_ACCESS";
const SUPPORTED: [&str; 2] = [MICROPHONE, "LOCAL_NETWORK"];

fn uses_dual_response(requested: bool, protocol: &str) -> bool {
    requested && protocol == PROTOCOL
}

fn deliver_response<P, R>(
    requested: bool,
    protocol: &str,
    publish: Option<P>,
    resolve: R,
) -> Result<(), &'static str>
where
    P: FnOnce(),
    R: FnOnce(),
{
    if uses_dual_response(requested, protocol) {
        publish.ok_or("protocol response native is unavailable")?();
    }
    resolve();
    Ok(())
}

fn compute_response(method: &str, request: &str) -> String {
    let object = serde_json::from_str::<serde_json::Value>(request).ok();
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
            let hidden = if permissions.is_some() {
                names
            } else {
                vec![MICROPHONE]
            };
            serde_json::json!({ "upsellStatus": "HIDE", "hiddenUpsellPermissions": hidden })
        }
        "HasPermissions" | "PermissionsRequest" => {
            let missing: Vec<&str> = names
                .iter()
                .copied()
                .filter(|name| !SUPPORTED.contains(name))
                .collect();
            let valid_list =
                permissions.is_some_and(|items| items.iter().all(serde_json::Value::is_string));
            let allowed = valid_list && missing.is_empty();
            serde_json::json!({
                "status": if allowed { "AUTHORIZED" } else { "DENIED" },
                "missingPermissions": missing
            })
        }
        _ => serde_json::json!({ "status": "DENIED", "missingPermissions": [MICROPHONE] }),
    };
    response.to_string()
}

#[derive(Clone, Copy)]
struct RustHandlerEntry {
    method: &'static str,
    dual: bool,
    publish: Option<usize>,
    respond: usize,
}

static RUST_PERM_HANDLERS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<u64, RustHandlerEntry>>,
> = std::sync::OnceLock::new();

fn async_handler_run(
    _vm: &jnivm::Vm,
    receiver: Option<jnivm::ObjectId>,
    args: &[jnivm::JniValue],
) -> jnivm::JniValue {
    let Some(receiver) = receiver else {
        eprintln!("[runtime-permissions] RequestHandlerAsyncRaw callback has no receiver");
        return jnivm::JniValue::Void;
    };
    let receiver_id = receiver.raw();
    let entry = {
        let guard = RUST_PERM_HANDLERS
            .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard.get(&receiver_id).copied()
    };
    let Some(entry) = entry else {
        eprintln!("[runtime-permissions] unknown callback receiver: {receiver_id:#x}");
        return jnivm::JniValue::Void;
    };

    let arg_a = match args.first() {
        Some(jnivm::JniValue::Object(Some(obj))) => jnivm::string_object(*obj).unwrap_or_default(),
        _ => String::new(),
    };
    let arg_b = match args.get(1) {
        Some(jnivm::JniValue::Object(Some(obj))) => jnivm::string_object(*obj).unwrap_or_default(),
        _ => String::new(),
    };

    let looks_like_object = |s: &str| s.trim_start().starts_with('{');
    let first_is_payload = looks_like_object(&arg_a) || !looks_like_object(&arg_b);
    let (payload, id) = if first_is_payload {
        (arg_a, arg_b)
    } else {
        (arg_b, arg_a)
    };

    let response_json = compute_response(entry.method, &payload);

    let Ok(env) = jnivm::current_env().ok_or("no current JNIEnv") else {
        eprintln!("[runtime-permissions] current JNIEnv unavailable");
        return jnivm::JniValue::Void;
    };
    let Ok(class) = jnivm::class_ref("com/roblox/universalapp/messagebus/MessageBus") else {
        eprintln!("[runtime-permissions] MessageBus class unavailable");
        return jnivm::JniValue::Void;
    };
    let Ok(id_ref) = jnivm::new_string_ref(&id) else {
        return jnivm::JniValue::Void;
    };
    let Ok(resp_ref) = jnivm::new_string_ref(&response_json) else {
        return jnivm::JniValue::Void;
    };

    type Respond = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void);
    // SAFETY: respond native was resolved from the engine library and passed into arm.
    let respond_fn: Respond = unsafe { std::mem::transmute(entry.respond as *mut c_void) };

    type Publish = unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        i32,
        *mut c_void,
    );
    let publish_delivery = if uses_dual_response(entry.dual, PROTOCOL) {
        let Some(publish) = entry.publish else {
            eprintln!("[runtime-permissions] protocol response native unavailable");
            return jnivm::JniValue::Void;
        };
        let Ok(proto_ref) = jnivm::new_string_ref(PROTOCOL) else {
            return jnivm::JniValue::Void;
        };
        let Ok(method_ref) = jnivm::new_string_ref(entry.method) else {
            return jnivm::JniValue::Void;
        };
        let Ok(telemetry_ref) = jnivm::new_string_ref("{}") else {
            return jnivm::JniValue::Void;
        };
        // SAFETY: publish native was resolved from the engine library and passed into arm.
        let publish_fn: Publish = unsafe { std::mem::transmute(publish as *mut c_void) };
        Some(move || {
            // SAFETY: all references were created on this attached JNI thread and
            // the native is the engine's registered publish method.
            unsafe {
                publish_fn(
                    env,
                    class,
                    proto_ref,
                    method_ref,
                    resp_ref,
                    0,
                    telemetry_ref,
                );
            }
        })
    } else {
        None
    };

    if let Err(error) = deliver_response(entry.dual, PROTOCOL, publish_delivery, || {
        // SAFETY: all references were created on this attached JNI thread and
        // the native is the engine's registered async-response method.
        unsafe { respond_fn(env, class, id_ref, resp_ref) };
    }) {
        eprintln!("[runtime-permissions] async response delivery failed: {error}");
        return jnivm::JniValue::Void;
    }

    eprintln!(
        "[runtime-permissions] async response {PROTOCOL}.{}: transport={}",
        entry.method,
        if entry.dual {
            "protocol+async"
        } else {
            "async"
        }
    );

    jnivm::JniValue::Void
}

#[cfg(test)]
mod tests {
    use super::{PROTOCOL, deliver_response};

    #[test]
    fn dual_permission_response_publishes_before_resolving_same_body() {
        let events = std::cell::RefCell::new(Vec::new());
        let body = String::from("{\"status\":\"AUTHORIZED\"}");
        let published = body.clone();
        let resolved = body.clone();

        deliver_response(
            true,
            PROTOCOL,
            Some(|| events.borrow_mut().push(("publish", published))),
            || events.borrow_mut().push(("resolve", resolved)),
        )
        .unwrap();

        assert_eq!(
            *events.borrow(),
            [("publish", body.clone()), ("resolve", body)]
        );
    }

    #[test]
    fn disabled_dual_response_only_resolves() {
        let events = std::cell::RefCell::new(Vec::new());
        deliver_response(false, PROTOCOL, None::<fn()>, || {
            events.borrow_mut().push("resolve")
        })
        .unwrap();
        assert_eq!(*events.borrow(), ["resolve"]);
    }

    #[test]
    fn other_protocol_ignores_permission_dual_response_setting() {
        let events = std::cell::RefCell::new(Vec::new());
        deliver_response(true, "Linking", None::<fn()>, || {
            events.borrow_mut().push("resolve")
        })
        .unwrap();
        assert_eq!(*events.borrow(), ["resolve"]);
    }

    #[test]
    fn requested_dual_response_without_publisher_fails_before_resolution() {
        let events = std::cell::RefCell::new(Vec::new());
        let result = deliver_response(true, PROTOCOL, None::<fn()>, || {
            events.borrow_mut().push("resolve")
        });
        assert_eq!(result, Err("protocol response native is unavailable"));
        assert!(events.borrow().is_empty());
    }
}

fn arm_rust(set: *mut c_void, publish: Option<*mut c_void>, respond: *mut c_void, dual: bool) {
    const ASYNC_HANDLER: &str = "com/roblox/universalapp/messagebus/RequestHandlerAsyncRaw";
    const MESSAGE_BUS: &str = "com/roblox/universalapp/messagebus/MessageBus";

    for cls in [ASYNC_HANDLER, "RequestHandlerAsyncRaw"] {
        if let Err(e) = jnivm::register_method_handler(
            cls,
            "run",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            async_handler_run,
        ) {
            eprintln!("[runtime-permissions] register_method_handler for {cls} failed: {e}");
        }
    }

    type SetHandler =
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void);
    // SAFETY: set native was resolved from the engine library and passed into arm
    let set_fn: SetHandler = unsafe { std::mem::transmute(set) };

    let handlers: [&'static str; 5] = [
        "PermissionsRequest",
        "HasPermissions",
        "SupportsPermissions",
        "ShouldShowPermissionUpsell",
        "ShouldShowRequestPermissionRationale",
    ];

    let Ok(env) = jnivm::current_env().ok_or("Rust JavaVM is not available") else {
        eprintln!("[runtime-permissions] Rust JavaVM is not available");
        return;
    };
    let Ok(class) = jnivm::class_ref(MESSAGE_BUS) else {
        eprintln!("[runtime-permissions] could not resolve MessageBus class");
        return;
    };

    for method in handlers {
        let callback = match jnivm::new_opaque_object(ASYNC_HANDLER) {
            Ok(cb) => cb,
            Err(e) => {
                eprintln!(
                    "[runtime-permissions] could not create callback object for {method}: {e}"
                );
                continue;
            }
        };
        let callback_id = callback as usize as u64;
        let entry = RustHandlerEntry {
            method,
            dual,
            publish: publish.map(|p| p as usize),
            respond: respond as usize,
        };

        {
            let mut guard = RUST_PERM_HANDLERS
                .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            guard.insert(callback_id, entry);
        }

        let Ok(protocol_ref) = jnivm::new_string_ref(PROTOCOL) else {
            continue;
        };
        let Ok(method_ref) = jnivm::new_string_ref(method) else {
            continue;
        };

        // SAFETY: set_fn is the engine's setRequestHandlerAsyncRaw export, invoked with valid JNI references
        unsafe {
            set_fn(env, class, protocol_ref, method_ref, callback);
        }
        eprintln!("[runtime-permissions] {PROTOCOL}.{method} handler bound");
    }
}

/// Bind the handlers after GameActivity has brought the app message bus up.
pub fn arm(mut symbol: impl FnMut(&str) -> Option<*mut c_void>) {
    let (Some(set), Some(respond)) = (
        symbol("Java_com_roblox_universalapp_messagebus_MessageBus_setRequestHandlerAsyncRaw"),
        symbol("Java_com_roblox_universalapp_messagebus_MessageBus_callResponseHandlerRaw"),
    ) else {
        eprintln!(
            "[runtime-permissions] message bus request natives are unavailable; voice permission requests cannot be answered"
        );
        return;
    };
    let dual =
        std::env::var_os("RBX_RUNTIME_PERMISSIONS_DUAL_RESPONSE").is_none_or(|value| value != "0");
    let publish = if dual {
        symbol(
            "Java_com_roblox_universalapp_messagebus_MessageBus_publishProtocolMethodResponseRaw",
        )
    } else {
        None
    };
    if dual && publish.is_none() {
        eprintln!(
            "[runtime-permissions] protocol response native unavailable; permission handlers were not bound"
        );
        return;
    }

    arm_rust(set, publish, respond, dual);
}
