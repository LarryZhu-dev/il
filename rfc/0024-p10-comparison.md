---
title: P10 reproducible workflow comparison
status: accepted
---

# P10 reproducible workflow comparison

P10 requires measured Rust and il decision data. The initial experiment uses
fixed, reviewable workflow scripts, not an LLM benchmark. It does not measure
model reasoning, token use, or human productivity and cannot establish that one
language is better. The independent Rust baseline uses only its standard
library; il application code still lowers directly to LLVM.

The locked task matrix contains route addition, type-error diagnosis/repair,
and rollback, repeated three times per implementation in fresh directories.
Both servers must satisfy the same external TCP assertions: `/health` returns
200 and `ok`; `/hello/World` returns JSON `{"message":"Hello, World"}` when
installed and 404 otherwise; unknown paths return 404; POST `/health` returns
405. Error bodies are empty. These subset assertions do not replace the full
MVP HTTP protocol suite. Services are sequential and always terminated/reaped.

Each trial records source/toolchain/compiler hashes, monotonic duration,
process argv, actual stdin/stdout/stderr bytes and exit codes, HTTP request and
response bytes, source snapshots and diffs, and failed assertions. Source setup
is outside measured time; inspection, edits/transactions, compilation, tests,
and rollback are inside. Tool calls count explicit recorded process/read/edit/
HTTP operations, including failing calls; they are not model API calls. Patches
measure Rust source lines or canonical il graph lines, with those different
representations explicitly identified. il rollback ignores only the monotonic
graph revision when comparing restored content. Old executables are rerun.

The type-error fixtures are language-appropriate rather than identical: Rust
returns an integer where its health function declares a string; il attempts a
route with a handler accepting the wrong request type. Both must emit structured
type diagnostics, preserve their last successful artifact, and accept a repair.
No simulated diagnostics, assumed durations, or preset success rates are used.

The trusted runner executes only its built-in commands, uses unique output
directories, retains all failed attempts, and never writes task status or a
release manifest. A partial matrix is explicitly incomplete. Completion of
all 18 attempts gives comparison data, including any failures; it does not by
itself verify G0–G3, five-platform support, self-hosting, or P10 publication.
Source dirtiness is recorded; release evidence requires a clean committed rerun.
An offline bundle checker only checks structure and consistency. Hashes protect
integrity, not the truth of a caller's claims about execution.
