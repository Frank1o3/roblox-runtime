//! Experimental, Rust-native ELF mapping and relocation for the runtime.
//!
//! This is deliberately opt-in at the embedding layer. It maps one ELF image,
//! applies its base and symbol relocations, exposes its exports, and leaves ELF
//! constructors pending until the caller has prepared Android services.
//! Dependency loading, TLS installation, and Android `dlopen` interception are
//! not implemented yet; unsupported TLS/IFUNC images fail explicitly.

#![allow(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

mod abi;
pub mod elf;
#[allow(dead_code)]
mod map;
#[allow(dead_code)]
mod reloc;
mod resolve;
#[allow(dead_code)]
mod tls;

use abi::Abi;
use elf::{ElfImage, PF_X};
use map::{MappedObject, host_page_size};
use reloc::{RelocKind, SymbolResolver};
use resolve::{AddressProvider, ObjectProvider, Scope, ScopedResolver};

/// A Rust-mapped shared object. Its memory mapping remains live until drop.
pub struct MappedLibrary {
    _mapped: MappedObject,
    symbols: BTreeMap<String, usize>,
    base: usize,
    code_region: (usize, usize),
    constructors: Vec<usize>,
    constructors_run: bool,
}

impl fmt::Debug for MappedLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedLibrary")
            .field("base", &format_args!("{:#x}", self.base))
            .field("code_region", &self.code_region)
            .field("exports", &self.symbols.len())
            .field("constructors", &self.constructors.len())
            .finish()
    }
}

impl MappedLibrary {
    /// Map and relocate one Android shared object using the supplied external
    /// symbol addresses. The map is keyed by ELF symbol name and should contain
    /// the runtime's Android compatibility symbols as well as host symbols.
    pub fn load(
        path: impl AsRef<Path>,
        external_symbols: &BTreeMap<String, usize>,
    ) -> Result<Self, Error> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        let image = ElfImage::parse(&bytes).map_err(|e| Error::Elf(e.to_string()))?;
        if image.abi != Abi::HOST {
            return Err(Error::WrongAbi {
                object: image.abi.label(),
                host: Abi::HOST.label(),
            });
        }
        if image.tls.is_some() {
            return Err(Error::Unsupported("PT_TLS / static TLS"));
        }
        let relas = image.relocations().map_err(|e| Error::Elf(e.to_string()))?;
        if relas.iter().any(|r| r.kind == RelocKind::TpOff64) {
            return Err(Error::Unsupported("TPOFF64 relocations"));
        }
        if relas.iter().any(|r| r.kind == RelocKind::IRelative) {
            return Err(Error::Unsupported("IRELATIVE relocations"));
        }

        let (mut mapped, _) = MappedObject::map_and_relocate(&image, &bytes, host_page_size())
            .map_err(|e| Error::Map(e.to_string()))?;
        let base = usize::try_from(mapped.load_base()).map_err(|_| Error::AddressOverflow)?;

        let own = ObjectProvider::new(base as u64, &image.dynsyms);
        let mut scope = Scope::new();
        scope.push(Box::new(own));
        scope.push(Box::new(AddressProvider(
            external_symbols
                .iter()
                .map(|(name, &address)| (name.clone(), address as u64))
                .collect(),
        )));
        let resolver = ScopedResolver::new(&scope, &image.dynsyms);
        let mut unresolved = std::collections::BTreeSet::new();
        for reloc in relas.iter().filter(|r| r.kind.writes_symbol_address()) {
            if resolver.resolve_symbol(reloc.sym_index).is_none() {
                if let Some(symbol) = image.dynsyms.get(reloc.sym_index as usize) {
                    unresolved.insert(symbol.name.clone());
                }
            }
        }
        if !unresolved.is_empty() {
            return Err(Error::UnresolvedSymbols(unresolved.into_iter().collect()));
        }
        mapped
            .relocate_symbols(&image, &scope, host_page_size())
            .map_err(|e| Error::Map(e.to_string()))?;
        if let Some(relro) = image.relro {
            mapped
                .apply_relro(&relro, host_page_size())
                .map_err(|e| Error::Map(e.to_string()))?;
        }

        if !image.loads.iter().any(|seg| seg.flags & PF_X != 0) {
            return Err(Error::InvalidImage("ELF has no executable PT_LOAD segment"));
        }
        let mut code_start = usize::MAX;
        let mut code_end = 0usize;
        for seg in image.loads.iter().filter(|seg| seg.flags & PF_X != 0) {
            let start = base
                .checked_add(usize::try_from(seg.vaddr).map_err(|_| Error::AddressOverflow)?)
                .ok_or(Error::AddressOverflow)?;
            let len = usize::try_from(seg.mem_size).map_err(|_| Error::AddressOverflow)?;
            let end = start.checked_add(len).ok_or(Error::AddressOverflow)?;
            code_start = code_start.min(start);
            code_end = code_end.max(end);
        }

        let symbols = image
            .dynsyms
            .iter()
            .filter(|s| s.shndx != 0 && matches!(s.bind, 1 | 2) && !s.name.is_empty())
            .filter_map(|s| {
                let addr = if s.shndx == 0xfff1 {
                    usize::try_from(s.value).ok()?
                } else {
                    base.checked_add(usize::try_from(s.value).ok()?)?
                };
                Some((s.name.clone(), addr))
            })
            .collect();

        let mut constructors = Vec::new();
        if let Some(init) = image.dyn_info.init {
            constructors.push(
                base.checked_add(init as usize)
                    .ok_or(Error::AddressOverflow)?,
            );
        }
        if let Some((array, size)) = image.dyn_info.init_array {
            let count = usize::try_from(size / 8).map_err(|_| Error::AddressOverflow)?;
            if count > mapped.span() / 8 {
                return Err(Error::InvalidImage("DT_INIT_ARRAY exceeds mapped image"));
            }
            for i in 0..count {
                let offset = usize::try_from(array)
                    .ok()
                    .and_then(|v| v.checked_add(i * 8))
                    .ok_or(Error::AddressOverflow)?;
                let address = usize::try_from(
                    mapped
                        .read_u64(offset)
                        .map_err(|e| Error::Map(e.to_string()))?,
                )
                .map_err(|_| Error::AddressOverflow)?;
                if address != 0 && address != usize::MAX {
                    constructors.push(address);
                }
            }
        }

        Ok(Self {
            _mapped: mapped,
            symbols,
            base,
            code_region: (code_start, code_end.saturating_sub(code_start)),
            constructors,
            constructors_run: false,
        })
    }

    pub fn base(&self) -> usize {
        self.base
    }

    pub fn code_region(&self) -> (usize, usize) {
        self.code_region
    }

    pub fn symbol(&self, name: &str) -> Option<*mut std::ffi::c_void> {
        self.symbols
            .get(name)
            .copied()
            .map(|address| address as *mut std::ffi::c_void)
    }

    /// Execute `DT_INIT` and `DT_INIT_ARRAY` functions once. The caller must
    /// first prepare all Android compatibility services the constructors use.
    pub unsafe fn run_constructors(&mut self) {
        if self.constructors_run {
            return;
        }
        for &address in &self.constructors {
            // SAFETY: addresses came from this validated and relocated ELF's
            // constructor metadata, and the caller opted into executing them.
            let constructor: unsafe extern "C" fn() = unsafe { std::mem::transmute(address) };
            // SAFETY: see the constructor pointer invariant above.
            unsafe { constructor() };
        }
        self.constructors_run = true;
    }

    pub fn export_map(&self) -> &BTreeMap<String, usize> {
        &self.symbols
    }
}

/// Errors returned while mapping and relocating an image.
#[derive(Debug)]
pub enum Error {
    Io {
        path: String,
        source: std::io::Error,
    },
    Elf(String),
    Map(String),
    WrongAbi {
        object: &'static str,
        host: &'static str,
    },
    Unsupported(&'static str),
    UnresolvedSymbols(Vec<String>),
    InvalidImage(&'static str),
    AddressOverflow,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "reading {}: {source}", path),
            Self::Elf(e) => write!(f, "ELF parse failed: {e}"),
            Self::Map(e) => write!(f, "ELF map/relocation failed: {e}"),
            Self::WrongAbi { object, host } => {
                write!(f, "image ABI {object} does not match host ABI {host}")
            }
            Self::Unsupported(feature) => write!(f, "unsupported ELF feature: {feature}"),
            Self::UnresolvedSymbols(names) => {
                write!(f, "unresolved strong imports: {}", names.join(", "))
            }
            Self::InvalidImage(reason) => write!(f, "invalid ELF image: {reason}"),
            Self::AddressOverflow => f.write_str("ELF address arithmetic overflowed"),
        }
    }
}

impl std::error::Error for Error {}
