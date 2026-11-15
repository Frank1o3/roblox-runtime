The direct ByteBuffer JNI entries in `src/abi/direct_buffer.rs` are adapted
from libjnivm's `internal/bytebuffer.cpp`: <https://github.com/ChristopherHX/libjnivm>
(maintained as part of the mcpelauncher project).

That file is available under the MIT License, copyright (c) 2019 ChristopherHX.
The remaining jnivm sources are independent Rust implementations developed
against the JNI specification, libjnivm's observed behavior, and the engine's
observed JNI surface (`OBSERVED_SURFACE.md`); they are part of this repository
and are licensed under its GPL-3.0-or-later license.
