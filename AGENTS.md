# il development rules

- Read `repository_state.json`, the selected task and its dependencies before work.
  Mutate a task only when every dependency is `VERIFIED`.
- The original development document plus accepted RFCs and repository state are
  authoritative. Product spelling is `Intelligent language`, `il`, `.il`; do not
  add compatibility for the old document spelling.
- Program changes go through canonical graph transactions. Compiler development
  is ordinary reviewed source work; do not confuse the two mutation domains.
- Use the Rust bootstrap and locked LLVM backend. Do not generate C, Rust,
  JavaScript or another high-level language as the target program.
- Preserve failed evidence and the previous verified revision. Never weaken a
  test, invent a hash, mark an unexecuted target verified, or fabricate a release.
- New locked semantics require an accepted RFC. Missing features are explicit
  unsupported diagnostics, never silent fallbacks or successful placeholders.
- Use subagents for complex independent work; assign disjoint file ownership.
- Run checks at module/stage milestones, after broad changes, or when requested;
  do not run formatting, lint, build and tests after every small edit.
- Do not use browsers without explicit permission in the current user turn or
  active goal mode. Screenshot analysis requires explicit user permission.
- Start development services only for necessary direct API tests; reuse existing
  services and hot reload. Shut down temporary extra services before finishing.
  The final response states services started/restarted and manual restart needs.
- Use Conventional Commits, split independent subsystem changes, and do not
  include another contributor's unrelated work. If repository location is
  unknown, inspect only the current directory and its immediate children.
- Replace superseded code completely. Do not retain compatibility branches,
  dual reads/writes, wrappers, old fields, commented-out logic or migration-only
  business constraints unless explicitly requested.
- The untrusted AI tool protocol never accepts arbitrary shell commands. Trusted
  development uses the locked bootstrap/CI commands described in RFC 0001.
