---
rfc: 102
title: Static trait bounds and coherent implementations
status: accepted
requires: [E01.generics]
extension: E01.traits
authority: user_approved_implementation_plan
---

Traits declare named function signatures with explicit effects and capabilities.
`impl Trait for Type` supplies every required method; generic parameters use
`T: Trait` bounds. Dispatch resolves before native lowering and monomorphizes
concrete methods. No trait objects, vtables, specialization or runtime reflection.

Exactly one applicable implementation exists for a trait/type pair. A package
may implement a trait only when it owns the trait or concrete outer type.
Overlapping generic implementations, missing methods and incompatible parameter,
result, effects or capability signatures are errors. Trait methods cannot
silently introduce effects absent from their declaration. Associated types and
default methods are out of the initial extension; unsupported syntax is rejected.

Acceptance covers successful static dispatch, multiple bounds, missing/overlapping
implementations, orphan violations and hidden effects. Compare native and
interpreter behavior and deterministic specialization across clean builds.
