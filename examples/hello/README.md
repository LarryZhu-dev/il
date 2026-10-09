# Hello from il

Compose, in order, `packages/core/lib.il`, `packages/alloc/lib.il`,
`packages/io/lib.il`, and `examples/hello/main.il` as one text projection.
The entry entity is `hello.main`; it prints `Hello from Intelligent language!`
followed by a newline, returning 0 on success, 1 on concatenation failure and
2 on output failure. The Full runtime is required. No host capability grants
are needed for inherited stdout.

`tests/runtime_host/package_sources.rs` parses and checks the real package
sources, executes representative functions, and builds/runs this application
using both debug and release native output. Source composition is explicit and
does not imply E04 dependency resolution.
