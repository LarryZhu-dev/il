# RFC 0012 — Deterministic verified-MIR interpreter

Status: Accepted for P04 implementation; acceptance remains gated by recorded tests.

## Boundary

`il_interpreter::execute(program, entry, arguments, limits)` accepts MIR 1.0.0,
an exact stable function ID, typed values, and explicit budgets. It first invokes
the independent MIR verifier. Malformed MIR, missing entry, invalid budgets,
argument arity/type/shape errors, and argument heap totals over budget return
`rejected` with zero steps, output, allocations, and lifecycle events. External
arguments cannot fabricate opaque resource handles. P04 injects no host grants.

Execution reads MIR operations and terminators directly. The interpreter never
executes a reconstructed graph and never invokes shell commands, host files,
network, or clock. Unsupported physical runtime operations remain unavailable;
capability checking can reject them before execution. This is not a claim of
P06 runtime support.

## Values and checked arithmetic

The public closed JSON value is `{ "type": "I64", "data": { "kind":
"integer", "value": "42" } }`. Decimal integers are strings to preserve U64
precision. They must equal their canonical decimal rendering: no plus sign,
leading zeros, whitespace, or negative zero. Bool, UTF-8 String, Bytes, Unit,
Record, Tuple and tagged Variant are distinct representations. Unit data has
only `kind`; `value: null` is rejected. Record and tuple fields follow declaration
order. Sum tags are names; `tag` returns their zero-based declaration index.
Option variants are None/Some; Result variants are Ok/Err. Refined `payload`
extracts the full declared field list, consuming an owned container.

I8/I16/I32/I64/U8/U16/U32/U64/Usize arithmetic uses the declared width. Overflow
traps with `E_INTEGER_OVERFLOW`; division and remainder by zero trap with
`E_DIVIDE_BY_ZERO`. Signed minimum divided or remaindered by -1 overflows.
Division truncates toward zero. Shift amounts outside `[0,width)` trap with
`E_INVALID_SHIFT`; left shift also checks the mathematical result against the
declared range. Signed right shift is arithmetic; unsigned right shift is
logical. Unsigned complement masks to the declared width. Casts are explicit
and checked. These contracts do not vary with Rust debug/release settings.

## Control flow and ownership

Each MIR operation, terminator and inserted cleanup action costs one step.
Only the selected edge transfers arguments. Owned arguments are removed from
the current frame; then the edge's explicit cleanup executes in its listed
order. Parameters still live after the transfer persist across blocks; block
locals do not. The target block receives transferred values under its own IDs.
Return transfers the result before executing explicit cleanup. Unit calls have
no output binding and Unit returns have no input operand.

Borrow and borrow_mut are non-owning views of the same managed allocation.
EndBorrow closes the view; Clone explicitly duplicates owned payloads. Copy
value movement/drop does not consume the value. Managed String/Bytes leaves
receive monotonically increasing allocation IDs, including empty payloads.
Aggregates carry their constituent IDs and do not implicitly allocate a heap
payload. Drop frees exactly those leaves once. Returned owned values remain
live under the host caller's ownership, reflected by `live_allocations`.

Lifecycle events record allocate/move/borrow/end_borrow/drop/return, stable
operation/value IDs, and allocation IDs. Explicit or arithmetic traps and
executor budget exhaustion do not unwind or run unvisited cleanup. Partial
captured output and live allocation evidence remain visible on `trapped`.
Every interpreter diagnostic identifies the offending entity and has stage
`interpreter`; arithmetic traps identify the arithmetic operation itself.

## Captured runtime

`print_i64` and `print_string` append exact bytes, with no implicit newline,
and return the declared `Result<Unit,core.IoError>::Ok(Unit)`. The captured sink
has no physical I/O failures. `string_len`/`bytes_len` count bytes, including NUL.
`string_concat` returns a newly allocated UTF-8 String inside its declared
Result wrapper. A length overflow yields CapacityOverflow; insufficient guest
heap capacity yields OutOfMemory. This bounded captured allocator behavior does
not replace future P06 real allocator/OS error contracts.

Direct managed constants/clones and instruction, call, capture or materialized
value budget exhaustion trap with `E_RESOURCE_LIMIT`. A future host allocator
or sink's recoverable errors continue through their typed Result interfaces;
executor limits are distinct from physical host failures. Captured stderr is
empty; structured diagnostics carry failures.

## Budgets and determinism

All four budget fields are required integers and reject unknown fields/null:

| Budget | Default | Inclusive permitted range |
| --- | ---: | ---: |
| max_steps | 100000 | 1–1000000 |
| max_call_depth | 128 | 1–128 |
| max_heap_bytes | 16777216 | 1–67108864 |
| max_output_bytes | 1048576 | 1–1048576 |

Guest heap counts live String/Bytes payload bytes. Calls count the entry as
depth one. Input, clone and materialized output shapes allow depth at most 64
and at most 100000 expanded nodes. Copy values use shared immutable internal
payloads; output materialization independently charges expanded nodes, avoiding
exponential serialized output from shared aggregate values.

Capture budget jointly counts raw stdout bytes, compact JSON lifecycle event
bytes including inter-event commas, and compact JSON returned Value bytes.
Fixed envelope fields and empty lifecycle array brackets are not charged.
Event charging precedes insertion. Value charging uses exact per-node encoded
overhead, JSON escaping, separators and decimal byte-array widths before
materializing content. Resource exhaustion cannot produce unbounded traces.

Given identical verified MIR, entry, arguments and limits, execution results,
step counts, allocations, lifecycle and diagnostics are deterministic. The CLI
binds that full execution input bundle and result hash into stage evidence.

## Acceptance

`tests/interpreter/contracts.rs` contains independent integer boundary tables,
explicit casts, copy/owned lifetimes, selected-edge transfers, loops, recursion,
budget failures, captured UTF-8/NUL output, typed concat allocation errors,
malformed-MIR rejection and strict serde wire tests. Additional independent CLI
acceptance compares exact results and locked fixture contracts. Testing occurs
in the pinned Rust/LLVM container; module completion is not phase verification
until the parent gate records all required reports.
