//! Build the Android library symbol tables from ABI implementations, host
//! graphics/math libraries and the generated compatibility stubs.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::c_void;

use roblox_linker::elf::Binding;

const ANDROID_PREFIXES: &[(&str, &str)] = &[
    ("AMedia", "libmediandk.so"),
    ("AImage", "libmediandk.so"),
    ("AndroidBitmap", "libjnigraphics.so"),
    ("__android_log", "liblog.so"),
    ("android_set_abort_message", "liblog.so"),
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

fn host_address(library: &str, symbol: &str) -> Option<(&'static str, *mut c_void)> {
    let registered = host_candidates(library)
        .iter()
        .find_map(|(candidate, android_library)| {
            roblox_linker::host_symbol(candidate, symbol).map(|address| (*android_library, address))
        });
    if registered.is_some() {
        return registered;
    }

    // These functions have matching call/data ABIs on glibc and bionic. Keep
    // this list explicit: resolving every libc import from glibc would silently
    // cross incompatible structures such as bionic's `stat` and `sigaction`.
    const HOST_LIBC_COMPATIBLE: &[&str] = &[
        "__cxa_atexit",
        "__ctype_get_mb_cur_max",
        "mbtowc",
        "memset",
        "newlocale",
        "strlen",
        "syscall",
        "uselocale",
    ];
    let host_pthread_mutex_compatible = cfg!(target_arch = "x86_64")
        && matches!(symbol, "pthread_mutex_lock" | "pthread_mutex_unlock");
    if library == "libc.so"
        && (HOST_LIBC_COMPATIBLE.contains(&symbol) || host_pthread_mutex_compatible)
    {
        return roblox_linker::host_symbol("libc.so.6", symbol).map(|address| ("libc.so", address));
    }
    None
}

pub(crate) fn build(imports: &roblox_linker::elf::Imports, host_libc: bool) -> SymbolTables {
    let mut result = SymbolTables::default();
    let overrides: BTreeMap<&str, *mut c_void> = roblox_abi::bionic::function_overrides()
        .into_iter()
        .chain(roblox_abi::bionic::data_overrides())
        .chain(roblox_android::overrides())
        .chain(crate::graphics::function_overrides())
        .collect();
    let override_names: BTreeSet<&str> = overrides.keys().copied().collect();

    let mut known = BTreeSet::new();
    for (name, stub) in roblox_abi::stubs::SYMBOLS.iter() {
        known.insert(*name);
        if overrides.contains_key(name) {
            continue;
        }
        let library = library_for(name);
        let host = host_address(library, name).or_else(|| {
            (host_libc && library == "libc.so")
                .then(|| roblox_linker::host_symbol("libc.so.6", name))
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

    for (name, address) in overrides {
        result
            .libraries
            .entry(library_for(name))
            .or_default()
            .push((name.to_owned(), address));
    }

    for (name, binding) in imports {
        if known.contains(name.as_str())
            || override_names.contains(name.as_str())
            || is_linker_symbol(name)
        {
            continue;
        }
        let library = library_for(name);
        let host = host_address(library, name).or_else(|| {
            (host_libc && library == "libc.so")
                .then(|| roblox_linker::host_symbol("libc.so.6", name))
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
    use roblox_linker::elf::Binding;

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
    fn observed_constructor_libc_calls_use_host_functions() {
        let tables = build(&BTreeMap::new(), false);
        for name in [
            "__cxa_atexit",
            "__ctype_get_mb_cur_max",
            "mbtowc",
            "memset",
            "newlocale",
            "strlen",
            "syscall",
            "uselocale",
        ] {
            assert!(
                tables.libraries["libc.so"]
                    .iter()
                    .any(|(registered, address)| registered == name && !address.is_null()),
                "{name} must resolve to the host libc function"
            );
        }

        #[cfg(target_arch = "x86_64")]
        for name in ["pthread_mutex_lock", "pthread_mutex_unlock"] {
            let host = roblox_linker::host_symbol("libc.so.6", name).unwrap();
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
}
