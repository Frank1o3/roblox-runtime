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
    host_candidates(library)
        .iter()
        .find_map(|(candidate, android_library)| {
            roblox_linker::host_symbol(candidate, symbol).map(|address| (*android_library, address))
        })
}

pub(crate) fn build(imports: &roblox_linker::elf::Imports) -> SymbolTables {
    let mut result = SymbolTables::default();
    let overrides: BTreeMap<&str, *mut c_void> = roblox_abi::bionic::function_overrides()
        .into_iter()
        .chain(roblox_abi::bionic::data_overrides())
        .chain(roblox_android::overrides())
        .collect();
    let override_names: BTreeSet<&str> = overrides.keys().copied().collect();

    let mut known = BTreeSet::new();
    for (name, stub) in roblox_abi::stubs::SYMBOLS.iter() {
        known.insert(*name);
        if overrides.contains_key(name) {
            continue;
        }
        let library = library_for(name);
        let (library, address) =
            host_address(library, name).unwrap_or((library, *stub as *const () as *mut c_void));
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
        if let Some((library, address)) = host_address(library, name) {
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
        let tables = build(&imports);

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
        let tables = build(&BTreeMap::new());
        assert!(
            tables.libraries["libandroid.so"]
                .iter()
                .any(|(name, address)| name == "AAssetManager_fromJava" && !address.is_null())
        );
    }
}
