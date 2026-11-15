# roblox-jnivm

Pure Rust JNI VM components for the Roblox runtime. Enable the Rust VM with
`USE_EXPERIMENTAL_JNIVM=1` when launching the client; the C++ libjnivm path
remains the default until the Rust implementation reaches parity.

The crate implements the `JNINativeInterface_` function table, object/class
registration with explicit descriptors, strings, primitive and object arrays,
direct ByteBuffers, fields, GC, and a C++ fallback bridge for the
not-yet-ported surface.

The direct ByteBuffer entries were adapted from [libjnivm]
(https://github.com/ChristopherHX/libjnivm); see `LIBJNIVM-NOTICE.md` for
attribution.
