---
rfc: 401
title: Locked packages, capability declarations and isolated builds
status: accepted
requires: [P10, E03]
extension: E04
authority: user_approved_implementation_plan
---

Packages use a UTF-8 `il.package.json` manifest containing name, SemVer version,
declared modules, direct dependency version constraints and requested capabilities.
`il.lock.json` records the complete transitive resolution, source identities and
SHA-256 content hashes. Both formats have closed schemas. No arbitrary install,
prepare or build scripts, native plugins or shell command fields are supported.

Support local package directories and explicitly configured HTTPS registries.
Resolution selects the highest non-prerelease version satisfying all constraints
for each package name; unsatisfiable constraints fail without silently creating
multiple versions. Prereleases require explicit selection. Resolve transitively
with deterministic name ordering and reject cycles. Lock generation is explicit;
ordinary builds never modify the lock or access the network.

Fetch is a separate capability-scoped operation. Verify downloaded archive hashes
before extraction, reject absolute/traversing paths and links escaping the package
root, and use a content-addressed cache. A local package is snapshotted and hashed
before building. Offline isolated builds receive only locked package contents,
toolchain and declared inputs; the sandbox denies network and undeclared host
paths. Missing sandbox support fails closed instead of claiming isolation.

Capabilities are declarations, never grants. The root host must explicitly inject
the union actually required by reachable code. Generate a CycloneDX JSON SBOM
including versions, source hashes, dependencies and declared licenses (unknown
when absent; never guessed). Artifact evidence binds lock and SBOM hashes.

Acceptance includes conflicting transitive versions, tampered archives, offline
success, missing cache, cyclic dependencies, traversal payloads, undeclared
capabilities, denied network/host access and deterministic lock/SBOM generation.
