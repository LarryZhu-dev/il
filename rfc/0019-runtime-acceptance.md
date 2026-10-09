---
rfc: 19
title: Independent host resources and runtime-profile acceptance
status: accepted
requires: [P05, P06]
authority: user_approved_implementation_plan
---

## Oracle and isolation

The P06 oracle is `tests/runtime_cli_contracts.json`. Expected bytes, counts,
variant tags, resource ownership, and forbidden dependencies are specified
before implementation execution. `tests/runtime_cli.py` supplies isolated host
fixtures and drives the final closed CLI protocol. Existing P05 contracts remain
unchanged. A report cannot call a scenario passed until its actual assertions
finish. Missing kernel/tool/profile support is a failed or blocked gate, never a
silent skip.

Each scenario gets independent read/write grant directories and an outside
directory containing a sentinel. Tests create symlinks pointing outside the
grant, distinct same-kind grants, binary files containing NUL and non-UTF-8 bytes,
and an existing file to exercise truncate semantics. No ambient directory grants
are supplied. The host policy is an independently written file; source graphs
only declare matching requirements. Native startup is also tested directly with
fd 4 missing, malformed, insufficient, and correct after compilation succeeded.

Interpreter and native debug/release runs use freshly reset fixtures so writes
cannot hide a difference. Every run compares its complete typed result, exact
output bytes, diagnostics, guest live handles, and observable file contents.
Timestamps and OS descriptor numbers are not cross-backend equality oracles.
Resource descriptors remain output-only; nested injected descriptors must be
rejected before guest operations execute.

## Resource and I/O scenarios

The matrix covers real open/read/write/close, EOF, zero-size read, partial I/O,
buffer offsets, whole-buffer convenience calls, explicit and implicit close,
aggregate-contained resources, returned owners, and ownership rejection. Native
runtime module tests exercise stale slot/generation and repeated close directly;
well-typed il programs cannot manufacture or duplicate a consumed File merely to
reach that path. This distinction prevents an unsafe public test-only escape.

Directory containment is checked using traversal, absolute and NUL paths, and
symlink escape. Outside sentinels must remain unchanged. Malformed and duplicated
host grants, missing capability selectors and a grant mismatch are rejected; a
second same-kind grant does not silently authorize the selected first grant.

Fault policies independently cover first-allocation failure, capped chunks, and
I/O failure after one actual successful nonempty syscall. The external client
checks partial output and file bytes as well as the returned result. Source code
and ordinary request JSON cannot set fault controls. Applications reject test
fault policies even when all grants otherwise match.

Deadline tests supply finite deadlines to real pipes whose nonblocking status is
chosen by the independent host. An empty nonblocking pipe with its writer held
open must time out; an otherwise identical blocking pipe must return InvalidData
without blocking. Data-ready and EOF fixtures distinguish timeout from empty
success. Signal interruption and fragment retries retain one absolute deadline.
Timing assertions use monotonic elapsed time with broad scheduling tolerance;
they never claim hard cancellation of regular-file disk calls.

Nested-call traps check explicit stable function IDs, call-site IDs and current
entity in innermost-first order. A host backtrace or a single trap location does
not satisfy this contract. Successful execution has no stack trace. Trap paths
do not unwind guest ownership; context destruction still closes host descriptors.

## Physical runtime-profile acceptance

Full, Minimal and None are independent of debug/release optimization. Full
is the default when `build` omits `runtime_profile`. Build requests otherwise
explicitly select `runtime_profile: full | minimal | none`; None requires a
nonempty unique `exports` list, while Full/Minimal reject that field. None's
executable artifact and runtime hash are null. The host-policy hash binds the
canonical parsed policy, including the canonical empty policy when no file was
provided; it is not a hash of incidental JSON whitespace.

Full retains host-resource support. Minimal must be an actual static executable:
`readelf` must find no interpreter segment or dynamic dependency, and `nm` must
find no undefined symbols or managed-runtime dependencies. Its integer output and
checked-trap exit 101 are executed directly. Managed values, forbidden effects,
and forbidden operations in unused functions are rejected before linking.

None must be an ELF relocatable object with the declared scalar exports under
their entity-ID-derived symbol names. It has no executable/runtime artifact,
Context argument, host runtime declarations, or hidden initializer. Independent
LLVM host harnesses link the object and call selected exports, including an
arithmetic boundary that triggers a machine trap. External aggregate signatures,
empty/unknown/private exports, host effects, and managed values are rejected.

Every supported profile is built twice from independent clean stores. Input,
artifact, tool identity, policy identity, entry mode, and runtime-profile hashes
are checked from the evidence; artifact bytes must agree within a fixed profile
and optimization mode. Invalid builds retain candidates and never destroy an
earlier executable or object. All fixture descriptors/processes are closed before
the suite returns, including failure paths.


The runtime CLI suite additionally checks that captured output uses actual blocking
pipes: a finite stdout deadline returns InvalidData consistently in interpreter
and native execution. It also exercises repeated 20,000-byte call identifiers.
The independent 1 MiB active stack budget traps without truncating accepted frames;
the CLI separately enforces its 256 KiB envelope budget and returns
E_CONTEXT_INSUFFICIENT for oversized execution responses. Native attack receipts
are therefore inspected through the retained artifact and a fresh fd-3 process
run, while interpreter unit acceptance directly inspects the full bounded report.
