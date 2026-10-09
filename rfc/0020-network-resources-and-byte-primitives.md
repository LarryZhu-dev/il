# RFC 0020: network resources and byte primitives

Status: Accepted. Authority: the approved implementation plan authorizes missing
semantics. Stage: P07; prerequisite P06 is VERIFIED. This RFC extends Full only.
HTTP remains an il package; no HTTP parser, router or handler evaluator belongs
in the host runtime. `spec/http.yaml` remains the locked HTTP contract.

## Resources and authority

The closed resource kinds are File, Listener and Stream. Their nominal types are
`core.File`, `net.Listener`, `net.Stream`; each is an opaque record with exactly
`slot: U64, generation: U64`, ABI size 16/alignment 8. They cannot be constructed,
cloned, inspected or supplied as captured inputs. All runtime operations validate
both generation and kind. Destruction recursively requires fs for File and net
for Listener/Stream, including both when an aggregate contains both kinds.
Passing ownership without destruction needs no cleanup effect. Explicit close
consumes its argument, even on error; shared operations preserve ownership.

Listen and Connect grants use canonical numeric IPv4 `address:port` or bracketed
IPv6 `[address]:port` scopes, with ports 1..65535. DNS names, port zero, leading
zero port spellings and noncanonical address spellings are invalid. The grant
itself selects the entire endpoint; guest code cannot substitute another one.
Graph declarations exactly match the launcher's grants. Connecting uses a
nonblocking socket and checks SO_ERROR after poll. Accepted sockets are
nonblocking and close-on-exec. No fallback to blocking or unscoped operations.

`net_listen[grant]()` returns Result<net.Listener,core.IoError> (net, Listen).
`net_connect[grant](deadline:core.Deadline)` returns
Result<net.Stream,core.IoError> (net, Connect).
`net_accept(listener:shared net.Listener, deadline:core.Deadline)` returns
Result<net.Stream,core.IoError> (net).
`net_read(stream:shared net.Stream, maximum:Usize, deadline:core.Deadline)`
returns Result<Bytes,core.IoError> (net, alloc).
`net_write(stream:shared net.Stream, bytes:shared Bytes, offset:Usize,
deadline:core.Deadline)` returns Result<Usize,core.IoError> (net).
`net_opened_at(stream:shared net.Stream)` returns U64 (net): the monotonic
timestamp captured immediately after successful accept/connect, before publishing
the owned handle. HTTP computes its read deadline from this stored timestamp.
`net_close_listener(owned net.Listener)` and `net_close_stream(owned net.Stream)`
return Result<Unit,core.IoError> (net).

Reads and writes are partial; EOF is an empty byte array. Writes suppress
SIGPIPE with MSG_NOSIGNAL. Deadlines are absolute monotonic nanoseconds and are
not reset on EINTR, EAGAIN, fragments or retries. Resource budgets, captured I/O
faults and opened/closed/dropped reports apply equally to sockets and files.
Socket kind misuse reports InvalidData; stale/closed generations report Closed.
Application bind/connect failures are IoError values, never silent retries on
another address. Backlog is 128; the MVP HTTP loop serves sequentially.

## General byte operations

All input buffers below are shared; all outputs own their newly allocated data.
No operation implicitly changes String to Bytes or validates bytes as UTF-8.

| Symbol | Parameters | Result | Effects |
| --- | --- | --- | --- |
| bytes_get | Bytes, Usize | U8 | none |
| bytes_slice | Bytes, Usize start, Usize length | Result<Bytes,core.IoError> | alloc |
| bytes_concat | Bytes, Bytes | Result<Bytes,core.IoError> | alloc |
| bytes_from_u8 | U8 | Result<Bytes,core.IoError> | alloc |
| string_to_bytes | String | Result<Bytes,core.IoError> | alloc |
| string_from_utf8 | Bytes | Result<String,core.IoError> | alloc |
| bytes_equal | Bytes, Bytes | Bool | none |
| string_equal | String, String | Bool | none |

bytes_get out of bounds traps E_INDEX_OUT_OF_BOUNDS. Slice checks start and
length without integer wrap, returning InvalidData for invalid bounds. UTF-8
decoding is strict (no replacement characters); invalid input returns InvalidData.
Allocation failure is Other and quota exhaustion traps E_RESOURCE_LIMIT,
consistent with P06 IoError-returning operations. Concatenation size overflow
traps E_RESOURCE_LIMIT. Equality compares byte contents, not allocation identity.

## Native ABI and validation

Each primitive has an il_rt_ external with context first. Result operations
return i32 (0 success, IoError discriminant + 1 failure) and take output pointer
second unless a consuming close. Resource outputs use the fixed handle ABI.
Grant selectors use u64 indices. Shared buffers/handles/deadlines use pointers;
Usize uses i64 and U8 uses i8. bytes_get returns i8; equality returns i1.
All execution engines use the same checked signatures and semantic failures.

Acceptance includes network grant rejection before user code, real IPv4/IPv6
socket I/O, partial reads/writes, absolute timeout, disconnect cleanup, stale
generation/kind rejection, nested resource cleanup, ownership/effect negatives,
and interpreter/debug/release parity for byte operations. Protocol acceptance
will separately execute native il packages through an external TCP client.
