#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

macro_rules! eprintln {
    () => {{
        roblox_logging::emit(String::new());
    }};
    ($($arg:tt)*) => {{
        roblox_logging::emit(format!($($arg)*));
    }};
}

thread_local! {
    static LAST_DLERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_dlerror(msg: impl Into<String>) {
    let s = CString::new(msg.into()).unwrap_or_default();
    LAST_DLERROR.with(|cell| {
        *cell.borrow_mut() = Some(s);
    });
}

#[derive(Debug, Clone)]
pub struct MappedObjectInfo {
    pub soname: String,
    pub base: usize,
    pub span: usize,
    pub symbols: BTreeMap<String, usize>,
    pub c_soname: CString,
    pub phdr_vaddr: u64,
    pub phnum: u16,
    pub phentsize: u16,
}

#[derive(Debug, Clone)]
pub enum HandleKind {
    Global,
    Object(usize),
    Registered(String),
}

pub struct LinkerState {
    pub search_dirs: Vec<PathBuf>,
    pub registered_libraries: BTreeMap<String, BTreeMap<String, usize>>,
    pub mapped_objects: Vec<MappedObjectInfo>,
    pub global_symbols: BTreeMap<String, usize>,
    pub handles: BTreeMap<usize, HandleKind>,
    next_handle: usize,
}

impl LinkerState {
    fn new() -> Self {
        let mut handles = BTreeMap::new();
        // Handle 1 is RTLD_DEFAULT / Global
        handles.insert(1, HandleKind::Global);
        Self {
            search_dirs: Vec::new(),
            registered_libraries: BTreeMap::new(),
            mapped_objects: Vec::new(),
            global_symbols: BTreeMap::new(),
            handles,
            next_handle: 2,
        }
    }

    fn alloc_handle(&mut self, kind: HandleKind) -> usize {
        let id = self.next_handle;
        self.next_handle = self.next_handle.checked_add(1).unwrap_or(2);
        self.handles.insert(id, kind);
        id
    }
}

static STATE: OnceLock<Mutex<LinkerState>> = OnceLock::new();

fn state() -> &'static Mutex<LinkerState> {
    STATE.get_or_init(|| Mutex::new(LinkerState::new()))
}

pub fn set_search_dir(dir: impl Into<PathBuf>) {
    let mut guard = state().lock().unwrap();
    guard.search_dirs.push(dir.into());
}

pub fn register_library(name: &str, symbols: BTreeMap<String, usize>) {
    let mut guard = state().lock().unwrap();
    guard.registered_libraries.insert(name.to_owned(), symbols);
}

pub fn register_global_symbols(symbols: &BTreeMap<String, usize>) {
    let mut guard = state().lock().unwrap();
    guard.global_symbols.extend(symbols.clone());
}

pub fn register_mapped_object(
    soname: String,
    base: usize,
    span: usize,
    symbols: BTreeMap<String, usize>,
    phdr_vaddr: u64,
    phnum: u16,
) {
    let mut guard = state().lock().unwrap();
    let c_soname = CString::new(soname.as_str()).unwrap_or_default();
    guard.mapped_objects.push(MappedObjectInfo {
        soname,
        base,
        span,
        symbols,
        c_soname,
        phdr_vaddr,
        phnum,
        phentsize: 56,
    });
}

pub unsafe extern "C" fn rust_dlopen(filename: *const c_char, _flags: c_int) -> *mut c_void {
    if filename.is_null() {
        return 1 as *mut c_void; // Global handle
    }

    let raw_name = unsafe { CStr::from_ptr(filename) }.to_string_lossy();
    let name = raw_name.trim();
    if name.is_empty() {
        return 1 as *mut c_void;
    }

    let soname = Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());

    let mut guard = state().lock().unwrap();

    // Check mapped objects
    let matched_obj = guard
        .mapped_objects
        .iter()
        .position(|obj| obj.soname == name || obj.soname == soname);
    if let Some(idx) = matched_obj {
        let h = guard.alloc_handle(HandleKind::Object(idx));
        return h as *mut c_void;
    }

    // Check registered libraries
    let matched_lib = guard
        .registered_libraries
        .keys()
        .find(|k| *k == name || *k == &soname)
        .cloned();
    if let Some(lib_name) = matched_lib {
        let h = guard.alloc_handle(HandleKind::Registered(lib_name));
        return h as *mut c_void;
    }

    eprintln!("[linker] unimplemented library dlopen: {name}");
    set_dlerror(format!("cannot open shared object file: {name}"));
    std::ptr::null_mut()
}

pub unsafe extern "C" fn rust_dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    if symbol.is_null() {
        set_dlerror("null symbol name passed to dlsym");
        return std::ptr::null_mut();
    }

    let raw_name = unsafe { CStr::from_ptr(symbol) }.to_string_lossy();
    let sym_name = raw_name.as_ref();

    let guard = state().lock().unwrap();

    let h_val = handle as usize;
    let kind = if h_val == 0 || h_val == 1 || handle == (-1isize as *mut c_void) {
        HandleKind::Global
    } else if let Some(k) = guard.handles.get(&h_val) {
        k.clone()
    } else {
        HandleKind::Global
    };

    match kind {
        HandleKind::Object(idx) => {
            if let Some(obj) = guard.mapped_objects.get(idx) {
                if let Some(&addr) = obj.symbols.get(sym_name) {
                    return addr as *mut c_void;
                }
            }
        }
        HandleKind::Registered(ref lib_name) => {
            if let Some(lib_symbols) = guard.registered_libraries.get(lib_name) {
                if let Some(&addr) = lib_symbols.get(sym_name) {
                    return addr as *mut c_void;
                }
            }
        }
        HandleKind::Global => {}
    }

    // Global search across all mapped objects
    for obj in &guard.mapped_objects {
        if let Some(&addr) = obj.symbols.get(sym_name) {
            return addr as *mut c_void;
        }
    }

    // Search registered libraries
    for symbols in guard.registered_libraries.values() {
        if let Some(&addr) = symbols.get(sym_name) {
            return addr as *mut c_void;
        }
    }

    // Search global external symbols
    if let Some(&addr) = guard.global_symbols.get(sym_name) {
        return addr as *mut c_void;
    }

    // Search runtime libc
    if let Some(addr) = crate::resolve_runtime_libc(sym_name) {
        return addr as *mut c_void;
    }

    eprintln!("[linker] unimplemented symbol lookup via dlsym: {sym_name}");
    set_dlerror(format!("undefined symbol: {sym_name}"));
    std::ptr::null_mut()
}

pub unsafe extern "C" fn rust_dlclose(_handle: *mut c_void) -> c_int {
    0
}

pub unsafe extern "C" fn rust_dlerror() -> *mut c_char {
    LAST_DLERROR.with(|cell| {
        cell.borrow_mut()
            .take()
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    })
}

pub unsafe extern "C" fn rust_dladdr(addr: *mut c_void, info: *mut libc::Dl_info) -> c_int {
    if info.is_null() {
        return 0;
    }

    let target_addr = addr as usize;
    let guard = state().lock().unwrap();

    for obj in &guard.mapped_objects {
        if target_addr >= obj.base && target_addr < obj.base + obj.span {
            unsafe {
                (*info).dli_fname = obj.c_soname.as_ptr();
                (*info).dli_fbase = obj.base as *mut c_void;

                // Find nearest symbol
                let mut best_sym: Option<(&String, usize)> = None;
                for (name, &sym_addr) in &obj.symbols {
                    if sym_addr <= target_addr {
                        match best_sym {
                            None => best_sym = Some((name, sym_addr)),
                            Some((_, best_addr)) if sym_addr > best_addr => {
                                best_sym = Some((name, sym_addr));
                            }
                            _ => {}
                        }
                    }
                }

                if let Some((best_name, best_addr)) = best_sym {
                    let c_best_name = CString::new(best_name.as_str()).unwrap_or_default();
                    (*info).dli_sname = Box::into_raw(c_best_name.into_boxed_c_str()) as *const c_char;
                    (*info).dli_saddr = best_addr as *mut c_void;
                } else {
                    (*info).dli_sname = std::ptr::null();
                    (*info).dli_saddr = std::ptr::null_mut();
                }
            }
            return 1;
        }
    }

    0
}

pub unsafe extern "C" fn rust_dlvsym(
    handle: *mut c_void,
    symbol: *const c_char,
    _version: *const c_char,
) -> *mut c_void {
    unsafe { rust_dlsym(handle, symbol) }
}

pub unsafe extern "C" fn rust_dl_iterate_phdr(
    callback: Option<
        unsafe extern "C" fn(
            info: *mut libc::dl_phdr_info,
            size: usize,
            data: *mut c_void,
        ) -> c_int,
    >,
    data: *mut c_void,
) -> c_int {
    let cb = match callback {
        Some(c) => c,
        None => return 0,
    };

    let guard = state().lock().unwrap();
    for obj in &guard.mapped_objects {
        let phdr_ptr = (obj.base + obj.phdr_vaddr as usize) as *const libc::Elf64_Phdr;

        let mut info = libc::dl_phdr_info {
            dlpi_addr: obj.base as libc::Elf64_Addr,
            dlpi_name: obj.c_soname.as_ptr(),
            dlpi_phdr: phdr_ptr,
            dlpi_phnum: obj.phnum,
            dlpi_adds: 0,
            dlpi_subs: 0,
            dlpi_tls_modid: 0,
            dlpi_tls_data: std::ptr::null_mut(),
        };

        let ret = unsafe { cb(&mut info, std::mem::size_of::<libc::dl_phdr_info>(), data) };
        if ret != 0 {
            return ret;
        }
    }

    0
}

pub fn linker_symbols() -> BTreeMap<String, usize> {
    let mut symbols = BTreeMap::new();
    symbols.insert("dlopen".to_owned(), rust_dlopen as *const () as usize);
    symbols.insert("dlsym".to_owned(), rust_dlsym as *const () as usize);
    symbols.insert("dlclose".to_owned(), rust_dlclose as *const () as usize);
    symbols.insert("dlerror".to_owned(), rust_dlerror as *const () as usize);
    symbols.insert("dladdr".to_owned(), rust_dladdr as *const () as usize);
    symbols.insert("dlvsym".to_owned(), rust_dlvsym as *const () as usize);
    symbols.insert("dl_iterate_phdr".to_owned(), rust_dl_iterate_phdr as *const () as usize);
    symbols
}
