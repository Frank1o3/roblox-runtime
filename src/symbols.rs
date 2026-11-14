//! Build the Android library symbol tables from ABI implementations, host
//! graphics/math libraries and the generated compatibility stubs.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::c_void;

use roblox_linker_sys::elf::Binding;

const ANDROID_PREFIXES: &[(&str, &str)] = &[
    ("AMedia", "libmediandk.so"),
    ("AMEDIA", "libmediandk.so"),
    ("AImage", "libmediandk.so"),
    ("AIMAGE", "libmediandk.so"),
    ("AndroidBitmap", "libjnigraphics.so"),
    ("__android_log", "liblog.so"),
    ("android_set_abort_message", "liblog.so"),
    ("android_get_device_api_level", "liblog.so"),
    ("ANative", "libandroid.so"),
    ("AAsset", "libandroid.so"),
    ("AInput", "libandroid.so"),
    ("AKey", "libandroid.so"),
    ("AMotion", "libandroid.so"),
    ("ALooper", "libandroid.so"),
    ("ASensor", "libandroid.so"),
    ("AChoreographer", "libandroid.so"),
    ("AConfiguration", "libandroid.so"),
    ("ATrace", "libandroid.so"),
    ("AHardwareBuffer", "libandroid.so"),
    ("ASharedMemory", "libandroid.so"),
    ("AStorageManager", "libandroid.so"),
    ("APerformanceHint", "libandroid.so"),
    ("AObb", "libandroid.so"),
    ("ASurface", "libandroid.so"),
    ("AFont", "libandroid.so"),
    ("ASystemFont", "libandroid.so"),
    ("SL_IID_", "libOpenSLES.so"),
    ("slCreateEngine", "libOpenSLES.so"),
];

const EMPTY_LIBRARIES: &[&str] = &[
    "libOpenMAXAL.so",
    "libmediandk.so",
    "libjnigraphics.so",
    "libandroid.so",
    "liblog.so",
    "libOpenSLES.so",
    "libm.so",
    "libGLESv2.so",
    "libEGL.so",
    "libc.so",
];

#[derive(Debug)]
pub(crate) struct MissingSymbol {
    pub name: String,
}

#[derive(Default)]
pub(crate) struct SymbolTables {
    pub libraries: BTreeMap<&'static str, Vec<(String, *mut c_void)>>,
    pub missing: Vec<MissingSymbol>,
}

fn library_for(name: &str) -> &'static str {
    if name.starts_with("egl") && name.chars().nth(3).is_some_and(char::is_uppercase) {
        return "libEGL.so";
    }
    if name.starts_with("gl") && name.chars().nth(2).is_some_and(char::is_uppercase) {
        return "libGLESv2.so";
    }
    ANDROID_PREFIXES
        .iter()
        .find_map(|(prefix, library)| name.starts_with(prefix).then_some(*library))
        .unwrap_or("libc.so")
}

fn host_candidates(library: &str) -> &'static [(&'static str, &'static str)] {
    match library {
        "libEGL.so" => &[("libEGL.so.1", "libEGL.so")],
        "libGLESv2.so" => &[("libGLESv2.so.2", "libGLESv2.so")],
        "libm.so" => &[("libm.so.6", "libm.so")],
        "libz.so" => &[("libz.so.1", "libz.so")],
        "libc.so" => &[
            ("libm.so.6", "libm.so"),
            ("libz.so.1", "libz.so"),
            ("libstdc++.so.6", "libc.so"),
            ("libgcc_s.so.1", "libc.so"),
        ],
        _ => &[],
    }
}

fn runtime_libc_symbol(symbol: &str) -> Option<*mut c_void> {
    roblox_linker_sys::runtime_symbol(symbol)
}

fn host_address(library: &str, symbol: &str) -> Option<(&'static str, *mut c_void)> {
    let registered = host_candidates(library)
        .iter()
        .find_map(|(candidate, android_library)| {
            roblox_linker_sys::host_symbol(candidate, symbol)
                .map(|address| (*android_library, address))
        });
    if registered.is_some() {
        return registered;
    }

    // These symbols have matching ABIs on glibc and bionic. Keep this list
    // explicit: resolving every libc import from glibc would silently cross
    // incompatible structures such as bionic's `stat` and `sigaction`.
    const HOST_LIBC_COMPATIBLE: &[&str] = &[
        // String and memory
        "__memcpy_chk",
        "__memmove_chk",
        "__memset_chk",
        "__strcat_chk",
        "__strcpy_chk",
        "memchr",
        "memcmp",
        "memcpy",
        "memmove",
        "memrchr",
        "memset",
        "strcasecmp",
        "strcat",
        "strchr",
        "strcmp",
        "strcoll",
        "strcoll_l",
        "strcpy",
        "strcspn",
        "strdup",
        "strerror",
        "strerror_r",
        "strlen",
        "strncasecmp",
        "strncat",
        "strncmp",
        "strncpy",
        "strndup",
        "strnlen",
        "strpbrk",
        "strrchr",
        "strsignal",
        "strspn",
        "strstr",
        "strtod",
        "strtod_l",
        "strtof",
        "strtold",
        "strtold_l",
        "strtoll",
        "strtoll_l",
        "strtoul",
        "strtoull",
        "strtoull_l",
        "strxfrm",
        "strxfrm_l",
        // Character & locale & wide chars
        "atoi",
        "atol",
        "atoll",
        "atof",
        "btowc",
        "freelocale",
        "isalnum",
        "isalpha",
        "isblank",
        "iscntrl",
        "isdigit",
        "isdigit_l",
        "isgraph",
        "islower",
        "islower_l",
        "isprint",
        "ispunct",
        "isspace",
        "isupper",
        "isupper_l",
        "iswalnum",
        "iswalpha",
        "iswalpha_l",
        "iswblank",
        "iswblank_l",
        "iswcntrl",
        "iswcntrl_l",
        "iswctype",
        "iswdigit",
        "iswdigit_l",
        "iswgraph",
        "iswlower",
        "iswlower_l",
        "iswprint",
        "iswprint_l",
        "iswpunct",
        "iswpunct_l",
        "iswspace",
        "iswspace_l",
        "iswupper",
        "iswupper_l",
        "iswxdigit",
        "iswxdigit_l",
        "isxdigit",
        "isxdigit_l",
        "localeconv",
        "mbrlen",
        "mbrtowc",
        "mbsnrtowcs",
        "mbsrtowcs",
        "mbtowc",
        "newlocale",
        "nl_langinfo",
        "nl_langinfo_l",
        "swprintf",
        "tolower",
        "tolower_l",
        "toupper",
        "toupper_l",
        "towlower",
        "towlower_l",
        "towupper",
        "towupper_l",
        "uselocale",
        "wcrtomb",
        "wcscat",
        "wcschr",
        "wcscmp",
        "wcscoll_l",
        "wcscpy",
        "wcslen",
        "wcsncat",
        "wcsncmp",
        "wcsncpy",
        "wcsnrtombs",
        "wcsrchr",
        "wcsrtombs",
        "wcstod",
        "wcstof",
        "wcstol",
        "wcstold",
        "wcstoll",
        "wcstoul",
        "wcstoull",
        "wcsxfrm_l",
        "wctob",
        "wmemchr",
        "wmemcmp",
        "wmemcpy",
        "wmemmove",
        "wmemset",
        // General libc / runtime
        "__cxa_atexit",
        "__ctype_get_mb_cur_max",
        "arc4random_buf",
        "bsearch",
        "clock",
        "closedir",
        "close",
        "clock_gettime",
        "daylight",
        "dirfd",
        "dup",
        "dup2",
        "environ",
        "eventfd",
        "feof",
        "ferror",
        "fflush",
        "fgetc",
        "fgetpos",
        "fgets",
        "fileno",
        "fputc",
        "fputs",
        "fread",
        "free",
        "freeaddrinfo",
        "fscanf",
        "fseeko",
        "fsetpos",
        "fstat",
        "fsync",
        "ftello",
        "ftruncate",
        "fwrite",
        "getaddrinfo",
        "getauxval",
        "getcwd",
        "getegid",
        "getenv",
        "geteuid",
        "getgid",
        "getpagesize",
        "getpeername",
        "getpid",
        "getppid",
        "getsockname",
        "getsockopt",
        "gettid",
        "gettimeofday",
        "getuid",
        "gmtime",
        "gmtime_r",
        "if_indextoname",
        "if_nametoindex",
        "in6addr_any",
        "in6addr_loopback",
        "inet_ntop",
        "inet_pton",
        "localtime_r",
        "madvise",
        "malloc",
        "mkdir",
        "mktime",
        "mlock",
        "mmap",
        "mmap64",
        "mprotect",
        "mremap",
        "msync",
        "munmap",
        "nanosleep",
        "openat",
        "opendir",
        "optarg",
        "opterr",
        "optind",
        "optopt",
        "pipe",
        "pipe2",
        "poll",
        "posix_fallocate",
        "posix_memalign",
        "prctl",
        "pread",
        "pread64",
        "pwrite",
        "pwrite64",
        "qsort",
        "rand",
        "random",
        "read",
        "readdir",
        "readlink",
        "realloc",
        "recv",
        "recvfrom",
        "recvmmsg",
        "recvmsg",
        "rmdir",
        "sched_get_priority_max",
        "sched_get_priority_min",
        "sched_getaffinity",
        "sched_getcpu",
        "sched_getparam",
        "sched_getscheduler",
        "sched_setaffinity",
        "sched_setscheduler",
        "sched_yield",
        "send",
        "sendmmsg",
        "sendmsg",
        "sendto",
        "setsockopt",
        "shutdown",
        "sleep",
        "snprintf",
        "socket",
        "socketpair",
        "sprintf",
        "sscanf",
        "stderr",
        "stdin",
        "stdout",
        "strftime",
        "strftime_l",
        "syscall",
        "sysinfo",
        "timerfd_create",
        "timerfd_gettime",
        "timerfd_settime",
        "timezone",
        "tzname",
        "tzset",
        "unlink",
        "usleep",
        "vsnprintf",
        "vsprintf",
        "write",
        "epoll_create",
        "epoll_create1",
        "epoll_ctl",
        "epoll_pwait",
        "epoll_wait",
        "fchmod",
        "fchown",
        "chown",
        "fcntl",
    ];
    let host_pthread_compatible = cfg!(target_arch = "x86_64")
        && matches!(
            symbol,
            "pthread_mutex_init"
                | "pthread_mutex_destroy"
                | "pthread_mutex_lock"
                | "pthread_mutex_trylock"
                | "pthread_mutex_unlock"
                | "pthread_mutex_timedlock"
                | "pthread_mutexattr_init"
                | "pthread_mutexattr_destroy"
                | "pthread_mutexattr_gettype"
                | "pthread_mutexattr_settype"
                | "pthread_self"
                | "pthread_setname_np"
                | "pthread_attr_init"
                | "pthread_attr_destroy"
                | "pthread_attr_setstacksize"
                | "pthread_attr_getstacksize"
                | "pthread_attr_setdetachstate"
                | "pthread_attr_getdetachstate"
                | "pthread_attr_setguardsize"
                | "pthread_attr_getguardsize"
                | "pthread_attr_setschedparam"
                | "pthread_attr_getschedparam"
                | "pthread_attr_setstack"
                | "pthread_attr_getstack"
                | "pthread_getattr_np"
                | "pthread_rwlock_init"
                | "pthread_rwlock_destroy"
                | "pthread_rwlock_rdlock"
                | "pthread_rwlock_tryrdlock"
                | "pthread_rwlock_timedrdlock"
                | "pthread_rwlock_wrlock"
                | "pthread_rwlock_trywrlock"
                | "pthread_rwlock_timedwrlock"
                | "pthread_rwlock_unlock"
                | "pthread_detach"
                | "pthread_equal"
                | "pthread_sigmask"
        );
    if library == "libc.so" && (HOST_LIBC_COMPATIBLE.contains(&symbol) || host_pthread_compatible) {
        return runtime_libc_symbol(symbol)
            .map(|address| ("libc.so", address));
    }
    None
}

pub(crate) fn build(imports: &roblox_linker_sys::elf::Imports, host_libc: bool) -> SymbolTables {
    let mut result = SymbolTables::default();
    let use_rust_linker = std::env::var("USE_EXPERIMENTAL_RUST_LINKER")
        .is_ok_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1"));
    let allow_libc_fallback = host_libc || use_rust_linker;
    let overrides: BTreeMap<&str, *mut c_void> = roblox_abi::bionic::function_overrides()
        .into_iter()
        .chain(roblox_abi::bionic::data_overrides())
        .chain(roblox_android::overrides())
        .chain(crate::graphics::function_overrides())
        .collect();

    let mut known = BTreeSet::new();
    for (name, stub) in roblox_abi::stubs::SYMBOLS.iter() {
        known.insert(*name);
        if overrides.contains_key(name) {
            continue;
        }
        let library = library_for(name);
        let host = host_address(library, name).or_else(|| {
            (allow_libc_fallback && library == "libc.so")
                .then(|| runtime_libc_symbol(name))
                .flatten()
                .map(|address| ("libc.so", address))
        });
        let (library, address) = host.unwrap_or((library, *stub as *const () as *mut c_void));
        result
            .libraries
            .entry(library)
            .or_default()
            .push(((*name).to_owned(), address));
    }

    for (&name, &address) in &overrides {
        result
            .libraries
            .entry(library_for(name))
            .or_default()
            .push((name.to_owned(), address));
    }

    for (name, binding) in imports {
        if known.contains(name.as_str())
            || overrides.contains_key(name.as_str())
            || is_linker_symbol(name)
        {
            continue;
        }
        let library = library_for(name);
        let host = host_address(library, name).or_else(|| {
            (allow_libc_fallback && library == "libc.so")
                .then(|| runtime_libc_symbol(name))
                .flatten()
                .map(|address| ("libc.so", address))
        });
        if let Some((library, address)) = host {
            result
                .libraries
                .entry(library)
                .or_default()
                .push((name.clone(), address));
        } else if *binding == Binding::Strong {
            result.missing.push(MissingSymbol { name: name.clone() });
        }
    }

    for library in EMPTY_LIBRARIES {
        result.libraries.entry(library).or_default();
    }
    result
}

fn is_linker_symbol(name: &str) -> bool {
    matches!(
        name,
        "dlopen" | "dlsym" | "dlclose" | "dlerror" | "dladdr" | "dl_iterate_phdr" | "dlvsym"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use roblox_linker_sys::elf::Binding;

    #[test]
    fn host_imports_are_selected_and_unknown_strong_imports_are_reported() {
        let imports = BTreeMap::from([
            ("sin".to_owned(), Binding::Strong),
            (
                "runtime_symbol_that_does_not_exist".to_owned(),
                Binding::Strong,
            ),
            (
                "optional_runtime_symbol_that_does_not_exist".to_owned(),
                Binding::Weak,
            ),
        ]);
        let tables = build(&imports, false);

        assert!(
            tables.libraries["libm.so"]
                .iter()
                .any(|(name, address)| name == "sin" && !address.is_null())
        );
        assert_eq!(
            tables
                .missing
                .iter()
                .map(|missing| missing.name.as_str())
                .collect::<Vec<_>>(),
            ["runtime_symbol_that_does_not_exist"]
        );
    }

    #[test]
    fn android_assets_are_registered_under_libandroid() {
        let tables = build(&BTreeMap::new(), false);
        assert!(
            tables.libraries["libandroid.so"]
                .iter()
                .any(|(name, address)| name == "AAssetManager_fromJava" && !address.is_null())
        );
    }

    #[test]
    fn explicitly_compatible_libc_symbols_use_host_abi() {
        let symbol_names = [
            "__cxa_atexit",
            "__ctype_get_mb_cur_max",
            "daylight",
            "environ",
            "in6addr_any",
            "in6addr_loopback",
            "madvise",
            "mmap",
            "mmap64",
            "mprotect",
            "munmap",
            "mbtowc",
            "memset",
            "newlocale",
            "optarg",
            "optind",
            "strlen",
            "syscall",
            "timezone",
            "tzname",
            "stdin",
            "stdout",
            "stderr",
            "uselocale",
        ];
        let imports: BTreeMap<String, Binding> = symbol_names
            .iter()
            .map(|&name| (name.to_owned(), Binding::Strong))
            .collect();
        let tables = build(&imports, false);
        for name in symbol_names {
            let host = roblox_linker_sys::host_symbol("libc.so.6", name)
                .unwrap_or_else(|| panic!("host libc must export {name}"));
            assert!(
                tables.libraries["libc.so"]
                    .iter()
                    .any(|(registered, address)| registered == name && *address == host),
                "{name} must resolve to the matching host libc symbol"
            );
        }

        #[cfg(target_arch = "x86_64")]
        for name in ["pthread_mutex_lock", "pthread_mutex_unlock"] {
            let host = roblox_linker_sys::host_symbol("libc.so.6", name).unwrap();
            assert!(
                tables.libraries["libc.so"]
                    .iter()
                    .any(|(registered, address)| registered == name && *address == host)
            );
        }

        #[cfg(target_arch = "aarch64")]
        for name in ["pthread_mutex_lock", "pthread_mutex_unlock"] {
            let wrapper = roblox_abi::bionic::pthread::overrides()
                .into_iter()
                .find(|(registered, _)| *registered == name)
                .expect("bionic pthread mutex wrapper");
            assert!(
                tables.libraries["libc.so"]
                    .iter()
                    .any(|(registered, address)| registered == wrapper.0 && *address == wrapper.1)
            );
        }
    }

    #[test]
    fn host_libc_diagnostic_resolves_the_remaining_libc_stubs() {
        let imports = BTreeMap::from([("stat".to_owned(), Binding::Strong)]);
        let tables = build(&imports, true);
        assert!(
            tables.libraries["libc.so"]
                .iter()
                .any(|(name, address)| name == "stat" && !address.is_null())
        );
    }

    #[test]
    fn runtime_libc_resolves_standard_functions_without_host_libc() {
        let symbol_names = [
            "strcspn",
            "strtoul",
            "strtoull",
            "strtof",
            "pthread_detach",
            "timerfd_settime",
            "epoll_wait",
            "pwrite",
            "geteuid",
            "fchown",
            "freelocale",
            "mbrtowc",
            "mbsrtowcs",
        ];
        let imports: BTreeMap<String, Binding> = symbol_names
            .iter()
            .map(|&name| (name.to_owned(), Binding::Strong))
            .collect();
        let tables = build(&imports, false);
        for name in symbol_names {
            let host = roblox_linker_sys::runtime_symbol(name)
                .unwrap_or_else(|| panic!("runtime libc must export {name}"));
            assert!(
                tables.libraries["libc.so"]
                    .iter()
                    .any(|(registered, address)| registered == name && *address == host),
                "{name} must resolve to runtime libc without host_libc"
            );
        }
    }
}
