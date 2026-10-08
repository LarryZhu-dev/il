---
rfc: 101
title: Statically monomorphized type and function parameters
status: accepted
requires: [P10]
extension: E01.generics
authority: user_approved_implementation_plan
---

Generic declarations bind named type parameters with `fn name<T>` or
`record Name<T>`. Use explicit concrete type arguments; omitted arguments may
be inferred only from uniquely determined typed operands. Ambiguous inference
is an error. MVP built-in Option/Result retain their existing semantics.

Each reachable concrete instantiation gets a deterministic specialization keyed
by declaration ID and ordered concrete type IDs. Share identical instances;
reject polymorphic recursion that creates unbounded specializations with
`E_RESOURCE_LIMIT`. The limit is 1024 instantiations per compilation and nesting
depth 64. Check generic bodies and recheck substituted operations. Ownership,
effects and capability constraints are preserved after substitution.

No type erasure, runtime dispatch, implicit boxing or new backend fallback is
introduced. Generic IDs and parameters are represented in the canonical graph
and text. Extend closed schemas and retest all graph/text paths together.

Acceptance covers nested records, recursive nongrowing calls, distinct argument
orders, deterministic specialization, inference ambiguity, ownership errors and
the specialization budget. Interpreter and native results must agree.
