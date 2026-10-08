---
rfc: 201
title: Owned concurrency and structured asynchronous execution
status: accepted
requires: [P10, E01]
extension: E02
authority: user_approved_implementation_plan
---

Implement and verify in order: Thread, Mutex, Channel, Atomic, structured tasks,
async. Spawn moves owned arguments into a child. `Send` and `Sync` are compiler
checked marker traits, recursively derived only for valid fields; capabilities
are not transferable unless their host declaration explicitly permits it.
Raw pointers and lexical borrows cannot cross thread boundaries. A thread handle
must be joined; dropping it joins rather than detaching untracked work.

Mutex access returns a unique lexical guard; unlock occurs on guard drop. Child
failure is a typed join error. Channels are bounded FIFO queues with explicit
capacity and move values on send; closed/disconnected/timeout are typed errors.
Channel endpoints have owned lifetimes. Queue operations cannot silently discard
values or clone owned payloads.

Atomics cover Bool and supported integer widths. Ordering is explicit relaxed,
acquire, release, acquire_release or sequentially_consistent. Reject release
loads, acquire stores and illegal compare-exchange failure orderings at check
time. Runtime and interpreter preserve the specified happens-before relations.

Structured scopes own all child tasks and cannot complete until children finish
or cancellation completes. Cancellation is cooperative at suspension points and
drops live resources exactly once. No borrowed value may be live across await.
Lower async functions to explicit state machines with owned captured fields,
poll/wake protocol and cancellation drops. Use OS readiness/completion facilities
for real asynchronous I/O; a synchronous wrapper on a task thread does not pass
async I/O acceptance. Implementation order follows target availability: Linux
epoll, macOS kqueue, Windows IOCP.

Acceptance includes ownership/Send failures, mutex exclusion, channel FIFO and
backpressure, atomic litmus tests, join failure, scope cancellation, child leak
detection, wake races and multiple concurrent socket operations. Random stress
is additional evidence and cannot replace deterministic race/fault scenarios.
