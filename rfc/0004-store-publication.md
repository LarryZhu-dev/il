---
rfc: 4
title: Application graph stores and atomic revision publication
status: accepted
requires: [P00]
authority: user_approved_implementation_plan
---

## Two revision domains

The compiler-development repository tracks milestone state, toolchain inputs,
compiler sources and its bootstrap graph. Its `repository_state.json` is bound
to Git by RFC 0001. Before dispatch, the CLI reads that state and verifies the
current Git commit/tree and the external binding journal. Missing bindings,
state divergence or invalid Git provenance return `E_STATE_INCONSISTENT` before
graph operations. This invariant is required even when the application store
is empty; initializing a store is not permission to bypass repository checks.

An application Store holds user program graphs and has its own monotonically
increasing graph revision. A graph transaction updates that Store, not the
compiler-development task list or its bootstrap revision. Every store snapshot
records the verified compiler source Git commit and tree used for the operation.
These values come from the CLI's repository verification, never untrusted request
strings. Commit provenance and application graph revision are separately named
and must not be compared as if they were the same counter.

## Files and commit point

The selected application root contains `.il/revisions/<20-digit-revision>/`
with immutable canonical `graph.json` and `manifest.json`. A manifest includes
`revision`, nullable `parent_revision`, nullable `parent_manifest_hash`,
`graph_hash` and `provenance {source_git_commit, source_tree_hash}`. Revision 0
has no parent. Subsequent manifests hash-link their actual parent, preserving
ancestry across interrupted unpublished candidates.

`.il/HEAD` is the single authoritative publication pointer containing
`revision`, `graph_hash` and `manifest_hash`. Snapshot manifests form the durable
revision journal; no independently mutable state file can disagree with HEAD.
The store uses an operating-system-backed exclusive writer lock, released by
process termination. Under that lock, a transaction:

1. Validates HEAD, its hash-linked ancestry and repository Git provenance.
2. Checks base revision, closed request schema, exact entity scopes and all
   mandatory structural or available semantic checks.
3. Chooses an unused revision greater than every previously allocated revision,
   including orphan candidates, and writes the complete immutable snapshot.
4. Syncs graph, manifest and directories needed for their durable publication.
5. Writes and syncs a temporary HEAD, then atomically renames it over HEAD and
   syncs the containing directory. This rename is the sole logical commit point.

No reader treats directory presence as a published revision. Readers accept
only HEAD and its verified ancestry. A crash before HEAD publication leaves an
invisible orphan; a crash after publication leaves a complete readable revision.
An orphan's revision number is never reused. Corrupt HEAD or snapshots fail
closed; the tool does not guess a replacement head from the largest directory.
Failed schema/semantic candidates are retained separately as experiments and
cannot appear through ordinary inspect, diff or restore.

The first supported store platform is Linux x86-64. An unverified platform cannot
claim durability by substituting a non-atomic delete-and-rename sequence. Native
target expansion must validate its actual filesystem and lock behavior.

## Operations and acceptance

P02's closed structural mutation catalog is `add_module`, `remove_module`,
`rename_module`, `set_module_visibility`, `set_module_imports`. Scope lists exact
modified IDs; a new module's ID must be included. Imported IDs are references,
not mutations. Module declarations remain empty until P03. Deletion never
silently cascades; dangling references in the final candidate reject the entire
transaction. Known semantic operations reject with `E_UNSUPPORTED_FEATURE`
until the full checker is available. Unknown fields/opcodes are schema errors.

Every accepted nonempty transaction creates a revision, even when resulting
content is unchanged. Restore validates a published historical snapshot, copies
its graph/configuration/build inputs and publishes a new child of the current
HEAD. It never rewrites or deletes history and never reverses external effects.
For P02 the graph is the only editable application input; explicit configuration
and build-input entries must join the manifest before such inputs become mutable.

Test stale bases, omitted scopes, out-of-scope edits, duplicates, dangling imports,
unknown operations, unavailable semantic checks and concurrent writers. Terminate
writers before/after each publication boundary and verify complete old/new state,
never mixed state. Test skipped orphan numbers, inaccessible orphans, ancestry
tampering, repository-state/Git mismatch and restore producing a fresh revision.
