//! Bindings to the AOSP bionic linker, retargeted to the host by
//! `mcpelauncher-linker` and wrapped in `native/shim.cpp`.
//!
//! This crate is deliberately thin: it exposes the linker's operations and
//! nothing else. Symbol-table policy — what Cordial provides for each Android
//! library — lives in `cordial-runtime`.

// The other ABI-edge crate: everything here is a `extern "C"` call across
// into `native/shim.cpp`, or a raw pointer handed back from it. See
// [ADR-036](../../../docs/adr/ADR-036-unsafe-is-a-boundary-not-a-convention.md).
#![allow(unsafe_code)]

// Every public function in `game_activity` and `jni` that takes a resolved
// native function pointer (`native`/`f`: `*mut c_void`) is `unsafe fn` with a
// `# Safety` doc, rather than wrapping the pointer in a handle newtype. Both
// were considered (see ADR-036's "A pre-existing clippy failure" section and
// the issue it links): the newtype was rejected because the pointer is not
// held here at all -- every call site in `cordial-runtime` resolves it once
// via `Library::symbol` and stores it as a raw `usize` in its own long-lived
// state (a bootstrap plan, a `OnceLock`, an `AtomicPtr`), re-casting it back
// to `*mut c_void` at each call. A newtype would have to live in that storage
// to buy anything, which means rewriting the zero-checks and struct fields
// those 84 call sites already use rather than just marking the call unsafe --
// a behaviour-risking change for a lint fix that is not supposed to change
// behaviour at all. Marking the function `unsafe` and documenting the
// contract puts the obligation where the pointer's real origin already is:
// the caller that resolved it.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    unsafe extern "C" {
        pub fn roblox_linker_init();
        pub fn roblox_linker_load_library(
            name: *const c_char,
            names: *const *const c_char,
            addrs: *const *mut c_void,
            n: usize,
        ) -> *mut c_void;
        pub fn roblox_linker_update_ld_library_path(path: *const c_char);
        pub fn roblox_linker_dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        // EXPERIMENTAL, cordial-agent-defer: see docs/analysis/flag-init.md
        // §26 and patches/README.md. Not called from the default load path.
        pub fn roblox_linker_defer_next_ctors(defer: c_int);
        pub fn roblox_linker_run_deferred_ctors(handle: *mut c_void);
        // docs/analysis/flag-init.md §31. Metadata only — see the comment on
        // the Rust wrapper below.
        pub fn roblox_linker_set_realpath(handle: *mut c_void, path: *const c_char);
        pub fn roblox_linker_dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        pub fn roblox_linker_dlerror() -> *const c_char;
        pub fn roblox_linker_get_library_base(handle: *mut c_void) -> usize;
        pub fn roblox_linker_get_library_code_region(
            handle: *mut c_void,
            base: *mut usize,
            size: *mut usize,
        );
    }
}

/// `RTLD_NOW` — resolve every relocation at load time. Cordial always uses this:
/// a lazy load would report success and then fail later on an unrelated call.
pub const RTLD_NOW: c_int = 2;
pub const RTLD_LAZY: c_int = 1;

/// A library loaded by, or registered with, the bionic linker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Library(*mut c_void);

// The linker keeps its own global state under its own lock; a handle is just an
// index into it. Sending one between threads is no less safe than using it.
// SAFETY: the linker owns global state under its lock; the handle is a stable
// index into that state and carries no thread-affine Rust allocation.
unsafe impl Send for Library {}

impl Library {
    pub fn as_ptr(self) -> *mut c_void {
        self.0
    }

    /// Base address the object was mapped at.
    pub fn base(self) -> usize {
        // SAFETY: only the linker creates `Library` handles, and it retains
        // them for the lifetime of the process.
        unsafe { ffi::roblox_linker_get_library_base(self.0) }
    }

    /// Address and length of the executable segment.
    pub fn code_region(self) -> (usize, usize) {
        let (mut base, mut size) = (0usize, 0usize);
        // SAFETY: `self` is a live linker handle and both output pointers refer
        // to writable locals for the duration of the call.
        unsafe { ffi::roblox_linker_get_library_code_region(self.0, &mut base, &mut size) };
        (base, size)
    }

    pub fn symbol(self, name: &str) -> Option<*mut c_void> {
        let c = CString::new(name).ok()?;
        // SAFETY: `self` is a live linker handle and `c` is NUL-terminated for
        // the duration of symbol lookup.
        let p = unsafe { ffi::roblox_linker_dlsym(self.0, c.as_ptr()) };
        (!p.is_null()).then_some(p)
    }
}

/// Initialise the linker's solist and register its built-in `libdl.so`.
///
/// Must be called once, before anything else in this module.
pub fn init() {
    // SAFETY: this is the linker's process-global initializer; callers invoke
    // it before registration or loading, as required by the native shim.
    unsafe { ffi::roblox_linker_init() }
}

/// Register a virtual library: an soname that exists only as the symbol table
/// given here. This is how Cordial provides `libc.so`, `libandroid.so`,
/// `libEGL.so` and the rest — the loaded object's `DT_NEEDED` entries resolve
/// against these instead of against anything on disk.
pub fn register(name: &str, symbols: &[(String, *mut c_void)]) -> Result<Library, Error> {
    // bionic's soinfo::set_soname() stores the pointer it is given rather than
    // copying the string (linker_soinfo.cpp: `soname_ = soname`). AOSP gets away
    // with it because callers pass string literals. A CString dropped at the end
    // of this function would leave every registered library with a dangling
    // soname, and DT_NEEDED lookups would then silently fail to match.
    //
    // So the name is leaked deliberately. There are a dozen of these for the
    // process lifetime.
    let cname: &'static CString = Box::leak(Box::new(CString::new(name)?));

    let cnames = symbols
        .iter()
        .map(|(s, _)| CString::new(s.as_str()))
        .collect::<Result<Vec<_>, _>>()?;
    let name_ptrs: Vec<*const c_char> = cnames.iter().map(|c| c.as_ptr()).collect();
    let addrs: Vec<*mut c_void> = symbols.iter().map(|(_, a)| *a).collect();

    // SAFETY: every pointer references a live CString or vector allocation
    // retained through the call; the linker copies symbol addresses and keeps
    // the leaked soname CString alive for its own stored pointer.
    let handle = unsafe {
        ffi::roblox_linker_load_library(
            cname.as_ptr(),
            name_ptrs.as_ptr(),
            addrs.as_ptr(),
            symbols.len(),
        )
    };
    if handle.is_null() {
        Err(Error::Linker(last_error()))
    } else {
        Ok(Library(handle))
    }
}

/// Directory the linker searches for real objects.
pub fn set_library_path(path: &str) -> Result<(), Error> {
    let c = CString::new(path)?;
    // SAFETY: `c` is NUL-terminated and alive for this synchronous update.
    unsafe { ffi::roblox_linker_update_ld_library_path(c.as_ptr()) };
    Ok(())
}

/// Load a real ELF object, resolving its imports against previously registered
/// libraries.
pub fn dlopen(soname: &str, flags: c_int) -> Result<Library, Error> {
    let c = CString::new(soname)?;
    // SAFETY: `c` is a live NUL-terminated soname and flags are passed through
    // to the native linker's documented dlopen interface.
    let handle = unsafe { ffi::roblox_linker_dlopen(c.as_ptr(), flags) };
    if handle.is_null() {
        Err(Error::Linker(last_error()))
    } else {
        Ok(Library(handle))
    }
}

/// EXPERIMENTAL, cordial-agent-defer: make the *next* [`dlopen`] call map and
/// relocate the object without running its ELF constructors. The caller must
/// follow up with [`run_deferred_ctors`] on the returned [`Library`] once it
/// wants them to run — nothing else runs them.
///
/// This exists to test whether `libroblox.so`'s constructors (which is where
/// `RbxStorage::init` lives — see `docs/analysis/flag-init.md` §26) can be
/// deferred past Cordial's own directory setup, which currently happens only
/// after `dlopen` returns. It is not wired into the default load path in
/// `cordial-run`; nothing calls this outside an explicit experiment.
pub fn defer_next_ctors(defer: bool) {
    // SAFETY: the native shim accepts a scalar process-global setting.
    unsafe { ffi::roblox_linker_defer_next_ctors(defer as c_int) }
}

/// EXPERIMENTAL, cordial-agent-defer: run whatever construction
/// [`defer_next_ctors`] left pending for `lib`. Idempotent — the underlying
/// `soinfo::call_constructors()` is itself guarded, so calling this on a
/// library that was never deferred (or already constructed) is harmless.
pub fn run_deferred_ctors(lib: Library) {
    // SAFETY: `lib` can only be created by this wrapper and remains loaded for
    // the process lifetime.
    unsafe { ffi::roblox_linker_run_deferred_ctors(lib.0) }
}

/// docs/analysis/flag-init.md §31: overrides what `dladdr()` reports as
/// `lib`'s own path (`Dl_info::dli_fname`), by writing the linker's internal
/// `soinfo::realpath_` directly. Nothing is reopened, remapped, or copied —
/// every byte the engine reads still comes from wherever it was actually
/// mapped from. Meant to be called after [`defer_next_ctors`] +
/// [`dlopen`] and before [`run_deferred_ctors`], so the override is visible
/// to any constructor-time code that asks the linker "what is my own path" —
/// which is the one form of self-location available before `JNI_OnLoad`,
/// since `RbxStorage::init`'s failing `stat("")` calls run during ELF
/// construction, strictly earlier.
pub fn set_realpath(lib: Library, path: &str) {
    let Ok(c) = CString::new(path) else { return };
    // SAFETY: `roblox_linker_set_realpath` copies the string; `c` need not
    // outlive the call.
    unsafe { ffi::roblox_linker_set_realpath(lib.0, c.as_ptr()) }
}

fn last_error() -> String {
    // SAFETY: the linker returns its own NUL-terminated error string, valid
    // until the next linker call on this thread; it is copied immediately.
    let p = unsafe { ffi::roblox_linker_dlerror() };
    if p.is_null() {
        "unknown linker error".into()
    } else {
        // SAFETY: non-null linker errors are NUL-terminated by the native API.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

#[derive(Debug)]
pub enum Error {
    Linker(String),
    NulByte(std::ffi::NulError),
}

impl From<std::ffi::NulError> for Error {
    fn from(e: std::ffi::NulError) -> Self {
        Error::NulByte(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Linker(s) => write!(f, "{s}"),
            Error::NulByte(e) => write!(f, "invalid name: {e}"),
        }
    }
}

impl std::error::Error for Error {}
