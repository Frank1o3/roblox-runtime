#![forbid(unsafe_code)]

use std::fmt;

use crate::abi::Abi;

const R_X86_64_NONE: u16 = 0;

const R_X86_64_64: u16 = 1;

const R_X86_64_GLOB_DAT: u16 = 6;

const R_X86_64_JUMP_SLOT: u16 = 7;

const R_X86_64_RELATIVE: u16 = 8;

const R_X86_64_TPOFF64: u16 = 18;

const R_X86_64_IRELATIVE: u16 = 37;

const R_AARCH64_NONE: u16 = 0;

const R_AARCH64_ABS64: u16 = 257;

const R_AARCH64_GLOB_DAT: u16 = 1025;

const R_AARCH64_JUMP_SLOT: u16 = 1026;

const R_AARCH64_RELATIVE: u16 = 1027;

const R_AARCH64_IRELATIVE: u16 = 1032;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocKind {
    None,

    Relative,

    GlobDat,

    JumpSlot,

    Abs64,

    TpOff64,

    IRelative,

    Unsupported(u16),
}

impl RelocKind {
    pub fn decode(abi: Abi, raw: u32) -> Option<Self> {
        let raw = u16::try_from(raw).ok()?;
        Some(match abi {
            Abi::X86_64 => match raw {
                R_X86_64_NONE => Self::None,
                R_X86_64_RELATIVE => Self::Relative,
                R_X86_64_GLOB_DAT => Self::GlobDat,
                R_X86_64_JUMP_SLOT => Self::JumpSlot,
                R_X86_64_64 => Self::Abs64,
                R_X86_64_TPOFF64 => Self::TpOff64,
                R_X86_64_IRELATIVE => Self::IRelative,
                _ => Self::Unsupported(raw),
            },
            Abi::Arm64V8a => match raw {
                R_AARCH64_NONE => Self::None,
                R_AARCH64_RELATIVE => Self::Relative,
                R_AARCH64_GLOB_DAT => Self::GlobDat,
                R_AARCH64_JUMP_SLOT => Self::JumpSlot,
                R_AARCH64_ABS64 => Self::Abs64,
                R_AARCH64_IRELATIVE => Self::IRelative,
                _ => Self::Unsupported(raw),
            },
        })
    }

    pub fn writes_symbol_address(self) -> bool {
        matches!(self, Self::GlobDat | Self::JumpSlot | Self::Abs64)
    }

    #[cfg(test)]
    pub fn encode(self, abi: Abi) -> u32 {
        if let Self::Unsupported(raw) = self {
            return raw.into();
        }
        (0..=u32::from(u16::MAX))
            .find(|&raw| Self::decode(abi, raw) == Some(self))
            .unwrap_or_else(|| panic!("{} code has no {self} relocation", abi.label()))
    }
}

impl fmt::Display for RelocKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "NONE"),
            Self::Relative => write!(f, "RELATIVE"),
            Self::GlobDat => write!(f, "GLOB_DAT"),
            Self::JumpSlot => write!(f, "JUMP_SLOT"),
            Self::Abs64 => write!(f, "ABS64"),
            Self::TpOff64 => write!(f, "TPOFF64"),
            Self::IRelative => write!(f, "IRELATIVE"),
            Self::Unsupported(raw) => write!(f, "type {raw}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rela {
    pub offset: u64,

    pub sym_index: u32,

    pub kind: RelocKind,

    pub addend: i64,
}

pub trait SymbolResolver {
    fn resolve_symbol(&self, sym_index: u32) -> Option<u64>;

    fn resolve_tls_offset(&self, sym_index: u32) -> Option<u64>;
}

pub trait RelocImage {
    fn abi(&self) -> Abi;

    fn base(&self) -> u64;

    fn static_tls_offset(&self) -> i64;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn read_u64(&self, offset: usize) -> Result<u64, RelocError>;

    fn write_u64(&mut self, offset: usize, value: u64) -> Result<(), RelocError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocError {
    UnsupportedType { abi: Abi, kind: RelocKind },

    UnresolvedSymbol(u32),

    OutOfBounds(usize),

    RelrAddressInvalid(u64),
}

impl fmt::Display for RelocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedType { abi, kind } => {
                write!(f, "unsupported {} relocation {kind}", abi.label())
            }
            Self::UnresolvedSymbol(i) => {
                write!(f, "unresolved symbol at dynamic-symtab index {i}")
            }
            Self::OutOfBounds(off) => {
                write!(
                    f,
                    "relocation target offset {off} is outside the image bounds"
                )
            }
            Self::RelrAddressInvalid(addr) => {
                write!(
                    f,
                    "DT_RELR relocated address {addr:#x} is not a valid in-image offset"
                )
            }
        }
    }
}

impl std::error::Error for RelocError {}

pub struct SliceImage<'a> {
    abi: Abi,
    base: u64,
    static_tls_offset: i64,
    bytes: &'a mut [u8],
}

impl<'a> SliceImage<'a> {
    pub fn new(abi: Abi, base: u64, static_tls_offset: i64, bytes: &'a mut [u8]) -> Self {
        Self {
            abi,
            base,
            static_tls_offset,
            bytes,
        }
    }
}

impl RelocImage for SliceImage<'_> {
    fn abi(&self) -> Abi {
        self.abi
    }

    fn base(&self) -> u64 {
        self.base
    }

    fn static_tls_offset(&self) -> i64 {
        self.static_tls_offset
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn read_u64(&self, offset: usize) -> Result<u64, RelocError> {
        let end = offset
            .checked_add(8)
            .ok_or(RelocError::OutOfBounds(offset))?;
        let slot = self
            .bytes
            .get(offset..end)
            .ok_or(RelocError::OutOfBounds(offset))?;

        Ok(u64::from_le_bytes(slot.try_into().expect("8-byte slice")))
    }

    fn write_u64(&mut self, offset: usize, value: u64) -> Result<(), RelocError> {
        let end = offset
            .checked_add(8)
            .ok_or(RelocError::OutOfBounds(offset))?;
        let slot = self
            .bytes
            .get_mut(offset..end)
            .ok_or(RelocError::OutOfBounds(offset))?;
        slot.copy_from_slice(&value.to_le_bytes());
        Ok(())
    }
}

fn image_offset_of(image: &dyn RelocImage, addr: u64) -> Result<usize, RelocError> {
    let rel = addr
        .checked_sub(image.base())
        .ok_or(RelocError::RelrAddressInvalid(addr))?;
    usize::try_from(rel).map_err(|_| RelocError::RelrAddressInvalid(addr))
}

pub fn apply_one(
    image: &mut dyn RelocImage,
    resolver: &dyn SymbolResolver,
    rela: &Rela,
) -> Result<(), RelocError> {
    let base = image.base();
    let target_off = image_offset_of(image, base.wrapping_add(rela.offset))?;

    let value = match rela.kind {
        RelocKind::Relative => base.wrapping_add(rela.addend as u64),
        RelocKind::GlobDat | RelocKind::JumpSlot => {
            let sym = resolver
                .resolve_symbol(rela.sym_index)
                .ok_or(RelocError::UnresolvedSymbol(rela.sym_index))?;
            match image.abi() {
                Abi::X86_64 => sym,
                Abi::Arm64V8a => sym.wrapping_add(rela.addend as u64),
            }
        }
        RelocKind::Abs64 => {
            let sym = resolver
                .resolve_symbol(rela.sym_index)
                .ok_or(RelocError::UnresolvedSymbol(rela.sym_index))?;
            sym.wrapping_add(rela.addend as u64)
        }
        RelocKind::TpOff64 => {
            let sym_tls = resolver
                .resolve_tls_offset(rela.sym_index)
                .ok_or(RelocError::UnresolvedSymbol(rela.sym_index))?;

            (image.static_tls_offset() as u64)
                .wrapping_add(sym_tls)
                .wrapping_add(rela.addend as u64)
        }
        kind @ (RelocKind::None | RelocKind::IRelative | RelocKind::Unsupported(_)) => {
            return Err(RelocError::UnsupportedType {
                abi: image.abi(),
                kind,
            });
        }
    };

    image.write_u64(target_off, value)
}

pub fn apply_rela(
    image: &mut dyn RelocImage,
    resolver: &dyn SymbolResolver,
    relas: &[Rela],
) -> Result<(), RelocError> {
    for rela in relas {
        apply_one(image, resolver, rela)?;
    }
    Ok(())
}

pub fn apply_relr(image: &mut dyn RelocImage, entries: &[u64]) -> Result<(), RelocError> {
    let base = image.base();

    let mut cursor: u64 = 0;

    for &entry in entries {
        if entry & 1 == 0 {
            relr_relocate_addr(image, base, entry)?;
            cursor = entry.wrapping_add(8);
        } else {
            let mut bits = entry >> 1;
            let mut addr = cursor;
            while bits != 0 {
                if bits & 1 != 0 {
                    relr_relocate_addr(image, base, addr)?;
                }
                bits >>= 1;
                addr = addr.wrapping_add(8);
            }

            cursor = cursor.wrapping_add(63 * 8);
        }
    }
    Ok(())
}

fn relr_relocate_addr(image: &mut dyn RelocImage, base: u64, addr: u64) -> Result<(), RelocError> {
    let off = image_offset_of(image, addr)?;
    let current = image.read_u64(off)?;
    image.write_u64(off, current.wrapping_add(base))
}
