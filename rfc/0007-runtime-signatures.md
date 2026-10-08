---
rfc: 7
title: Closed runtime signatures and explicit runtime errors
status: accepted
requires: [P03]
authority: user_approved_implementation_plan
---

The static checker recognizes a closed ABI registry shared with later interpreter
and native runtime implementations. Recognition is a static contract, not a claim
that an executable runtime implementation already exists. Unknown symbols are
`E_UNSUPPORTED_FEATURE`. Core opaque resource APIs without complete contracts are
also rejected, rather than treating integer handles as owned resources.

`core.IoError` is an ordinary nominal Sum, with these payload-free variants in
order: `Read`, `Write`, `Closed`, `Timeout`, `PermissionDenied`, `InvalidData`,
`Other`. `core.AllocError` is an ordinary Sum with payload-free variants
`OutOfMemory`, `CapacityOverflow`. Their layout is inferred and they have no type
parameters. The checker verifies these exact shapes when a runtime signature uses
them. A result wrapper can have any stable ID, but its kind must be Result and its
parameters must match the success type and error type exactly.

| Symbol | Inputs | Output | Effects | Required capability |
| --- | --- | --- | --- | --- |
| print_i64 | I64 copy | Result<Unit, core.IoError> | process | none |
| print_string | String shared | Result<Unit, core.IoError> | process | none |
| string_len | String shared | Usize | none | none |
| bytes_len | Bytes shared | Usize | none | none |
| string_concat | String shared, String shared | Result<String, core.AllocError> | alloc | none |
| clock_now | none | U64 | clock | ClockRead |
| file_read | String shared | Result<Bytes, core.IoError> | fs, alloc | FileRead |
| file_write | String shared, Bytes shared | Result<Usize, core.IoError> | fs | FileWrite |

Printing is process I/O, not process creation; it does not grant or require
SpawnProcess. File capability path containment is checked at runtime against the
actual path; static checking additionally requires a matching trusted host grant.
Runtime calls cannot construct grants. Graph capability declarations only name
requirements. Host injection must match the declared entity ID, kind and scope.

`net_listen`, `net_connect`, `net_accept`, `net_read`, `net_write`, `net_close` and
`spawn_process` remain reserved unsupported symbols until the typed resource ABI
is implemented in its dependency stage. They cannot be invoked through a generic
FFI or unknown runtime symbol escape hatch.

Inline aggregate construction does not allocate. String/Bytes constants and owned
clone require alloc. Operation effects must equal inferred requirements; the
function declaration must cover them, and calls require the entire declared
callee effect set. `consumes` is the exact set of owned values moved or dropped;
`produces` is the exact ordered list of owned outputs, excluding borrowed views.
The checker infers both and rejects misleading metadata.

Borrowed views use the owner's TypeRef but are distinguished by loan analysis.
They cannot be returned, stored, transferred to another block, or passed to an
owned parameter. Explicit `end_borrow` ends the loan; live loans may not cross a
block terminator. Owned locals are implicitly dropped in reverse initialization
order when their lexical block ends, and all live owned parameters are dropped
on return. Field and tuple projections copy scalar fields; projecting an owned
field is an unsupported partial move. `payload` extracts an entire proven sum
variant and consumes its owned sum. A switch must explicitly enumerate every
variant; its mandatory default is an invalid-discriminant trap, not a wildcard
that hides a missing case.
