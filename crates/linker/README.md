# roblox-linker

Rust-native ELF mapper and relocator used by the runtime. It is the only engine
loader; the former AOSP-derived C++ linker integration has been removed.

This first implementation maps an ELF shared object and available
`DT_NEEDED` files beside it, applies supported x86-64 base and symbol
relocations, exposes the root object's dynamic exports, and delays
`DT_INIT` / `DT_INIT_ARRAY` constructors until the runtime has prepared its
Android services. The runtime supplies imported symbol addresses from its
existing compatibility tables.

It currently supports only x86-64 hosts and explicitly rejects `PT_TLS`,
`TPOFF64`, and `IRELATIVE` images. Its supported ELF and relocation surface
remains limited to the behavior described above. The AArch64-specific bionic
pthread wrappers elsewhere in the workspace are ABI groundwork only; they do
not provide an AArch64 host loader or end-to-end runtime.

The ELF parser, memory mapper, relocation decoder, and TLS layout foundations
were adapted from [Eclipse](https://github.com/Kuenec/Eclipse); see
`ECLIPSE-NOTICE.md` for attribution.
