---
rfc: 5
title: Annotated text grammar and lossless graph projection
status: accepted
requires: [P02]
authority: user_approved_implementation_plan
---

## Core syntax

Source is UTF-8 `.il` text. Identifiers are ASCII names; JSON-escaped quoted
names can represent every logical identifier. `//` line comments are discarded.
Braces delimit declarations and blocks; operations and statements end in `;`.
`@id("stable.id")` annotates modules, types, functions, parameters, blocks,
operations, and values. Missing annotations allocate `owner.new_rR_N`, where R
is the candidate's base revision and N is a monotonically increasing parse-local
counter. Existing IDs are never inferred from a display name or source position.
The formatter annotates every entity, so formatting never reallocates identity.

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

Modules may declare `visibility private|public`, `imports ["module.id"]`, and
`declarations ["declaration.id"]` before their body. Nested type and function
declarations append to their containing module. Canonical formatting emits
module membership explicitly and emits types and functions as top-level
declarations; this preserves each graph array's original order exactly.

Function signatures contain annotated parameters, an explicit `-> Type`, and
optional `effects`, `capabilities`, and `contracts` lists. Bodies contain explicit
annotated `block name(args) { ... }` blocks or a sequence of readable statements
in an automatically created entry block. Parameters are available in every block;
other values cross blocks only through explicit typed block arguments.

## Lossless operation and declaration forms

All graph operation fields have one canonical spelling:

```il
@id("add") op add inputs ["left", "right"]
  outputs [@id("sum") I32] attributes {} effects [] consumes [] produces [];
```

The opcode is one of the closed graph vocabulary. Inputs/consumes are stable ID
lists; outputs/produces are annotated type lists. Attributes are strict JSON
objects validated against the opcode's typed attribute definition. Duplicate JSON
keys, unknown fields, and unknown opcodes are rejected. Explicit core operations
never infer or silently replace effects, ownership, or branch metadata.

Types have both readable declarations and an exact field form:

```il
@id("Pair") type Pair = record { first: I32, second: I32 };
@id("Maybe") type Maybe = Option<I32>;
@id("Outcome") type Outcome = sum { Ok(I32), Err(String) };
@id("Pair") type Pair kind record parameters [] layout inferred integer null
  fields [{"name":"first","type":"I32"},{"name":"second","type":"I32"}]
  variants [];
```

The exact type form preserves all supported `TypeDef` fields. `Tuple<T...>` and
`Result<T,E>` are built-in parameterized declarations, not user generics. Aliases
for primitive types create the corresponding type definition and integer layout.
Capabilities, packages, and contracts have explicit field declarations matching
their graph schemas, including scopes, package capabilities, and predicates.

## Readable statements and validation

`let name: Type = expression;` defines one value; its default stable value ID is
`operation.id.value`. Expressions support typed literals, arithmetic and integer
comparisons, unary `not`, explicit `cast<T>(value)`, and calls. Owned-value moves,
clone/drop/borrow operations and runtime calls remain explicit operations when
their complete metadata cannot be inferred from declarations. No operation is
silently treated as a no-op. Unsupported syntax returns `E_UNSUPPORTED_FEATURE`.

`return`, `branch`, `cond_branch`, and `switch` have explicit core terminators.
Structured `if`, `while`, `match`, and `?` surface lowering is the immediately
following P04 HIR milestone, where the execution document explicitly eliminates
syntactic sugar into MIR. All four forms remain mandatory for MVP completion.
P03 verifies the complete explicit block/SSA text projection and shared semantic
checker. Until P04 implements a surface form, P03 rejects it explicitly rather
than accepting an incomplete graph. P03 acceptance does not claim MVP completion.

Parsing only constructs a candidate. It does not publish a revision, run a
program, or claim semantic validation. The shared semantic checker validates
both textual and graph candidates before the transaction layer may commit them.
Source is limited to 1 MiB, 131072 tokens, and nesting depth 32. Malformed input
produces stable structured diagnostics with byte offsets and never partial success.

## Acceptance

Round-trip complete typed graph fields and arbitrary stable IDs through canonical
text; preserve module membership and list ordering. Parse readable integer and
record examples. Reject malformed UTF-8 escapes, duplicate JSON keys, missing
terminators, unknown operations, excessive nesting and unsupported syntax. Run
deterministic malformed-input fuzz cases without panic.
