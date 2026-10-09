---
rfc: 22
title: Revision-bound AI tools, bounded task context and verifiable receipts
status: accepted
requires: [P02, P03, P07]
authority: user_approved_implementation_plan_integration_review
---

## Status and scope

Accepted after integration review. P02, P03 and P07 are VERIFIED; P08 is RUNNING.
This proposal supplies the missing operational semantics of original sections
5, 15, 17, 18 and 19. It does not change language, ownership, HTTP or backend
semantics. Implementations and schemas must follow the accepted revision of this
RFC; acceptance still requires actual execution evidence.

The trust boundary is the JSON task/tool request. Repository, application store,
host policy, compiler executable and fixed evaluator paths are selected by the
trusted launcher. Requests cannot select an executable, shell, environment,
working directory, linker flag, filesystem path, URL, host or port. Graph source,
diagnostic text, operation names, artifact metadata and RFC excerpts are data,
never executable instructions. No shell evaluation or command-string fallback.

## Audit against the P07 implementation

The existing CLI implements state, schema-check, inspect, callers, dependencies,
slice, validate, transact, diff, restore, build and test. Native build records
already retain actual graph/compiler/runtime/toolchain/policy identities, fixed
LLVM argv, exits, logs and artifacts. Graph publication already checks scope,
stale bases, immutable snapshots and failed candidates.

P08 must close these gaps:

* explain, blackbox and evidence are absent; interpreter test results and most
  diagnostics have no durable lookup record.
* historical reads put HEAD in base_revision and the examined revision in
  result_revision, but failures can lose the requested revision entirely. A revision number alone also does not
  distinguish a failed candidate from its published base graph.
* context budgeting counts portions of result, not the delivered envelope;
  slice discards missing IDs when returning a budget failure. There is no task
  context sequence or cumulative call/context accounting.
* transaction requests do not themselves enforce a task record's prerequisites,
  declared scope, acceptance registry and total resource limits.
* adding an HTTP route requires replacing the whole program. A large checked
  application cannot be edited through bounded route context that way.

## Public command registry

The registry below is closed. Semantic names from original section 5 map to one
CLI spelling; they are not additional command aliases. callers, dependencies
and explain complete the section 5 interfaces omitted from section 19's shorter
category list. Unknown commands, options, fields, enum values, duplicate JSON
keys and explicit null in optional nonnullable fields are rejected.

| Semantic interface | CLI | JSON request |
| --- | --- | --- |
| repository state | state | `{}` |
| checked text candidate | schema-check | `{source}` |
| inspect | inspect | `{entity_id, revision, fields?, budget?}` |
| callers | callers | `{entity_id, revision, budget?}` |
| dependencies | dependencies | `{entity_id, revision, direction, budget?}` |
| slice | slice | `{root_entities, revision, max_nodes, max_tokens}` |
| validate | validate | `{graph_or_revision, checks}` |
| transact | transact | existing closed transaction object |
| diff | diff | `{base_revision, target_revision, scope?}` |
| build | build | existing revision/target/profile/runtime-profile/export schema |
| test | test | existing `{revision, suite, isolation}` schema |
| run_blackbox | blackbox | `{revision, contract}` |
| explain_diagnostic | explain | `{run_id, diagnostic_id, context_budget}` |
| restore | restore | `{revision, reason}` |
| emit_evidence | evidence | `{revision, task_id, artifacts, tests}` |

Read requests with an explicit revision must use it throughout. For existing
optional-revision queries, omission resolves HEAD once at admission and the
resolved revision is recorded; it is not re-read during execution. The runner
always sends explicit revisions after its initial state query. direction is
forward or reverse; the existing omission default remains forward. No query
accepts a revision from a path or environment variable.

blackbox.contract is the registered string `http_mvp`. It selects the committed
spec/http.yaml contract and fixed external client, and executes both native
optimization profiles. Its fixed wire subset is health, hello, 404/405, encoding,
fragmentation, read timeout and disconnect recovery. It does not add test-only
routes or claim P07's independent partial-write/backpressure matrix. That matrix
remains an independent P07 acceptance suite. No request-selected contract file
or test script exists.
explain's required run_id supplies the immutable diagnostic subject; diagnostic_id
selects a diagnostic inside that run. This additional selector resolves the
original interface's ambiguity between failed candidates at the same revision.

evidence.artifacts is a nonempty unique array of `{run_id, role}` selectors;
role identifies an artifact actually registered by a build/test/blackbox run.
For blackbox build artifacts, roles carry a debug. or release. prefix.
evidence.tests is a nonempty unique array
of test/blackbox run IDs. Clients cannot submit passed/count/exit_code/hash
claims in place of records. The trusted registry validates run kinds.

## Revision and candidate binding

Keep the existing response envelope: ok, tool, tool_version, base_revision,
result_revision, diagnostics, artifacts, evidence_id and result. Every admitted
run also has a receipt described below. Successful result objects identify that
receipt with `run_id`; failures with a known binding do so as well.

* Preserve query envelope semantics: base_revision is HEAD observed at admission,
  result_revision is the examined revision. This applies to failure as well as
  success. The receipt explicitly distinguishes observed HEAD and subject revision.
  A direct graph candidate uses graph.revision as subject and its actual
  candidate hash; the receipt marks it as a candidate, not a stored revision.
* transact uses its requested base_revision, and on success the new committed
  result_revision. A stale rejection records the requested base and observed
  HEAD separately in its receipt; no graph is committed.
* diff preserves observed HEAD as envelope base_revision and requested target as
  result_revision; result and receipt include requested base/target and both hashes.
* restore uses the observed current HEAD as base and the newly created revision
  as result; the receipt separately records the requested historical revision.
  A runner checks its expected current revision before invoking restore.
* schema-check binds to the resolved current base and records the candidate
  hash when parsing succeeds. Parsing failure binds the source/request hash;
  it must not claim that a graph existed.
* explain puts the stored diagnostic subject in result_revision, with observed
  HEAD in base_revision. evidence likewise uses its requested result revision and
  verifies every selected artifact/test matches.

Before trustworthy state or request parsing succeeds there may be no meaningful
revision. The fixed envelope then uses zero, but result/receipt binding must be
explicitly absent; this is never interpreted as a verified revision-zero run.
No failed load silently substitutes HEAD. Diagnostics keep the examined or
transaction-base revision rather than whatever HEAD happens to be at printing.

## Immutable tool-run receipts

Persist each admitted request and result, including failures, before returning.
Use an application-store journal independent of graph HEAD; read tools may append
audit records but never create a new graph revision or mutate old evidence.
An uninitialized application can keep this journal without pretending a graph
store has been initialized. State/provenance verification precedes journal writes.

The receipt payload is a closed object with these required fields:

```text
schema_version: "1.0.0"
tool: registered CLI name
request: normalized parsed JSON (including invalid nonobject requests retained as failures)
request_hash: SHA256(canonical request bytes)
source_git_commit, source_tree_hash: verified source provenance
binding: null | {
  base_revision, result_revision, observed_head_revision,
  graph_hash: hash|null,
  candidate_hash: hash|null,
  comparison_graph_hash: hash|null
}
compiler_hash, toolchain_lock_hash: actual hashes
runtime_hash: hash|null (actual linked runtime when this run uses one)
policy_hash: hash of canonical injected host policy
response: complete protocol response, without run_id
delivery: bounded delivery envelope, without run_id
commands: [{stage, program, arguments, exit_code: integer|null, stdout, stderr}]
artifacts: [{role, path, sha256}]
tests: [{suite, passed, count, contract_hash: hash|null}]
```

Command stdout/stderr are bounded actual captured strings; the stage/program/
arguments tuple records the actual fixed-tool invocation. Signal termination has
null exit_code and its process report retains the actual signal and cleanup
reason. Artifact paths are journal-relative `blobs/<hash>` locators. Full reports
and blackbox process logs are separately registered artifacts, never inferred
from a passed boolean. Nonexecuting queries have empty
commands/tests arrays. Test execution_input and native build records are retained
as registered artifacts. Record the source graph bytes, normalized input, policy
bytes, toolchain lock bytes and tool identities needed for replay. For interpreter
tests this includes entry, arguments, limits, MIR identity and actual report.

Canonicalization uses UTF-8/LF/trailing newline, deterministic field order and
semantic array order. `run_id = run_<full SHA256(payload bytes)>`; the ID is outside
its payload. Store the payload under that ID with create-new/flush/atomic publish.
Identical content can reuse an existing byte-identical receipt. A collision with
different bytes or a tampered/missing referenced artifact is E_STATE_INCONSISTENT.
Unknown run IDs are E_NAME_NOT_FOUND. Publication failure cannot be reported as a
successful audit; after a committed graph mutation, preserve committed_revision
in the failure response so clients never retry it as an uncommitted transaction.

Paths returned in artifacts are locator data only. The evidence resolver accepts
registered IDs/roles, rejects traversal and symlink escapes, and verifies hashes
again before copying bytes into its immutable evidence directory. No request can
read arbitrary paths through explain, blackbox or evidence. The journal records
durable failures even when the response is too large to deliver; a bounded error
returns their ID, without dumping unbounded logs.
Receipts retain both the complete semantic response and the selected delivery
envelope before attaching the new run_id. `response` preserves the actual
semantic outcome; `delivery` preserves budget diagnostics that the client saw.
The fixed-width future run_id is included in byte-budget calculations before
hashing, so there is no receipt/ID cycle. Receipts have an exact
32 MiB maximum file size. Exceeding that limit produces an explicit failed
candidate, never a partial successful receipt. A smaller delivery response does
not alter the retained execution's success/failure judgment.

## Diagnostic identity and explanation

Persist complete original diagnostics in receipts for schema, language,
transaction, build, execution, protocol and budget failures whenever source
context is trustworthy. Preserve the engine's diagnostic_id; do not rewrite
envelope copies while leaving native retained execution reports different.
The lookup key is the pair (run_id, diagnostic_id). Different candidates have
different request/candidate hashes and therefore different run IDs, even when
the engine diagnostic ID is equal. Within a receipt, byte-identical copies in
top-level/nested reports are deduplicated for lookup; genuinely different records
sharing one diagnostic ID are E_STATE_INCONSISTENT rather than a guessed match.
No global diagnostic-ID index overwrites another run. No old one-argument
explain command or compatibility lookup is provided.

explain loads exactly that immutable run record, verifies its hash and references,
and returns `{subject_run_id, diagnostic, binding, related_entities, rules,
suggested_operations}` under context_budget. rules consists of registered,
bounded spec/RFC fragments selected by diagnostic code. Each reference records
source, source_sha256, reference_kind, available, start_line, fragment and
truncated. Fragments are taken from compiler-locked source text, at most 900
UTF-8 bytes each and at most two references per code. An unregistered code gets
an explicitly general reference; it is never assigned invented code-specific
guidance. Missing compiled anchors are explicitly unavailable. Related entities come
from the retained failed candidate when relevant, otherwise its exact snapshot.
No current-HEAD substitution. Unknown IDs fail; corruption fails; unavailable
candidate context is reported missing rather than reconstructed by guessing.
suggested_operations remain structured suggestions and never apply themselves.
The delivered result.run_id identifies this new explanation call's own receipt;
subject_run_id identifies the earlier call being explained. Lookups inspect
both retained response and delivery diagnostics, so context-budget errors remain
explainable. Returned prose may explain a diagnostic but cannot replace machine fields or
invent a successful repair. Restarting il must not lose explanation ability.

## Context and task execution

One conservative token unit is one UTF-8 byte of the serialized response. This
preserves the existing conservative budget rule without introducing a model
tokenizer dependency. Per-call budgets include envelope, diagnostics and receipt
identifier; they do not count only entity bodies. Default query budget is 16384,
maximum 65536; request/response hard limits remain 262144 bytes. slice max_nodes
is 1..4096 and counts distinct returned entity IDs. The explicit virtual program
query stays available to trusted development; the AI task runner forbids it and
forbids full source/graph dumps even if the program currently fits.

An insufficient context response has ok=false, E_CONTEXT_INSUFFICIENT, and
`result: {run_id, status:"BLOCKED", missing:[stable IDs], missing_truncated,
missing_source, budget, required}`.
No partial answer is labeled complete. Delivery budget failures identify the
requested root_entities/entity_id whose complete context could not be delivered;
the ordered complete root list is retained in receipt.request. missing_source
is `receipt.response.result.missing` when the semantic result already contains
a missing dependency list, otherwise `receipt.request`. missing has at most 16
whole IDs and at most 1024 serialized UTF-8 bytes; missing_truncated explicitly
reports omission. An oversized ID is omitted intact, never shortened to a fake
entity ID. A slice that stopped at a dependency frontier retains its exact
omitted dependency IDs in receipt.response.result.missing; request roots are
traversal starting points, not the full dependency frontier.
The requested budget governs useful context; a fixed bounded error envelope may
exceed an impossibly small budget, and that actual byte count is still charged
to the task. required is a known lower bound if full expansion is not computed.

The trusted eval/runner loads a task record and a fixed registry of acceptance
predicates. Its untrusted step input is only `{tool, request}`. Before the first
step it validates task schema, VERIFIED prerequisites, base_revision against
application HEAD, scope existence/creation rules, declared inputs, acceptance
names and all positive resource limits. Unknown acceptance strings are rejected,
never run as commands. Before every mutation it checks the current task revision
and requested scope again. Failed validation must cause zero transact calls.

The runner persists a task session with task-file hash, selected base revision,
current revision, resource counters, ordered run IDs and status. It serializes
steps per session. Successful transactions advance current_revision; failed ones
do not. Admitting a call consumes a tool-call unit even if the tool rejects it.
Context bytes accumulate over all delivered responses, including errors; no
reset by changing commands. To avoid exceeding a remaining context allowance,
request bounded projections or return BLOCKED before delivering the oversized
response. The complete result can remain in the receipt.

Entity projections read only requested fields; inspecting a function signature
must not serialize its blocks. Dependency IDs are sorted and deduplicated.
Function/block views filter definitions already embedded in their returned body;
operation views filter their own outputs while preserving external input, call,
capability, type and control-flow references. A contract already contains full
route objects, so its dependencies are subject, entry, capability, handlers and
nonbuiltin parameter types, not duplicate route IDs. An independent route view
depends on its handler/types; packages depend on modules/capabilities, capability
views have no dependencies, and parameters/values depend on nonbuiltin types.

Acquisition phases follow the original order: contract, target entities, direct
dependencies, callers, related types, related tests, current diagnostics,
relevant RFC fragments. The runner records which phase each read fulfills;
unavailable or empty phases are explicit. Required refs absent from those phases
produce missing context, not invented entity IDs. Mutation is admitted only once
the task's required context has been gathered within the budget. Tests/RFCs are
fixed registry references or bounded graph entities, never arbitrary file reads.

The existing resource_budget fields remain unchanged. The trusted AI session
registry fixes graph_nodes=4096 per slice and environment_retries at most2 per step
(a session may select zero retries);
these are session rules, not new mandatory fields on historical stage records.
Retries reuse the normalized request and revision and require recorded retryable
environment failures. cpu_seconds/memory_bytes are enforced by the trusted worker launcher,
not assertions in model output. Linux workers use process-group accounting and
resource limits; exhaustion kills/reaps only that task's process group, records
the final input/run and returns BLOCKED. Unsupported host enforcement fails
explicitly. Semantic failures are never hidden through automatic retries.

## Bounded route mutation

Add the transaction operation (there is no existing ReplaceContract operation):

```text
{op:"add_route", route_table_id: EntityId, route: HttpRoute}
```

route_table_id must identify exactly one HTTP server contract. HttpRoute has the
existing RFC0021 closed fields entity_id, method, path, handler and parameters.
The original document's flat method/path/handler_id example omitted route ID and
typed parameters. The final request uses the complete route object above; do not
also accept the incomplete flat shape or add a compatibility adapter.
The operation appends one route without requiring the whole source graph. It
does not create a handler, capability or new authority. The handler must already
exist and satisfy the ordinary checker; a separate exact-scoped function edit
can introduce it. Duplicate IDs and overlapping routes are rejected.

Require exact scope for the contract ID, new route ID and subject dispatcher ID.
There is no program wildcard exception. Generated dispatcher changes require the same dispatcher scope
as RFC0021; this permission does not authorize unrelated function changes.
The stored candidate transaction includes the exact route object. Run ordinary
elaboration, name/type/effect/capability/ownership/contract checks atomically.
Generated IDs depend only on existing stable IDs, never route position.

## Blackbox process ownership

blackbox builds the selected revision using the existing native driver, then
launches the actual generated full-runtime ELF under the trusted policy. Its
external TCP client uses only the locked contract, not graph routes, parser
helpers or handler internals. Pin and record contract/client/compiler/runtime
and executable hashes, actual request/response receipts, exits and deadlines.
Both profiles must run on the real target; a cross-build is not execution proof.

The contract supplies the loopback endpoint. If occupied, report a retryable
environment failure; never kill an unrelated listener, choose another port or
substitute another URL. Track the exact child/process group. On success, test
failure, timeout, interrupted client or diagnostic failure, close client sockets
and terminate/reap only owned children. Startup, test wall time, captured streams
and reports are bounded. A passing report requires every assertion in the fixed
subset, including malformed encoding, fragmentation, read timeout and disconnect
recovery. It is not a claim that every independent P07 test ran.

## Evidence publication and task verification

evidence resolves selected receipts, checks their identity and graph revision,
source/compiler/runtime/target/toolchain/policy consistency, artifact bytes and
test exits. It derives tests/passed/count/commands from those records. Failed
tests remain evidence and cannot be relabeled passed. Missing/corrupt artifacts,
wrong revision or inconsistent inputs produce E_EVIDENCE_INCOMPLETE (with
specific machine-readable diagnostics), never a synthetic completion record.

Publish a self-contained immutable evidence directory with canonical graph,
policy, request/execution inputs, source/toolchain identities, build records,
selected outputs and actual reports. Compiler/runtime bytes may be included or
referenced by an immutable accessible bundle with verified hashes; a mutable
local path alone is not sufficient to reconstruct a build. Enforce a fixed
512 MiB total content budget, counting manifest, unique blobs and retained run
payloads. Each blob is at most 256 MiB. Reject limit excess
before publishing a manifest; do not claim a partial directory is complete.
Hash the evidence payload without its evidence_id, then use `ev_<full SHA256>`
as the outer ID.
The application evidence command does not load or trust a caller-selected task
file; task_id is a label, not proof of task authorization. The trusted runner
computes effects_delta and capabilities_delta against its validated
task.base_revision and records them in the task evidence. Application evidence
retains the authentic run/input/artifact bundle needed for that calculation.
known_limits are registered facts, not a way to waive a failing assertion.

Preserve existing immutable bootstrap/MVP phase evidence and its applicability
rules. Do not reinterpret old artifacts as newly verified AI sessions. Evidence
emission alone does not set a development stage or application task VERIFIED;
the trusted runner checks every task acceptance predicate and dependency first.
Interrupted publication leaves an incomplete candidate without a published
manifest. Retry revalidates inputs; it never promotes a partial bundle blindly.

## P08 acceptance

1. Closed schema/registry tests reject unknown commands/fields, shell metacharacter
   payloads, argv/env/path/URL injection, arbitrary acceptance names and forged
   evidence passed/count claims before any child process or graph mutation.
2. Historical success and failure envelopes/receipts identify the requested
   revision. A failed candidate and published base at the same revision retain
   distinct hashes. Concurrent HEAD advancement cannot change an admitted read.
3. Persist and explain a type error after a fresh process start and a subsequent
   successful transaction. Two different candidates with the same code/entity
   remain distinct by (run_id, diagnostic_id). Tampering/unknown IDs and
   too-small budgets fail honestly.
4. Exercise byte/node/cumulative-context/call limits at and over their boundaries.
   Verify deterministic missing refs, no whole-program dump, no mutation after
   budget/dependency/scope preflight failure, and no budget reset on restart.
5. Start from an HTTP baseline with only /health and a checked dormant hello
   handler. Read its contract, handler signature and bounded related context;
   add /hello/{name} with add_route; build and run locked external blackbox tests.
   The test records actual context/tool-call costs and checks stable generated IDs.
6. A stale route transaction and invalid handler/collision candidate preserve the
   old revision and ELF; explain the retained failure, then fix with a new checked
   transaction. Do not weaken P03/P07 checks to pass this task.
7. Emit evidence from genuine runs, copy the bundle into a clean location and
   reconstruct the build using its recorded graph/policy/toolchain inputs.
   Compare artifact hashes and execute the rebuilt program. Reject a tampered
   report/artifact, mismatched revision and unattested test selection.
8. Fault cases leave receipts/failed candidates, enforce task retry caps and
   clean up only owned processes. P09 will expand the interruption/fault matrix;
   P08 must already prove its own publication and wrapper boundaries.
