use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root");
    let native = root.join("native");

    let dst = cmake::Config::new(&native)
        .define("CMAKE_C_COMPILER", "clang")
        .define("CMAKE_CXX_COMPILER", "clang++")
        .define("CMAKE_BUILD_TYPE", "Release")
        .define(
            "RBX_RUNTIME_JNI_TRACE",
            if std::env::var_os("RBX_RUNTIME_JNI_TRACE").is_some() {
                "ON"
            } else {
                "OFF"
            },
        )
        .build();

    println!("cargo:root={}", dst.display());
    println!("cargo:rerun-if-env-changed=RBX_RUNTIME_JNI_TRACE");
    println!("cargo:rerun-if-changed={}", native.display());
}
