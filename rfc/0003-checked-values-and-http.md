---
rfc: 3
title: Checked integers, runtime boundary and HTTP failure contract
status: accepted
requires: [P03]
authority: user_approved_implementation_plan
---

## Decision

All fixed-width integer add/subtract/multiply and explicit narrowing conversions
trap on overflow. Signed minimum divided by -1, and its remainder operation,
trap on overflow. Division by zero traps. Shift counts below zero or at least the
value width trap; left shift traps when the mathematical result does not fit.
Signed division truncates toward zero and signed right shift is arithmetic.
Debug, release, interpreter and constant evaluation implement the same rules;
LLVM poison and undefined behavior must never implement language-level errors.

Strings are owned valid UTF-8; bytes may contain arbitrary octets. Invalid UTF-8
conversion returns a typed error. Heap allocation and resource operations return
typed `Result`; irreversible panic/trap emits a stable diagnostic and exits 101.
Panic does not unwind. Every ordinary return and `?` error edge drops all live
owned locals in reverse initialization order. Runtime FFI uses the ABI in
`spec/abi.yaml`, with no unwinding across the boundary.

HTTP rules and exact limits are normative in `spec/http.yaml`. The parser accepts
one origin-form HTTP/1.1 request with CRLF and a single Host header, then closes.
Any Transfer-Encoding or Upgrade is rejected. Duplicate Content-Length is 400
even when values agree; a single positive length is 413. Bodies are never read
as a second request. Percent decoding occurs exactly once within each segment;
bad escapes, invalid UTF-8, decoded NUL or slash are 400. Query text is excluded
from matching. Parameter length is measured after decoding in UTF-8 bytes.

Routing compares full segment patterns. Patterns that can match the same path
and method are rejected rather than resolved by declaration order. Unknown paths
return 404 and known GET-only paths with another method return 405 with Allow.
CONNECT returns 405. Error responses have empty bodies and explicit length.
Read and write deadlines are absolute phase deadlines; fragments cannot extend
them. Partial writes retry the remaining bytes. Disconnect and write timeout
close and release resources with structured diagnostics; they do not restart the
process or leak a connection. Independent TCP tests read the contract file, not
implementation tables. The HTTP layer is a package, never a compiler keyword.

## Validation

Exercise each integer boundary in interpreter/debug/release and UTF-8 conversion
failure. Send fragmented, conflicting-length, malformed, oversized and timed-out
requests over real TCP; inject partial writes, disconnect and allocation failure.
