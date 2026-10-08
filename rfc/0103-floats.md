---
rfc: 103
title: Explicit IEEE floating point types
status: accepted
requires: [E01.traits]
extension: E01.floats
authority: user_approved_implementation_plan
---

Add `F32` and `F64` with IEEE 754 binary32/binary64 representation, round-to-nearest
ties-to-even and no implicit conversion from integers or between widths. Floating
literals default to F64. Disable fast-math and contraction; source arithmetic
does not fuse multiply/add. Comparisons are ordered except `!=`, which is true
for NaN. Division by floating zero yields signed infinity or NaN, not an integer
division trap. Preserve signed zero; canonicalize arithmetic NaN results to one
quiet positive NaN per width for reproducible execution.

Explicit integer-to-float conversion rounds; finite float-to-integer truncates
toward zero and traps `E_INTEGER_OVERFLOW` when out of range, NaN or infinite.
Graph constants use exact fixed-width hexadecimal bit strings, avoiding JSON
nonfinite numbers and decimal round-trip ambiguity. Text supports typed finite
literals and explicit bit constructors for nonfinite test cases.

Acceptance covers signed zero, subnormals, infinities, NaN comparisons, conversion
boundaries and rounding in interpreter/debug/release. Reproducibility checks
compare exact result bits, not approximate formatted decimal output.
