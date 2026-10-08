---
rfc: 13
title: Captured interpreter test suites and bounded execution evidence
status: accepted
requires: [P03]
authority: user_approved_implementation_plan
---

`il test` implements `test(revision, suite, isolation)`. The request has exactly
these three required fields. `revision` selects an immutable application graph.
`suite` is a closed object with `entry` (stable function ID), `arguments` (typed
interpreter values) and `limits` (steps, call depth, heap bytes and output bytes).
`isolation` must be `captured`. It grants no filesystem, network, process-creation
or clock capability and accepts no command or shell text.

The tool checks the selected graph, lowers through HIR and MIR, verifies MIR,
and executes only the verified MIR. It returns structured `execution` and
`stages` records. Each stage records actual input/output hashes and compiler
version. Execution results contain captured stdout/stderr bytes, typed return
value, deterministic diagnostics, step count, memory accounting and lifecycle
events. A language trap or rejected input sets envelope `ok:false`; its execution
record is retained in the response. It never becomes a successful test merely
because an error was anticipated by the caller.

The execution stage hashes the canonical `execution_input` bundle containing
the verified MIR hash, entry, arguments, budgets and isolation. It does not hash
only the program while omitting the inputs that determine its behavior.

The test command does not publish a graph revision. Its execution cannot modify
host files or start a server. Real operating-system runtime acceptance is P06;
P04 proves deterministic language/MIR semantics with captured host boundaries.
JSON schemas specify exact value and budget forms. Native execution and named
external blackbox contracts are separate acceptance paths in their later stages.
