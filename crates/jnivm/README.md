# roblox-jnivm

Pure Rust JNI VM components for the Roblox runtime. The runtime creates this
VM directly and uses it for engine JNI calls.

The crate implements the `JNINativeInterface_` function table, object/class
registration with explicit descriptors, strings, primitive and object arrays,
direct ByteBuffers, fields, and GC. Remaining Android class hooks are being
migrated out of the native compatibility layer.

The direct ByteBuffer entries were adapted from [libjnivm]
(<https://github.com/ChristopherHX/libjnivm>); see `LIBJNIVM-NOTICE.md` for
attribution.
