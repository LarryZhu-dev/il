---
rfc: 14
title: Memory-explicit native IR and direct LLVM compilation
status: accepted
requires: [P04]
authority: user_approved_implementation_plan
---

P05 lowers verified MIR to an independent, serialized Native IR containing
fixed-width integers, pointers, allocation extents, byte offsets, aligned loads
and stores, memory copies, calls, explicit branches and entity debug locations.
It retains no executable graph or MIR. Its immutable input hash binds the MIR
hash and the application or captured build mode, including captured arguments
and budgets. Native verification examines the supplied instructions, layouts,
function signatures, control flow, SSA definitions, dominance and the closed
runtime ABI. Atomic instructions are introduced and verified with E02.

Indirect parameter minimum extents, alignment and mutability requirements are
inferred from memory uses and propagated backward through native calls. A call
must supply its current runtime context and sufficiently large, correctly
aligned storage. Raw byte constructors and JSON primitives take proven static
extents; dynamic owned buffers use registered-buffer clone/encoding entrypoints
that validate their allocation identity before accessing memory.

The first physical ABI is exactly `spec/abi.yaml`: 8-bit Bool, declared-width
integers, three-word owned UTF-8 String and Bytes buffers, declaration-order
record fields, tuple positions, and u32 tagged sums with a maximally aligned
payload. Variant tag numbers follow declaration order. Option and Result use
this same layout with no niche optimization. Padding is initialized. Aggregates
are passed indirectly, with an explicit output pointer. Every compiled function
receives the runtime context first, then the aggregate result pointer when
required, followed by source parameters. Unit occupies zero bytes and has no
physical return value. Symbols use hex-encoded stable entity IDs.

Graph SSA values have entry-block stack slots. Every selected CFG edge first
captures all outgoing values, then performs its verified ordered cleanup, then
writes successor arguments. This preserves parallel assignment in loops and
ensures cleanup cannot destroy a value before it is transferred. Returns copy
their result before cleanup. Move, borrow, end-borrow, allocation, drop and
return events use source entity IDs and flatten active owned aggregate fields
in declaration order. Traps never unwind or execute ordinary cleanup.

Native compilation limits a function's explicit frame to 32768 bytes, a type's
physical layout to 64 MiB, and emitted IR to 500000 instructions. Exceeding a
limit rejects compilation; it never silently switches to interpretation.
Opaque host handles and capability-dependent runtime operations are rejected
until their real runtime ABI is available in P06. This is an explicit feature
boundary, not an executable placeholder.

Checked arithmetic is a Native IR operation with language-defined failure
semantics. LLVM emission expands it to real guard branches: add/sub/mul use
overflow intrinsics; div/rem test zero and signed MIN/-1 before LLVM division;
shifts validate the shift count before execution, and left shift uses a double
width intermediate plus a range test; casts compare mathematical values in
i128 before truncating. No `nsw`, `nuw`, `inbounds` or unchecked division/shift
is used to assume the answer. Debug and optimized builds share these rules.

The application entry is `() -> I32`. A captured build instead generates a
specialized LLVM entry wrapper that imports the validated typed arguments into
their physical ABI, calls the actual compiled function, and serializes its
typed result using generated LLVM control flow. Runtime JSON primitives only
encode bytes and scalar values; they cannot read graphs, MIR or opcodes. A
separate inherited report descriptor carries the execution record, while
stdout/stderr remain real process streams. Captured execution uses the same
step, depth, allocation, output and value traversal limits as the interpreter.

P05 supplies real allocation, buffers, concatenation, stdio, traps and captured
evidence as the permanent native runtime foundation. P06 extends that runtime
with resource handles, deadlines, host capabilities and runtime modes. There is
no alternate Rust frontend or interpreter path hidden inside a native binary.

Every LLVM instruction has an entity mapping. Available debug lines refer to
the generated `program.ll`; they do not fabricate original `.il` source spans.
The object emitter verifies emitted LLVM before producing machine code. Build
stage records bind Native IR, LLVM, object and ELF bytes with actual hashes.
