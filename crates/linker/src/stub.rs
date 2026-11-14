#![allow(unsafe_code)]

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::Mutex;
use rustix::mm::{MapFlags, ProtFlags, mmap_anonymous};

macro_rules! eprintln {
    () => {{
        roblox_logging::emit(String::new());
    }};
    ($($arg:tt)*) => {{
        roblox_logging::emit(format!($($arg)*));
    }};
}

extern "C" fn generic_unimplemented_function(name_ptr: *const c_char) -> u64 {
    if !name_ptr.is_null() {
        let name = unsafe { CStr::from_ptr(name_ptr) }.to_string_lossy();
        eprintln!("[linker] unimplemented function call: {name}");
    } else {
        eprintln!("[linker] unimplemented function call: <unknown>");
    }
    0
}

struct TrampolinePool {
    current_page: *mut u8,
    offset: usize,
    capacity: usize,
}

unsafe impl Send for TrampolinePool {}
unsafe impl Sync for TrampolinePool {}

static POOL: Mutex<Option<TrampolinePool>> = Mutex::new(None);
static CREATED_STUBS: Mutex<Option<BTreeMap<String, usize>>> = Mutex::new(None);
static VARIABLE_PLACEHOLDERS: Mutex<Option<BTreeMap<String, usize>>> = Mutex::new(None);

pub fn get_or_create_function_stub(name: &str) -> usize {
    let mut stubs_guard = CREATED_STUBS.lock().unwrap();
    let stubs = stubs_guard.get_or_insert_with(BTreeMap::new);
    if let Some(&addr) = stubs.get(name) {
        return addr;
    }

    let c_name = CString::new(name).unwrap_or_default();
    let name_raw = Box::into_raw(c_name.into_boxed_c_str()) as *const c_char as usize;

    let handler_addr = generic_unimplemented_function as *const () as usize;

    // x86_64 trampoline:
    // 48 bf <8 bytes name_raw>   movabs $name_raw, %rdi
    // 48 b8 <8 bytes handler>    movabs $handler, %rax
    // ff e0                      jmp *%rax
    let mut code = [0u8; 22];
    code[0] = 0x48;
    code[1] = 0xbf;
    code[2..10].copy_from_slice(&name_raw.to_le_bytes());
    code[10] = 0x48;
    code[11] = 0xb8;
    code[12..20].copy_from_slice(&handler_addr.to_le_bytes());
    code[20] = 0xff;
    code[21] = 0xe0;

    let mut pool_guard = POOL.lock().unwrap();
    let pool = pool_guard.get_or_insert_with(|| {
        let page_size = 4096;
        let ptr = unsafe {
            mmap_anonymous(
                std::ptr::null_mut(),
                page_size,
                ProtFlags::READ | ProtFlags::WRITE | ProtFlags::EXEC,
                MapFlags::PRIVATE,
            )
        }
        .expect("failed to mmap executable page for linker trampolines");
        TrampolinePool {
            current_page: ptr.cast::<u8>(),
            offset: 0,
            capacity: page_size,
        }
    });

    if pool.offset + code.len() > pool.capacity {
        let page_size = 4096;
        let ptr = unsafe {
            mmap_anonymous(
                std::ptr::null_mut(),
                page_size,
                ProtFlags::READ | ProtFlags::WRITE | ProtFlags::EXEC,
                MapFlags::PRIVATE,
            )
        }
        .expect("failed to mmap executable page for linker trampolines");
        pool.current_page = ptr.cast::<u8>();
        pool.offset = 0;
        pool.capacity = page_size;
    }

    let stub_addr = unsafe { pool.current_page.add(pool.offset) } as usize;
    unsafe {
        std::ptr::copy_nonoverlapping(code.as_ptr(), stub_addr as *mut u8, code.len());
    }
    pool.offset += (code.len() + 15) & !15; // align to 16 bytes

    stubs.insert(name.to_owned(), stub_addr);
    stub_addr
}

pub fn get_or_create_variable_placeholder(name: &str) -> usize {
    let mut vars_guard = VARIABLE_PLACEHOLDERS.lock().unwrap();
    let vars = vars_guard.get_or_insert_with(BTreeMap::new);
    if let Some(&addr) = vars.get(name) {
        return addr;
    }

    // Allocate a zeroed 256-byte buffer for the variable so reads/writes don't crash
    let zeroed_box = vec![0u8; 256].into_boxed_slice();
    let addr = Box::into_raw(zeroed_box) as *mut u8 as usize;
    vars.insert(name.to_owned(), addr);
    addr
}
