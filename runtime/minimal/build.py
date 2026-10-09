#!/usr/bin/env python3
"""Build the freestanding archive with the exact bootstrap compiler, outside Cargo profiles."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    rustc = Path(subprocess.check_output(["/usr/local/cargo/bin/rustup", "which", "--toolchain", "1.90.0", "rustc"], text=True).strip()).resolve(strict=True)
    version = subprocess.check_output([str(rustc), "--version"], text=True).strip()
    if version != "rustc 1.90.0 (1159e78c4 2025-09-14)":
        raise SystemExit("minimal runtime requires the locked Rust 1.90.0 compiler")
    source = Path(__file__).resolve().parent / "src/lib.rs"
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    command = [str(rustc), str(source), "--crate-name", "il_minimal_runtime", "--crate-type", "staticlib",
               "--edition=2021", "--target", "x86_64-unknown-linux-gnu", "-C", "panic=abort", "-C", "opt-level=2",
               "-C", "codegen-units=1", "-C", "relocation-model=static", "-C", "overflow-checks=on",
               "--remap-path-prefix", f"{source.parent}=il/runtime/minimal", "-o", str(output)]
    subprocess.run(command, check=True)
    digest = lambda path: "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
    output.with_suffix(".build.json").write_text(json.dumps({"runtime_profile": "minimal", "compiler": version,
        "compiler_hash": digest(rustc.resolve()), "source_hash": digest(source), "archive_hash": digest(output),
        "command": command}, indent=2) + "\n", encoding="utf-8")

if __name__ == "__main__":
    main()
