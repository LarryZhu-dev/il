# core (P06)

`lib.il` defines the fixed `core.File`, `core.Deadline`, `core.IoError`, and
`core.AllocError` types. `File` is an opaque, non-cloneable owner. Only runtime
open operations create it; explicit close consumes it and implicit cleanup closes
it. Its declared fields describe ABI layout and cannot be accessed by il code.

`infinite()`, `deadline_at(U64)` and `max_i64(I64,I64)` are pure il functions.
A deadline is an absolute monotonic nanosecond instant. Reading that clock still
requires an explicit trusted `ClockRead` grant through `runtime.clock_now`.

Compose this source before the alloc/io sources and the application. These are
ordinary checked modules; the package resolver is introduced in E04.
