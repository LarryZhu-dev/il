---
rfc: 18
title: Owned host resources, explicit grants and real runtime profiles
status: accepted
requires: [P05, P06]
authority: user_approved_implementation_plan
---

P06 implements the operating-system runtime on Linux x86-64. This RFC fills the
resource, injection and runtime-profile contracts left explicitly unsupported by
P05. Acceptance remains separate from implementation: P06 cannot be VERIFIED
until the language, interpreter, native backend and independent process tests
exercise these contracts. Networking belongs to P07.

## Authority and startup

Runtime operations requiring a capability name its stable entity explicitly in
`attributes.capability`. This optional attribute is absent for operations with no
capability requirement; it is never inferred from another grant of the same kind.
Surface spelling is `runtime.symbol[capability_id](arguments)`. Graph declarations
describe requirements, not permission. A required capability must be present in
the containing function and exactly match a trusted host grant's ID, kind and
scope. Extra selectors on an operation without a capability are invalid.

The trusted CLI launch option `--host-policy <file>` selects a closed JSON host
policy; ordinary tool requests cannot provide or modify it. A policy contains
`schema_version: "1.0.0"`, `grants: [Capability]`, and `test_faults: null | Faults`.
Faults has exactly three required nullable u64 fields: `allocation_fail_after`,
`io_max_chunk`, `io_fail_after`. A non-null chunk limit must be positive. Duplicate
grant IDs, invalid scopes and unknown fields fail closed. Policies are bounded
to 64 KiB. Missing policy means an empty grant set and no injected faults.

Transactions may add/remove requirement declarations only within their existing
scope and the exact host-authorized grant set. An edit cannot mint a capability,
change a host scope, infer authority from a path string or evade checking with an
empty checks list. Declaration edits do not mutate the host policy.

Native programs contain an ordered requirement table (sort by stable entity ID),
never an embedded authority grant. Native startup receives the policy on reserved
descriptor 4; descriptor 3 remains the captured execution report. The host opens
and passes a bounded policy input with a fixed descriptor mapping. No environment
variable or language value enables authority or fault injection. Missing policy
descriptor is acceptable only for programs with no requirements. Malformed or
insufficient policy fails before user code runs. Captured execution permits
trusted test faults; ordinary applications reject a non-null fault configuration.

Runtime startup independently checks the requirement table against the policy,
opens directory grants and holds their descriptors. A compiled grant index names
that checked table; untrusted dynamic integers cannot supply an index to a
privileged runtime call. Build evidence binds requirements and policy identity;
execution evidence binds the policy actually used. Compiler checks alone do not
authorize a subsequently launched application.

## Owned file resources

`core.File` is a fixed nominal opaque record with two U64 fields, `slot` and
`generation`. Its ABI is size 16, alignment 8. The fields describe the ABI only:
language code cannot construct, inspect, cast, clone or partially move a File.
Unknown opaque resource types remain unsupported. Containing aggregates are
owned and recursive cleanup releases their active File leaves.

The runtime's handle table is the sole owner of each OS descriptor. A handle
resolves only when its slot, generation, resource kind and access mode match.
Explicit close consumes the language owner regardless of its result. The slot
is invalidated before closing the descriptor; a stale or repeated close returns
`IoError::Closed` without touching any replacement descriptor. Linux close is
not retried on EINTR. Implicit drop uses the same invalidation and close path,
records any close failure, and never duplicates ownership. Context destruction
closes any remaining host resources, including after a captured returned owner
has been reported. Unrecoverable guest traps do not unwind language frames.

File paths are UTF-8 relative paths beneath the selected directory grant; NUL,
absolute paths and escaping components are denied. Linux `openat2` uses a held
directory fd and `RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS` to check containment
atomically. Path-prefix checks and canonicalize-then-open are not permitted.
Unavailable kernel support is an explicit unsupported failure. No insecure
fallback is provided. A File carries its opened access mode and derived authority;
later operations require that explicit File argument rather than a second grant.

## Deadlines and partial I/O

`core.Deadline` is a fixed copy Sum `Infinite | At(U64)` where At is an absolute
`CLOCK_MONOTONIC` nanosecond instant. Its natural ABI is size 16/alignment 8.
`clock_now[ClockRead]()` reports the same clock. Constructing a deadline value
does not grant access to clock readings. Deadline enforcement internal to an I/O
operation is included in that operation's effect.

Read and write operations retry EINTR while retaining the original absolute
deadline. Readiness waits round upward to milliseconds, then recheck the absolute
instant. They never restart a timeout for a fragment, retry or partial write.
Read returns one actual chunk; an empty chunk means EOF (or a zero-sized request).
Write returns the actual byte count. An offset equal to the buffer length returns
zero; an offset beyond it is InvalidData. Partial I/O is never reported as full
completion. Allocation failure in a Result whose error is IoError returns Other;
executor quota violations remain diagnostic E_RESOURCE_LIMIT traps.

Regular-file disk syscalls cannot be cancelled by poll or O_NONBLOCK. Their
deadline is checked before a syscall; P06 does not claim hard real-time disk
cancellation. Finite deadlines for pipes/standard streams require independently
injected nonblocking descriptors. A blocking stream rejects a finite deadline
with InvalidData instead of silently blocking or changing a shared descriptor's
flags. HTTP's nonblocking socket deadlines are implemented and tested in P07.

| Symbol | Inputs | Result | Effects | Selector |
| --- | --- | --- | --- | --- |
| file_open_read | String shared | Result<File,IoError> | fs | FileRead |
| file_open_write | String shared | Result<File,IoError> | fs | FileWrite |
| file_read_some | File shared, Usize max, Deadline copy | Result<Bytes,IoError> | fs,alloc | none |
| file_write_some | File shared, Bytes shared, Usize offset, Deadline copy | Result<Usize,IoError> | fs | none |
| file_close | File owned | Result<Unit,IoError> | fs | none |
| stdin_read | Usize max, Deadline copy | Result<Bytes,IoError> | process,alloc | none |
| stdout_write | Bytes shared, Usize offset, Deadline copy | Result<Usize,IoError> | process | none |
| stderr_write | Bytes shared, Usize offset, Deadline copy | Result<Usize,IoError> | process | none |

File write-open uses create/truncate semantics (mode 0600 subject to host umask).
Existing file_read/file_write retain their separately declared whole-buffer
convenience contracts and must use the same authority and resource core. Existing
print_i64/print_string keep their complete-write Result<Unit,IoError> contract.
The IoError and AllocError variant order from RFC0007 remains unchanged.

## Runtime implementation boundaries

Safe host-resource code lives in `runtime/handles`; grant validation and inherited
descriptor setup live in `runtime/startup`; explicit allocation policy lives in
`runtime/allocator`. The interpreter and native facade use the same OS resource
primitives, without sharing a language opcode evaluator. The native runtime
remains free of graph readers, frontend dispatch and MIR interpretation.

Host allocation faults reject the configured allocation attempt and subsequent
attempts, with zero-based counters (zero rejects the first). I/O chunk limits cap
each actual syscall. I/O failure thresholds count successful nonempty I/O calls
across the execution and fail subsequent calls with the appropriate Read/Write
error. Fault counters are execution-local, deterministic and never host commands.

Full runtime stack traces contain language function IDs, call-site IDs and the
current entity, in innermost-first order. They are maintained on enter/leave;
host backtrace text cannot substitute for language frames. Execution reports add
`live_handles`, `stack_trace` and structured handle events. Success has an empty
stack trace. Resource returns use an output-only typed resource descriptor with
kind, slot and generation; captured arguments always reject resource descriptors,
including nested descriptors. They cannot recreate authority. Reported live
owners are guest ownership counts before the harness releases returned resources.

## Independent runtime profiles

Runtime profile Full, Minimal or None is independent of optimization. It is a
build input and appears in Native IR, validation, artifacts and evidence.
Captured execution requires Full; neither reduced profile secretly links a full
capture harness. Profile checks cover the entire supplied program, not just a
convenient reachable subset.

Full uses the managed runtime, host handles, capability bridge, language stack,
deadlines and typed I/O. Application entry remains `main() -> I32` and traps exit
101 without unwinding.

Minimal is an actual separate no_std static runtime, with Linux x86-64 syscall
startup, SIGPIPE handling, checked traps and integer stdout. It supports copy
scalars/aggregates and print_i64, but rejects managed values, handles, alloc, fs,
net and clock operations. It has no heap, libc, JSON, full Context or dynamic
loader dependency. It is linked with fixed `-nostdlib -static` arguments. Any
compiler-generated memory primitives must be provided by this minimal runtime or
lowered to actual inline operations. Minimal traps exit 101.

None uses an explicit exports entry mode with a nonempty selected list of public
functions and emits a freestanding ELF relocatable object. All functions are
pure and unmanaged. Exported parameters/results are scalar or Unit under the
fixed System V ABI; copy aggregates may be used internally, while external
aggregate passing remains unsupported under spec/abi.yaml. It has no context,
runtime declarations, archive, startup or hidden interpreter path. Checked
integer/explicit traps lower to llvm.trap; the embedding host receives a machine
trap, not the process-exit guarantee supplied by Full/Minimal. Symbols retain
the locked entity-ID encoding.

The build API replaces its internal lowering/linking interfaces with explicit
profile and entry-mode inputs. It does not retain legacy forwarding wrappers or
fake empty archives/executables. None's absent runtime/executable artifacts are
represented explicitly. Minimal/None artifacts are inspected with nm/readelf,
linked/executed by independent native test harnesses, and built twice from clean
stores before support can be claimed.

## Acceptance

Positive and negative contracts cover grants and ambiguous selection; traversal,
symlink escape and stale handles; explicit/implicit close and fd leak counts;
partial reads/writes, EOF, allocation/I/O faults, real timeout and EINTR behavior;
panic exit and language stack order; text/graph/transaction agreement; interpreter
and native observable behavior; and each runtime profile's allowed/rejected
features and actual dependencies. Tests use isolated temporary files/processes
and shut down their endpoints. P06 remains RUNNING until all applicable contracts
and module/schema checks pass with immutable evidence.
