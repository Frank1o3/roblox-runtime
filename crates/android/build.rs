use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(
        std::env::var_os("DEP_ROBLOX_LINKER_SHIM_ROOT")
            .expect("roblox-linker must expose its native build directory"),
    );
    println!("cargo:rustc-link-search=native={}/lib", root.display());
    // The Android path setup crosses into `android_classes.cpp`; depending on
    // `roblox-linker` for its Rust API alone does not pull this static archive
    // into a client that only prepares the Android filesystem environment.
    for library in [
        "roblox_linker_shim",
        "roblox_jni_shim",
        "roblox_liblog",
        "jnivm",
        "logger",
        "linker",
    ] {
        println!("cargo:rustc-link-lib=static={library}");
    }
    for library in ["stdc++", "z", "dl", "pthread"] {
        println!("cargo:rustc-link-lib=dylib={library}");
    }
}
