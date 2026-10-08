---
rfc: 8
title: Checked text import and complete semantic transactions
status: accepted
requires: [P02]
authority: user_approved_implementation_plan
---

`il schema-check` accepts a closed `{source: string}` request and returns a checked
candidate graph and its canonical `.il` projection without publishing either.
`il inspect` accepts the reserved virtual entity `program` to inspect the complete
graph under its normal context budget. Real graph entities cannot use that ID.

`il transact` additionally accepts `import_text {source}`. The CLI strictly parses
the source into a `replace_program {graph}` operation at the transaction base;
the stored experiment/transaction contains the canonical graph operation. Both
operations require explicit `program` scope. This grants replacement of the
program, and is deliberately broader than an individual module/function scope.
Host-injected capabilities must equal the existing graph's capabilities; source
text or normal graph transactions cannot create authority.

Individual type/function edits and module declaration-list edits retain their
exact entity scopes. Every candidate runs structural validation and the mandatory
semantic checker before publication, regardless of requested checks. The Store
requires a checker callback; the language CLI always supplies the complete checker.
Restoration revalidates historical content with the current checker before creating
a new revision. There is no unchecked entry point or structural-only CLI mode.

P02 tests for structural durability remain, with their explicitly injected
validation policy. Acceptance of semantic requests is now tested by positive and
negative P03 fixtures; the temporary rejection of all semantic requests is removed.

For example, send this JSON to `il transact` to replace revision 0 with a module:

```json
{"task_id":"edit_app","base_revision":0,"scope":["program"],"operations":[{"op":"import_text","source":"@id(\"app\") module app {}"}],"required_checks":[]}
```

An empty `required_checks` list still runs every mandatory check. To preview the
same source without initializing a store, send
`{"source":"@id(\"app\") module app {}"}` to `il schema-check`.
