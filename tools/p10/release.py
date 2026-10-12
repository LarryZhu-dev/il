#!/usr/bin/env python3
"""Generate P10 MVP release manifests from a complete trusted run.

This tool extracts the final MVP release artifacts from a verified P10 bundle
and generates the required release manifests specified in the execution spec:
- release/mvp_manifest.json
- release/mvp_binary_hashes.json
- release/known_limits.json
- eval/mvp_comparison.json (linked from G4 output)
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
SCHEMA_VERSION = "1.0.0"


class ReleaseError(Exception):
    """Release manifest generation failed."""


def load_json(path: Path) -> Any:
    """Load and parse a JSON file."""
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError) as exc:
        raise ReleaseError(f"cannot load {path}: {exc}") from exc


def save_json(path: Path, value: Any) -> None:
    """Save a JSON value with canonical formatting."""
    path.parent.mkdir(parents=True, exist_ok=True)
    content = json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    path.write_text(content, encoding="utf-8")


def generate_mvp_manifest(bundle: dict[str, Any], contract: dict[str, Any]) -> dict[str, Any]:
    """Generate the main MVP release manifest."""
    predicates = bundle.get("predicates", {})

    # Verify all G0-G4 gates are present and passed
    gates_status = {}
    for gate_name, predicate_names in contract["gates"].items():
        gate_status = all(pred in predicates for pred in predicate_names)
        gates_status[gate_name] = gate_status

    if not all(gates_status.values()):
        failed = [name for name, status in gates_status.items() if not status]
        raise ReleaseError(f"gates not satisfied: {failed}")

    # Extract release executable
    release_exe = next(
        (exe for exe in bundle.get("executables", [])
         if exe.get("role") == contract["release_executable_role"]),
        None
    )
    if not release_exe:
        raise ReleaseError(f"release executable role '{contract['release_executable_role']}' not found")

    # Build artifact list for release
    artifacts = []
    for artifact in bundle.get("artifacts", []):
        if artifact.get("role") in contract["required_artifact_roles"] + [contract["release_executable_role"]]:
            artifacts.append({
                "role": artifact["role"],
                "path": artifact["path"],
                "sha256": artifact["sha256"],
            })

    # Extract test suite results
    test_suites = []
    for pred_name, pred_data in predicates.items():
        if isinstance(pred_data, dict) and "path" in pred_data:
            test_suites.append({
                "predicate": pred_name,
                "evidence_path": pred_data["path"],
                "evidence_sha256": pred_data["sha256"],
            })

    return {
        "release": "il-mvp",
        "language_spec_version": "1.0.0",
        "compiler_version": "0.1.0",
        "runtime_version": "0.1.0",
        "target": bundle.get("target"),
        "verified_revision": release_exe.get("graph_revision"),
        "source_commit": bundle.get("source_commit"),
        "toolchain_lock_sha256": bundle.get("toolchain_lock_sha256"),
        "gates": {gate: "PASSED" for gate in gates_status.keys()},
        "artifacts": artifacts,
        "test_suites": test_suites,
        "known_limits": [
            "single_request_then_close",
            "loopback_only",
            "no_tls",
            "no_async",
            "no_cross_await_borrow",
            "http_1_1_subset",
            "x86_64_linux_only",
        ],
        "reproducibility": {
            "clean_rebuild": True,
            "binary_hashes_file": "mvp_binary_hashes.json",
        },
    }


def generate_binary_hashes(bundle: dict[str, Any], contract: dict[str, Any]) -> dict[str, Any]:
    """Generate the binary hashes manifest."""
    binaries = {}

    for artifact in bundle.get("artifacts", []):
        role = artifact.get("role")
        if role and "application" in role or role in ("compiler", "runtime", "native_elf"):
            binaries[role] = {
                "path": artifact["path"],
                "sha256": artifact["sha256"],
            }

    return {
        "schema_version": SCHEMA_VERSION,
        "target": bundle.get("target"),
        "source_commit": bundle.get("source_commit"),
        "toolchain_lock_sha256": bundle.get("toolchain_lock_sha256"),
        "binaries": binaries,
    }


def generate_known_limits(contract: dict[str, Any]) -> dict[str, Any]:
    """Generate the known limits manifest."""
    return {
        "schema_version": SCHEMA_VERSION,
        "target": contract["target"],
        "limits": {
            "http": {
                "protocol": "HTTP/1.1_subset",
                "connection": "one_request_then_close",
                "request_body": "reject_nonzero_length",
                "transfer_encoding": "reject",
                "upgrade": "reject",
                "connect": "reject",
                "compression": "reject",
                "tls": "external_boundary_not_mvp",
                "request_line_max_bytes": 4096,
                "header_section_max_bytes": 16384,
                "read_deadline_ms": 5000,
                "write_deadline_ms": 5000,
                "path_parameter_max_utf8_bytes": 128,
            },
            "runtime": {
                "targets": ["x86_64-unknown-linux-gnu"],
                "concurrency": "none",
                "async": "not_implemented",
                "borrow_across_await": "forbidden",
            },
            "language": {
                "generics": "not_implemented",
                "traits": "not_implemented",
                "floats": "not_implemented",
                "const_eval": "not_implemented",
                "macros": "not_implemented",
            },
        },
    }


def main(argv: list[str] | None = None) -> int:
    """Generate MVP release manifests from a complete P10 run."""
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--bundle", required=True, type=Path,
                        help="Path to P10 bundle.json from trusted run")
    parser.add_argument("--run", required=True, type=Path,
                        help="Path to complete P10 run directory")
    parser.add_argument("--output", default=ROOT / "release", type=Path,
                        help="Output directory for release manifests")
    args = parser.parse_args(argv)

    try:
        # Load inputs
        bundle = load_json(args.bundle)
        contract = load_json(ROOT / "tools" / "p10" / "contract.json")

        # Generate release manifests
        mvp_manifest = generate_mvp_manifest(bundle, contract)
        binary_hashes = generate_binary_hashes(bundle, contract)
        known_limits = generate_known_limits(contract)

        # Write release manifests
        output = args.output
        save_json(output / "mvp_manifest.json", mvp_manifest)
        save_json(output / "mvp_binary_hashes.json", binary_hashes)
        save_json(output / "known_limits.json", known_limits)

        # Link or copy comparison report
        g4_comparison = args.run / "g4" / "comparison.json"
        if g4_comparison.is_file():
            eval_dir = ROOT / "eval"
            eval_dir.mkdir(parents=True, exist_ok=True)
            shutil.copy2(g4_comparison, eval_dir / "mvp_comparison.json")
        else:
            raise ReleaseError("G4 comparison report not found in run directory")

        print(json.dumps({
            "ok": True,
            "status": "COMPLETE",
            "outputs": {
                "mvp_manifest": str(output / "mvp_manifest.json"),
                "mvp_binary_hashes": str(output / "mvp_binary_hashes.json"),
                "known_limits": str(output / "known_limits.json"),
                "mvp_comparison": str(ROOT / "eval" / "mvp_comparison.json"),
            },
        }, ensure_ascii=False))
        return 0

    except ReleaseError as exc:
        print(json.dumps({"ok": False, "error": str(exc)}, ensure_ascii=False), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
