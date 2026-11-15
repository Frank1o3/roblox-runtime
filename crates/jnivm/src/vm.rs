//! Runtime state shared by the JNI invocation and native interfaces.
//!
//! This layer contains no JNI function table yet. It establishes stable class,
//! method, field, reference, and thread records for the ABI adapters to use.

use crate::{MethodDescriptor, Type, parse_field_descriptor, parse_method_descriptor};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Mutex, OnceLock, RwLock};
use std::thread::{self, ThreadId};

include!("vm/types.rs");

/// The element type of a primitive JNI array, over the [`ObjectValue`]
/// variants the VM stores them as.
///
/// Sealed: the implementors are exactly Java's primitive element types.
pub trait PrimitiveElement: private_arrays::Sealed + Copy + Default + 'static {
    const LOG_NAME: &'static str;
    fn slot(value: &ObjectValue) -> Option<&Vec<Self>>;
    fn slot_mut(value: &mut ObjectValue) -> Option<&mut Vec<Self>>;
    fn value(values: Vec<Self>) -> ObjectValue;
}

mod private_arrays {
    pub trait Sealed {}
    impl Sealed for i8 {}
    impl Sealed for bool {}
    impl Sealed for u16 {}
    impl Sealed for i16 {}
    impl Sealed for i32 {}
    impl Sealed for i64 {}
    impl Sealed for f32 {}
    impl Sealed for f64 {}
}

impl PrimitiveElement for i8 {
    const LOG_NAME: &'static str = "byte";
    fn slot(value: &ObjectValue) -> Option<&Vec<Self>> {
        match value {
            ObjectValue::ByteArray(v) => Some(v),
            _ => None,
        }
    }
    fn slot_mut(value: &mut ObjectValue) -> Option<&mut Vec<Self>> {
        match value {
            ObjectValue::ByteArray(v) => Some(v),
            _ => None,
        }
    }
    fn value(values: Vec<Self>) -> ObjectValue {
        ObjectValue::ByteArray(values)
    }
}

impl PrimitiveElement for bool {
    const LOG_NAME: &'static str = "boolean";
    fn slot(value: &ObjectValue) -> Option<&Vec<Self>> {
        match value {
            ObjectValue::BooleanArray(v) => Some(v),
            _ => None,
        }
    }
    fn slot_mut(value: &mut ObjectValue) -> Option<&mut Vec<Self>> {
        match value {
            ObjectValue::BooleanArray(v) => Some(v),
            _ => None,
        }
    }
    fn value(values: Vec<Self>) -> ObjectValue {
        ObjectValue::BooleanArray(values)
    }
}

impl PrimitiveElement for u16 {
    const LOG_NAME: &'static str = "char";
    fn slot(value: &ObjectValue) -> Option<&Vec<Self>> {
        match value {
            ObjectValue::CharArray(v) => Some(v),
            _ => None,
        }
    }
    fn slot_mut(value: &mut ObjectValue) -> Option<&mut Vec<Self>> {
        match value {
            ObjectValue::CharArray(v) => Some(v),
            _ => None,
        }
    }
    fn value(values: Vec<Self>) -> ObjectValue {
        ObjectValue::CharArray(values)
    }
}

impl PrimitiveElement for i16 {
    const LOG_NAME: &'static str = "short";
    fn slot(value: &ObjectValue) -> Option<&Vec<Self>> {
        match value {
            ObjectValue::ShortArray(v) => Some(v),
            _ => None,
        }
    }
    fn slot_mut(value: &mut ObjectValue) -> Option<&mut Vec<Self>> {
        match value {
            ObjectValue::ShortArray(v) => Some(v),
            _ => None,
        }
    }
    fn value(values: Vec<Self>) -> ObjectValue {
        ObjectValue::ShortArray(values)
    }
}

impl PrimitiveElement for i32 {
    const LOG_NAME: &'static str = "int";
    fn slot(value: &ObjectValue) -> Option<&Vec<Self>> {
        match value {
            ObjectValue::IntArray(v) => Some(v),
            _ => None,
        }
    }
    fn slot_mut(value: &mut ObjectValue) -> Option<&mut Vec<Self>> {
        match value {
            ObjectValue::IntArray(v) => Some(v),
            _ => None,
        }
    }
    fn value(values: Vec<Self>) -> ObjectValue {
        ObjectValue::IntArray(values)
    }
}

impl PrimitiveElement for i64 {
    const LOG_NAME: &'static str = "long";
    fn slot(value: &ObjectValue) -> Option<&Vec<Self>> {
        match value {
            ObjectValue::LongArray(v) => Some(v),
            _ => None,
        }
    }
    fn slot_mut(value: &mut ObjectValue) -> Option<&mut Vec<Self>> {
        match value {
            ObjectValue::LongArray(v) => Some(v),
            _ => None,
        }
    }
    fn value(values: Vec<Self>) -> ObjectValue {
        ObjectValue::LongArray(values)
    }
}

impl PrimitiveElement for f32 {
    const LOG_NAME: &'static str = "float";
    fn slot(value: &ObjectValue) -> Option<&Vec<Self>> {
        match value {
            ObjectValue::FloatArray(v) => Some(v),
            _ => None,
        }
    }
    fn slot_mut(value: &mut ObjectValue) -> Option<&mut Vec<Self>> {
        match value {
            ObjectValue::FloatArray(v) => Some(v),
            _ => None,
        }
    }
    fn value(values: Vec<Self>) -> ObjectValue {
        ObjectValue::FloatArray(values)
    }
}

impl PrimitiveElement for f64 {
    const LOG_NAME: &'static str = "double";
    fn slot(value: &ObjectValue) -> Option<&Vec<Self>> {
        match value {
            ObjectValue::DoubleArray(v) => Some(v),
            _ => None,
        }
    }
    fn slot_mut(value: &mut ObjectValue) -> Option<&mut Vec<Self>> {
        match value {
            ObjectValue::DoubleArray(v) => Some(v),
            _ => None,
        }
    }
    fn value(values: Vec<Self>) -> ObjectValue {
        ObjectValue::DoubleArray(values)
    }
}

impl Vm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach the calling native thread, creating its local-reference scope.
    /// Repeated attachment is idempotent, as required by `AttachCurrentThread`.
    pub fn attach_current_thread(&self) -> ThreadEnv {
        let owner = thread::current().id();
        self.attached
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(owner);
        self.state
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .locals
            .entry(owner)
            .or_default();
        ThreadEnv { owner }
    }

    /// Return an environment token only if the calling thread is attached.
    pub fn get_env(&self) -> Option<ThreadEnv> {
        let owner = thread::current().id();
        self.attached
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&owner)
            .then_some(ThreadEnv { owner })
    }

    /// Detach the calling thread and release all its local references.
    pub fn detach_current_thread(&self) -> Result<(), JniError> {
        let owner = thread::current().id();
        let removed = self
            .attached
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&owner);
        if !removed {
            return Err(JniError::NotAttached);
        }
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if let Some(locals) = state.locals.remove(&owner) {
            for id in locals {
                state.objects.remove(&id);
            }
        }
        state.local_frames.remove(&owner);
        state.pending_exceptions.remove(&owner);
        Ok(())
    }

    /// Make `exception` the calling thread's pending throwable, as JNI
    /// `Throw` does. The reference must already be visible to the thread
    /// (local or global); a previously pending throwable is replaced, with a
    /// diagnostic in case a hook lost track of one.
    pub fn throw(&self, env: &ThreadEnv, exception: ObjectId) -> Result<(), JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        let record = state
            .objects
            .get(&exception)
            .ok_or(JniError::UnknownReference)?;
        if !record.global
            && !state
                .locals
                .get(&env.owner)
                .is_some_and(|locals| locals.contains(&exception))
        {
            return Err(JniError::ReferenceNotLocal);
        }
        if let Some(previous) = state.pending_exceptions.get(&env.owner) {
            eprintln!(
                "[jnivm] Throw replaced pending exception {:#x} with {:#x}",
                previous.0,
                exception.0
            );
        }
        state.pending_exceptions.insert(env.owner, exception);
        Ok(())
    }

    /// Construct a `java/lang/Throwable` instance of `class` carrying
    /// `message` and make it pending, as JNI `ThrowNew` does. Returns the
    /// created local reference, which keeps the throwable alive until the
    /// thread deletes it or detaches.
    pub fn throw_new(
        &self,
        env: &ThreadEnv,
        class: ClassId,
        message: Option<&str>,
    ) -> Result<ObjectId, JniError> {
        let class_name = self
            .class_name(class)
            .ok_or_else(|| JniError::UnknownClass(format!("class id {}", class.0)))?;
        if !is_throwable_class_name(&class_name) {
            eprintln!(
                "[jnivm] ThrowNew on non-Throwable class {class_name}; throwing it anyway"
            );
        }
        let exception = self.new_local_object(env, class, ObjectValue::Throwable(message.map(String::from)))?;
        self.throw(env, exception)?;
        Ok(exception)
    }

    /// The calling thread's pending throwable, if one was thrown and not
    /// cleared. JNI-wise this is what `ExceptionOccurred` exposes.
    pub fn pending_exception(&self, env: &ThreadEnv) -> Option<ObjectId> {
        if self.check_env(env).is_err() {
            return None;
        }
        self.state
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .pending_exceptions
            .get(&env.owner)
            .copied()
    }

    /// Clear the calling thread's pending throwable, returning whether one
    /// was pending (JNI `ExceptionClear` discards the status without touching
    /// the caller's references).
    pub fn clear_pending_exception(&self, env: &ThreadEnv) -> bool {
        if self.check_env(env).is_err() {
            return false;
        }
        self.state
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .pending_exceptions
            .remove(&env.owner)
            .is_some()
    }

    /// The Java class name and detail message of a throwable object, for
    /// `ExceptionDescribe`-style reporting. Returns `None` for non-throwable
    /// objects.
    pub fn exception_description(&self, exception: ObjectId) -> Option<String> {
        let state = self.state.read().unwrap_or_else(|p| p.into_inner());
        let record = state.objects.get(&exception)?;
        let message = match &record.value {
            ObjectValue::Throwable(message) => message.clone(),
            _ => return None,
        };
        let class_name = state.class_records.get(&record.class)?.name.clone();
        let display = class_name.replace('/', ".");
        Some(match message {
            Some(message) => format!("{display}: {message}"),
            None => display,
        })
    }

    pub fn register_class(&self, name: &str) -> Result<ClassId, JniError> {
        validate_class_name(name)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if state.classes.contains_key(name) {
            return Err(JniError::DuplicateClass(name.to_owned()));
        }
        let id = ClassId(state.id());
        state.classes.insert(name.to_owned(), id);
        state.class_records.insert(
            id,
            ClassRecord {
                name: name.to_owned(),
                methods: HashMap::new(),
                fields: HashMap::new(),
                natives: HashMap::new(),
            },
        );
        Ok(id)
    }

    pub fn find_class(&self, name: &str) -> Result<ClassId, JniError> {
        let found = self
            .state
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .classes
            .get(name)
            .copied();
        found.ok_or_else(|| {
            eprintln!("[jnivm] unimplemented class lookup: {name}");
            JniError::UnknownClass(name.to_owned())
        })
    }

    /// JNI `FindClass` creates a placeholder for classes supplied by Java in
    /// the real Android VM. Keep the same behavior for unregistered classes.
    pub fn find_or_define_class(&self, name: &str) -> Result<ClassId, JniError> {
        validate_class_name(name)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if let Some(class) = state.classes.get(name) {
            return Ok(*class);
        }
        eprintln!("[jnivm] unimplemented class {name}; creating placeholder");
        let id = ClassId(state.id());
        state.classes.insert(name.to_owned(), id);
        state.class_records.insert(
            id,
            ClassRecord {
                name: name.to_owned(),
                methods: HashMap::new(),
                fields: HashMap::new(),
                natives: HashMap::new(),
            },
        );
        Ok(id)
    }

    pub fn class_name(&self, class: ClassId) -> Option<String> {
        self.state
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .class_records
            .get(&class)
            .map(|record| record.name.clone())
    }

    pub fn register_method(
        &self,
        class: ClassId,
        name: &str,
        descriptor: &str,
        is_static: bool,
    ) -> Result<MethodId, JniError> {
        validate_member_name(name)?;
        let parsed = parse_method_descriptor(descriptor)
            .map_err(|error| JniError::InvalidDescriptor(error.to_string()))?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if !state.class_records.contains_key(&class) {
            return Err(JniError::UnknownClass(format!("class id {}", class.0)));
        }
        let key = MethodKey {
            name: name.to_owned(),
            descriptor: descriptor.to_owned(),
            is_static,
        };
        if let Some(id) = state
            .class_records
            .get(&class)
            .and_then(|c| c.methods.get(&key))
        {
            return Ok(*id);
        }
        let class_name = state.class_records.get(&class).unwrap().name.clone();
        let id = MethodId(state.id());
        state.method_descriptors.insert(id, parsed);
        state.method_metadata.insert(
            id,
            MethodMeta {
                class_id: class,
                class_name,
                name: name.to_owned(),
                descriptor: descriptor.to_owned(),
                is_static,
            },
        );
        state
            .class_records
            .get_mut(&class)
            .unwrap()
            .methods
            .insert(key, id);
        Ok(id)
    }

    pub fn find_method(
        &self,
        class: ClassId,
        name: &str,
        descriptor: &str,
        is_static: bool,
    ) -> Result<MethodId, JniError> {
        let state = self.state.read().unwrap_or_else(|p| p.into_inner());
        let record = state
            .class_records
            .get(&class)
            .ok_or_else(|| JniError::UnknownClass(format!("class id {}", class.0)))?;
        let found = record.methods.get(&MethodKey {
            name: name.to_owned(),
            descriptor: descriptor.to_owned(),
            is_static,
        });
        if let Some(id) = found {
            return Ok(*id);
        }
        // `java/lang/String` is the only `CharSequence` implementation this VM
        // hands out, so a hook registered under one descriptor and a call
        // compiled against the other describe the same method. The C++ path
        // worked around this by registering both descriptors (the original
        // failure was silent: `getProductName` returning String never matched
        // the `CharSequence` call the dex compiled, so every device name was
        // null); matching at lookup time removes the duplicate registration.
        let normalized = normalize_descriptor(descriptor);
        let compatible = record
            .methods
            .iter()
            .find(|(key, _)| {
                key.name == name && key.is_static == is_static && normalize_descriptor(&key.descriptor) == normalized
            })
            .map(|(_, id)| *id);
        match compatible {
            Some(id) => {
                let registered = state
                    .method_metadata
                    .get(&id)
                    .map(|m| m.descriptor.clone())
                    .unwrap_or_default();
                log_once(&format!("{name}{descriptor}~{registered}"), || {
                    format!(
                        "[jnivm] {}.{} matched {descriptor} through CharSequence/String descriptor compatibility (registered {registered})",
                        record.name, name
                    )
                });
                Ok(id)
            }
            None => {
                eprintln!(
                    "[jnivm] unimplemented method lookup: {}.{name}{descriptor}",
                    record.name
                );
                Err(JniError::UnknownMethod(format!(
                    "{}.{name}{descriptor}",
                    record.name
                )))
            }
        }
    }

    /// Resolve a method ID, creating a typed placeholder when the Java method
    /// has no Rust implementation yet. This mirrors libjnivm's permissive
    /// lookup and gives the later invocation a chance to log and return a safe
    /// type-correct default.
    pub fn resolve_method(
        &self,
        class: ClassId,
        name: &str,
        descriptor: &str,
        is_static: bool,
    ) -> Result<MethodId, JniError> {
        match self.find_method(class, name, descriptor, is_static) {
            Ok(id) => Ok(id),
            Err(JniError::UnknownMethod(_)) => {
                // The VM is inventing a registration the engine will call
                // through; that is exactly the moment the C++ path logged
                // `Constructed Unresolved symbol` and this report should name.
                let class_name = self.class_name(class).unwrap_or_else(|| format!("class id {}", class.0));
                crate::unresolved::record_unresolved_method(&class_name, name, descriptor, is_static);
                self.register_method(class, name, descriptor, is_static)
            }
            Err(error) => Err(error),
        }
    }

    pub fn install_method_handler(
        &self,
        method: MethodId,
        handler: MethodHandler,
    ) -> Result<(), JniError> {
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if !state.method_descriptors.contains_key(&method) {
            return Err(JniError::UnknownMethod(format!("method id {}", method.0)));
        }
        state.method_handlers.insert(method, handler);
        Ok(())
    }

    pub fn invoke_method(
        &self,
        method: MethodId,
        receiver: Option<ObjectId>,
        arguments: &[JniValue],
    ) -> Result<JniValue, JniError> {
        let (descriptor, handler, method_name) = {
            let state = self.state.read().unwrap_or_else(|p| p.into_inner());
            let descriptor = state
                .method_descriptors
                .get(&method)
                .cloned()
                .ok_or_else(|| JniError::UnknownMethod(format!("method id {}", method.0)))?;
            let handler = state.method_handlers.get(&method).copied();
            let method_name = state
                .method_metadata
                .get(&method)
                .map(|m| {
                    format!(
                        "{}.{}`{}{}",
                        m.class_name,
                        if m.is_static { "static " } else { "" },
                        m.name,
                        m.descriptor
                    )
                })
                .unwrap_or_else(|| format!("method id {}", method.0));
            (descriptor, handler, method_name)
        };
        if arguments.len() != descriptor.parameters.len() {
            return Err(JniError::InvalidDescriptor(format!(
                "{method_name} expects {} arguments, got {}",
                descriptor.parameters.len(),
                arguments.len()
            )));
        }
        if let Some(handler) = handler {
            Ok(handler(self, receiver, arguments))
        } else {
            eprintln!("[jnivm] unimplemented method call: {method_name}");
            Ok(JniValue::default_for(&descriptor.result))
        }
    }

    /// Invoke a method without an implementation using descriptor-correct
    /// zero/null arguments. Useful for JNI table entry points that receive a
    /// platform `va_list` which the current Rust backend cannot decode yet.
    pub fn invoke_default(
        &self,
        method: MethodId,
        receiver: Option<ObjectId>,
    ) -> Result<JniValue, JniError> {
        let arguments = {
            let state = self.state.read().unwrap_or_else(|p| p.into_inner());
            let descriptor = state
                .method_descriptors
                .get(&method)
                .cloned()
                .ok_or_else(|| JniError::UnknownMethod(format!("method id {}", method.0)))?;
            descriptor
                .parameters
                .iter()
                .map(JniValue::default_for)
                .collect::<Vec<_>>()
        };
        self.invoke_method(method, receiver, &arguments)
    }

    pub fn method_descriptor(&self, method: MethodId) -> Option<MethodDescriptor> {
        self.state
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .method_descriptors
            .get(&method)
            .cloned()
    }

    /// Return the declaring class, name, descriptor, staticness, and whether a
    /// Rust handler is installed for this method ID.
    pub fn method_info(&self, method: MethodId) -> Option<(String, String, String, bool, bool)> {
        let state = self.state.read().unwrap_or_else(|p| p.into_inner());
        let meta = state.method_metadata.get(&method)?;
        let has_handler = state.method_handlers.contains_key(&method);
        Some((
            meta.class_name.clone(),
            meta.name.clone(),
            meta.descriptor.clone(),
            meta.is_static,
            has_handler,
        ))
    }

    /// Record a JNI `RegisterNatives` entry. Function addresses are opaque
    /// integers here; the unsafe ABI adapter owns their conversion/calling.
    pub fn register_native(
        &self,
        class: ClassId,
        name: &str,
        descriptor: &str,
        function: usize,
    ) -> Result<(), JniError> {
        validate_member_name(name)?;
        parse_method_descriptor(descriptor)
            .map_err(|error| JniError::InvalidDescriptor(error.to_string()))?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        let record = state
            .class_records
            .get_mut(&class)
            .ok_or_else(|| JniError::UnknownClass(format!("class id {}", class.0)))?;
        record
            .natives
            .insert((name.to_owned(), descriptor.to_owned()), function);
        Ok(())
    }

    pub fn find_native(&self, class: ClassId, name: &str, descriptor: &str) -> Option<usize> {
        self.state
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .class_records
            .get(&class)?
            .natives
            .get(&(name.to_owned(), descriptor.to_owned()))
            .copied()
    }

    pub fn register_field(
        &self,
        class: ClassId,
        name: &str,
        descriptor: &str,
        is_static: bool,
    ) -> Result<FieldId, JniError> {
        validate_member_name(name)?;
        let ty = parse_field_descriptor(descriptor)
            .map_err(|error| JniError::InvalidDescriptor(error.to_string()))?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if !state.class_records.contains_key(&class) {
            return Err(JniError::UnknownClass(format!("class id {}", class.0)));
        }
        let key = FieldKey {
            name: name.to_owned(),
            descriptor: descriptor.to_owned(),
            is_static,
        };
        if let Some(id) = state
            .class_records
            .get(&class)
            .and_then(|c| c.fields.get(&key))
        {
            return Ok(*id);
        }
        let class_name = state.class_records.get(&class).unwrap().name.clone();
        let id = FieldId(state.id());
        state.field_types.insert(id, ty);
        state.field_metadata.insert(
            id,
            FieldMeta {
                class_id: class,
                class_name,
                name: name.to_owned(),
                descriptor: descriptor.to_owned(),
                is_static,
            },
        );
        state
            .class_records
            .get_mut(&class)
            .unwrap()
            .fields
            .insert(key, id);
        Ok(id)
    }

    pub fn find_field(
        &self,
        class: ClassId,
        name: &str,
        descriptor: &str,
        is_static: bool,
    ) -> Result<FieldId, JniError> {
        let state = self.state.read().unwrap_or_else(|p| p.into_inner());
        let record = state
            .class_records
            .get(&class)
            .ok_or_else(|| JniError::UnknownClass(format!("class id {}", class.0)))?;
        record
            .fields
            .get(&FieldKey {
                name: name.to_owned(),
                descriptor: descriptor.to_owned(),
                is_static,
            })
            .copied()
            .ok_or_else(|| {
                eprintln!(
                    "[jnivm] unimplemented field lookup: {}.{name}:{descriptor}",
                    record.name
                );
                JniError::UnknownField(format!("{}.{name}:{descriptor}", record.name))
            })
    }

    pub fn resolve_field(
        &self,
        class: ClassId,
        name: &str,
        descriptor: &str,
        is_static: bool,
    ) -> Result<FieldId, JniError> {
        match self.find_field(class, name, descriptor, is_static) {
            Ok(id) => Ok(id),
            Err(JniError::UnknownField(_)) => {
                let class_name = self.class_name(class).unwrap_or_else(|| format!("class id {}", class.0));
                crate::unresolved::record_unresolved_field(&class_name, name, descriptor, is_static);
                self.register_field(class, name, descriptor, is_static)
            }
            Err(error) => Err(error),
        }
    }

    pub fn field_type(&self, field: FieldId) -> Option<Type> {
        self.state
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .field_types
            .get(&field)
            .cloned()
    }

    pub fn field_info(&self, field: FieldId) -> Option<(String, String, String, bool)> {
        let state = self.state.read().unwrap_or_else(|p| p.into_inner());
        let meta = state.field_metadata.get(&field)?;
        Some((
            meta.class_name.clone(),
            meta.name.clone(),
            meta.descriptor.clone(),
            meta.is_static,
        ))
    }

    pub fn field_value_is_set(&self, field: FieldId, receiver: Option<ObjectId>) -> bool {
        self.state
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .field_values
            .contains_key(&(field, receiver))
    }

    pub fn field_value(
        &self,
        field: FieldId,
        receiver: Option<ObjectId>,
    ) -> Result<JniValue, JniError> {
        let state = self.state.read().unwrap_or_else(|p| p.into_inner());
        let ty = state
            .field_types
            .get(&field)
            .ok_or_else(|| JniError::UnknownField(format!("field id {}", field.0)))?;
        Ok(state
            .field_values
            .get(&(field, receiver))
            .cloned()
            .unwrap_or_else(|| JniValue::default_for(ty)))
    }

    pub fn set_field_value(
        &self,
        field: FieldId,
        receiver: Option<ObjectId>,
        value: JniValue,
    ) -> Result<(), JniError> {
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if !state.field_types.contains_key(&field) {
            return Err(JniError::UnknownField(format!("field id {}", field.0)));
        }
        state.field_values.insert((field, receiver), value);
        Ok(())
    }

    /// Allocate a local object reference in the current thread's scope.
    pub fn new_local_object(
        &self,
        env: &ThreadEnv,
        class: ClassId,
        value: ObjectValue,
    ) -> Result<ObjectId, JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if !state.class_records.contains_key(&class) {
            return Err(JniError::UnknownClass(format!("class id {}", class.0)));
        }
        let id = ObjectId(state.id());
        state.objects.insert(
            id,
            ObjectRecord {
                class,
                value,
                identity: id.raw(),
                global: false,
            },
        );
        state.locals.entry(env.owner).or_default().insert(id);
        if let Some(frames) = state.local_frames.get_mut(&env.owner) {
            if let Some(top) = frames.last_mut() {
                top.insert(id);
            }
        }
        Ok(id)
    }

    /// Allocate a local Java object array. `initial` is the reference copied
    /// into every element, matching JNI `NewObjectArray` semantics.
    pub fn new_local_object_array(
        &self,
        env: &ThreadEnv,
        length: usize,
        initial: Option<ObjectId>,
    ) -> Result<ObjectId, JniError> {
        let class = self.find_or_define_class("java/lang/Object")?;
        self.new_local_object(env, class, ObjectValue::ObjectArray(vec![initial; length]))
    }

    pub fn object_array_length(&self, env: &ThreadEnv, array: ObjectId) -> Result<usize, JniError> {
        let ObjectValue::ObjectArray(elements) = self.object_value(env, array)? else {
            return Err(JniError::UnknownReference);
        };
        Ok(elements.len())
    }

    pub fn object_array_element(
        &self,
        env: &ThreadEnv,
        array: ObjectId,
        index: usize,
    ) -> Result<Option<ObjectId>, JniError> {
        let ObjectValue::ObjectArray(elements) = self.object_value(env, array)? else {
            return Err(JniError::UnknownReference);
        };
        elements
            .get(index)
            .copied()
            .ok_or(JniError::UnknownReference)
    }

    pub fn set_object_array_element(
        &self,
        env: &ThreadEnv,
        array: ObjectId,
        index: usize,
        value: Option<ObjectId>,
    ) -> Result<(), JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if !state.objects.contains_key(&array)
            || (!state.objects[&array].global
                && !state
                    .locals
                    .get(&env.owner)
                    .is_some_and(|locals| locals.contains(&array)))
        {
            return Err(JniError::UnknownReference);
        }
        if let Some(value) = value {
            let object = state
                .objects
                .get(&value)
                .ok_or(JniError::UnknownReference)?;
            if !object.global
                && !state
                    .locals
                    .get(&env.owner)
                    .is_some_and(|locals| locals.contains(&value))
            {
                return Err(JniError::ReferenceNotLocal);
            }
        }
        let ObjectValue::ObjectArray(elements) = &mut state.objects.get_mut(&array).unwrap().value
        else {
            return Err(JniError::UnknownReference);
        };
        let element = elements.get_mut(index).ok_or(JniError::UnknownReference)?;
        *element = value;
        Ok(())
    }

    /// Promote a local reference to a VM-wide reference.
    pub fn new_global_ref(&self, env: &ThreadEnv, object: ObjectId) -> Result<ObjectId, JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        let record = state
            .objects
            .get(&object)
            .ok_or(JniError::UnknownReference)?;
        if !record.global
            && !state
                .locals
                .get(&env.owner)
                .is_some_and(|locals| locals.contains(&object))
        {
            return Err(JniError::ReferenceNotLocal);
        }
        let (class, value, identity) = (record.class, record.value.clone(), record.identity);
        let global = ObjectId(state.id());
        state.objects.insert(
            global,
            ObjectRecord {
                class,
                value,
                identity,
                global: true,
            },
        );
        Ok(global)
    }

    /// Compare JNI references by the Java object they name. Local and global
    /// handles can have different values while retaining the same identity.
    pub fn same_object(&self, left: ObjectId, right: ObjectId) -> bool {
        if left == right {
            return true;
        }
        let state = self.state.read().unwrap_or_else(|p| p.into_inner());
        match (state.objects.get(&left), state.objects.get(&right)) {
            (Some(left), Some(right)) => left.identity == right.identity,
            _ => false,
        }
    }

    pub fn delete_local_ref(&self, env: &ThreadEnv, object: ObjectId) -> Result<(), JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        let locals = state
            .locals
            .get_mut(&env.owner)
            .ok_or(JniError::NotAttached)?;
        if !locals.remove(&object) {
            return Err(JniError::ReferenceNotLocal);
        }
        state.objects.remove(&object);
        Ok(())
    }

    pub fn delete_global_ref(&self, object: ObjectId) -> Result<(), JniError> {
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        match state.objects.get(&object) {
            Some(record) if record.global => {
                state.objects.remove(&object);
                Ok(())
            }
            Some(_) => Err(JniError::ReferenceNotLocal),
            None => Err(JniError::UnknownReference),
        }
    }

    pub fn object_value(&self, env: &ThreadEnv, object: ObjectId) -> Result<ObjectValue, JniError> {
        self.check_env(env)?;
        let state = self.state.read().unwrap_or_else(|p| p.into_inner());
        let record = state
            .objects
            .get(&object)
            .ok_or(JniError::UnknownReference)?;
        if !record.global
            && !state
                .locals
                .get(&env.owner)
                .is_some_and(|locals| locals.contains(&object))
        {
            return Err(JniError::ReferenceNotLocal);
        }
        Ok(record.value.clone())
    }

    /// Return the stable backing allocation for JNI's direct byte-array
    /// element access. ByteArray values are created at their final length and
    /// are never resized, so the pointer remains valid until the object ref is
    /// deleted or its owning thread detaches.
    pub fn byte_array_elements(
        &self,
        env: &ThreadEnv,
        array: ObjectId,
    ) -> Result<*mut i8, JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        let is_global = state
            .objects
            .get(&array)
            .ok_or(JniError::UnknownReference)?
            .global;
        if !is_global
            && !state
                .locals
                .get(&env.owner)
                .is_some_and(|locals| locals.contains(&array))
        {
            return Err(JniError::ReferenceNotLocal);
        }
        let record = state
            .objects
            .get_mut(&array)
            .ok_or(JniError::UnknownReference)?;
        match &mut record.value {
            ObjectValue::ByteArray(values) => Ok(values.as_mut_ptr()),
            _ => Err(JniError::UnknownReference),
        }
    }

    pub fn byte_array_region(
        &self,
        env: &ThreadEnv,
        array: ObjectId,
        start: usize,
        length: usize,
    ) -> Result<Vec<i8>, JniError> {
        let ObjectValue::ByteArray(values) = self.object_value(env, array)? else {
            return Err(JniError::UnknownReference);
        };
        let end = start
            .checked_add(length)
            .ok_or(JniError::UnknownReference)?;
        values
            .get(start..end)
            .map(<[i8]>::to_vec)
            .ok_or(JniError::UnknownReference)
    }

    pub fn set_byte_array_region(
        &self,
        env: &ThreadEnv,
        array: ObjectId,
        start: usize,
        values: &[i8],
    ) -> Result<(), JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        let is_global = state
            .objects
            .get(&array)
            .ok_or(JniError::UnknownReference)?
            .global;
        if !is_global
            && !state
                .locals
                .get(&env.owner)
                .is_some_and(|locals| locals.contains(&array))
        {
            return Err(JniError::ReferenceNotLocal);
        }
        let record = state
            .objects
            .get_mut(&array)
            .ok_or(JniError::UnknownReference)?;
        let ObjectValue::ByteArray(elements) = &mut record.value else {
            return Err(JniError::UnknownReference);
        };
        let end = start
            .checked_add(values.len())
            .ok_or(JniError::UnknownReference)?;
        let target = elements
            .get_mut(start..end)
            .ok_or(JniError::UnknownReference)?;
        target.copy_from_slice(values);
        Ok(())
    }

    /// Allocate a primitive array of `length` zeroed elements.
    pub fn new_primitive_array<E: PrimitiveElement>(
        &self,
        env: &ThreadEnv,
        length: usize,
    ) -> Result<ObjectId, JniError> {
        self.check_env(env)?;
        let class = self.find_or_define_class("java/lang/Object")?;
        self.new_local_object(env, class, E::value(vec![E::default(); length]))
    }

    /// Return the stable backing allocation for a primitive array's direct
    /// element access. Arrays are created at their final length and never
    /// resized, so the pointer remains valid until the object ref is deleted
    /// or its owning thread detaches — the same lifetime rule
    /// [`Vm::byte_array_elements`] documents.
    pub fn primitive_array_elements<E: PrimitiveElement>(
        &self,
        env: &ThreadEnv,
        array: ObjectId,
    ) -> Result<*mut E, JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        let is_global = state
            .objects
            .get(&array)
            .ok_or(JniError::UnknownReference)?
            .global;
        if !is_global
            && !state
                .locals
                .get(&env.owner)
                .is_some_and(|locals| locals.contains(&array))
        {
            return Err(JniError::ReferenceNotLocal);
        }
        let record = state
            .objects
            .get_mut(&array)
            .ok_or(JniError::UnknownReference)?;
        E::slot_mut(&mut record.value)
            .map(Vec::as_mut_ptr)
            .ok_or(JniError::UnknownReference)
    }

    /// Copy a range out of a primitive array.
    pub fn primitive_array_region<E: PrimitiveElement>(
        &self,
        env: &ThreadEnv,
        array: ObjectId,
        start: usize,
        length: usize,
    ) -> Result<Vec<E>, JniError> {
        let values = self.object_value(env, array)?;
        E::slot(&values)
            .and_then(|values| values.get(start..start.checked_add(length)?))
            .map(<[E]>::to_vec)
            .ok_or(JniError::UnknownReference)
    }

    /// Copy a range into a primitive array.
    pub fn set_primitive_array_region<E: PrimitiveElement>(
        &self,
        env: &ThreadEnv,
        array: ObjectId,
        start: usize,
        values: &[E],
    ) -> Result<(), JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        let is_global = state
            .objects
            .get(&array)
            .ok_or(JniError::UnknownReference)?
            .global;
        if !is_global
            && !state
                .locals
                .get(&env.owner)
                .is_some_and(|locals| locals.contains(&array))
        {
            return Err(JniError::ReferenceNotLocal);
        }
        let record = state
            .objects
            .get_mut(&array)
            .ok_or(JniError::UnknownReference)?;
        let elements = E::slot_mut(&mut record.value).ok_or(JniError::UnknownReference)?;
        let end = start
            .checked_add(values.len())
            .ok_or(JniError::UnknownReference)?;
        let target = elements
            .get_mut(start..end)
            .ok_or(JniError::UnknownReference)?;
        target.copy_from_slice(values);
        Ok(())
    }

    pub fn object_class_name(&self, object: u64) -> Option<String> {
        let state = self.state.read().unwrap_or_else(|p| p.into_inner());
        let record = state.objects.get(&ObjectId(object))?;
        state
            .class_records
            .get(&record.class)
            .map(|class| class.name.clone())
    }

    pub fn clone_local_ref(&self, env: &ThreadEnv, object: u64) -> Result<ObjectId, JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        let source = state
            .objects
            .get(&ObjectId(object))
            .ok_or(JniError::UnknownReference)?;
        if !source.global
            && !state
                .locals
                .get(&env.owner)
                .is_some_and(|locals| locals.contains(&ObjectId(object)))
        {
            return Err(JniError::ReferenceNotLocal);
        }
        let (class, value, identity) = (source.class, source.value.clone(), source.identity);
        let local = ObjectId(state.id());
        state.objects.insert(
            local,
            ObjectRecord {
                class,
                value,
                identity,
                global: false,
            },
        );
        state.locals.entry(env.owner).or_default().insert(local);
        if let Some(frames) = state.local_frames.get_mut(&env.owner) {
            if let Some(top) = frames.last_mut() {
                top.insert(local);
            }
        }
        Ok(local)
    }

    pub fn push_local_frame(&self, env: &ThreadEnv, _capacity: i32) -> Result<(), JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        state
            .local_frames
            .entry(env.owner)
            .or_default()
            .push(HashSet::new());
        Ok(())
    }

    pub fn pop_local_frame(
        &self,
        env: &ThreadEnv,
        result: Option<ObjectId>,
    ) -> Result<Option<ObjectId>, JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        let frame = state
            .local_frames
            .get_mut(&env.owner)
            .and_then(|frames| frames.pop());
        let Some(frame) = frame else {
            return Ok(result);
        };
        let mut preserved_id = None;
        let mut to_remove = Vec::new();
        for id in frame {
            if Some(id) == result {
                preserved_id = Some(id);
                if let Some(top) = state
                    .local_frames
                    .get_mut(&env.owner)
                    .and_then(|f| f.last_mut())
                {
                    top.insert(id);
                }
            } else {
                to_remove.push(id);
            }
        }
        if let Some(locals) = state.locals.get_mut(&env.owner) {
            for id in &to_remove {
                locals.remove(id);
            }
        }
        for id in to_remove {
            state.objects.remove(&id);
        }
        Ok(preserved_id.or(result))
    }

    /// Run a mark-and-sweep garbage collection cycle over the VM state.
    /// Collects unrooted objects, prunes detached thread locals, and sweeps
    /// abandoned instance field values.
    pub fn gc(&self) -> (crate::GcStats, Vec<ObjectId>) {
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        let attached = self.attached.lock().unwrap_or_else(|p| p.into_inner());

        // 1. Prune dead threads from locals and local_frames
        let dead_threads: Vec<ThreadId> = state
            .locals
            .keys()
            .copied()
            .filter(|tid| !attached.contains(tid))
            .collect();
        for tid in dead_threads {
            if let Some(unattached_locals) = state.locals.remove(&tid) {
                for id in unattached_locals {
                    state.objects.remove(&id);
                }
            }
            state.local_frames.remove(&tid);
        }

        // 2. Mark phase: identify all reachable ObjectIds
        let mut reachable = HashSet::new();
        let mut worklist = Vec::new();

        // Roots: all globals
        for (&id, record) in &state.objects {
            if record.global && reachable.insert(id) {
                worklist.push(id);
            }
        }

        // Roots: all locals from currently attached threads
        for (&tid, thread_locals) in &state.locals {
            if attached.contains(&tid) {
                for &id in thread_locals {
                    if reachable.insert(id) {
                        worklist.push(id);
                    }
                }
            }
        }

        // Roots: static field values
        for ((_field_id, receiver), val) in &state.field_values {
            if receiver.is_none() {
                if let JniValue::Object(Some(id)) = val {
                    if reachable.insert(*id) {
                        worklist.push(*id);
                    }
                }
            }
        }

        // Traverse object graph
        while let Some(current_id) = worklist.pop() {
            if let Some(record) = state.objects.get(&current_id) {
                if let ObjectValue::ObjectArray(ref elements) = record.value {
                    for elem in elements.iter().flatten() {
                        if reachable.insert(*elem) {
                            worklist.push(*elem);
                        }
                    }
                }
            }

            for ((_field_id, receiver), val) in &state.field_values {
                if *receiver == Some(current_id) {
                    if let JniValue::Object(Some(target_id)) = val {
                        if reachable.insert(*target_id) {
                            worklist.push(*target_id);
                        }
                    }
                }
            }
        }

        // 3. Sweep dead objects
        let mut collected_ids = Vec::new();
        let total_before = state.objects.len();
        state.objects.retain(|&id, _| {
            let keep = reachable.contains(&id);
            if !keep {
                collected_ids.push(id);
            }
            keep
        });

        // 4. Sweep dead instance field values (receiver is not live)
        let fields_before = state.field_values.len();
        state.field_values.retain(|(_, receiver), _| {
            match receiver {
                None => true, // static field
                Some(rec_id) => reachable.contains(rec_id),
            }
        });
        let collected_fields = fields_before.saturating_sub(state.field_values.len());

        // 5. Shrink allocations if significant garbage was reclaimed (> 25%)
        if collected_ids.len() > 100 && collected_ids.len() * 4 > total_before {
            state.objects.shrink_to_fit();
            state.field_values.shrink_to_fit();
        }

        let stats = crate::GcStats {
            collected_objects: collected_ids.len(),
            collected_fields,
            live_objects: state.objects.len(),
        };

        (stats, collected_ids)
    }

    fn check_env(&self, env: &ThreadEnv) -> Result<(), JniError> {
        if env.owner != thread::current().id() {
            return Err(JniError::WrongThread);
        }
        if !self
            .attached
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&env.owner)
        {
            return Err(JniError::NotAttached);
        }
        Ok(())
    }
}

fn validate_class_name(name: &str) -> Result<(), JniError> {
    if name.is_empty()
        || name.starts_with('/')
        || name.ends_with('/')
        || name.contains('.')
        || name
            .split('/')
            .any(|part| part.is_empty() || part.contains(';') || part.contains('['))
    {
        return Err(JniError::InvalidClassName);
    }
    Ok(())
}

/// Whether a JNI class name names a `java/lang/Throwable` descendant. JNI
/// class names use `/` separators; Java's hierarchy has no multiple
/// inheritance of classes, so matching the well-known Throwable descendants
/// by suffix covers the exception classes the runtime registers.
fn is_throwable_class_name(class_name: &str) -> bool {
    class_name == "java/lang/Throwable"
        || class_name == "java/lang/Exception"
        || class_name == "java/lang/Error"
        || class_name.ends_with("Exception")
        || class_name.ends_with("Error")
        || class_name.ends_with("Throwable")
}

/// Fold `Ljava/lang/CharSequence;` to `Ljava/lang/String;` for lookup
/// compatibility: `String` is the only `CharSequence` implementation this VM
/// hands out, so descriptors differing in exactly that pair name the same
/// method.
fn normalize_descriptor(descriptor: &str) -> String {
    if descriptor.contains("Ljava/lang/CharSequence;") {
        descriptor.replace("Ljava/lang/CharSequence;", "Ljava/lang/String;")
    } else {
        descriptor.to_owned()
    }
}

static COMPAT_MATCHES_LOGGED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Emit a diagnostic once per distinct key for the process lifetime, so a
/// hot lookup path cannot turn the compat note into line noise.
fn log_once(key: &str, message: impl FnOnce() -> String) {
    let logged = COMPAT_MATCHES_LOGGED.get_or_init(|| Mutex::new(HashSet::new()));
    if logged
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key.to_owned())
    {
        eprintln!("{}", message());
    }
}

fn validate_member_name(name: &str) -> Result<(), JniError> {
    if name.is_empty() || name.chars().any(|ch| matches!(ch, '/' | '.' | ';' | '[')) {
        return Err(JniError::InvalidMemberName);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitive_array_roundtrip() {
        let vm = Vm::new();
        let env = vm.attach_current_thread();

        // Allocate a zeroed array of the requested length.
        let ints = vm.new_primitive_array::<i32>(&env, 4).unwrap();
        assert_eq!(
            vm.primitive_array_region::<i32>(&env, ints, 0, 4).unwrap(),
            [0, 0, 0, 0]
        );

        // Write through a region copy.
        vm.set_primitive_array_region(&env, ints, 1, &[7, 8, 9])
            .unwrap();
        assert_eq!(
            vm.primitive_array_region::<i32>(&env, ints, 0, 4).unwrap(),
            [0, 7, 8, 9]
        );
        // Partial reads respect the requested range.
        assert_eq!(vm.primitive_array_region::<i32>(&env, ints, 2, 2).unwrap(), [8, 9]);

        // Direct element access sees the same storage and can mutate it.
        let elements = vm.primitive_array_elements::<i32>(&env, ints).unwrap();
        unsafe {
            assert_eq!(*elements.add(1), 7);
            *elements.add(3) = 42;
        }
        assert_eq!(
            vm.primitive_array_region::<i32>(&env, ints, 3, 1).unwrap(),
            [42]
        );
    }

    #[test]
    fn test_primitive_array_all_element_types() {
        let vm = Vm::new();
        let env = vm.attach_current_thread();

        macro_rules! roundtrip {
            ($ty:ty, $values:expr) => {{
                let array = vm.new_primitive_array::<$ty>(&env, $values.len()).unwrap();
                vm.set_primitive_array_region(&env, array, 0, &$values).unwrap();
                assert_eq!(
                    vm.primitive_array_region::<$ty>(&env, array, 0, $values.len()).unwrap(),
                    $values
                );
            }};
        }

        roundtrip!(i8, [-1i8, 0, 127]);
        roundtrip!(bool, [true, false, true]);
        roundtrip!(u16, [0u16, 0x4e2d, u16::MAX]);
        roundtrip!(i16, [-32768i16, 0, 32767]);
        roundtrip!(i32, [-2147483648i32, 0, i32::MAX]);
        roundtrip!(i64, [-1i64, 0, i64::MAX]);
        roundtrip!(f32, [1.5f32, -0.25, f32::INFINITY]);
        roundtrip!(f64, [2.5f64, -0.125, f64::MIN]);
    }

    #[test]
    fn test_primitive_array_rejects_type_mismatch_and_bad_ranges() {
        let vm = Vm::new();
        let env = vm.attach_current_thread();

        let bytes = vm.new_primitive_array::<i8>(&env, 4).unwrap();

        // A byte array's storage cannot be read back as another element type.
        assert_eq!(
            vm.primitive_array_elements::<i32>(&env, bytes),
            Err(JniError::UnknownReference)
        );
        assert_eq!(
            vm.primitive_array_region::<i32>(&env, bytes, 0, 4),
            Err(JniError::UnknownReference)
        );
        assert_eq!(
            vm.set_primitive_array_region(&env, bytes, 0, &[1i32, 2, 3]),
            Err(JniError::UnknownReference)
        );

        // Out-of-bounds regions fail without corrupting the array.
        assert_eq!(
            vm.primitive_array_region::<i8>(&env, bytes, 3, 2),
            Err(JniError::UnknownReference)
        );
        assert_eq!(
            vm.set_primitive_array_region(&env, bytes, 3, &[1i8, 2]),
            Err(JniError::UnknownReference)
        );
        assert_eq!(
            vm.primitive_array_region::<i8>(&env, bytes, 0, 4).unwrap(),
            [0, 0, 0, 0]
        );

        // Unknown references fail for every accessor.
        let missing = ObjectId::from_raw(u64::MAX);
        assert!(vm.primitive_array_elements::<i8>(&env, missing).is_err());
    }

    #[test]
    fn test_throw_new_pending_and_clear() {
        let vm = Vm::new();
        let env = vm.attach_current_thread();
        let class = vm
            .find_or_define_class("java/lang/IllegalArgumentException")
            .unwrap();

        let exception = vm
            .throw_new(&env, class, Some("bad input"))
            .unwrap();
        // ThrowNew creates the throwable as a live local reference.
        assert!(vm.object_value(&env, exception).is_ok());
        assert_eq!(vm.pending_exception(&env), Some(exception));

        // ExceptionDescribe-style reporting shows class and message with
        // Java-style dots.
        assert_eq!(
            vm.exception_description(exception).as_deref(),
            Some("java.lang.IllegalArgumentException: bad input")
        );

        // Clearing makes the exception pending no more; the local reference
        // survives because clearing does not free caller-owned references.
        assert!(vm.clear_pending_exception(&env));
        assert_eq!(vm.pending_exception(&env), None);
        assert!(vm.object_value(&env, exception).is_ok());
        // Clearing again reports there was nothing to clear.
        assert!(!vm.clear_pending_exception(&env));
    }

    #[test]
    fn test_throw_new_without_message() {
        let vm = Vm::new();
        let env = vm.attach_current_thread();
        let class = vm.find_or_define_class("java/lang/IllegalStateException").unwrap();

        // JNI ThrowNew allows a null message pointer.
        let exception = vm.throw_new(&env, class, None).unwrap();
        assert_eq!(vm.pending_exception(&env), Some(exception));
        assert_eq!(
            vm.exception_description(exception).as_deref(),
            Some("java.lang.IllegalStateException")
        );
    }

    #[test]
    fn test_throw_roundtrip_and_thread_visibility() {
        let vm = Vm::new();
        let env = vm.attach_current_thread();
        let class = vm.find_or_define_class("java/lang/RuntimeException").unwrap();
        let thrown = vm
            .new_local_object(&env, class, ObjectValue::Throwable(Some("via Throw".into())))
            .unwrap();

        vm.throw(&env, thrown).unwrap();
        assert_eq!(vm.pending_exception(&env), Some(thrown));
        assert_eq!(
            vm.exception_description(thrown).as_deref(),
            Some("java.lang.RuntimeException: via Throw")
        );
        vm.clear_pending_exception(&env);

        // Pending exceptions are per-thread: another attached thread sees none.
        let vm_clone = std::sync::Arc::new(Vm::new());
        let vm2 = vm_clone.clone();
        let foreign = std::thread::spawn(move || {
            let env = vm2.attach_current_thread();
            let class = vm2.find_or_define_class("java/lang/RuntimeException").unwrap();
            let object = vm2
                .new_local_object(&env, class, ObjectValue::Throwable(None))
                .unwrap();
            assert_eq!(vm2.pending_exception(&env), None);
            object
        })
        .join()
        .unwrap();
        let main_env = vm_clone.attach_current_thread();

        // A foreign thread's local reference cannot be thrown from here.
        assert_eq!(
            vm_clone.throw(&main_env, foreign),
            Err(JniError::ReferenceNotLocal)
        );
        // Unknown references fail too.
        assert_eq!(
            vm_clone.throw(&main_env, ObjectId::from_raw(u64::MAX)),
            Err(JniError::UnknownReference)
        );
        // And a non-throwable object has no description.
        let opaque_class = vm_clone.find_or_define_class("com/test/Plain").unwrap();
        let opaque = vm_clone
            .new_local_object(&main_env, opaque_class, ObjectValue::Opaque)
            .unwrap();
        assert_eq!(vm_clone.exception_description(opaque), None);
    }

    #[test]
    fn test_throw_unknown_class_and_detach_clears_pending() {
        let vm = std::sync::Arc::new(Vm::new());
        let vm_clone = vm.clone();

        let exception = std::thread::spawn(move || {
            let env = vm_clone.attach_current_thread();
            let class = vm_clone.find_or_define_class("java/lang/Exception").unwrap();
            let thrown = vm_clone
                .throw_new(&env, class, Some("dies with thread"))
                .unwrap();
            assert_eq!(vm_clone.pending_exception(&env), Some(thrown));
            // Detaching with a pending exception clears the status.
            vm_clone.detach_current_thread().unwrap();
            assert_eq!(vm_clone.pending_exception(&env), None);
            thrown
        })
        .join()
        .unwrap();

        // The detached thread's pending status is gone and the throwable
        // object died with its local scope.
        let main_env = vm.attach_current_thread();
        assert!(vm.object_value(&main_env, exception).is_err());
        assert_eq!(vm.pending_exception(&main_env), None);

        // An unregistered class id cannot be thrown.
        assert!(vm
            .throw_new(&main_env, ClassId(u64::MAX), Some("x"))
            .is_err());
    }

    #[test]
    fn test_char_sequence_descriptor_compatibility() {
        let vm = Vm::new();
        let env = vm.attach_current_thread();
        let class = vm.find_or_define_class("android/media/AudioDeviceInfo").unwrap();

        // A hook registered under the String descriptor.
        let string_method = vm
            .register_method(class, "getProductName", "()Ljava/lang/String;", false)
            .unwrap();
        // The call the dex compiled against CharSequence must find it.
        assert_eq!(
            vm.find_method(class, "getProductName", "()Ljava/lang/CharSequence;", false),
            Ok(string_method)
        );
        // And the reverse direction for the parameter case (setText-style).
        let char_seq_setter = vm
            .register_method(
                class,
                "setText",
                "(Ljava/lang/CharSequence;)V",
                false,
            )
            .unwrap();
        assert_eq!(
            vm.find_method(class, "setText", "(Ljava/lang/String;)V", false),
            Ok(char_seq_setter)
        );

        // Compatibility is descriptor-shape specific: other mismatches still
        // miss, and the exact registered descriptor keeps its fast path.
        assert!(vm
            .find_method(class, "getProductName", "()Ljava/lang/Object;", false)
            .is_err());
        assert_eq!(
            vm.find_method(class, "getProductName", "()Ljava/lang/String;", false),
            Ok(string_method)
        );

        // resolve_method (the GetMethodID path) also flows through the
        // compatibility fallback instead of inventing a duplicate method.
        assert_eq!(
            vm.resolve_method(class, "getProductName", "()Ljava/lang/CharSequence;", false),
            Ok(string_method)
        );
    }

    #[test]
    fn test_method_and_field_metadata() {
        let vm = Vm::new();
        let class = vm.find_or_define_class("com/test/Sample").unwrap();
        let method = vm
            .register_method(class, "testMethod", "()V", false)
            .unwrap();
        let field = vm.register_field(class, "count", "I", false).unwrap();

        let info = vm.method_info(method).unwrap();
        assert_eq!(info.0, "com/test/Sample");
        assert_eq!(info.1, "testMethod");
        assert_eq!(info.2, "()V");
        assert!(!info.3);

        let finfo = vm.field_info(field).unwrap();
        assert_eq!(finfo.0, "com/test/Sample");
        assert_eq!(finfo.1, "count");
        assert_eq!(finfo.2, "I");
    }

    #[test]
    fn test_push_pop_local_frame() {
        let vm = Vm::new();
        let env = vm.attach_current_thread();
        let class = vm.find_or_define_class("java/lang/String").unwrap();

        vm.push_local_frame(&env, 16).unwrap();
        let obj1 = vm
            .new_local_object(&env, class, ObjectValue::Opaque)
            .unwrap();
        let obj2 = vm
            .new_local_object(&env, class, ObjectValue::Opaque)
            .unwrap();

        assert!(vm.object_value(&env, obj1).is_ok());
        assert!(vm.object_value(&env, obj2).is_ok());

        // Pop frame while preserving obj2
        let preserved = vm.pop_local_frame(&env, Some(obj2)).unwrap();
        assert_eq!(preserved, Some(obj2));

        // obj1 should have been dropped with the popped frame
        assert!(vm.object_value(&env, obj1).is_err());
        // obj2 was preserved and remains valid in parent frame
        assert!(vm.object_value(&env, obj2).is_ok());
    }

    #[test]
    fn test_detach_cleans_locals() {
        let vm = std::sync::Arc::new(Vm::new());
        let vm_clone = vm.clone();

        let (obj, class_id) = std::thread::spawn(move || {
            let env = vm_clone.attach_current_thread();
            let class = vm_clone.find_or_define_class("java/lang/Object").unwrap();
            let obj = vm_clone
                .new_local_object(&env, class, ObjectValue::Opaque)
                .unwrap();
            vm_clone.detach_current_thread().unwrap();
            (obj, class)
        })
        .join()
        .unwrap();

        // The detached thread's local object should be removed from objects map
        let main_env = vm.attach_current_thread();
        assert!(vm.object_value(&main_env, obj).is_err());
        assert_eq!(vm.class_name(class_id).as_deref(), Some("java/lang/Object"));
    }

    #[test]
    fn test_gc_collects_unreachable_and_orphaned_fields() {
        let vm = Vm::new();
        let env = vm.attach_current_thread();
        let class = vm.find_or_define_class("com/test/Data").unwrap();
        let field = vm.register_field(class, "tag", "I", false).unwrap();

        // Create a local object
        let local_obj = vm
            .new_local_object(&env, class, ObjectValue::Opaque)
            .unwrap();

        // Create a global object and set field value on it
        let global_obj = vm.new_global_ref(&env, local_obj).unwrap();
        vm.set_field_value(field, Some(global_obj), JniValue::Int(42))
            .unwrap();

        // Delete local reference
        vm.delete_local_ref(&env, local_obj).unwrap();

        // Run GC; global_obj is rooted, so field on it should be preserved
        let (stats, collected) = vm.gc();
        assert_eq!(stats.live_objects, 1);
        assert_eq!(collected.len(), 0);
        assert!(vm.field_value_is_set(field, Some(global_obj)));
        assert_eq!(
            vm.field_value(field, Some(global_obj)),
            Ok(JniValue::Int(42))
        );

        // Now delete global ref
        vm.delete_global_ref(global_obj).unwrap();

        // Run GC again; the orphaned field value should be swept
        let (stats, collected) = vm.gc();
        assert_eq!(stats.collected_fields, 1);
        assert_eq!(stats.live_objects, 0);
        assert!(collected.is_empty()); // already deleted via delete_global_ref
        assert!(!vm.field_value_is_set(field, Some(global_obj)));
    }
}
