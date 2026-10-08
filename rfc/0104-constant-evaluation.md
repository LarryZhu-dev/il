---
rfc: 104
title: Bounded pure compile-time evaluation
status: accepted
requires: [E01.floats]
extension: E01.const_eval
authority: user_approved_implementation_plan
---

`const` expressions execute verified MIR using the interpreter semantics.
Only pure operations and other const functions are callable. All effects,
capabilities, runtime handles, raw pointers and host/environment access are
forbidden. Bounded evaluator-owned memory is an implementation resource, not
permission for source-level `alloc` effects.

Each root evaluation permits 1000000 MIR operations, recursion depth 128 and
16 MiB of evaluator storage. Limits are deterministic, count nested evaluation,
and fail with `E_RESOURCE_LIMIT`; they cannot fall back to runtime execution.
Integer traps are compile-time diagnostics with the originating entity ID.
Memoize by function/argument/type/spec hashes without changing budget accounting.
Use the same numeric semantics and canonical NaN rules as runtime evaluation.

Acceptance covers valid aggregates and calls, effect rejection, loops and
recursive exhaustion, arithmetic traps, identical values across profiles and
deterministic diagnostics independent of cache state.
