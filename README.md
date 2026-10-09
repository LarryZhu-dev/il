# Intelligent language (il)

il is an AI-oriented, ahead-of-time compiled programming language. Text files use
the `.il` extension; the canonical program representation is a structured graph.
The bootstrap compiler is written in Rust and the native backend is LLVM.

P00–P04 are verified, including a successful
[GitHub Actions run](https://github.com/LarryZhu-dev/il/actions/runs/37772237811).
The verified implementation includes the shared static checker, structured text
frontend, semantic graph transactions, HIR/MIR, bounded interpreter and LLVM native
compiler. Operating-system runtime, HTTP packages, extensions and self-hosting
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

P05 native compilation is verified on Linux x86-64, including interpreter/native
parity, real debug/release ELF execution, output failures and reproducible native
artifacts. Immutable evidence is in `eval/evidence/ev_P05_0.json`; the matching
[hosted acceptance run](https://github.com/LarryZhu-dev/il/actions/runs/37871320400)
passed. P06 operating-system resources and runtime modes remain under development.
Build the CLI and native runtime
archive together with `./tools/dev.ps1 build`. To publish and compile
an application into a real Linux ELF in Docker:

```powershell
@{
    task_id = 'native-hello'
    base_revision = 0
    scope = @('program')
    operations = @(@{
        op = 'import_text'
        source = Get-Content examples/core/native_hello.il -Raw -Encoding utf8
    })
    required_checks = @()
} | ConvertTo-Json -Depth 10 | ./tools/dev.ps1 il --store /workspace/.il/hello transact

@{ revision = 1; target = 'x86_64-unknown-linux-gnu'; profile = 'release' } |
    ConvertTo-Json | ./tools/dev.ps1 il --store /workspace/.il/hello build
```

The example uses a fresh store; subsequent transactions use its current revision.
`result.native.artifacts.executable.path` identifies the ELF inside Docker. Native
compilation requires the runtime archive beside the CLI executable. The request's profile controls optimization of the generated application.
`test` also accepts `native_debug` and `native_release` isolation with the same
explicit suite arguments and limits as `captured`. RFC 0017 defines the build,
execution and persistent candidate records.

## P06 runtime integration

P06 is under acceptance. RFC 0018 defines owned `core.File` resources, absolute
`core.Deadline` values, partial I/O, failure injection and independent runtime
profiles. The checked source packages in `packages/core`, `packages/alloc` and
`packages/io` compose with `examples/hello/main.il`; their manifests describe the
available APIs and ownership. Package dependency resolution remains E04 work.

A trusted launcher supplies `--host-policy /absolute/policy.json` before the CLI
command. `schema/host_policy.schema.json` defines the closed policy. Graph
capabilities must exactly match injected IDs, kinds and scopes; privileged calls
name a selector, for example `runtime.file_read[read_grant](path)`. File paths are
relative to held directory grants. Standalone Full executables receive policy
JSON on fd 4; captured reports use fd 3. Fault injection is captured-test-only.

Build requests may set `runtime_profile` independently of `profile`: `full`
(default), `minimal` (static syscall runtime), or `none` (freestanding object).
`none` requires a nonempty `exports` list of public scalar-ABI functions and
returns null executable/runtime artifacts. `./tools/dev.ps1 build` also builds
the Minimal archive beside the CLI. The independent P06 process suite checks
real file and pipe behavior, authorization, resource cleanup and clean-build
hashes; workflow configuration is not evidence that these gates passed.

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
