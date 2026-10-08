---
rfc: 11
title: Structured text, SSA lowering, and explicit failure propagation
status: accepted
requires: [P03]
authority: user_approved_implementation_plan
---

## One frontend pipeline

The frontend parses declarations and complete surface ASTs before lowering any
function body. All function signatures are available during expression lowering,
including forward calls. Integer argument literals use the declared parameter
type; casts retain the source expression type. Canonical explicit operations use
the same AST path and preserve their metadata exactly. The replaced direct
statement/expression lowering implementation is removed.

Function signatures and local annotations use named type references. Built-in
parameterized types are instantiated by named declarations such as
`type Output = Result<I32, Error>;`; inline signature applications and user type
parameters are deferred to E01.

Every generated entity ID is derived from its annotated statement/function and
a deterministic creation counter. Generated runtime Result types include the
owning function ID, so distinct functions cannot collide. The formatter emits
the final explicit graph with every generated ID annotated.

## Statements and control flow

`if condition { ... } else { ... }` requires Bool; `else if` is nesting.
`while condition { ... }`, `break;`, and `continue;` lower to header/body/exit
blocks and explicit back edges. Mutable bindings use `let mut name: Type = expr;`
and assignments use `name = expr;`. Assignment evaluates its RHS before dropping
an overwritten owned value. Immutable assignment and duplicate live names fail.
There are no implicit function returns, including Unit functions.

`match value { Tag(bindings) => { ... }, Other => { ... } }` is a statement;
arm bodies use explicit `return` when returning from the function. Every legal
variant is covered. A final `_` arm explicitly covers remaining variants without
binding payload fields; `_` payload fields are discarded, releasing owned data.
The generated switch's default traps only for an invalid discriminant. The whole
sum is decomposed once, never partially moved.

Structured functions transfer parameters through a prologue into body block
arguments. SSA environments map stable binding identities to current values,
so assignment, joins, loops, and error propagation cannot retain stale cross-block
IDs. Edges carry each owner once; mutually exclusive edges may each carry it.
Joins reject inconsistent live ownership. Borrowed views cannot cross edges.
MIR supplies reverse-initialization-order cleanup on normal and error edges;
trap paths do not unwind.

## Expressions and runtime boundaries

Constructors use `Type::Variant(args)` or contextual `Ok(...)`, `Err(...)`,
`Some(...)`, and `None()`. Records use `Type { field: expression, ... }`; field
expressions evaluate in source order, while the resulting storage uses declaration
order. Tuple construction is contextual `tuple(...)`.

`expression?` evaluates its operand once, switches Result into Ok/Err, extracts
the active payload, and propagates Err only when the enclosing Result has exactly
the same error type. The successful payload continues in a fresh SSA block.

`runtime.symbol(args)` uses the shared closed runtime signature registry.
Ownership operations are `move`, `clone`, `borrow`, `borrow_mut`, `end_borrow`,
and `drop`; they use shared checker ownership classification. The compiler never
adds capabilities. Missing standard runtime error types are materialized as the
registry-defined public core sum types; generated Result types belong to the
calling module, whose explicit graph imports record the public core dependency.

Templates use `f"hello ${name}"`. Literal fragments and interpolation expressions
evaluate left-to-right, once each, before concatenation. Interpolations must be
String; there are no implicit numeric or other conversions. Concatenation folds
left-to-right through `string_concat`, with the first allocation failure becoming
the expression's Err. The template type is `Result<String, core.AllocError>`;
callers may inspect it, return it, or apply `?`. All success and failure paths
retain explicit ownership and cleanup information.

## Acceptance

Execute lowered source through checked HIR and verified MIR: both if outcomes,
zero/multiple loop iterations, mutable loop state, break/continue, exhaustive
matching, forward I32 calls with literals, successful and error `?`, template
success and bounded-allocation failure. Check exact returned values, type errors,
stable formatting, source-order record evaluation, and no leaked owned locals.
Malformed input and excessive nesting continue to produce structured diagnostics.
The parser bounds syntax nesting and expression AST depth to 32, including flat
binary and postfix chains and `else if` chains, before recursive lowering begins.
