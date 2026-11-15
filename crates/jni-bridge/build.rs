use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(
        std::env::var_os("DEP_ROBLOX_NATIVE_SYS_ROOT")
            .expect("roblox-native-sys must expose its native build directory"),
    );
    println!("cargo:rustc-link-search=native={}/lib", root.display());
    for library in ["roblox_jni_shim", "roblox_liblog", "jnivm", "logger"] {
        println!("cargo:rustc-link-lib=static={library}");
    }
    for library in ["stdc++", "z", "dl", "pthread"] {
        println!("cargo:rustc-link-lib=dylib={library}");
    }
}
