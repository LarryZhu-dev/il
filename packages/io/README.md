# io (P06)

Compose `core/lib.il` and this source. `print_string`/`print_i64` return
`UnitResult`; `read_stdin(maximum,deadline)` returns one actual chunk, with empty
Bytes indicating EOF or a zero request. `write_stdout`/`write_stderr` loop over
actual partial writes using one unchanged deadline and return the completed
byte count. A zero-progress write returns `IoError::Write`. A failed complete
write may already have emitted a prefix; the error does not imply rollback.

String/Bytes/File parameters transfer ownership and are released by the callee.
`close_file` consumes its File even when close fails. A caller needing repeated
operations on an owned file uses `runtime.file_read_some`/`file_write_some`, which
borrow inside its own function. Open calls require an explicit application grant
selector and do not receive ambient authority from this package.

Finite deadlines on blocking inherited streams return InvalidData. Hosts must
inject independent nonblocking streams to obtain finite stream deadlines. Output
faults and chunk limits are trusted captured-test policy, never guest options.
