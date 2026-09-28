use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(
        std::env::var_os("DEP_ROBLOX_LINKER_SHIM_ROOT")
            .expect("roblox-linker must expose its native build directory"),
    );
    println!("cargo:rustc-link-search=native={}/lib", root.display());
    println!("cargo:rustc-link-lib=static=roblox_liblog");
    for library in ["stdc++", "z", "dl", "pthread"] {
        println!("cargo:rustc-link-lib=dylib={library}");
    }
}
