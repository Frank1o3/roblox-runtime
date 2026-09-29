//! Building blocks for a Rust implementation of the JNI VM surface used by
//! the Roblox runtime. This crate is intentionally independent from the current
//! C++ VM until its JNI behavior reaches runtime parity.

/// A type from a JVM method or field descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Boolean,
    Byte,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
    Object(String),
    Array(Box<Type>),
    Void,
}

/// A parsed JVM method descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodDescriptor {
    pub parameters: Vec<Type>,
    pub result: Type,
}

/// An invalid or incomplete JNI descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptorError {
    pub offset: usize,
    pub message: &'static str,
}

impl std::fmt::Display for DescriptorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for DescriptorError {}

/// Parse a JVM method descriptor such as `(Ljava/lang/String;I)Z`.
pub fn parse_method_descriptor(input: &str) -> Result<MethodDescriptor, DescriptorError> {
    let bytes = input.as_bytes();
    let mut cursor = 0;
    if bytes.get(cursor) != Some(&b'(') {
        return Err(error(cursor, "method descriptor must begin with '('"));
    }
    cursor += 1;
    let mut parameters = Vec::new();
    while bytes.get(cursor) != Some(&b')') {
        if cursor >= bytes.len() {
            return Err(error(cursor, "unterminated parameter list"));
        }
        let parameter = parse_type(bytes, &mut cursor, false)?;
        parameters.push(parameter);
    }
    cursor += 1;
    let result = parse_type(bytes, &mut cursor, true)?;
    if cursor != bytes.len() {
        return Err(error(cursor, "trailing bytes after method descriptor"));
    }
    Ok(MethodDescriptor { parameters, result })
}

/// Parse a JVM field descriptor. `void` is not a valid field type.
pub fn parse_field_descriptor(input: &str) -> Result<Type, DescriptorError> {
    let mut cursor = 0;
    let ty = parse_type(input.as_bytes(), &mut cursor, false)?;
    if cursor != input.len() {
        return Err(error(cursor, "trailing bytes after field descriptor"));
    }
    Ok(ty)
}

fn parse_type(input: &[u8], cursor: &mut usize, allow_void: bool) -> Result<Type, DescriptorError> {
    let start = *cursor;
    let Some(tag) = input.get(*cursor).copied() else {
        return Err(error(*cursor, "missing type descriptor"));
    };
    *cursor += 1;
    let ty = match tag {
        b'Z' => Type::Boolean,
        b'B' => Type::Byte,
        b'C' => Type::Char,
        b'S' => Type::Short,
        b'I' => Type::Int,
        b'J' => Type::Long,
        b'F' => Type::Float,
        b'D' => Type::Double,
        b'V' if allow_void => Type::Void,
        b'V' => return Err(error(start, "void is only valid as a method result")),
        b'[' => Type::Array(Box::new(parse_type(input, cursor, false)?)),
        b'L' => {
            let name_start = *cursor;
            let Some(relative_end) = input[name_start..].iter().position(|byte| *byte == b';')
            else {
                return Err(error(name_start, "unterminated object type"));
            };
            let end = name_start + relative_end;
            if end == name_start
                || input[name_start..end]
                    .iter()
                    .any(|byte| matches!(*byte, b'.' | b'[' | b'(' | b')'))
            {
                return Err(error(name_start, "invalid object type name"));
            }
            let name = std::str::from_utf8(&input[name_start..end])
                .map_err(|_| error(name_start, "object type name is not UTF-8"))?;
            *cursor = end + 1;
            Type::Object(name.to_owned())
        }
        _ => return Err(error(start, "unknown type descriptor")),
    };
    Ok(ty)
}

fn error(offset: usize, message: &'static str) -> DescriptorError {
    DescriptorError { offset, message }
}
