# Intelligent language (il)

il is an AI-oriented, ahead-of-time compiled programming language. Text files use
the `.il` extension; the canonical program representation is a structured graph.
The bootstrap compiler is written in Rust and the native backend is LLVM.

P00–P04 are verified, including a successful
[GitHub Actions run](https://github.com/LarryZhu-dev/il/actions/runs/37772237811).
The verified implementation includes the shared static checker, structured text
frontend, semantic graph transactions, HIR/MIR and bounded interpreter. The full compiler, runtime, HTTP package, extensions and self-hosting
are not yet verified. `repository_state.json`, task
records and successful machine-readable evidence determine milestone status.
No release is available until all of its gates pass.

The bootstrap CLI accepts JSON on standard input. `schema-check` previews `.il`
text as a checked graph and canonical annotated source; `transact` with an
`import_text` operation and `program` scope publishes it atomically. `validate`
always runs all applicable checks, even when the requested check list is empty.
See RFC 0005 for the text syntax and RFC 0008 for transaction examples. Text and
graph callers share the same checker and cannot declare their own host authority.

On Windows with Docker Desktop, build the locked environment with
`./tools/dev.ps1 image` and pass JSON requests through standard input:

```powershell
'{}' | ./tools/dev.ps1 il state
```

P04's `il test` executes a published graph through checked HIR and verified MIR
with captured output and explicit step, call-depth, heap and output budgets.
Its JSON interface is defined by `schema/tool.schema.json` and RFC 0013. A passing
interpreter result does not claim native-code or operating-system runtime support.

## Normative inputs

- The development document is
  [il_execution_spec_v1.md](il_execution_spec_v1.md). Its product spelling was
  normalized by accepted RFC 0001; former spellings are not compatibility aliases.
- [Language specifications](spec/language.yaml) and the other files in `spec/`
  make the locked rules machine-readable.
- [Accepted bootstrap RFC](rfc/0001-bootstrap-and-identity.md) resolves initial
  evidence and Git binding; other accepted RFCs specify missing semantics.
- Schemas in `schema/` define structured interfaces. Unknown fields are rejected.

## Roadmap and acceptance

| Milestone | Deliverable |
| --- | --- |
| P00 | Repository, schemas, locked toolchain, CI and evidence contract |
| P01–P02 | Native ELF probe; canonical graphs and atomic transactions |
| P03–P05 | Static semantics, text frontend, MIR interpreter and LLVM compiler |
| P06–P07 | Runtime, standard packages and native HTTP service |
| P08–P10 | AI protocol, fault recovery, independent tests and MVP gates |
| E01 | Generics, static traits, floats, constant evaluation, controlled macros |
| E02 | Threads, mutexes, channels, atomics, tasks and async |
| E03 | Linux x86-64/AArch64, macOS x86-64/AArch64, Windows x86-64 MSVC |
| E04 | Reproducible package resolution, isolated builds and SBOM |
| E05 | il formatter, graph tools, frontend and self-hosted compiler |

The first target is `x86_64-unknown-linux-gnu`. Rust 1.90.0 and LLVM 14.0.6
are the baseline bootstrap tools; exact dependency and tool identities belong in
the lock files. Windows is a development host, not evidence that Linux native
execution or Windows target support has passed. A CI configuration alone is not
execution evidence.

Development follows verified dependencies, keeps failed candidates and previous
verified revisions, and records real command exits and artifact hashes. Extensions
start only after P10; each target needs native execution and reproducibility
evidence. The eventual compiler must emit LLVM/native code directly, never
translate target programs into another high-level language.
