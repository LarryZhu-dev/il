---
rfc: 501
title: Independently functioning il compiler bootstrap chain
status: accepted
requires: [P10, E04]
extension: E05
authority: user_approved_implementation_plan
---

Port and validate in order: formatter, documentation generator, standard-library
subset, graph tools, frontend and compiler. il implementations use the same
canonical schemas, diagnostics and semantics. Replace each superseded normal
path once its acceptance suite passes; do not keep a hidden Rust delegate or
interpreter fallback. The pinned Rust stage0 source and binary remain as explicit
bootstrap provenance, not a runtime dependency of the final compiler frontend.

Stage0 compiles the il compiler sources to stage1. Stage1 compiles those identical
sources to stage2. Stage2 compiles them to stage3 with the same input graph,
configuration, pinned LLVM/toolchain and deterministic environment. Stage2 and
stage3 binaries and intermediate canonical outputs must match exactly. A wrapper
that launches the Rust compiler, copies a seed binary or treats source as canned
fixtures cannot satisfy any self-hosting gate.

The il compiler may call the fixed LLVM backend/tool executables through the
declared process capability. Parsing, graph transactions, static checking and
lowering must execute il-produced native code. Record the process tree and
input/output hashes so the absence of a Rust semantic path can be checked.

Stage2 independently compiles positive and negative full-language suites, the
native HTTP example and fault cases in a clean environment without stage0 on
PATH. Verify stage2/stage3 reproducibility and repeat from the pinned stage0 seed.
Publish the bootstrap chain, dependency/target evidence and immutable final
manifest only after all preceding E tasks are VERIFIED.
