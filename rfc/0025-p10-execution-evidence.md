---
rfc: 25
title: Trusted P10 execution evidence and publication
status: accepted
requires: [P09]
authority: user_approved_implementation_plan
---

## Authority and boundary

The accepted implementation plan authorizes completing missing acceptance
semantics without changing the original G0-G4 requirements. This RFC defines
how a trusted P10 supervisor observes those requirements. It does not change
language semantics or turn the existing bundle structure checker into an
execution verifier. P09 must be VERIFIED before P10 work is accepted. A failed,
interrupted, incomplete, or structurally valid-only run cannot publish a release.

The supervisor executes a fixed, reviewed command plan from committed source.
It does not accept commands, expected outcomes, successful gate records, or a
purported previously executed bundle from an untrusted caller. Expected language
and HTTP behavior comes from committed contracts. It measures each invocation,
captures actual bytes and termination, validates the corresponding contract,
and derives predicates from these observations. A content hash establishes
byte identity; it does not establish that a command ran. Offline validation
remains explicitly provisional, even for a bundle originally produced by the
supervisor. Verification of a previously published run needs its trusted CI or
supervisor execution identity and retained logs, or a new supervised execution.

## Source and process identity

Release acceptance requires a clean committed source checkout, locked dependency
inputs, a supported native Linux x86-64 environment, and the actual locked host
compiler and LLVM tools. Record the source commit and tree, supervisor and
contract hashes, toolchain lock hash, executable tool hashes, tool versions,
container image identity when used, and actual environment restrictions. A
version string or image tag alone is insufficient. Record the CI run/job identity
when executed in CI. Dirty exploratory runs retain their result but cannot publish.

Each command observation binds its invocation ID, argv, working directory,
permitted environment, input bytes, stdout/stderr bytes, executable identity,
monotonic duration, and actual exit status or signal. Artifact references retain
the referenced bytes. Unexpected exit, signal, timeout, malformed output, absent
case, failed assertion, or omitted required artifact fails the affected gate.
Unknown future record fields are rejected unless the committed schema permits
them. Partial progress is atomically recorded before later commands run.

Application fixtures have their own graph revision and source repository
identity. These are recorded separately from the compiler source commit. Never
label an isolated fixture commit as the compiler commit or change a historical
graph's revision to make an artifact hash match. Rollback content comparison may
exclude only the monotonic graph revision, as specified in RFC 0024; preserve
both raw snapshots and their original hashes as well.

## Artifact roles and HTTP observations

The evidence inventory identifies artifacts by their content hash and explicit
role, target, profile, entry point, graph revision/hash, runtime profile, and
build invocation. A release application, debug application, captured test entry,
compiler, runtime, object, and LLVM IR are different artifacts even when they
come from one graph. A full acceptance bundle can contain many executables.
It must identify the release application explicitly; it must not require every
test to execute that one binary.

An external TCP observation binds the executable actually serving the request,
its build, server process instance, startup mode, policy hash, case ID, profile,
request fragments, actual send order and delays, response bytes, duration, and
contract assertions. Each referenced executable must be present in the artifact
inventory and have an observed native build. Debug/release and captured/application
observations cannot substitute for one another. The required HTTP case matrix
retains all cases already locked by `spec/http.yaml`, RFC 0023, and the P10
contract, including partial writes, deadlines, disconnect recovery, malformed
input, and resource cleanup. Success of G4's smaller request matrix cannot satisfy
the full HTTP gate.

Startup probes are recorded separately from acceptance requests. A disconnect
case records the initial partial request and intentional socket reset as well
as the later recovery request. A deadline case records the delayed fragment
schedule and the measured interval checked against the locked bounds. Partial
I/O tests retain the captured executable identity, trusted fault policy, wire
exchange, and actual execution report. They require the existing live-handle and
live-allocation cleanup assertions; a successful ordinary request alone does
not prove partial-write behavior. Runtime failure diagnostics and slow-reader
observations are retained with the relevant process, not inferred from a final
health response.

Server lifetime is distinct from request success. The normal HTTP application
runs indefinitely and may be intentionally terminated by the supervisor after
its assertions. Record that actual termination signal, the supervisor's cleanup
intent, whether the process had already exited unexpectedly, and the wait/reap
result. Never replace a negative signal exit with zero. A request can pass while
its supervised server is later terminated for cleanup. Finite captured test
entries instead must return or trap with their contractually expected process
exit and execution report. These modes have separate schema alternatives.

Clean-environment execution means the actual application or captured process is
launched with the stated empty environment in an isolated working directory,
with only its explicitly required descriptors and policy. A finite process must
finish with its expected exit. For an indefinite service, successful contract
requests followed by recorded owned-process cleanup prove execution; cleanup
does not masquerade as successful program termination.

## G0 and G2-G3 observation requirements

G0 derives from actually executed schema, checker, ownership, MIR mutation, and
interpreter contract tests. The supervisor retains named test/case outcomes and
locked fixture hashes, not only a zero process exit or aggregate count. The MIR
gate must include direct invalid-MIR tests that do not trust source validation.
Type and ownership gates include their relevant negative fixtures and stable
diagnostics. Test selection is fixed by the producer's committed command plan;
empty or incomplete selection fails.

G2-G3 observations include real inspect/slice responses, a route transaction
creating a new revision, stale-base rejection without HEAD change, rejected
candidate retention without published graph change, a structured diagnostic and
its explanation, repair/build/external validation, and rollback into a new
revision. Retain original input/response bytes, graph snapshots, candidate bytes,
tool receipts, and executable bytes. Run the previous verified executable after
failure and rollback. Record that executable's own identity independently of
the restored graph's new build. Existing AI protocol and process interruption
tests remain required; a scripted direct-CLI route experiment does not replace
the bounded AI protocol or real interruption acceptance.

## Independent clean rebuilds

Reproducibility requires two independently created empty build roots from the
same committed source and locked dependencies. Build the bootstrap compiler and
runtime independently in both roots, then build the same application graph with
each newly built compiler/runtime in separate initially absent graph/build
stores. Neither root may copy a compiler, runtime, object, executable, or
incremental build output from the other. Record creation/emptiness observations,
source materialization identity, each command, and all resulting artifact hashes.
Read-only copies of identical locked dependency source inputs may be shared;
compiled dependency and compiler output caches may not be shared.

The two build commands use identical explicitly recorded toolchain/environment
settings and run without network access once locked inputs are provisioned.
Compare corresponding compiler, runtime, Native IR, LLVM IR, object, and native
application hashes for the same profile. Record both sets even on mismatch.
Execute the independently produced application from each root with the empty
environment contract above. Merely copying one binary twice, creating two stores
under one existing compiler, or comparing headers is insufficient for this P10
clean compiler-and-application rebuild requirement.

## G4 and publication

The supervisor executes RFC 0024's complete 18-trial matrix, preserving measured
failures as decision data. Every expected trial must finish with its actual
result; missing attempts are incomplete. Include source/toolchain identities,
locked baseline commit, trace operations, assertions, source changes, external
requests, duration, call counts, and failure distribution. The comparison must
identify its scripted-workflow methodology and must not imply an LLM or human
productivity result. Genuine measured task failure can appear in complete G4
decision data; it never turns a failed G0-G3 predicate into success.

Only a completed trusted run satisfying all predicates and dependency checks
can produce the immutable MVP release manifest, artifact hash inventory, known
limitations, and comparison record. Bind these to the complete retained bundle
and source/toolchain identities. Publication must refuse to overwrite an existing
release with different bytes. A correction requires a new release manifest and
revision. Repository task-state updates are a separate reviewed operation and
must reference the verified trusted run. An offline structure-check result,
handwritten successful receipt, green workflow declaration, or unexecuted target
is never sufficient to mark P10 VERIFIED.
