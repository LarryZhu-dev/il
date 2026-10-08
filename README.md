# Intelligent language (il)

il is an AI-oriented, ahead-of-time compiled programming language. Text files use
the `.il` extension; the canonical program representation is a structured graph.
The bootstrap compiler is written in Rust and the native backend is LLVM.

This repository is under initial development. The documents below describe the
intended language, not a claim that the compiler, runtime, HTTP package, extensions,
or self-hosting already work. `repository_state.json`, task records and successful
machine-readable evidence determine which milestones are verified. No release is
available until all of its gates pass.

## Normative inputs

- The original development document is retained in
  [aip_ai_execution_spec_v1.md](aip_ai_execution_spec_v1.md) for provenance. Its
  former product spelling is superseded by accepted RFC 0001; it is not an alias.
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
