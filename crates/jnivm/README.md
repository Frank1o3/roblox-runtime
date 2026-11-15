# jnivm

Pure Rust JNI VM for the Roblox runtime. The runtime owns one process JavaVM
here, including its class/object state and thread attachment.

Roblox Android class hooks, GameActivity callbacks, and the `JNIEnv` bridge
belong in `roblox-jni-bridge`. This VM crate supplies the execution and state
model those hooks use; it should not own Roblox-specific Android behavior.

The direct ByteBuffer entries were adapted from [libjnivm]
(<https://github.com/ChristopherHX/libjnivm>); see `LIBJNIVM-NOTICE.md` for
attribution.
