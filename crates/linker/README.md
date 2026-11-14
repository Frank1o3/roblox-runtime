# roblox-linker

Experimental Rust-native ELF mapper and relocator for the runtime. Enable it
with `USE_EXPERIMENTAL_RUST_LINKER=1` when launching the client; the existing
AOSP-derived linker remains the default.

This first implementation maps one ELF shared object, applies supported
x86-64 base and symbol relocations, exposes its dynamic exports, and delays
`DT_INIT` / `DT_INIT_ARRAY` constructors until the runtime has prepared its
Android services. The runtime supplies imported symbol addresses from its
existing compatibility tables.

It currently supports only x86-64 hosts and explicitly rejects `PT_TLS`,
`TPOFF64`, and `IRELATIVE` images. Recursive `DT_NEEDED` loading and Android
`dlopen` / `dlsym` emulation are also not implemented yet, so this path is an
initial experiment and is not expected to load every Roblox client build. The
existing linker is still required to build the rest of the runtime.

The ELF parser, memory mapper, relocation decoder, and TLS layout foundations
were adapted from [Eclipse](https://github.com/Kuenec/Eclipse); see
`ECLIPSE-NOTICE.md` for attribution.
