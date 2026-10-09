---
rfc: 15
title: Native runtime substrate and fixed LLVM link driver
status: accepted
requires: [P04, P05]
authority: user_approved_implementation_plan
---

P05 implements native execution of verified language functions, with runtime
services restricted to allocation, buffer operations, process output, bounded
test execution, and result/evidence encoding. The Rust static library has no
opcode dispatcher, Graph/MIR reader, name resolver, frontend or interpreter.
Typed LLVM instructions implement arithmetic, calls, control flow, aggregates,
variant dispatch, moves and explicit cleanup. P06 adds OS resources and trusted
capability bridges to this same substrate; unsupported file/clock operations
are rejected rather than replaced by successful stubs.

## Value representation and function boundary

`spec/abi.yaml` remains authoritative. Scalars use declared integer width; Bool
is a byte containing zero or one. Record/Tuple layout follows declaration order
and natural alignment. Sum/Option/Result use a u32 tag then a payload large and
aligned enough for the largest alternative, with no niche optimization.
String/Bytes are `{i8* pointer, u64 length, u64 capacity}`, size 24/alignment 8
on x86-64 Linux. The allocating runtime alone frees buffers. Empty buffers have
zero guest payload but a distinct one-byte physical allocation, so lifetime
identities remain meaningful. Memory is copied or cloned only where generated
code explicitly requests it. Aggregate values are passed indirectly and returned
through an explicit result pointer. A hidden runtime-context pointer is the
first function argument; an aggregate result pointer is second if required.

## Frozen runtime C ABI

All lengths and unsigned language values are u64; signed language values are
i64. `ctx` is an opaque pointer. `text` below means a byte pointer plus a u64
length; strings passed as text must be UTF-8. C ABI functions never unwind.

| Symbol prefix `il_rt_` | Parameters after ctx | Return / behavior |
| --- | --- | --- |
| context_new | mode:u32, steps:u64, depth:u32, heap:u64, output:u64, revision:u64, report_fd:i32; no ctx | ctx |
| finish | none | report returned Value in captured mode, release context |
| enter | entity:text | increment call depth; entry depth is one |
| leave | none | decrement call depth |
| tick | entity:text | charge one language MIR operation/terminator/cleanup |
| location | entity:text | restore diagnostic location without charging a step |
| trap | code:text, message:text, entity:text | report then exit 101; no unwind |
| buffer_new | out:Buffer*, bytes:text, entity:text | new managed buffer; resource trap on failure |
| buffer_free | buffer:Buffer* | release allocation; no implicit lifecycle event |
| concat | out:Buffer*, a:Buffer*, b:Buffer*, entity:text | u32: 0 success, 1 OutOfMemory, 2 CapacityOverflow |
| print_i64 | value:i64 | u32: 0 success, otherwise IoError tag + 1 |
| print_buffer | buffer:Buffer* | same process-output status |
| trace_begin | kind:u32, entity:text | begin one owner event |
| trace_buffer | buffer:Buffer* | append a leaf allocation ID |
| trace_end | none | atomically append the completed event |
| json_raw | bytes:text | append generated structural JSON |
| json_quoted | value:text | append a JSON-escaped UTF-8 string |
| json_i64 / json_u64 | value:i64 / u64 | append a quoted canonical decimal integer |
| json_bytes | bytes:text | append a decimal JSON byte array |
| shape_begin | kind:u32 | reset traversal budget; 0 export, 1 clone |
| shape_enter / shape_leave | none | enforce depth 64 and 100000 expanded nodes |

Trace kind numbers are Allocate=0, Move=1, Borrow=2, EndBorrow=3, Drop=4,
Return=5. Buffer creation records Allocate itself. Generated typed traversal
flattens aggregate owners into their active buffer leaves for other events.
Free does not duplicate Drop, and returned owners stay live until report
statistics have been collected. A trap exits without running pending cleanup.

## Application versus captured harness

Mode 0 is an ordinary application whose runtime startup calls compiled
`main() -> I32`. It performs real standard output and returns the main result as
the process status. There is no default instruction, call, heap or output test
budget, no capture trace and no JSON report. An application trap writes a
diagnostic to stderr and exits 101.

Mode 1 is a compiled acceptance harness. LLVM code initializes typed fixed
arguments, directly calls the same compiled entry function, and traverses the
typed native result to serialize it with the encoding primitives above. The
runtime never computes the function result from IR or source. Successful
harnesses return zero; trapped harnesses exit 101. Execution JSON goes only to
the explicitly inherited report descriptor (fd 3). The report carries the same
public Value/Execution representation as the interpreter, including U64 decimal
strings, structured diagnostic code/entity, live allocations and lifecycle.

Print operations write actual stdout and record the exact successfully written
bytes. They add no newline. Captured traps add no diagnostic bytes to stderr;
their diagnostics are structured report fields. The host compares real process
stdout/stderr against the report to detect inconsistent capture. Ordinary OS
write failures produce the typed IoError result; captured output budget
exhaustion is an executor `E_RESOURCE_LIMIT` trap. Concat returns its declared
typed allocation error, while direct constants/clone allocation failures trap.

Budget limits match RFC0012. Guest heap counts live payload lengths. Capture
charges raw print bytes, compact JSON events with inter-event commas, and exact
generated Value JSON bytes including escaping. Fixed Execution envelope bytes
are not charged. All checks precede visible insertion into the capture buffer.
The harness owns returned allocations after report collection and releases them
when the context ends; guest trap paths deliberately do not unwind.

## Link driver and evidence

`il_link_driver::compile(llvm_ir, candidate_dir, runtime_archive, profile)` runs
only on Linux x86-64. A caller creates a fresh private candidate directory and
selects Debug or Release. No caller-supplied command strings or extra argv are
accepted. The tools are absolute `/usr/bin/opt-14`, `/usr/bin/llc-14`, and
`/usr/bin/clang-14`; each must report LLVM 14.0.6 and its actual file hash is
recorded. The runtime archive is an explicit, hashed build input produced by the
locked Cargo build, never implicitly rebuilt by the driver.

The fixed pipeline verifies LLVM, optimizes with O0/O2, emits a PIC x86-64 object,
and links the object and runtime with fixed platform libraries. The output ELF
header must identify little-endian, 64-bit, x86-64 executable/PIE. The linker
disables build IDs. Input/optimized LLVM, object, ELF, runtime and tools have
SHA-256 evidence; every actual command, argv, exit and output is recorded.
Important files are synced and the success report must persist before compile
returns success. Failures retain their candidate files and a failure report;
failure-report persistence errors are themselves reported to the caller.

`il-object-emitter` owns actual LLVM verification, optimization, object emission,
and validation of the ELF relocatable header. It also provides shared typed
artifact/command evidence and the bounded process runner. `il-link-driver`
consumes that emitted object, identifies the fixed linker, links the runtime,
validates the final executable, and handles captured execution. Neither stage
duplicates or forwards a hidden second object-emission implementation.

`run(executable, report_path, timeout)` executes the ELF without shell arguments,
with an empty environment and the report directory as cwd. A controlled child
setup maps the private report file to fd 3. Linux RLIMIT_FSIZE bounds report
growth at 8 MiB. Separate pipe readers limit stdout/stderr to 8 MiB each; excess
output or timeout kills and reaps the child. Timeout must be positive and at
most 60 seconds. A report must deserialize as Execution, match real output, and
agree with exit status 0/101. Missing/malformed/oversized reports and process
failures remain failures with preserved output evidence. The CLI publishes a
candidate only after these lower-level records satisfy its release gate.

## Validation boundary

Runtime contracts exercise real buffer layout and contents, empty allocation
identity, no application test quotas, concat allocation errors, exact JSON
encoding, lifecycle accounting, and subprocess trap/no-unwind behavior. Native
acceptance separately links/runs LLVM-generated applications and captured
harnesses in debug/release and compares interpreter semantics. P05 does not
claim cross-platform E03 ABI support or completed P06 resource APIs.
