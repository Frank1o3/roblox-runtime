# roblox-jni-bridge

Rust-side integration between Roblox's Android native code and the runtime's
Java VM. The public `roblox_runtime::jni` facade is backed by this crate.

The accessibility and GameActivity modules live here because they implement
JNI-facing callbacks and compatibility state. The `jnivm` crate owns the Rust
VM and its object model. Platform services such as audio, battery, storage,
and system paths belong to their platform crates.

This is the first boundary cleanup: the existing JNI bridge modules have been
moved here, while their remaining native compatibility bindings are still
being replaced with Rust implementations.
