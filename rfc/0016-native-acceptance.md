---
rfc: 16
title: Independent native execution and ABI acceptance
status: accepted
requires: [P04]
authority: user_approved_implementation_plan
---

## Independent oracle

The P05 external Python suite reuses all 180 cases in
`tests/execution_cli_contracts.json` without generating expected answers from a
compiler or interpreter. The supplemental source/ABI contract is
`tests/native_cli_contracts.json`. Every case executes through captured MIR,
debug native code, and release native code. Native artifacts are actual x86-64
Linux ELF executables produced through verified LLVM IR, object emission, and
linking; passing a compiler API test alone is insufficient.

`test(revision, suite, isolation)` uses the closed isolation values `captured`,
`native_debug`, and `native_release`. The suite's existing entry, argument, and
limit fields remain typed. Native compilation binds those fields to the captured
build input. LLVM creates the typed ABI argument initializer, invokes the compiled
function, and serializes its actual result. Neither the native runtime nor the
test wrapper interprets graph operations, MIR, or source text.

The ELF emits a machine-readable execution report on file descriptor 3. Ordinary
stdout and stderr remain independently observable. Successful captured execution
exits 0; a language trap exits 101. A process timeout, signal, malformed report,
missing report, or unexpected descriptor output is a failure, not a language
trap. The external suite directly reruns each ELF with an empty environment in a
separate working directory and compares the result with the CLI invocation.

## Observable comparison

All backends must agree on status, the entire typed result, stdout/stderr bytes,
live allocation count, and diagnostic code/entity ID. Full-width integers are
decimal strings; no result is inferred from the operating system's truncated
exit status. Aggregate values retain nominal type identities, field order, tags,
and payload types. Steps, peak memory, and lifecycle instrumentation are recorded
but are not required to be instruction-identical across optimized backends.

The supplemental contract covers mixed-width record padding, nested aggregates,
aggregate calls and returns, owned arguments, selected-edge cleanup, cloning,
empty and UTF-8 strings, simultaneous loop block-argument transfers, explicit
failure propagation, bounded-allocation failure, and observable expression order.
The external suite also checks object/executable ELF headers, artifact hashes,
and physical LLVM verifier rejection of malformed supplied IR. Direct Native IR
mutation tests validate layouts, def/use, calls, branches, and ownership cleanup
without trusting a source graph or original MIR.

## Reproducibility and publication

Native results retain paths and SHA-256 hashes for Native IR, LLVM IR, object,
and executable artifacts, plus stage evidence. Profiles are tested separately;
debug and release binaries need not have the same hash. Two independent clean
stores build the same input/profile and must agree on every native artifact hash.
An invalid build request must leave the graph revision and previously verified
artifacts intact and executable. Failed reports preserve actual diagnostics;
the suite never marks an unexecuted case successful.

The default suite requires Linux x86-64 and the pinned LLVM tools. Missing
dependencies fail the suite explicitly. Later E03 target suites must execute on
the corresponding real operating system and architecture.
