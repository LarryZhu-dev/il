---
rfc: 6
title: Independent static-semantic acceptance contracts
status: accepted
requires: [P02]
authority: user_approved_implementation_plan
---

## Decision

P03 acceptance invokes the built `il validate` executable with canonical graph
JSON and all checks: schema, names, types, ownership, effects, capabilities and
contracts. Mandatory reference validation remains enabled. The external runner
imports no compiler, checker, graph-store or parser implementation. Expected
validity and required diagnostic codes are committed in
`tests/fixtures/semantics/cases.json`; individual graph fixtures are independent
inputs. A failing expectation is not changed to accommodate an implementation
defect.

Positive cases cover fixed-width constants and arithmetic, typed calls, nominal
records and fields, explicit branch arguments, ownership moves, lexical borrows,
exhaustive sums, built-in Option/Result constructors and effect propagation.
Negative cases require stable diagnostics for wrong types, missing return paths,
non-exhaustive matches, moved resources, escaping borrows, double drops,
undeclared effects, missing host capabilities, unresolved names, bad branch
arguments, incompatible contracts and out-of-range integer literals.
Closed-schema regressions also reject explicit null for an optional integer
layout, omission of a required nullable capability scope, empty package/trap
strings, malformed parameter names and duplicate package module references.

Owned inputs transferred by move, call, aggregate construction, drop or return
appear exactly in `consumes`; newly produced owned values appear exactly in
`produces`. Scalars and aggregates with exclusively copy fields do not acquire
owned-value metadata. A borrowed value retains its underlying TypeRef but its
identity carries lexical borrow provenance; borrowing does not produce an owner.
Returning that borrowed identity fails `E_BORROW_ESCAPE`. An ended borrow restores
the original owner for a subsequent owned return.

A sum switch lists every declared variant explicitly. Its mandatory default
edge is an invalid-discriminant trap, not an implicit catch-all pattern that
makes omitted variants legal. Missing a variant produces
`E_NON_EXHAUSTIVE_MATCH`. A graph capability declaration describes a requirement;
it never grants host authority. The public validation request does not accept
injected capability objects from untrusted JSON.

## Robustness and evidence

Run every invalid contract twice and require identical structured diagnostics.
Run exactly 200 reproducible fuzz scenarios with seed `20261008`: 40 arbitrary
text requests, 40 truncated JSON requests, 60 generated concrete type graphs and
60 operation-graph mutations. The graph scenarios include recursive type shapes,
missing references, operand/type disagreement and unknown fields. Fuzz outcomes
may be accepted or rejected according to the generated graph; every invocation
must terminate within its budget, avoid crashes and return the structured CLI
envelope. Every rejected request includes structured diagnostics.

The report records each scenario, input or fixture hashes, actual process count,
elapsed time and failed-input reproducers. Fuzz robustness does not substitute
for the expected diagnostic and successful semantic contracts. P03 remains
unverified until these checks and its text frontend acceptance actually pass.
