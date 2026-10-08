---
rfc: 9
title: Explicit module ownership and imported declaration visibility
status: accepted
requires: [P03]
authority: user_approved_implementation_plan
---

Every function and nominal type has exactly one owning module, recorded in that
module's declarations list. Orphan top-level declarations are not an alternative
global namespace. Built-in scalar, String and Bytes type names are globally
available and do not require graph declarations.

A declaration may reference another declaration in its own module. To reference
a function or type from a different module, the source module must directly
import the target's owning module, and the target module must be public. Imports
are not implicitly transitive and do not re-export declarations. Importing a
foreign private module is rejected even when no declaration is subsequently
referenced. These rules apply to calls, signatures, block values, and every
nominal type used in another type's fields, variants or parameters.

Graph references remain stable entity IDs. IDs do not bypass module visibility.
Same-module display names for functions must be unique; function names can repeat
in distinct modules because the graph references their distinct logical IDs.
The primitive type namespace remains reserved.

Missing module ownership, an unavailable private module, and a missing explicit
import produce `E_NAME_NOT_FOUND`. This avoids exposing a separate successful
lookup path for inaccessible declarations. Duplicate ownership remains a
structural graph error. Core runtime error types are ordinary nominal types and
obey the same rules; a core module must be explicitly imported when separate.

Validation covers cross-module calls with and without imports, private imports,
transitive imports, orphan declarations, imported record fields and signatures,
and preservation of valid same-module source and graph fixtures.
