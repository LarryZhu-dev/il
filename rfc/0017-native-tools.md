---
rfc: 17
title: Native build and captured execution tools
status: accepted
requires: [P01, P03, P04]
authority: user_approved_implementation_plan
---

`build(revision, target, profile)` accepts exactly those fields. The target is
`x86_64-unknown-linux-gnu`; profile is `debug` or `release`. It compiles the unique
function named `main`, whose signature must be `() -> I32`, into an application
ELF. Building does not execute the application. Ordinary applications have no
test instruction budget.

`test(revision, suite, isolation)` additionally accepts `native_debug` and
`native_release` isolation. The same closed suite/value/limit schemas apply to
all execution engines. Native tests compile a specialized LLVM entry that
constructs the arguments, calls the compiled language function and reports its
typed result. They never evaluate MIR or dispatch graph operations in the host
runtime. No caller-supplied commands, linker flags or environment variables are
accepted. Unsupported runtime capabilities are rejected before execution.

The runtime archive is the `libil_native_runtime.a` beside the running compiler;
its actual hash is recorded. The driver uses fixed LLVM tools and records their
versions, file hashes, exact argument vectors, exit statuses and output. The
toolchain lock and compiler executable hashes bind the invocation as well.

Each build creates a distinct persistent directory beneath the application's
`.il/builds/candidates`. Source IR, tool logs, object and ELF files remain there,
including failed candidates. After all compilation gates succeed, the tool
flushes the artifacts and atomically publishes a `verified.json` record in that
directory. The record verifies compilation only; it does not certify an
application test or a project task. A failed build never replaces an earlier
record or changes the graph revision. Interrupted candidates have no published
record. Recovery may inspect them; they are never silently promoted.

The result contains `stages` and `native`. The latter includes `profile`,
`target`, and `artifacts` with `native_ir`, `llvm_ir`, `object`, `executable`.
Each artifact has an absolute `path` and actual `sha256` digest. `build_record`
identifies the published compilation record the same way. Native tests also
include `argv`, `report_fd:3`, `exit_code`, `execution`, and `execution_input`.
The generated executable remains independently runnable after the tool exits.

Captured native execution uses inherited descriptor 3 for the report. Actual
stdout and stderr are captured separately and checked against report bytes.
The parent enforces a wall timeout and bounded outputs, kills and reaps a
timed-out process, and retains failure details. Language traps return exit 101;
successful captured calls return exit 0 regardless of their typed return value.
Ordinary application exit codes follow the I32 return value and platform rules.

`execution_input` binds graph, MIR, Native IR, compiler, runtime and toolchain
hashes, the actual LLVM executable identities, plus entry, arguments, limits,
target, profile and isolation. The
`execute_native` stage hashes that complete bundle and the actual report. A
trapped or rejected execution sets the response envelope `ok:false`.

`il-execution-model` owns the shared value, limit, lifecycle and report types.
It contains no evaluator. The interpreter and native runtime depend on it
independently. Native code generation has no dependency on the interpreter.
