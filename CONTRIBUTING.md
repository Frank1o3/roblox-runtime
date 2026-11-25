# Contributing to roblox-runtime

The runtime loads the Roblox Android client inside a Linux process. It contains
Rust subsystems and a remaining native C/C++ compatibility layer. The current
runtime and linker support x86-64 hosts only.

## Before contributing

- Read the Code of Conduct.
- Check open issues and pull requests before starting duplicate work.
- Identify the actual caller, ABI contract, or observed runtime behavior behind
  a proposed compatibility change.
- Distinguish tested facts from assumptions. Mark inferred behavior as such.
- Do not include account cookies, tokens, APKs containing private information,
  or unredacted logs in issues or pull requests.

## Development requirements

Use a current stable Rust toolchain. The workspace uses Rust 2024 and builds
native compatibility targets, so the development environment also requires
Clang/Clang++, CMake, the Git submodules, and the Linux development packages
needed by the enabled audio, graphics, and window-system backends.

Clone or initialize the repository with its submodules:

~~~sh
git clone --recurse-submodules https://github.com/Frank1o3/roblox-runtime.git
cd roblox-runtime
git submodule update --init --recursive
~~~

## Checks

Run the workspace checks before submitting:

~~~sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
~~~

Build a release binary when a change affects runtime loading or integration:

~~~sh
cargo build --release
~~~

Some behavior, including graphics, Android ABI interaction, and audio device
lifecycle, requires a real Linux desktop environment for meaningful manual
verification. State clearly which checks were run and which were not.

## Engineering expectations

- Keep ABI wrappers layout-aware. Do not pass bionic structures directly to
  glibc when their layouts or semantics differ.
- Document every unsafe block with the safety invariant that makes it valid.
- Add focused tests for ABI sizes, guard regions, ownership, error handling,
  and concurrency when those contracts are affected.
- Do not replace real behavior with success-shaped stubs. Record unsupported
  symbols and inferred behavior honestly.
- Keep microphone capture demand-driven. Enumeration and stream discovery must
  not open capture devices; stop and pause paths must release active streams.
- Never log cookies, authentication tokens, or other secrets.
- Keep URL and redirect validation, file permissions, and path boundaries
  intact.
- Keep third-party license and copyright notices with adapted files. New
  third-party code requires a provenance and license review before it is
  included.

## Architecture and platform scope

The Rust ELF mapper currently rejects non-x86-64 hosts. AArch64-specific ABI
wrappers are groundwork only and do not constitute supported AArch64 runtime
execution. Do not describe the runtime as cross-architecture until the full
loader and integration path are implemented and tested.

The project still compiles C/C++ compatibility code. Do not remove CMake,
Clang, native sources, or submodules based only on the presence of a Rust
replacement. Verify build-graph references, runtime reachability, symbol
ownership, and parity tests first.

## Pull requests

Explain the affected subsystem, its observed contract, and why the change is
needed. Include regression tests and exact verification commands. Update the
README, porting plan, and relevant crate documentation when current behavior or
build requirements change. Submit bootstrapper changes to
[Frank1o3/rusty-blox](https://github.com/Frank1o3/rusty-blox) separately; if the
bootstrapper needs a newer runtime revision, update its submodule pointer to
the pushed runtime commit.
