# Intelligent language AI tool protocol

P08 protocol design is **Accepted** in
[RFC0022](../../rfc/0022-ai-tool-protocol.md). Implementation is under P08
acceptance. JSON schemas, CLI behavior and runner evidence must agree with the
accepted RFC before P08 is VERIFIED.

The untrusted boundary accepts structured tool requests only. A trusted launcher
selects the repository, application store, host policy and locked tools. Model
requests never supply shell commands, executables, argv, environment, arbitrary
paths or URLs. Unknown commands and fields are errors.

The CLI registry maps original semantic interfaces to one spelling:

| Interface | CLI |
| --- | --- |
| inspect / callers / dependencies / slice | il inspect / callers / dependencies / slice |
| validate / transact / diff / restore | il validate / transact / diff / restore |
| build / test | il build / test |
| run_blackbox | il blackbox |
| explain_diagnostic | il explain |
| emit_evidence | il evidence |

state and schema-check remain structured discovery/check tools. Semantic names
in the left column are documentation names, not compatibility aliases.

Every result must identify the examined revision and have an immutable receipt
binding its actual graph or failed candidate, request, diagnostics, execution
inputs and artifacts. explain retrieves a persisted diagnostic by run_id and
diagnostic_id; evidence consumes artifact selectors `{run_id,role}` and test run
IDs and derives results from genuine records. A request cannot assert that an
unexecuted test passed.

Receipts preserve the full semantic response and a separate bounded delivery
envelope. The delivered result.run_id identifies the current call. An explanation
also returns subject_run_id identifying the prior call being explained. Compact
budget errors identify missing context and missing_truncated. missing_source
points to receipt.request for requested roots or receipt.response.result.missing
for detailed slice omissions. The prefix contains at most 16 whole IDs and 1024
serialized bytes. Evidence bundles are bounded to 512 MiB, with
256 MiB per blob, and do not change development-stage verification state.

Task context follows contract, target entities, direct dependencies, callers,
related types, tests, diagnostics and relevant RFC fragments. The runner checks
prerequisites, revision, scope, acceptance names and budgets before mutation,
counts delivered UTF-8 bytes conservatively as tokens, and returns structured
BLOCKED/missing context when insufficient. Whole-repository and unbounded-log
dumps are forbidden in this interface.

The external HTTP test owns its generated server process and sockets,
uses the locked HTTP contract and cleans them up on every outcome. An occupied
port is an environment failure, never permission to stop another service.

See the RFC for the exact request/receipt fields, add_route transaction,
diagnostic identity, evidence reconstruction and milestone acceptance matrix.
