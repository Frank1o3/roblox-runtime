//! Safe Rust-facing API for loading Roblox's Android ELF objects.
//!
//! ELF mapping and relocation remain in the bundled AOSP-derived bionic
//! linker. This crate presents its Rust interface; it does not claim to
//! reimplement that platform loader in Rust.
