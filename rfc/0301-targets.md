---
rfc: 301
title: Independently verified native target matrix
status: accepted
requires: [P10, E02]
extension: E03
authority: user_approved_implementation_plan
---

Retain Linux x86-64 and add, in order, Linux AArch64, macOS x86-64/AArch64 and
Windows x86-64 MSVC. Target identities are `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin` and
`x86_64-pc-windows-msvc`. Object formats are ELF, Mach-O and PE/COFF respectively.
All five are little-endian with 64-bit Usize; external calls obey their actual
platform C ABI. Internal aggregate calls keep explicit pointer passing.

Each target provides its own ABI layout, syscall/runtime capability bridge,
linker selection and deterministic flags. Do not infer target configuration from
the development host. Pin LLVM, SDK/sysroot/linker identities and record them in
evidence. Unsupported hosts or missing tools produce explicit diagnostics.

Run the semantic, runtime, HTTP, fault and recovery suites on a real runner of
each target architecture. Two independent clean builds on the same target and
locked input must yield matching hashes. Reproducibility is per target; binaries
for different architectures are not expected to match. Normalize paths and
timestamps at production, not by stripping meaningful differences from evidence.
CI runner unavailability leaves that target BLOCKED, not VERIFIED. Cross-builds
may provide additional checks but never replace native execution acceptance.
