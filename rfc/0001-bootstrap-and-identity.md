---
rfc: 1
title: Product identity, bootstrap evidence and revision binding
status: accepted
requires: []
authority: user_approved_implementation_plan
---

## Decision

The product is **Intelligent language**, `il`, extension `.il`. The original
document filename is provenance only. Public commands, package names, schemas and
releases use `il`; no old spelling or compatibility interface is accepted.

P00 runs before a product compiler or runtime exists. Its `host_compiler_hash` is
the real SHA-256 of the pinned host Rust compiler executable. `compiler_hash` and
`runtime_hash` are explicitly null with stage applicability recorded, never
fabricated. This is an explicit P00-only exception to the original evidence
requirement for an existing product compiler hash. `graph_hash` hashes the real
canonical empty revision-0 graph at `examples/bootstrap/graph.json`. Validation commands,
their actual exits, the schema checks and toolchain lock hash remain mandatory.
P01 records the actual probe compiler and runtime artifact hashes; subsequent
stages record the product artifacts they verify. Earlier evidence stays immutable.

Rust 1.90.0 and LLVM 14.0.6 are the baseline. P00 target verification concerns the
configured output target `x86_64-unknown-linux-gnu`; running the probe on that target
is the separate P01 gate. A Windows development host does not fail configuration
validation and does not prove target execution. Dependencies, tool binaries,
container identity where used and CI actions must have exact recorded identities.

Graph revisions are monotonic integers and are distinct from Git commits. Commit
the canonical graph and repository state, then atomically update
`.git/il/revision_bindings.json`. Its shape is `{schema_version: "1.0.0", bindings:
[{revision, git_commit, tree_hash}]}`. Git's tree identity binds tracked state,
graph and toolchain lock without a tracked file containing its own commit SHA.
Tools verify the binding against Git blobs before mutation. Explicit P00 initialization
is the only unbound state; later missing/inconsistent bindings reject mutation.
CI independently reconstructs a missing local journal from committed state and
graph blobs using the trusted bootstrap verifier, not an untrusted AI command.

The AI protocol command allowlist restricts untrusted task/model requests. It does
not prohibit trusted repository bootstrap, compiler builds, pinned dependency
fetches, schema validation, Git operations or CI setup necessary to implement il.
Those operations use explicit structured arguments and locked tools; a tool
protocol request can never supply a shell fragment or arbitrary executable.

P02 precedes the semantic checker. Until P03 is verified, accept only structural
module creation/removal and graph metadata operations whose references and
invariants can be completely validated. Reject functions, executable blocks,
routes, types, effects or capability edits requiring semantic checks with
`E_UNSUPPORTED_FEATURE`. A requested but unavailable check cannot be silently
skipped. P03 removes this bootstrap restriction when complete semantic checking
is available; there is no permissive bypass.

## Validation

Hash existing executables and graph bytes; reject invented or missing evidence.
Check Git binding mismatch, stale graph revision and interrupted publication.
Verify P02 rejects semantic edits, retains failed candidates and never changes
the published head on failure. P00 does not claim P01 or later acceptance.
