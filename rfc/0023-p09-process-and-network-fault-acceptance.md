---
title: P09 process recovery and network fault acceptance
status: accepted
---

# P09 process recovery and network fault acceptance

P09 extends the accepted HTTP and AI-tool contracts without changing their
published behavior. Fault controls are trusted test inputs only. They are not
available through `.il` source, graph transactions, public tool requests,
environment variables, or ordinary application host policy.

## Captured accept failure

The closed `Faults` object gains required nullable `accept_fail_after: u64 | null`.
Its value counts successful `accept4` calls in one captured host. Before the
next accept syscall, once the count is greater than or equal to this value, the
host returns the same typed read error used for an operating-system `EIO`.
The counter is deterministic, does not consume the failure, and does not alter
the listener. `null` disables injection. The setting is valid only for captured
execution; application startup rejects every non-null fault setting. Missing,
unknown, negative, or non-integer fields reject the complete policy.

The independent HTTP blackbox must invoke the compiled service over a real TCP
connection and assert the externally observable failure contract, structured
diagnostic, and complete listener/client cleanup. Expected status and response
bytes come from a committed test contract, never from the implementation under
test. External HTTP tests must not import package HTTP parsing, routing, or
response implementation.

## Process interruption and recovery

Process recovery tests use a trusted test supervisor and private synchronization
barriers. Barriers are compiled into test-only harnesses or injected through
private descriptors; they are not command-line, environment, request, or source
language features. The supervisor records the child PID, exact executable hash,
input identity, signal, and cleanup result before terminating only the owned
process group.

For graph publication, terminate a real writer before snapshot completion, after
the durable snapshot, immediately before HEAD rename, and immediately after
HEAD rename. Reopen the store in a fresh process. Before rename, the previous
verified HEAD remains authoritative, complete orphan revisions are not reused,
and a subsequent transaction succeeds. After rename, report the committed
revision as uncertain and accept only the complete new snapshot. No state may
mix bytes from two revisions.

For native builds, terminate a real build worker at a locked linker barrier.
Retain its structured failure record and partial candidate separately. The
previous verified executable and its hash remain usable. A clean retry creates
a distinct candidate and either verifies it completely or retains its own
diagnostic; it never promotes partial output.

## P09 evidence predicates

P09 blackbox evidence is produced by an independent external TCP client and
includes a static import-boundary check. Every injected failure records a
machine-readable diagnostic and the owned-process result. Recovery evidence
reopens state after an actual process termination, executes the old verified
binary after a failed build, validates each persisted report against its schema,
and hashes all inputs and outputs. A Rust in-process returned error, unit test
fault point, workflow declaration, or cross-compilation is not process-crash or
external-runtime evidence.
