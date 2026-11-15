//! Rust-owned accessibility compatibility state and hooks.
//!
//! The state is intentionally separate from the Linux AT-SPI transport. It
//! mirrors only the Android values this compatibility surface consumes.

use jnivm::{JniValue, MethodHandler, ObjectId, ObjectValue, Vm};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

/// Node state bits translated by the client-side AT-SPI adapter.
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct Event {
    event_type: i32,
    class_name: String,
    text: String,
}

#[derive(Default)]
struct State {
    bridge_connected: bool,
    next_id: u32,
    generation: u32,
    nodes: BTreeMap<u32, Node>,
    node_ids: HashMap<ObjectId, u32>,
    events: VecDeque<Event>,
}

const MAX_QUEUED_EVENTS: usize = 256;
static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static MANAGER: OnceLock<Mutex<Option<ObjectId>>> = OnceLock::new();
#[cfg(test)]
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}

fn manager() -> &'static Mutex<Option<ObjectId>> {
    MANAGER.get_or_init(|| Mutex::new(None))
}

fn lock_state() -> std::sync::MutexGuard<'static, State> {
    state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Report whether the host accessibility transport connected successfully.
pub fn set_bridge_connected(connected: bool) {
    lock_state().bridge_connected = connected;
}

/// Copy the current flat node snapshot. The generation can be polled first to
/// avoid copying when no node has changed.
pub fn snapshot() -> Vec<Node> {
    lock_state().nodes.values().cloned().collect()
}

/// Generation counter incremented whenever the node registry changes.
pub fn generation() -> u32 {
    lock_state().generation
}

/// Remove and return one pending accessibility event.
pub fn next_event() -> Option<(i32, String, String)> {
    let event = lock_state().events.pop_front()?;
    Some((event.event_type, event.class_name, event.text))
}

/// Seed a node for transport tests without invoking JNI.
#[allow(clippy::too_many_arguments)]
pub fn test_seed_node(
    class_name: &str,
    text: &str,
    content_description: &str,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    state_bits: u32,
) -> u32 {
    let mut state = lock_state();
    let id = next_node_id(&mut state);
    state.nodes.insert(
        id,
        Node {
            id,
            class_name: class_name.to_owned(),
            text: text.to_owned(),
            content_description: content_description.to_owned(),
            left,
            top,
            right,
            bottom,
            state: state_bits,
            actions: Vec::new(),
        },
    );
    state.generation = state.generation.wrapping_add(1);
    id
}

/// Clear transport test data and advance the generation.
pub fn test_clear() {
    let mut state = lock_state();
    state.nodes.clear();
    state.node_ids.clear();
    state.events.clear();
    state.generation = state.generation.wrapping_add(1);
}

fn next_node_id(state: &mut State) -> u32 {
    state.next_id = state.next_id.wrapping_add(1).max(1);
    state.next_id
}

fn node_id(receiver: Option<ObjectId>) -> Option<u32> {
    let receiver = receiver?;
    lock_state().node_ids.get(&receiver).copied()
}

fn update_node(receiver: Option<ObjectId>, update: impl FnOnce(&mut Node)) {
    let Some(id) = node_id(receiver) else { return };
    let mut state = lock_state();
    if let Some(node) = state.nodes.get_mut(&id) {
        update(node);
        state.generation = state.generation.wrapping_add(1);
    }
}

fn as_object(value: &JniValue) -> Option<ObjectId> {
    match value {
        JniValue::Object(Some(id)) => Some(*id),
        _ => None,
    }
}

fn as_int(value: &JniValue) -> Option<i32> {
    match value {
        JniValue::Int(value) => Some(*value),
        _ => None,
    }
}

fn as_bool(value: &JniValue) -> Option<bool> {
    match value {
        JniValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn read_string(vm: &Vm, value: &JniValue) -> String {
    let Some(object) = as_object(value) else {
        return String::new();
    };
    let Some(env) = vm.get_env() else {
        return String::new();
    };
    match vm.object_value(&env, object) {
        Ok(ObjectValue::String(units)) => String::from_utf16_lossy(&units),
        _ => String::new(),
    }
}

fn new_object(vm: &Vm, class_name: &str) -> Option<ObjectId> {
    let class = vm.find_or_define_class(class_name).ok()?;
    let env = vm.get_env()?;
    vm.new_local_object(&env, class, ObjectValue::Opaque).ok()
}

fn new_string(vm: &Vm, value: &str) -> Option<ObjectId> {
    let class = vm.find_or_define_class("java/lang/String").ok()?;
    let env = vm.get_env()?;
    vm.new_local_object(
        &env,
        class,
        ObjectValue::String(value.encode_utf16().collect()),
    )
    .ok()
}

fn register_method(
    vm: &Vm,
    class_name: &str,
    name: &str,
    descriptor: &str,
    is_static: bool,
    handler: MethodHandler,
) -> Result<(), String> {
    let class = vm
        .find_or_define_class(class_name)
        .map_err(|e| e.to_string())?;
    let method = vm
        .register_method(class, name, descriptor, is_static)
        .map_err(|e| e.to_string())?;
    vm.install_method_handler(method, handler)
        .map_err(|e| e.to_string())
}

fn register_field(
    vm: &Vm,
    class_name: &str,
    name: &str,
    descriptor: &str,
    is_static: bool,
) -> Result<jnivm::FieldId, String> {
    let class = vm
        .find_or_define_class(class_name)
        .map_err(|e| e.to_string())?;
    vm.register_field(class, name, descriptor, is_static)
        .map_err(|e| e.to_string())
}

fn rect_constructor(vm: &Vm, _receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let Some(rect) = new_object(vm, "android/graphics/Rect") else {
        return JniValue::Object(None);
    };
    if args.len() == 4 {
        for (name, value) in ["left", "top", "right", "bottom"].into_iter().zip(args) {
            let Ok(field) = register_field(vm, "android/graphics/Rect", name, "I", false) else {
                continue;
            };
            if let Some(value) = as_int(value) {
                let _ = vm.set_field_value(field, Some(rect), JniValue::Int(value));
            }
        }
    }
    JniValue::Object(Some(rect))
}

fn action_constructor(vm: &Vm, _receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let Some(action) = new_object(
        vm,
        "android/view/accessibility/AccessibilityNodeInfo$AccessibilityAction",
    ) else {
        return JniValue::Object(None);
    };
    let Some(id) = args.first().and_then(as_int) else {
        return JniValue::Object(Some(action));
    };
    let Ok(field) = register_field(
        vm,
        "android/view/accessibility/AccessibilityNodeInfo$AccessibilityAction",
        "id",
        "I",
        false,
    ) else {
        return JniValue::Object(Some(action));
    };
    let _ = vm.set_field_value(field, Some(action), JniValue::Int(id));
    if let Some(label) = args.get(1).cloned() {
        if let Ok(field) = register_field(
            vm,
            "android/view/accessibility/AccessibilityNodeInfo$AccessibilityAction",
            "label",
            "Ljava/lang/CharSequence;",
            false,
        ) {
            let _ = vm.set_field_value(field, Some(action), label);
        }
    }
    JniValue::Object(Some(action))
}

fn action_id(vm: &Vm, receiver: Option<ObjectId>, _args: &[JniValue]) -> JniValue {
    let Some(receiver) = receiver else {
        return JniValue::Int(0);
    };
    let Ok(field) = register_field(
        vm,
        "android/view/accessibility/AccessibilityNodeInfo$AccessibilityAction",
        "id",
        "I",
        false,
    ) else {
        return JniValue::Int(0);
    };
    vm.field_value(field, Some(receiver))
        .unwrap_or(JniValue::Int(0))
}

fn action_label(vm: &Vm, receiver: Option<ObjectId>, _args: &[JniValue]) -> JniValue {
    let Some(receiver) = receiver else {
        return JniValue::Object(None);
    };
    let Ok(field) = register_field(
        vm,
        "android/view/accessibility/AccessibilityNodeInfo$AccessibilityAction",
        "label",
        "Ljava/lang/CharSequence;",
        false,
    ) else {
        return JniValue::Object(None);
    };
    vm.field_value(field, Some(receiver))
        .unwrap_or(JniValue::Object(None))
}

fn node_obtain(vm: &Vm, _receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let class_name = "android/view/accessibility/AccessibilityNodeInfo";
    let Some(object) = new_object(vm, class_name) else {
        return JniValue::Object(None);
    };
    let mut state = lock_state();
    let id = next_node_id(&mut state);
    let node = args
        .first()
        .and_then(as_object)
        .and_then(|other| {
            let old_id = state.node_ids.get(&other)?;
            state.nodes.get(old_id).cloned()
        })
        .unwrap_or_default();
    let mut node = node;
    node.id = id;
    state.node_ids.insert(object, id);
    state.nodes.insert(id, node);
    state.generation = state.generation.wrapping_add(1);
    JniValue::Object(Some(object))
}

fn node_recycle(_vm: &Vm, receiver: Option<ObjectId>, _args: &[JniValue]) -> JniValue {
    if let Some(receiver) = receiver {
        let mut state = lock_state();
        if let Some(id) = state.node_ids.remove(&receiver) {
            state.nodes.remove(&id);
            state.generation = state.generation.wrapping_add(1);
        }
    }
    JniValue::Void
}

fn node_set_text(vm: &Vm, receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let value = args.first().map(|v| read_string(vm, v)).unwrap_or_default();
    update_node(receiver, |node| node.text = value);
    JniValue::Void
}

fn node_set_description(vm: &Vm, receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let value = args.first().map(|v| read_string(vm, v)).unwrap_or_default();
    update_node(receiver, |node| node.content_description = value);
    JniValue::Void
}

fn node_set_class(vm: &Vm, receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let value = args.first().map(|v| read_string(vm, v)).unwrap_or_default();
    update_node(receiver, |node| node.class_name = value);
    JniValue::Void
}

fn node_set_bounds(vm: &Vm, receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let Some(rect) = args.first().and_then(as_object) else {
        return JniValue::Void;
    };
    let mut values = [0; 4];
    for (index, name) in ["left", "top", "right", "bottom"].into_iter().enumerate() {
        let Ok(field) = register_field(vm, "android/graphics/Rect", name, "I", false) else {
            continue;
        };
        values[index] = vm
            .field_value(field, Some(rect))
            .ok()
            .and_then(|value| as_int(&value))
            .unwrap_or(0);
    }
    update_node(receiver, |node| {
        [node.left, node.top, node.right, node.bottom] = values;
    });
    JniValue::Void
}

fn node_get_bounds(vm: &Vm, _receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let Some(rect) = args.first().and_then(as_object) else {
        return JniValue::Void;
    };
    let id = node_id(_receiver);
    let Some(node) = id.and_then(|id| lock_state().nodes.get(&id).cloned()) else {
        return JniValue::Void;
    };
    for (name, value) in [
        ("left", node.left),
        ("top", node.top),
        ("right", node.right),
        ("bottom", node.bottom),
    ] {
        if let Ok(field) = register_field(vm, "android/graphics/Rect", name, "I", false) {
            let _ = vm.set_field_value(field, Some(rect), JniValue::Int(value));
        }
    }
    JniValue::Void
}

fn node_set_bool(_vm: &Vm, receiver: Option<ObjectId>, args: &[JniValue], bit: u32) -> JniValue {
    let Some(value) = args.first().and_then(as_bool) else {
        return JniValue::Void;
    };
    update_node(receiver, |node| {
        if value {
            node.state |= bit
        } else {
            node.state &= !bit
        }
    });
    JniValue::Void
}

fn set_checkable(vm: &Vm, r: Option<ObjectId>, a: &[JniValue]) -> JniValue {
    node_set_bool(vm, r, a, state_bit::CHECKABLE)
}
fn set_checked(vm: &Vm, r: Option<ObjectId>, a: &[JniValue]) -> JniValue {
    node_set_bool(vm, r, a, state_bit::CHECKED)
}
fn set_clickable(vm: &Vm, r: Option<ObjectId>, a: &[JniValue]) -> JniValue {
    node_set_bool(vm, r, a, state_bit::CLICKABLE)
}
fn set_enabled(vm: &Vm, r: Option<ObjectId>, a: &[JniValue]) -> JniValue {
    node_set_bool(vm, r, a, state_bit::ENABLED)
}
fn set_focusable(vm: &Vm, r: Option<ObjectId>, a: &[JniValue]) -> JniValue {
    node_set_bool(vm, r, a, state_bit::FOCUSABLE)
}
fn set_focused(vm: &Vm, r: Option<ObjectId>, a: &[JniValue]) -> JniValue {
    node_set_bool(vm, r, a, state_bit::FOCUSED)
}
fn set_long_clickable(vm: &Vm, r: Option<ObjectId>, a: &[JniValue]) -> JniValue {
    node_set_bool(vm, r, a, state_bit::LONG_CLICKABLE)
}
fn set_password(vm: &Vm, r: Option<ObjectId>, a: &[JniValue]) -> JniValue {
    node_set_bool(vm, r, a, state_bit::PASSWORD)
}
fn set_scrollable(vm: &Vm, r: Option<ObjectId>, a: &[JniValue]) -> JniValue {
    node_set_bool(vm, r, a, state_bit::SCROLLABLE)
}
fn set_selected(vm: &Vm, r: Option<ObjectId>, a: &[JniValue]) -> JniValue {
    node_set_bool(vm, r, a, state_bit::SELECTED)
}
fn set_visible(vm: &Vm, r: Option<ObjectId>, a: &[JniValue]) -> JniValue {
    node_set_bool(vm, r, a, state_bit::VISIBLE_TO_USER)
}

fn node_add_action(_vm: &Vm, receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let action = args.first().and_then(as_int).or_else(|| {
        let object = args.first().and_then(as_object)?;
        let vm = jnivm::process_vm()?;
        let field = register_field(
            vm,
            "android/view/accessibility/AccessibilityNodeInfo$AccessibilityAction",
            "id",
            "I",
            false,
        )
        .ok()?;
        as_int(&vm.field_value(field, Some(object)).ok()?)
    });
    if let Some(action) = action {
        update_node(receiver, |node| node.actions.push(action));
    }
    JniValue::Void
}

fn manager_instance(vm: &Vm, _receiver: Option<ObjectId>, _args: &[JniValue]) -> JniValue {
    let mut manager = manager()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(object) = *manager {
        return JniValue::Object(Some(object));
    }
    let Some(object) = new_object(vm, "android/view/accessibility/AccessibilityManager") else {
        return JniValue::Object(None);
    };
    let Some(env) = vm.get_env() else {
        return JniValue::Object(None);
    };
    let Ok(global) = vm.new_global_ref(&env, object) else {
        return JniValue::Object(None);
    };
    *manager = Some(global);
    JniValue::Object(Some(global))
}

fn manager_enabled(_vm: &Vm, _receiver: Option<ObjectId>, _args: &[JniValue]) -> JniValue {
    if let Some(value) = std::env::var_os("RBX_RUNTIME_ACCESSIBILITY") {
        if let Some(value) = value.to_str() {
            return JniValue::Boolean(!value.is_empty() && !value.starts_with('0'));
        }
    }
    JniValue::Boolean(lock_state().bridge_connected)
}

fn manager_false(_vm: &Vm, _receiver: Option<ObjectId>, _args: &[JniValue]) -> JniValue {
    JniValue::Boolean(false)
}
fn manager_noop(_vm: &Vm, _receiver: Option<ObjectId>, _args: &[JniValue]) -> JniValue {
    JniValue::Void
}

fn event_obtain(vm: &Vm, _receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let class_name = "android/view/accessibility/AccessibilityEvent";
    let Some(object) = new_object(vm, class_name) else {
        return JniValue::Object(None);
    };
    for (name, value) in [("eventType", args.first().and_then(as_int).unwrap_or(0))] {
        if let Ok(field) = register_field(vm, class_name, name, "I", false) {
            let _ = vm.set_field_value(field, Some(object), JniValue::Int(value));
        }
    }
    JniValue::Object(Some(object))
}

fn event_set_type(vm: &Vm, receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    if let (Some(receiver), Some(value)) = (receiver, args.first().and_then(as_int)) {
        if let Ok(field) = register_field(
            vm,
            "android/view/accessibility/AccessibilityEvent",
            "eventType",
            "I",
            false,
        ) {
            let _ = vm.set_field_value(field, Some(receiver), JniValue::Int(value));
        }
    }
    JniValue::Void
}

fn event_get_type(vm: &Vm, receiver: Option<ObjectId>, _args: &[JniValue]) -> JniValue {
    let Some(receiver) = receiver else {
        return JniValue::Int(0);
    };
    let Ok(field) = register_field(
        vm,
        "android/view/accessibility/AccessibilityEvent",
        "eventType",
        "I",
        false,
    ) else {
        return JniValue::Int(0);
    };
    vm.field_value(field, Some(receiver))
        .unwrap_or(JniValue::Int(0))
}

fn event_set_class(vm: &Vm, receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let value = args.first().map(|v| read_string(vm, v)).unwrap_or_default();
    if let Some(receiver) = receiver {
        if let Ok(field) = register_field(
            vm,
            "android/view/accessibility/AccessibilityEvent",
            "className",
            "Ljava/lang/String;",
            false,
        ) {
            if let Some(string) = new_string(vm, &value) {
                let _ = vm.set_field_value(field, Some(receiver), JniValue::Object(Some(string)));
            }
        }
    }
    JniValue::Void
}

fn event_set_text(vm: &Vm, receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let value = args.first().map(|v| read_string(vm, v)).unwrap_or_default();
    if let Some(receiver) = receiver {
        if let Ok(field) = register_field(
            vm,
            "android/view/accessibility/AccessibilityEvent",
            "text",
            "Ljava/lang/String;",
            false,
        ) {
            if let Some(string) = new_string(vm, &value) {
                let _ = vm.set_field_value(field, Some(receiver), JniValue::Object(Some(string)));
            }
        }
    }
    JniValue::Void
}

fn manager_send_event(vm: &Vm, _receiver: Option<ObjectId>, args: &[JniValue]) -> JniValue {
    let Some(event) = args.first().and_then(as_object) else {
        return JniValue::Void;
    };
    let class = "android/view/accessibility/AccessibilityEvent";
    let get = |field_name: &str, descriptor: &str| -> Option<JniValue> {
        let field = register_field(vm, class, field_name, descriptor, false).ok()?;
        vm.field_value(field, Some(event)).ok()
    };
    let event_type = get("eventType", "I").as_ref().and_then(as_int).unwrap_or(0);
    let class_name = get("className", "Ljava/lang/String;")
        .as_ref()
        .map(|v| read_string(vm, v))
        .unwrap_or_default();
    let text = get("text", "Ljava/lang/String;")
        .as_ref()
        .map(|v| read_string(vm, v))
        .unwrap_or_default();
    let mut state = lock_state();
    if state.events.len() == MAX_QUEUED_EVENTS {
        state.events.pop_front();
    }
    state.events.push_back(Event {
        event_type,
        class_name,
        text,
    });
    JniValue::Void
}

/// Register the Android accessibility hooks that the compatibility surface
/// exposes. Every method has an explicit JNI descriptor.
pub fn register_hooks() -> Result<(), String> {
    let vm = jnivm::process_vm().ok_or("Rust JavaVM has not been created")?;
    register_hooks_for_vm(vm)
}

fn register_hooks_for_vm(vm: &Vm) -> Result<(), String> {
    let rect = "android/graphics/Rect";
    register_field(vm, rect, "left", "I", false)?;
    register_field(vm, rect, "top", "I", false)?;
    register_field(vm, rect, "right", "I", false)?;
    register_field(vm, rect, "bottom", "I", false)?;
    register_method(vm, rect, "<init>", "()V", true, rect_constructor)?;
    register_method(vm, rect, "<init>", "(IIII)V", true, rect_constructor)?;

    let action = "android/view/accessibility/AccessibilityNodeInfo$AccessibilityAction";
    register_method(
        vm,
        action,
        "<init>",
        "(ILjava/lang/CharSequence;)V",
        true,
        action_constructor,
    )?;
    register_method(vm, action, "getId", "()I", false, action_id)?;
    register_method(
        vm,
        action,
        "getLabel",
        "()Ljava/lang/CharSequence;",
        false,
        action_label,
    )?;
    register_field(vm, action, "id", "I", false)?;
    register_field(vm, action, "label", "Ljava/lang/CharSequence;", false)?;
    for (name, id) in [
        ("ACTION_FOCUS", 1),
        ("ACTION_CLEAR_FOCUS", 2),
        ("ACTION_SELECT", 4),
        ("ACTION_CLEAR_SELECTION", 8),
        ("ACTION_CLICK", 16),
        ("ACTION_LONG_CLICK", 32),
        ("ACTION_ACCESSIBILITY_FOCUS", 64),
        ("ACTION_CLEAR_ACCESSIBILITY_FOCUS", 128),
        ("ACTION_SCROLL_FORWARD", 4096),
        ("ACTION_SCROLL_BACKWARD", 8192),
    ] {
        let field = register_field(
            vm,
            action,
            name,
            "Landroid/view/accessibility/AccessibilityNodeInfo$AccessibilityAction;",
            true,
        )?;
        let Some(object) = new_object(vm, action) else {
            continue;
        };
        let Some(env) = vm.get_env() else { continue };
        let Ok(global) = vm.new_global_ref(&env, object) else {
            continue;
        };
        let id_field = register_field(vm, action, "id", "I", false)?;
        let _ = vm.set_field_value(id_field, Some(global), JniValue::Int(id));
        vm.set_field_value(field, None, JniValue::Object(Some(global)))
            .map_err(|e| e.to_string())?;
    }

    let node = "android/view/accessibility/AccessibilityNodeInfo";
    for (descriptor, handler) in [
        (
            "()Landroid/view/accessibility/AccessibilityNodeInfo;",
            node_obtain as MethodHandler,
        ),
        (
            "(Landroid/view/accessibility/AccessibilityNodeInfo;)Landroid/view/accessibility/AccessibilityNodeInfo;",
            node_obtain,
        ),
    ] {
        register_method(vm, node, "obtain", descriptor, true, handler)?;
    }
    register_method(vm, node, "recycle", "()V", false, node_recycle)?;
    for (name, descriptor, handler) in [
        (
            "setText",
            "(Ljava/lang/CharSequence;)V",
            node_set_text as MethodHandler,
        ),
        ("setText", "(Ljava/lang/String;)V", node_set_text),
        (
            "setContentDescription",
            "(Ljava/lang/CharSequence;)V",
            node_set_description,
        ),
        (
            "setContentDescription",
            "(Ljava/lang/String;)V",
            node_set_description,
        ),
        (
            "setClassName",
            "(Ljava/lang/CharSequence;)V",
            node_set_class,
        ),
        ("setClassName", "(Ljava/lang/String;)V", node_set_class),
        (
            "setPackageName",
            "(Ljava/lang/CharSequence;)V",
            manager_noop,
        ),
        ("setPackageName", "(Ljava/lang/String;)V", manager_noop),
        (
            "setViewIdResourceName",
            "(Ljava/lang/String;)V",
            manager_noop,
        ),
        (
            "setBoundsInScreen",
            "(Landroid/graphics/Rect;)V",
            node_set_bounds,
        ),
        (
            "getBoundsInScreen",
            "(Landroid/graphics/Rect;)V",
            node_get_bounds,
        ),
        ("setCheckable", "(Z)V", set_checkable),
        ("setChecked", "(Z)V", set_checked),
        ("setClickable", "(Z)V", set_clickable),
        ("setEnabled", "(Z)V", set_enabled),
        ("setFocusable", "(Z)V", set_focusable),
        ("setFocused", "(Z)V", set_focused),
        ("setLongClickable", "(Z)V", set_long_clickable),
        ("setPassword", "(Z)V", set_password),
        ("setScrollable", "(Z)V", set_scrollable),
        ("setSelected", "(Z)V", set_selected),
        ("setVisibleToUser", "(Z)V", set_visible),
        ("addAction", "(I)V", node_add_action),
        (
            "addAction",
            "(Landroid/view/accessibility/AccessibilityNodeInfo$AccessibilityAction;)V",
            node_add_action,
        ),
    ] {
        register_method(vm, node, name, descriptor, false, handler)?;
    }

    let event = "android/view/accessibility/AccessibilityEvent";
    register_method(
        vm,
        event,
        "obtain",
        "()Landroid/view/accessibility/AccessibilityEvent;",
        true,
        event_obtain,
    )?;
    register_method(
        vm,
        event,
        "obtain",
        "(I)Landroid/view/accessibility/AccessibilityEvent;",
        true,
        event_obtain,
    )?;
    register_method(vm, event, "setEventType", "(I)V", false, event_set_type)?;
    register_method(vm, event, "getEventType", "()I", false, event_get_type)?;
    register_method(
        vm,
        event,
        "setClassName",
        "(Ljava/lang/CharSequence;)V",
        false,
        event_set_class,
    )?;
    register_method(
        vm,
        event,
        "setContentDescription",
        "(Ljava/lang/CharSequence;)V",
        false,
        event_set_text,
    )?;

    let manager_class = "android/view/accessibility/AccessibilityManager";
    register_method(
        vm,
        manager_class,
        "getInstance",
        "(Landroid/content/Context;)Landroid/view/accessibility/AccessibilityManager;",
        true,
        manager_instance,
    )?;
    register_method(
        vm,
        manager_class,
        "isEnabled",
        "()Z",
        false,
        manager_enabled,
    )?;
    register_method(
        vm,
        manager_class,
        "isTouchExplorationEnabled",
        "()Z",
        false,
        manager_false,
    )?;
    register_method(
        vm,
        manager_class,
        "addTouchExplorationStateChangeListener",
        "(Landroid/view/accessibility/AccessibilityManager$TouchExplorationStateChangeListener;)V",
        false,
        manager_noop,
    )?;
    register_method(
        vm,
        manager_class,
        "removeTouchExplorationStateChangeListener",
        "(Landroid/view/accessibility/AccessibilityManager$TouchExplorationStateChangeListener;)V",
        false,
        manager_noop,
    )?;
    register_method(
        vm,
        manager_class,
        "sendAccessibilityEvent",
        "(Landroid/view/accessibility/AccessibilityEvent;)V",
        false,
        manager_send_event,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_generation_events_and_gate_are_rust_owned() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        test_clear();
        let start = generation();
        let id = test_seed_node(
            "Button",
            "Play",
            "Start the game",
            1,
            2,
            3,
            4,
            state_bit::CLICKABLE,
        );
        assert!(generation() > start);
        let node = snapshot().into_iter().find(|node| node.id == id).unwrap();
        assert_eq!(node.content_description, "Start the game");
        set_bridge_connected(true);
        assert!(lock_state().bridge_connected);
        set_bridge_connected(false);
        test_clear();
    }

    #[test]
    fn hooks_register_explicit_descriptors_and_handlers() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let vm = Vm::new();
        let _env = vm.attach_current_thread();
        register_hooks_for_vm(&vm).unwrap();

        let class = vm
            .find_class("android/view/accessibility/AccessibilityNodeInfo")
            .unwrap();
        let obtain = vm
            .find_method(
                class,
                "obtain",
                "()Landroid/view/accessibility/AccessibilityNodeInfo;",
                true,
            )
            .unwrap();
        assert!(vm.method_info(obtain).unwrap().4);

        let class = vm
            .find_class("android/view/accessibility/AccessibilityManager")
            .unwrap();
        let enabled = vm.find_method(class, "isEnabled", "()Z", false).unwrap();
        assert!(vm.method_info(enabled).unwrap().4);

        let action_class = "android/view/accessibility/AccessibilityNodeInfo$AccessibilityAction";
        let action_label = new_string(&vm, "Activate").unwrap();
        let JniValue::Object(Some(action)) = action_constructor(
            &vm,
            None,
            &[JniValue::Int(16), JniValue::Object(Some(action_label))],
        ) else {
            panic!("AccessibilityAction constructor should return an object");
        };
        let label_field = register_field(
            &vm,
            action_class,
            "label",
            "Ljava/lang/CharSequence;",
            false,
        )
        .unwrap();
        assert_eq!(
            vm.field_value(label_field, Some(action)).unwrap(),
            JniValue::Object(Some(action_label))
        );

        let click_field = register_field(
            &vm,
            action_class,
            "ACTION_CLICK",
            "Landroid/view/accessibility/AccessibilityNodeInfo$AccessibilityAction;",
            true,
        )
        .unwrap();
        let JniValue::Object(Some(click)) = vm.field_value(click_field, None).unwrap() else {
            panic!("ACTION_CLICK should be an object constant");
        };
        let id_field = register_field(&vm, action_class, "id", "I", false).unwrap();
        assert_eq!(
            vm.field_value(id_field, Some(click)).unwrap(),
            JniValue::Int(16)
        );

        let info_class = vm
            .find_class("android/view/accessibility/AccessibilityNodeInfo")
            .unwrap();
        let JniValue::Object(Some(info)) = vm.invoke_method(obtain, None, &[]).unwrap() else {
            panic!("obtain should return a node object");
        };
        let text = new_string(&vm, "Play").unwrap();
        let set_text = vm
            .find_method(info_class, "setText", "(Ljava/lang/CharSequence;)V", false)
            .unwrap();
        vm.invoke_method(set_text, Some(info), &[JniValue::Object(Some(text))])
            .unwrap();
        let checkable = vm
            .find_method(info_class, "setCheckable", "(Z)V", false)
            .unwrap();
        vm.invoke_method(checkable, Some(info), &[JniValue::Boolean(true)])
            .unwrap();
        let JniValue::Object(Some(bounds)) = rect_constructor(
            &vm,
            None,
            &[
                JniValue::Int(1),
                JniValue::Int(2),
                JniValue::Int(3),
                JniValue::Int(4),
            ],
        ) else {
            panic!("Rect constructor should return an object");
        };
        let set_bounds = vm
            .find_method(
                info_class,
                "setBoundsInScreen",
                "(Landroid/graphics/Rect;)V",
                false,
            )
            .unwrap();
        vm.invoke_method(set_bounds, Some(info), &[JniValue::Object(Some(bounds))])
            .unwrap();
        let node = snapshot()
            .into_iter()
            .find(|node| node.text == "Play")
            .unwrap();
        assert_eq!((node.left, node.top, node.right, node.bottom), (1, 2, 3, 4));
        assert_ne!(node.state & state_bit::CHECKABLE, 0);

        let event_class = vm
            .find_class("android/view/accessibility/AccessibilityEvent")
            .unwrap();
        let obtain_event = vm
            .find_method(
                event_class,
                "obtain",
                "(I)Landroid/view/accessibility/AccessibilityEvent;",
                true,
            )
            .unwrap();
        let JniValue::Object(Some(event)) = vm
            .invoke_method(obtain_event, None, &[JniValue::Int(32)])
            .unwrap()
        else {
            panic!("obtain should return an event object");
        };
        let set_class = vm
            .find_method(
                event_class,
                "setClassName",
                "(Ljava/lang/CharSequence;)V",
                false,
            )
            .unwrap();
        let set_description = vm
            .find_method(
                event_class,
                "setContentDescription",
                "(Ljava/lang/CharSequence;)V",
                false,
            )
            .unwrap();
        let label = new_string(&vm, "Button").unwrap();
        let message = new_string(&vm, "Start").unwrap();
        vm.invoke_method(set_class, Some(event), &[JniValue::Object(Some(label))])
            .unwrap();
        vm.invoke_method(
            set_description,
            Some(event),
            &[JniValue::Object(Some(message))],
        )
        .unwrap();
        let manager_class = vm
            .find_class("android/view/accessibility/AccessibilityManager")
            .unwrap();
        let get_manager = vm
            .find_method(
                manager_class,
                "getInstance",
                "(Landroid/content/Context;)Landroid/view/accessibility/AccessibilityManager;",
                true,
            )
            .unwrap();
        let JniValue::Object(Some(manager)) = vm
            .invoke_method(get_manager, None, &[JniValue::Object(None)])
            .unwrap()
        else {
            panic!("getInstance should return the manager singleton");
        };
        let send = vm
            .find_method(
                manager_class,
                "sendAccessibilityEvent",
                "(Landroid/view/accessibility/AccessibilityEvent;)V",
                false,
            )
            .unwrap();
        vm.invoke_method(send, Some(manager), &[JniValue::Object(Some(event))])
            .unwrap();
        assert_eq!(next_event(), Some((32, "Button".into(), "Start".into())));
    }
}
