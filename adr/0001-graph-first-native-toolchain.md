# ADR 0001: Graph-first native toolchain

Status: accepted design; implementation evidence tracked separately.

il uses a canonical, versioned graph as program authority and `.il` as a stable
text projection. Rust bootstraps the compiler, and LLVM 14.0.6 emits native object
files. Target programs are not translated into C, Rust or another high-level
language. The first target is Linux x86-64 ELF; other targets follow P10.

Transactions isolate candidate edits, check all available required invariants
and publish atomically. A failed edit never replaces the previous verified
program. Monotonic graph revisions and an external Git binding journal avoid
self-referential state hashes. RFC 0001 defines initial evidence without claiming
that an unavailable product compiler exists.

Accepted RFCs authorize semantic designs only. Task state becomes VERIFIED only
after the corresponding commands and independent acceptance checks succeed with
real hashes and exits. Native execution is required per target; cross-compilation
and CI workflow presence alone do not satisfy that condition.
