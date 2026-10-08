---
rfc: 10
title: Checked HIR and explicit cleanup MIR
status: accepted
requires: [P04]
authority: user_approved_implementation_plan
---

HIR carries the checked canonical graph after the text frontend has eliminated
surface syntax. Only the closed graph operation vocabulary is permitted; no
implicit returns, question operators, template strings or surface control flow
remain. Lowering validates the actual graph and binds its canonical SHA-256 to
the HIR input hash. Serialized HIR is not a trusted bypass of semantic checking.

MIR has typed functions, typed block arguments, a nonterminator operation list,
and distinct Return, Trap, Branch, CondBranch and Switch terminators. Borrow,
end_borrow, move, explicit drop, error-variant construction and calls remain
explicit operations. A Result error edge uses the same typed switch and block
argument representation as every other control-flow edge. MIR retains types and
capability requirements but has no executable source-graph fallback.

Each branch edge and ordinary return owns an explicit ordered cleanup list of
`{value_id, type_ref}` actions. Transfers of the returned value or selected edge
arguments occur first. Cleanup then releases every remaining live owned local in
reverse initialization order. Parameters remain live across blocks until moved
or explicitly dropped; on ordinary return they are cleaned after block locals,
in reverse parameter order. Branch cleanup never releases a still-live function
parameter. Different edges can move different values, so cleanup is per edge and
is never unconditionally executed before choosing a successor. A trap terminates
without unwinding and has no cleanup list. Loan views are never owned cleanup
targets and cannot escape across control flow.

The MIR verifier consumes the supplied MIR rather than an original graph. It
validates the current operation/type/control-flow view through the shared static
rules, then separately infers owner transfers from opcodes and signatures and
checks explicit cleanup on each reachable path. It does not trust consumes or
produces annotations to establish cleanup. Missing cleanup, extra/dead cleanup,
double drop, incorrect types, wrong reverse order, invalid edge arguments and
borrow escape are rejected. The verifier has deterministic size limits and
iterative control-flow traversal.

Default HIR/MIR APIs have no host grants. Explicit `_with_capabilities` APIs take
trusted host injections separately; a capability declaration in an IR payload
cannot grant itself authority. Runtime execution must independently enforce its
host capability boundary.

Stage records are separate from serialized IR to avoid output-hash
self-reference. Each records stage, actual canonical input/output hashes,
compiler version and diagnostics. Successful lowering returns verified IR;
failure returns structured diagnostics without a runnable candidate.

Additional diagnostics are `E_MIR_MISSING_DROP`, `E_MIR_INVALID_CLEANUP` and
`E_MIR_CLEANUP_ORDER`. Existing ownership, type, schema and resource-limit codes
remain applicable. Tests mutate otherwise-valid MIR to omit/duplicate/reorder
cleanup and move it to the wrong branch, preserve returned owners, validate
explicit-drop interaction and verify trap behavior and canonical stage hashes.
