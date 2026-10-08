---
rfc: 105
title: Hygienic deterministic graph macros
status: accepted
requires: [E01.const_eval]
extension: E01.controlled_macros
authority: user_approved_implementation_plan
---

Macros are typed graph-to-graph transformations evaluated by the bounded pure
constant evaluator. Invocation is explicit `macro_name!(...)`; arguments are
typed syntax fragments. There is no arbitrary token grammar extension or host
plugin loading. Macro code cannot execute processes, read files, access the
network, observe time, load environment variables or invoke shell commands.

Fresh names carry hygienic identity derived from macro declaration ID, invocation
entity ID and expansion ordinal. Caller name capture is possible only through an
explicit typed name argument. Expansion precedes normal semantic checking, and
generated entities retain an origin chain for diagnostics and text projection.
Macros never manufacture capability values or bypass transaction scope.

Limit nested expansion to 32, generated entities to 10000 per invocation, and
apply const evaluator operation/memory limits. Exceeding any limit produces
`E_RESOURCE_LIMIT`; malformed generated graphs fail ordinary validation.

Acceptance covers hygiene under caller renaming, stable IDs after formatting,
deterministic output, forbidden host operations, recursive expansion limits and
generated ownership/effect errors. Compare macro-expanded text and graph entry
points and retain the expansion provenance in build evidence.
