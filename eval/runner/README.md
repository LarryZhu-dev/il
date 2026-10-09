# Bounded task worker

`run.py` is a trusted Linux launcher. Its command-line paths are selected by the
host; untrusted stdin has exactly `tool` and `request`. It never executes a task
acceptance string, shell command, supplied executable, URL or environment.

The registered P08 task plans one `add_route` operation. Its contract and handler
IDs define the context sequence: contract inspection, handler signature,
dependencies, callers, related type slice, fixed HTTP acceptance contract,
explicit empty initial diagnostics, and the accepted route RFC section. Missing
or out-of-order context blocks mutation. The runner discovers type IDs from the
returned signature and related IDs from dependency queries; it does not dump the
program or formatter output.

Every invocation locks and reloads `session.json`. The task, compiler and policy
hashes must match the session. Internal state/scope preflight calls are recorded
and charged as real tool calls and context bytes, although their full responses
are not delivered to the caller. Delivered CLI responses, fixed context and
runner envelope bytes also count. An oversized result remains in the CLI receipt
and local stdout artifact; the caller receives only a bounded terminal error.
An impossibly small remaining allowance can be exceeded by that terminal error,
which is still charged. No further task work is admitted after exhaustion.
The closed `schema/ai_session.schema.json` is checked before every save and load;
call indices, reservations, run IDs, revision order and context counters must
agree with the retained ledger. A malformed session cannot reset its budget.
Effect and capability deltas are measured against the task's base graph using
hash-verified receipt blobs, without delivering the whole graph as AI context.

The session reserves each call durably before launch. A restart with an unfinished
reservation blocks automatic replay, including ambiguous transaction publication.
Successful mutations advance its current revision before response delivery. Every
mutation rechecks HEAD. Environment retries are disabled; actual worker failures
become `BLOCKED`, and nonretryable semantic failures become `DESIGN_REQUIRED`.
In `DESIGN_REQUIRED`, bounded read/explanation calls remain available. A failed
transaction can resume only after its diagnostic run has been explained and the
caller explicitly submits a different repair transaction; the original request
is never replayed. This transition and both requests remain in the session audit.

Workers run in a dedicated process group with real address-space/CPU/file limits.
The launcher samples aggregate POSIX-session CPU and resident memory, bounds wall
time and captured streams, and kills only groups in that owned session at completion
or failure. This includes LLVM and HTTP children that create separate process groups.
The launcher acts as a Linux subreaper and reaps adopted descendants before return.
Raw stdout/stderr, receipt IDs, request hashes and usage survive failure. Non-Linux
hosts fail explicitly because equivalent enforcement has not yet been implemented.

The worker does not mark a task or development phase `VERIFIED`. Its transcript
feeds the independent acceptance evaluator, which must also verify reconstruction
and actual HTTP execution. `tests/ai_protocol_cli.py` without `--binary` checks only
the worker boundary using a fixed isolated fixture process; with `--binary` it
additionally runs the real il protocol. These are identified separately in reports.
