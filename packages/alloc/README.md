# alloc (P06)

Compose `core/lib.il` and this source. The module provides `concat`,
`clone_string`, `clone_bytes`, `string_size`, and `bytes_size` as il functions.
String sizes count UTF-8 bytes. `concat` returns `StringResult` and preserves
allocation errors; explicit clone can trap on allocation failure.

All String/Bytes function parameters transfer ownership to the function. The
functions release their inputs on return. In particular, `clone_string` and
`clone_bytes` consume the supplied owner and return a distinct allocation. To
retain an original in the caller, use the language's `clone(value)` expression.
No borrowed function-parameter API is claimed in P06. Runtime primitives borrow
locally, and implicit cleanup releases consumed parameters.
