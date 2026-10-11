#!/usr/bin/env python3
"""Reduce trusted CI observations into a P10 manifest, failing closed.

The CI runner emits gate summaries for ordinary build reporting.  Those
summaries are not P10 evidence: they do not contain the measured predicate
records required by ``runner.py``.  This reducer therefore accepts a complete
manifest only when the trusted CI report explicitly carries one (or points at
one), and otherwise emits a BLOCKED receipt.  It never invents predicate
records, hashes, durations, or artifact references.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "tools" / "p10" / "contract.json"
REQUIRED_MANIFEST_FIELDS = {
    "schema_version", "kind", "source_commit", "target", "toolchain_lock_sha256",
    "executables", "predicates", "artifacts", "comparison",
}


class ReductionError(ValueError):
    pass


def _read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise ReductionError(f"cannot read JSON report: {exc}") from exc


def required_predicates(contract: dict[str, Any]) -> tuple[str, ...]:
    gates = contract.get("gates")
    if not isinstance(gates, dict):
        raise ReductionError("P10 contract has no gate matrix")
    names: list[str] = []
    for group, predicates in gates.items():
        if not isinstance(predicates, list) or not all(isinstance(item, str) and item for item in predicates):
            raise ReductionError(f"P10 contract gate {group!r} is malformed")
        names.extend(predicates)
    if len(names) != len(set(names)):
        raise ReductionError("P10 contract repeats a predicate")
    return tuple(names)


def _candidate(report: dict[str, Any], base: Path) -> dict[str, Any]:
    value: Any = report.get("p10_manifest")
    if value is None:
        value = report.get("manifest")
    if isinstance(value, str):
        path = Path(value)
        if not path.is_absolute():
            path = base / path
        value = _read_json(path)
    if not isinstance(value, dict):
        raise ReductionError(
            "trusted CI report does not contain a complete p10_manifest; "
            "gate summaries cannot be promoted to P10 evidence"
        )
    return value


def _sha256(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _validate_gate_reference(
    reference: Any, *, predicate: str, base: Path
) -> dict[str, Any]:
    if not isinstance(reference, dict) or set(reference) != {"path", "sha256"}:
        raise ReductionError(f"P10 predicate {predicate} is not a record reference")
    path_value = reference["path"]
    digest = reference["sha256"]
    if not isinstance(path_value, str) or not path_value or not isinstance(digest, str):
        raise ReductionError(f"P10 predicate {predicate} has an invalid record reference")
    if (len(digest) != 71 or not digest.startswith("sha256:")
            or any(char not in "0123456789abcdef" for char in digest[7:])):
        raise ReductionError(f"P10 predicate {predicate} has an invalid record reference")
    candidate = Path(path_value)
    if not candidate.is_absolute():
        candidate = base / candidate
    # Check the path as named before resolve() follows links.  Checking only
    # the resolved destination would silently accept an in-tree symlink and
    # would make the reference change meaning if the link were retargeted.
    absolute = Path(candidate.absolute())
    try:
        lexical_relative = absolute.relative_to(base.absolute())
    except ValueError as exc:
        raise ReductionError(f"P10 predicate {predicate} reference is outside the trusted report") from exc
    cursor = base.absolute()
    if cursor.is_symlink():
        raise ReductionError("trusted report directory is a symbolic link")
    for part in lexical_relative.parts:
        cursor = cursor / part
        if cursor.is_symlink():
            raise ReductionError(f"P10 predicate {predicate} reference traverses a symbolic link")
    try:
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(base.resolve(strict=True))
    except (OSError, ValueError) as exc:
        raise ReductionError(f"P10 predicate {predicate} reference is outside the trusted report") from exc
    if not resolved.is_file():
        raise ReductionError(f"P10 predicate {predicate} reference is not a regular file")
    raw = resolved.read_bytes()
    if _sha256(raw) != digest:
        raise ReductionError(f"P10 predicate {predicate} record hash mismatch")
    try:
        record = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ReductionError(f"P10 predicate {predicate} record is not valid JSON") from exc
    if (not isinstance(record, dict) or record.get("kind") != "p10_gate_record"
            or record.get("predicate") != predicate):
        raise ReductionError(f"P10 predicate {predicate} record is not a matching p10_gate_record")
    return record


def reduce_report(
    report: Any, *, contract: dict[str, Any], base: Path | None = None
) -> dict[str, Any]:
    """Return a complete manifest or raise ``ReductionError``.

    Only fields already emitted by the trusted CI producer are accepted.  In
    particular, this function deliberately does not create references for the
    18 predicates or fill missing inventory/comparison data.
    """
    if not isinstance(report, dict) or report.get("status") not in {"PASSED", "COMPLETE"}:
        raise ReductionError("trusted CI report is not a successful completed report")
    manifest = _candidate(report, base or Path.cwd())
    if set(manifest) != REQUIRED_MANIFEST_FIELDS:
        missing = sorted(REQUIRED_MANIFEST_FIELDS - set(manifest))
        extra = sorted(set(manifest) - REQUIRED_MANIFEST_FIELDS)
        raise ReductionError(f"P10 manifest fields are incomplete (missing={missing}, extra={extra})")
    if manifest.get("schema_version") != "1.0.0" or manifest.get("kind") != "p10_acceptance_bundle":
        raise ReductionError("trusted CI manifest is not a p10_acceptance_bundle")
    if manifest.get("target") != contract.get("target"):
        raise ReductionError("P10 manifest target differs from the locked contract")
    source_commit = manifest.get("source_commit")
    toolchain_hash = manifest.get("toolchain_lock_sha256")
    if (not isinstance(source_commit, str) or len(source_commit) != 40
            or any(char not in "0123456789abcdef" for char in source_commit)):
        raise ReductionError("P10 manifest source commit is invalid")
    if (not isinstance(toolchain_hash, str) or len(toolchain_hash) != 71
            or not toolchain_hash.startswith("sha256:")
            or any(char not in "0123456789abcdef" for char in toolchain_hash[7:])):
        raise ReductionError("P10 manifest toolchain lock hash is invalid")
    predicates = manifest.get("predicates")
    names = set(required_predicates(contract))
    if not isinstance(predicates, dict):
        raise ReductionError("P10 manifest predicate records are not an object")
    if set(predicates) != names:
        missing = sorted(names - set(predicates))
        extra = sorted(set(predicates) - names)
        raise ReductionError(
            f"P10 manifest predicate records are incomplete (missing={missing}, extra={extra})"
        )
    for predicate, reference in predicates.items():
        if not isinstance(reference, dict) or set(reference) != {"path", "sha256"}:
            raise ReductionError(f"P10 predicate {predicate} is not a record reference")
        record = _validate_gate_reference(reference, predicate=predicate, base=base or Path.cwd())
        if (record.get("schema_version") != "1.0.0" or record.get("status") != "PASSED"
                or record.get("source_commit") != source_commit
                or record.get("target") != manifest["target"]
                or record.get("toolchain_lock_sha256") != toolchain_hash):
            raise ReductionError(f"P10 predicate {predicate} record provenance or status differs from manifest")
    if not isinstance(manifest.get("executables"), list) or not manifest["executables"]:
        raise ReductionError("P10 manifest executable inventory is incomplete")
    if not isinstance(manifest.get("artifacts"), list) or not manifest["artifacts"]:
        raise ReductionError("P10 manifest artifact inventory is incomplete")
    comparison = manifest.get("comparison")
    if not isinstance(comparison, dict) or not isinstance(comparison.get("trials"), list):
        raise ReductionError("P10 manifest G4 trial inventory is incomplete")
    g4_contract = contract.get("g4", {})
    tasks = g4_contract.get("tasks", {})
    implementations = g4_contract.get("implementations", [])
    trial_count = g4_contract.get("trial_count")
    expected = {
        (task, implementation, index)
        for task in tasks
        for implementation in implementations
        for index in range(1, trial_count + 1)
    }
    observed: set[tuple[str, str, int]] = set()
    for index, reference in enumerate(comparison["trials"]):
        if not isinstance(reference, dict) or set(reference) != {"path", "sha256"}:
            raise ReductionError(f"P10 G4 trial {index} is not a hashed record reference")
        trial_path = _validate_json_reference(reference, base=base or Path.cwd(), label=f"G4 trial {index}")
        record = _read_json(trial_path)
        if (not isinstance(record, dict) or record.get("schema_version") != "1.0.0"
                or record.get("kind") != "g4_trial"):
            raise ReductionError(f"P10 G4 trial {index} is not a g4_trial record")
        key = (record.get("task_id"), record.get("implementation"), record.get("trial_index"))
        if key not in expected or key in observed:
            raise ReductionError(f"P10 G4 trial {index} has an unknown or duplicate identity")
        observed.add(key)
    if observed != expected:
        raise ReductionError("P10 manifest G4 trial matrix is incomplete")
    return manifest


def _validate_json_reference(reference: dict[str, Any], *, base: Path, label: str) -> Path:
    path_value = reference.get("path")
    digest = reference.get("sha256")
    if (not isinstance(path_value, str) or not path_value
            or not isinstance(digest, str) or len(digest) != 71
            or not digest.startswith("sha256:")
            or any(char not in "0123456789abcdef" for char in digest[7:])):
        raise ReductionError(f"P10 {label} reference is malformed")
    candidate = Path(path_value)
    if not candidate.is_absolute():
        candidate = base / candidate
    absolute = Path(candidate.absolute())
    try:
        lexical_relative = absolute.relative_to(base.absolute())
    except ValueError as exc:
        raise ReductionError(f"P10 {label} reference is outside the trusted report") from exc
    cursor = base.absolute()
    if cursor.is_symlink():
        raise ReductionError("trusted report directory is a symbolic link")
    for part in lexical_relative.parts:
        cursor = cursor / part
        if cursor.is_symlink():
            raise ReductionError(f"P10 {label} reference traverses a symbolic link")
    try:
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(base.resolve(strict=True))
    except (OSError, ValueError) as exc:
        raise ReductionError(f"P10 {label} reference is outside the trusted report") from exc
    if not resolved.is_file():
        raise ReductionError(f"P10 {label} reference is not a regular file")
    raw = resolved.read_bytes()
    if _sha256(raw) != digest:
        raise ReductionError(f"P10 {label} reference hash mismatch")
    return resolved


def reduce_file(report_path: Path, output_path: Path) -> dict[str, Any]:
    contract = _read_json(CONTRACT)
    report = _read_json(report_path)
    try:
        manifest = reduce_report(report, contract=contract, base=report_path.resolve().parent)
    except ReductionError as exc:
        result = {"ok": False, "status": "BLOCKED", "code": "E_EVIDENCE_INCOMPLETE", "error": str(exc)}
    else:
        result = {"ok": True, "status": "READY", "manifest": manifest}
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    result = reduce_file(args.report, args.output)
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return 0 if result["ok"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
