// JNI identifiers, state records, errors, and dispatch value types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClassId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MethodId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FieldId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjectId(pub(crate) u64);

impl ObjectId {
    /// The stable opaque JNI handle value used by this VM's function table.
    pub fn raw(self) -> u64 {
        self.0
    }

    /// Reconstruct a handle previously returned by this VM's JNI function
    /// table.
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// An attached thread's environment token. The eventual JNI ABI adapter maps
/// this token to the stable `JNIEnv*` table for that thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadEnv {
    owner: ThreadId,
}

impl ThreadEnv {
    pub fn owner(&self) -> ThreadId {
        self.owner
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JniError {
    InvalidClassName,
    InvalidMemberName,
    InvalidDescriptor(String),
    DuplicateClass(String),
    UnknownClass(String),
    UnknownMethod(String),
    UnknownField(String),
    NotAttached,
    WrongThread,
    UnknownReference,
    ReferenceNotLocal,
}

impl fmt::Display for JniError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidClassName => f.write_str("invalid JNI class name"),
            Self::InvalidMemberName => f.write_str("invalid JNI member name"),
            Self::InvalidDescriptor(error) => write!(f, "invalid JNI descriptor: {error}"),
            Self::DuplicateClass(name) => write!(f, "JNI class is already registered: {name}"),
            Self::UnknownClass(name) => write!(f, "JNI class is not registered: {name}"),
            Self::UnknownMethod(name) => write!(f, "JNI method is not registered: {name}"),
            Self::UnknownField(name) => write!(f, "JNI field is not registered: {name}"),
            Self::NotAttached => f.write_str("current thread is not attached to the JNI VM"),
            Self::WrongThread => f.write_str("JNI environment belongs to a different thread"),
            Self::UnknownReference => f.write_str("unknown JNI object reference"),
            Self::ReferenceNotLocal => f.write_str("JNI reference is not local to this thread"),
        }
    }
}

impl std::error::Error for JniError {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MethodKey {
    name: String,
    descriptor: String,
    is_static: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FieldKey {
    name: String,
    descriptor: String,
    is_static: bool,
}

#[derive(Debug)]
struct ClassRecord {
    name: String,
    methods: HashMap<MethodKey, MethodId>,
    fields: HashMap<FieldKey, FieldId>,
    natives: HashMap<(String, String), usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectValue {
    /// Java strings are represented as Rust UTF-16 units, matching JNI's
    /// `jchar` storage and preserving unpaired surrogates.
    String(Vec<u16>),
    ByteArray(Vec<i8>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
    ObjectArray(Vec<Option<ObjectId>>),
    /// A `java/nio/ByteBuffer` wrapper over native memory, matching
    /// libjnivm's `ByteBuffer { void* buffer; jlong capacity; }`.
    DirectByteBuffer { address: usize, capacity: i64 },
    /// A global object reference owned by the companion C++ libjnivm VM.
    /// This is only used for values returned from the experimental fallback
    /// bridge; Rust code must not dereference the pointer directly.
    CppObject(usize),
    Opaque,
}

/// JNI values used by the Rust-side method dispatcher.
#[derive(Clone, Debug, PartialEq)]
pub enum JniValue {
    Boolean(bool),
    Byte(i8),
    Char(u16),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Object(Option<ObjectId>),
    Void,
}

impl JniValue {
    pub(crate) fn default_for(ty: &Type) -> Self {
        match ty {
            Type::Boolean => Self::Boolean(false),
            Type::Byte => Self::Byte(0),
            Type::Char => Self::Char(0),
            Type::Short => Self::Short(0),
            Type::Int => Self::Int(0),
            Type::Long => Self::Long(0),
            Type::Float => Self::Float(0.0),
            Type::Double => Self::Double(0.0),
            Type::Object(_) | Type::Array(_) => Self::Object(None),
            Type::Void => Self::Void,
        }
    }
}

/// Safe Rust implementation of a Java method. The JNI adapter validates and
/// converts raw `jvalue`s before entering this function.
pub type MethodHandler = fn(&Vm, Option<ObjectId>, &[JniValue]) -> JniValue;

#[derive(Clone, Debug)]
pub struct MethodMeta {
    #[allow(dead_code)]
    pub class_id: ClassId,
    pub class_name: String,
    pub name: String,
    pub descriptor: String,
    pub is_static: bool,
}

#[derive(Clone, Debug)]
pub struct FieldMeta {
    #[allow(dead_code)]
    pub class_id: ClassId,
    pub class_name: String,
    pub name: String,
    pub descriptor: String,
    pub is_static: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GcStats {
    pub collected_objects: usize,
    pub collected_fields: usize,
    pub live_objects: usize,
}

#[derive(Debug)]
struct ObjectRecord {
    class: ClassId,
    value: ObjectValue,
    identity: u64,
    global: bool,
}

#[derive(Default)]
struct State {
    next_id: u64,
    classes: HashMap<String, ClassId>,
    class_records: HashMap<ClassId, ClassRecord>,
    method_descriptors: HashMap<MethodId, MethodDescriptor>,
    method_handlers: HashMap<MethodId, MethodHandler>,
    field_types: HashMap<FieldId, Type>,
    field_values: HashMap<(FieldId, Option<ObjectId>), JniValue>,
    objects: HashMap<ObjectId, ObjectRecord>,
    locals: HashMap<ThreadId, HashSet<ObjectId>>,
    local_frames: HashMap<ThreadId, Vec<HashSet<ObjectId>>>,
    method_metadata: HashMap<MethodId, MethodMeta>,
    field_metadata: HashMap<FieldId, FieldMeta>,
}

impl State {
    fn id(&mut self) -> u64 {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.next_id
    }
}

/// Process-wide JNI VM model. JNI environments are tracked per native thread;
/// classes and member IDs are stable for the VM lifetime.
#[derive(Default)]
pub struct Vm {
    attached: Mutex<HashSet<ThreadId>>,
    state: RwLock<State>,
}
