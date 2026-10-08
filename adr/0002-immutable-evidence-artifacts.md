# Immutable evidence artifacts

Status: accepted under the approved implementation plan.

Verification artifacts are retained as gzip-compressed, content-addressed files in
`eval/artifacts/<uncompressed-sha256>.gz`. The evidence records the original output
path and the SHA-256 of the actual uncompressed bytes. Archives are immutable and
are checked by decompression and hashing, including on clean CI checkouts. Source
provenance and the toolchain lock are resolved at the evidence's recorded Git commit.

Rebuilding or extending a compiler must not invalidate historical evidence by
overwriting its binary. Keeping only a path into a mutable build directory would
lose the previous verified revision. A new verification adds new evidence and
new content-addressed artifacts; it never rewrites old records. Release gates
separately test current outputs and reproducibility; possessing an old archive
does not prove the current source or a new platform passes.
