---
rfc: 2
title: Stable text projection and core operation vocabulary
status: accepted
requires: [P02]
authority: user_approved_implementation_plan
---

## Decision

Every module, declaration, block and operation has an explicit immutable logical
ID. Creation accepts deterministic names such as `app.main.entry.return`; rename
changes a display name, never identity. IDs are globally unique within a graph.
They never depend on source location, random values or in-memory addresses.

The `.il` format uses braces and mandatory semicolons, explicit result types and
explicit `return`. Canonical projection places `@id("...")` before entities,
including blocks and statements. An editor may omit IDs only for newly added
entities: import assigns namespace-qualified monotonic creation counters from
the transaction base and returns the annotated projection. Such allocation is
serialized with the transaction and is deterministic for the same base/input.
Existing text without a recoverable ID is not matched by line number or name.

```il
@id("app") module app {
  @id("app.main") fn main() -> I32 effects [] capabilities [] {
    @id("app.main.entry") block entry {
      @id("app.main.zero") let zero: I32 = 0;
      @id("app.main.return") return zero;
    }
  }
}
```

The core graph vocabulary is `const`, `add`, `sub`, `mul`, `div`, `rem`, `shl`,
`shr`, `bit_and`, `bit_or`, `bit_xor`, `eq`, `ne`, `lt`, `le`, `gt`, `ge`, `not`,
`cast`, `call`, `record`, `field`, `tuple`, `tuple_get`, `variant`, `tag`,
`payload`, `move`, `clone`, `borrow`, `borrow_mut`, `end_borrow`, `drop`,
`branch`, `cond_branch`, `switch`, `return`, `trap` and declared `runtime_call`.
Each opcode has a closed per-opcode attribute schema; unimplemented operations
produce `E_UNSUPPORTED_FEATURE`, unknown operations produce `E_SCHEMA_INVALID`.
Values have a single definition; branches pass typed block arguments. Exactly
one terminator ends each block. Calls identify functions by stable entity ID.

Text parsing creates a candidate graph; all names, types, ownership, effects,
capabilities and contracts use the same checking pipeline as graph input. Only
an accepted transaction publishes text changes. Formatting preserves IDs and
semantics. String literals use JSON escapes and strict UTF-8. Surface `if`,
`match`, loops and `?` lower to explicit block control flow. No implicit return,
numeric conversion or implicit owned-value clone is introduced.

## Validation

Round-trip annotated text and canonical graph; preserve identity on rename and
reformat. Check stale import, duplicate IDs, unknown opcodes, use-before-definition,
missing terminators and all semantic errors through both entry points.
