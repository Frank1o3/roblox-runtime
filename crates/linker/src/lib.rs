//! Experimental, Rust-native ELF mapping and relocation for the runtime.
//!
//! The loader maps the requested object and any `DT_NEEDED` objects found
//! beside it, applies base and symbol relocations, and defers constructors
//! until the embedding runtime has prepared Android services.

#![allow(unsafe_code)]

macro_rules! eprintln {
    () => {{
        roblox_logging::emit(String::new());
    }};
    ($($arg:tt)*) => {{
        roblox_logging::emit(format!($($arg)*));
    }};
}

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};

mod abi;
pub mod dl;
pub mod elf;
#[allow(dead_code)]
mod map;
#[allow(dead_code)]
mod reloc;
mod resolve;
pub mod stub;
#[allow(dead_code)]
mod tls;

pub use dl::{linker_symbols, register_library, set_search_dir};

use std::ffi::CString;

pub fn resolve_runtime_libc(name: &str) -> Option<usize> {
    let c_name = CString::new(name).ok()?;
    let addr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c_name.as_ptr()) };
    if !addr.is_null() {
        Some(addr as usize)
    } else {
        None
    }
}

use abi::Abi;
use elf::{DynSym, ElfImage, PF_X};
use map::{MappedObject, host_page_size};
use reloc::SymbolResolver;
use resolve::{AddressProvider, ObjectProvider, Scope, ScopedResolver};

struct LoadedObject {
    soname: String,
    bytes: Vec<u8>,
    mapped: MappedObject,
    base: usize,
    code_region: (usize, usize),
    symbols: BTreeMap<String, usize>,
    constructors: Vec<usize>,
}

/// A Rust-mapped image set. All mappings remain live until this value drops.
pub struct MappedLibrary {
    objects: Vec<LoadedObject>,
    root: usize,
    constructors_run: bool,
}

impl fmt::Debug for MappedLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedLibrary")
            .field("root", &self.objects[self.root].soname)
            .field("objects", &self.objects.len())
            .field("base", &format_args!("{:#x}", self.base()))
            .field("exports", &self.objects[self.root].symbols.len())
            .finish()
    }
}

impl MappedLibrary {
    /// Map the root Android shared object and available `DT_NEEDED` files in
    /// its directory. Runtime-provided symbols are resolved from `external`.
    /// Missing files are tolerated only if their imports can be resolved from
    /// the runtime symbol table.
    pub fn load(
        path: impl AsRef<Path>,
        external_symbols: &BTreeMap<String, usize>,
    ) -> Result<Self, Error> {
        let root_path = path.as_ref().to_path_buf();
        let search_dir = root_path.parent().unwrap_or_else(|| Path::new("."));
        dl::set_search_dir(search_dir);
        dl::register_global_symbols(external_symbols);
        let mut queue = VecDeque::from([root_path.clone()]);
        let mut queued = BTreeSet::from([root_path.clone()]);
        let mut loaded_sonames = BTreeSet::new();
        let mut objects = Vec::new();

        while let Some(object_path) = queue.pop_front() {
            let bytes = std::fs::read(&object_path).map_err(|source| Error::Io {
                path: object_path.display().to_string(),
                source,
            })?;
            let image = ElfImage::parse(&bytes).map_err(|e| Error::Elf(e.to_string()))?;
            if image.abi != Abi::HOST {
                return Err(Error::WrongAbi {
                    object: image.abi.label(),
                    host: Abi::HOST.label(),
                });
            }
            ensure_supported(&image)?;
            let soname = image
                .soname()
                .map_err(|e| Error::Elf(e.to_string()))?
                .or_else(|| {
                    object_path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| object_path.display().to_string());
            if !loaded_sonames.insert(soname.clone()) {
                continue;
            }
            let needed = image.needed().map_err(|e| Error::Elf(e.to_string()))?;
            let (mapped, _) = MappedObject::map_and_relocate(&image, &bytes, host_page_size())
                .map_err(|e| Error::Map(format!("{soname}: {e}")))?;
            let base = usize::try_from(mapped.load_base()).map_err(|_| Error::AddressOverflow)?;
            let code_region = executable_region(base, &image)?;
            objects.push(LoadedObject {
                soname,
                bytes,
                mapped,
                base,
                code_region,
                symbols: BTreeMap::new(),
                constructors: Vec::new(),
            });

            for dependency in needed {
                let candidate = if dependency.contains('/') {
                    PathBuf::from(&dependency)
                } else {
                    search_dir.join(&dependency)
                };
                if candidate.is_file() && queued.insert(candidate.clone()) {
                    queue.push_back(candidate);
                }
            }
        }

        if objects.is_empty() {
            return Err(Error::InvalidImage("no ELF objects were mapped"));
        }
        // The requested root is always queued before its dependencies.
        let root = 0;

        let mut tls_layout = tls::TlsLayout::new();
        let mut tls_modules = Vec::new();
        for (idx, object) in objects.iter().enumerate() {
            let image = ElfImage::parse(&object.bytes).map_err(|e| Error::Elf(e.to_string()))?;
            if let Some(ref tls) = image.tls {
                let module = tls_layout
                    .add_module(tls, &object.bytes, tls.offset, &image.dynsyms)
                    .map_err(|_e| Error::InvalidImage("invalid TLS layout"))?;
                tls_modules.push((idx, module.tp_offset));
            }
        }

        let mut scope = Scope::new();
        for object in &objects {
            let image = ElfImage::parse(&object.bytes).map_err(|e| Error::Elf(e.to_string()))?;
            scope.push(Box::new(ObjectProvider::new(
                object.base as u64,
                &image.dynsyms,
            )));
        }

        let mut all_external = external_symbols.clone();
        for (name, addr) in dl::linker_symbols() {
            all_external.entry(name).or_insert(addr);
        }

        scope.push(Box::new(AddressProvider(
            all_external
                .iter()
                .map(|(name, &address)| (name.clone(), address as u64))
                .collect(),
        )));

        // Detect any unresolvable symbols, log them by type, and create placeholders
        let mut placeholders: BTreeMap<String, u64> = BTreeMap::new();
        for object in &objects {
            let image = ElfImage::parse(&object.bytes).map_err(|e| Error::Elf(e.to_string()))?;
            let relas = image.relocations().map_err(|e| Error::Elf(e.to_string()))?;
            let resolver = ScopedResolver::new(&scope, &image.dynsyms);
            for relocation in relas.iter().filter(|r| r.kind.writes_symbol_address()) {
                if resolver.resolve_symbol(relocation.sym_index).is_none() {
                    if let Some(symbol) = image.dynsyms.get(relocation.sym_index as usize) {
                        if !symbol.name.is_empty() && !placeholders.contains_key(&symbol.name) {
                            if let Some(libc_addr) = resolve_runtime_libc(&symbol.name) {
                                placeholders.insert(symbol.name.clone(), libc_addr as u64);
                                continue;
                            }
                            let kind_str = match symbol.sym_type {
                                1 => "variable",
                                2 | 10 => "function",
                                6 => "thread-local variable",
                                _ => "symbol",
                            };
                            eprintln!(
                                "[linker] unimplemented {kind_str}: {} (object: {}); creating placeholder",
                                symbol.name, object.soname
                            );
                            let placeholder_addr = match symbol.sym_type {
                                1 => stub::get_or_create_variable_placeholder(&symbol.name),
                                _ => stub::get_or_create_function_stub(&symbol.name),
                            };
                            placeholders.insert(symbol.name.clone(), placeholder_addr as u64);
                        }
                    }
                }
            }
        }
        if !placeholders.is_empty() {
            scope.push(Box::new(AddressProvider(placeholders)));
        }

        for (idx, object) in objects.iter_mut().enumerate() {
            let image = ElfImage::parse(&object.bytes).map_err(|e| Error::Elf(e.to_string()))?;
            let scoped_resolver = ScopedResolver::new(&scope, &image.dynsyms);
            let own_tp = tls_modules
                .iter()
                .find(|(i, _)| *i == idx)
                .map(|(_, tp)| *tp);
            let resolver = tls::TlsResolver::new(&scoped_resolver, &image.dynsyms, &tls_layout, own_tp);
            object
                .mapped
                .relocate_symbols(&image, &resolver, host_page_size())
                .map_err(|e| Error::Map(format!("{}: {e}", object.soname)))?;
        }

        for object in &mut objects {
            let image = ElfImage::parse(&object.bytes).map_err(|e| Error::Elf(e.to_string()))?;
            if let Some(relro) = image.relro {
                object
                    .mapped
                    .apply_relro(&relro, host_page_size())
                    .map_err(|e| Error::Map(format!("{}: {e}", object.soname)))?;
            }
            object.symbols = exported_symbols(object.base, &image.dynsyms);
            object.constructors = constructor_addresses(&image, &object.mapped, object.base)?;

            dl::register_mapped_object(
                object.soname.clone(),
                object.base,
                object.mapped.span(),
                object.symbols.clone(),
                image.phdr_vaddr,
                image.phnum,
            );
        }

        Ok(Self {
            objects,
            root,
            constructors_run: false,
        })
    }

    pub fn base(&self) -> usize {
        self.objects[self.root].base
    }

    pub fn code_region(&self) -> (usize, usize) {
        self.objects[self.root].code_region
    }

    pub fn symbol(&self, name: &str) -> Option<*mut std::ffi::c_void> {
        self.objects[self.root]
            .symbols
            .get(name)
            .copied()
            .map(|address| address as *mut std::ffi::c_void)
    }

    /// Execute dependency constructors and then the root object's constructors
    /// once. The caller must first prepare the Android services they use.
    pub unsafe fn run_constructors(&mut self) {
        if self.constructors_run {
            return;
        }
        for object in self.objects.iter().rev() {
            for &address in &object.constructors {
                // SAFETY: each address came from a relocated DT_INIT or
                // DT_INIT_ARRAY in this mapped object; caller opted in.
                let constructor: unsafe extern "C" fn() = unsafe { std::mem::transmute(address) };
                // SAFETY: guaranteed by the constructor metadata invariant.
                unsafe { constructor() };
            }
        }
        self.constructors_run = true;
    }

    pub fn export_map(&self) -> &BTreeMap<String, usize> {
        &self.objects[self.root].symbols
    }
}

fn ensure_supported(_image: &ElfImage<'_>) -> Result<(), Error> {
    Ok(())
}

fn executable_region(base: usize, image: &ElfImage<'_>) -> Result<(usize, usize), Error> {
    let mut start = usize::MAX;
    let mut end = 0usize;
    for segment in image
        .loads
        .iter()
        .filter(|segment| segment.flags & PF_X != 0)
    {
        let segment_start = base
            .checked_add(usize::try_from(segment.vaddr).map_err(|_| Error::AddressOverflow)?)
            .ok_or(Error::AddressOverflow)?;
        let segment_end = segment_start
            .checked_add(usize::try_from(segment.mem_size).map_err(|_| Error::AddressOverflow)?)
            .ok_or(Error::AddressOverflow)?;
        start = start.min(segment_start);
        end = end.max(segment_end);
    }
    if start == usize::MAX {
        return Err(Error::InvalidImage("ELF has no executable PT_LOAD segment"));
    }
    Ok((start, end - start))
}

fn exported_symbols(base: usize, symbols: &[DynSym]) -> BTreeMap<String, usize> {
    symbols
        .iter()
        .filter(|sym| sym.shndx != 0 && matches!(sym.bind, 1 | 2) && !sym.name.is_empty())
        .filter_map(|sym| {
            let address = if sym.shndx == 0xfff1 {
                usize::try_from(sym.value).ok()?
            } else {
                base.checked_add(usize::try_from(sym.value).ok()?)?
            };
            Some((sym.name.clone(), address))
        })
        .collect()
}

fn constructor_addresses(
    image: &ElfImage<'_>,
    mapped: &MappedObject,
    base: usize,
) -> Result<Vec<usize>, Error> {
    let mut constructors = Vec::new();
    if let Some(init) = image.dyn_info.init {
        constructors.push(
            base.checked_add(init as usize)
                .ok_or(Error::AddressOverflow)?,
        );
    }
    if let Some((array, size)) = image.dyn_info.init_array {
        if size % 8 != 0 {
            return Err(Error::InvalidImage(
                "DT_INIT_ARRAY size is not pointer aligned",
            ));
        }
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
    Ok(constructors)
}

/// Errors returned while mapping and relocating an image set.
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
    UnresolvedSymbols {
        object: String,
        names: Vec<String>,
    },
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
            Self::UnresolvedSymbols { object, names } => write!(
                f,
                "{object} has unresolved strong imports: {}",
                names.join(", ")
            ),
            Self::InvalidImage(reason) => write!(f, "invalid ELF image: {reason}"),
            Self::AddressOverflow => f.write_str("ELF address arithmetic overflowed"),
        }
    }
}

impl std::error::Error for Error {}
