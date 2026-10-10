# P10 evidence structure helper

`runner.py` validates the shape and internal byte consistency of a proposed P10
bundle. It does **not** run the compiler, execute acceptance commands, establish
execution provenance, approve a release, or change a task status.

```powershell
python tools/p10/runner.py --bundle path/to/bundle
python -m unittest discover -s tests/p10 -v
```

The CLI requires a clean checkout and binds the proposed source and toolchain
identities to the current checkout. The Python entry point is
`validate_bundle_structure`. `ok: true` and exit code 0 mean only that structural
checks passed. Its result is always `evidence_structure_validated`, with
`status: PROVISIONAL`, `acceptance_verified: false`, and
`provenance_verified: false`. The reported comparison is explicitly named
`claimed_comparison`; its metrics are calculated from caller-supplied records.
No output from this helper may be used to mark P10 VERIFIED.

The helper checks closed record fields, required predicate records, bundle size
limits, relative paths, SHA-256 identities against supplied file bytes, an ELF
header's declared architecture, and the shape of 18 G4 records (three tasks,
two implementations, three repetitions). It checks internal consistency between
claimed source, toolchain, command, HTTP, transaction and rollback fields. It
counts supplied trace entries and source patch lines. These are format and
consistency checks, not independent observations.

The following remain unobserved until a trusted execution harness produces and
verifies actual receipts:

- Whether the locked compiler/test commands ran with the claimed arguments,
  environment, output, exit status and duration.
- Whether native bytes are runnable compiler outputs. An ELF header alone does
  not establish executability or successful execution.
- Whether the two builds used clean independent environments and produced the
  supplied bytes, rather than copying one artifact twice.
- Whether external HTTP requests reached the stated binary and exercised every
  locked behavior, including timeouts and partial writes.
- Whether graph mutations, rejection, diagnostics, recovery and rollback occurred
  and the previous verified binary was rerun.
- Whether G4 workspaces were isolated, tasks used the locked inputs and Rust
  baseline, actual tool calls match the supplied trace, durations were measured,
  patches represent real workspace changes, and reported outcomes are authentic.

Content hashes detect mismatches between a record and supplied bytes. They do not
authenticate a record's author or prove an event occurred. Tests deliberately
construct synthetic successful records and a non-runnable ELF header; the
regression test requires that these can never become P10 acceptance verification.
Neither those fixtures nor derived comparison numbers are release evidence.

`contract.json` fixes the required record matrix and `bundle.schema.json`
describes the manifest. The helper validates referenced records directly and
rejects missing or inconsistent inputs. Missing evidence returns
`E_EVIDENCE_INCOMPLETE` / `BLOCKED`; it never creates release files.
