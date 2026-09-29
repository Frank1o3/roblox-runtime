//! Runtime state shared by the JNI invocation and native interfaces.
//!
//! This layer contains no JNI function table yet. It establishes stable class,
//! method, field, reference, and thread records for the ABI adapters to use.

use crate::{MethodDescriptor, Type, parse_field_descriptor, parse_method_descriptor};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Mutex, RwLock};
use std::thread::{self, ThreadId};

include!("vm/types.rs");

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
        self.state
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .locals
            .remove(&owner);
        Ok(())
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
        let id = MethodId(state.id());
        state.method_descriptors.insert(id, parsed);
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
        record
            .methods
            .get(&MethodKey {
                name: name.to_owned(),
                descriptor: descriptor.to_owned(),
                is_static,
            })
            .copied()
            .ok_or_else(|| {
                eprintln!(
                    "[jnivm] unimplemented method lookup: {}.{name}{descriptor}",
                    record.name
                );
                JniError::UnknownMethod(format!("{}.{name}{descriptor}", record.name))
            })
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
                .class_records
                .values()
                .find_map(|class| {
                    class.methods.iter().find_map(|(key, id)| {
                        (*id == method).then(|| {
                            format!(
                                "{}.{}`{}{}",
                                class.name,
                                if key.is_static { "static " } else { "" },
                                key.name,
                                key.descriptor
                            )
                        })
                    })
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
        let id = FieldId(state.id());
        state.field_types.insert(id, ty);
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
                global: false,
            },
        );
        state.locals.entry(env.owner).or_default().insert(id);
        Ok(id)
    }

    /// Promote a local reference to a VM-wide reference.
    pub fn new_global_ref(&self, env: &ThreadEnv, object: ObjectId) -> Result<ObjectId, JniError> {
        self.check_env(env)?;
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if !state
            .locals
            .get(&env.owner)
            .is_some_and(|locals| locals.contains(&object))
        {
            return Err(JniError::ReferenceNotLocal);
        }
        let record = state
            .objects
            .get(&object)
            .ok_or(JniError::UnknownReference)?;
        let (class, value) = (record.class, record.value.clone());
        let global = ObjectId(state.id());
        state.objects.insert(
            global,
            ObjectRecord {
                class,
                value,
                global: true,
            },
        );
        Ok(global)
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

    pub fn object_class_name(&self, object: u64) -> Option<String> {
        let state = self.state.read().unwrap_or_else(|p| p.into_inner());
        let record = state.objects.get(&ObjectId(object))?;
        state
            .class_records
            .get(&record.class)
            .map(|class| class.name.clone())
    }

    pub fn clone_local_ref(&self, env: &ThreadEnv, object: u64) -> Result<ObjectId, JniError> {
        let value = self.object_value(env, ObjectId(object))?;
        let class = {
            let state = self.state.read().unwrap_or_else(|p| p.into_inner());
            state
                .objects
                .get(&ObjectId(object))
                .ok_or(JniError::UnknownReference)?
                .class
        };
        self.new_local_object(env, class, value)
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

fn validate_member_name(name: &str) -> Result<(), JniError> {
    if name.is_empty() || name.chars().any(|ch| matches!(ch, '/' | '.' | ';' | '[')) {
        return Err(JniError::InvalidMemberName);
    }
    Ok(())
}
