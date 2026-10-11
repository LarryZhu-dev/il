#!/usr/bin/env python3
"""Export explicit, auditable P10 gate records from a trusted run.

This helper is deliberately fail-closed: ordinary CI summaries, exit codes,
and test counts are never promoted to predicates.  Only already-emitted
``p10_gate_record`` documents with the minimum trusted provenance are copied.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any, Iterable

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "tools" / "p10" / "contract.json"
HASH_PREFIX = "sha256:"


class GateRecordError(ValueError):
    """Malformed trusted evidence input."""


def sha256_bytes(data: bytes) -> str:
    return HASH_PREFIX + hashlib.sha256(data).hexdigest()


def _read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise GateRecordError(f"cannot read JSON source {path}: {exc}") from exc


def required_predicates(contract: dict[str, Any] | None = None) -> tuple[str, ...]:
    if contract is None:
        contract = _read_json(CONTRACT)
    gates = contract.get("gates") if isinstance(contract, dict) else None
    if not isinstance(gates, dict):
        raise GateRecordError("P10 contract has no gate matrix")
    names: list[str] = []
    for group, predicates in gates.items():
        if not isinstance(predicates, list) or not all(isinstance(p, str) and p for p in predicates):
            raise GateRecordError(f"P10 contract gate {group!r} is malformed")
        names.extend(predicates)
    if len(names) != len(set(names)):
        raise GateRecordError("P10 contract repeats a predicate")
    return tuple(names)


def _iter_json_sources(root: Path) -> Iterable[Path]:
    if not root.is_dir():
        raise GateRecordError(f"trusted run is not a directory: {root}")
    # Supervisor runs contain very large compiler caches and G4 workspaces.
    # Walk them without descending into binary/cache trees; gate records are
    # expected in the small evidence/record areas (or a shallow test folder).
    skipped = {".git", "blobs", "target", "reproducibility", "g4", "build"}
    for directory, child_dirs, filenames in __import__("os").walk(root):
        child_dirs[:] = [name for name in child_dirs if name not in skipped]
        for filename in sorted(filenames):
            if not filename.endswith(".json"):
                continue
            path = Path(directory) / filename
            if path.is_symlink() or not path.is_file():
                continue
            yield path


def _is_digest(value: Any) -> bool:
    return (isinstance(value, str) and len(value) == 71 and value.startswith(HASH_PREFIX)
            and all(c in "0123456789abcdef" for c in value[7:]))


def _valid_record(record: Any, predicate: str) -> bool:
    """Check only identity fields; runner.py performs full semantic checks."""
    if not isinstance(record, dict) or record.get("kind") != "p10_gate_record":
        return False
    if record.get("predicate") != predicate or record.get("schema_version") != "1.0.0":
        return False
    if record.get("status") != "PASSED" or record.get("exit_code") != 0:
        return False
    commit = record.get("source_commit")
    if (not isinstance(commit, str) or len(commit) != 40
            or any(c not in "0123456789abcdef" for c in commit)):
        return False
    return (_is_digest(record.get("toolchain_lock_sha256"))
            and _is_digest(record.get("argv_sha256")))


def export_gate_records(run_root: str | Path, *, contract: dict[str, Any] | None = None,
                        output_root: str | Path | None = None) -> dict[str, Any]:
    """Discover explicit gate records and return a fail-closed export receipt."""
    root = Path(run_root).resolve()
    names = required_predicates(contract)
    records: dict[str, dict[str, Any]] = {}
    sources: list[dict[str, Any]] = []
    malformed: list[dict[str, str]] = []
    for path in _iter_json_sources(root):
        try:
            value = _read_json(path)
        except GateRecordError:
            sources.append({"path": path.relative_to(root).as_posix(), "kind": "unreadable"})
            continue
        if not isinstance(value, dict) or value.get("kind") != "p10_gate_record":
            continue
        predicate = value.get("predicate")
        relative = path.relative_to(root).as_posix()
        raw = path.read_bytes()
        if predicate not in names or not isinstance(predicate, str):
            malformed.append({"path": relative, "reason": "unknown predicate"})
            continue
        reference = {"path": relative, "sha256": sha256_bytes(raw)}
        sources.append({"path": relative, "kind": "p10_gate_record", "predicate": predicate,
                        "sha256": reference["sha256"]})
        if not _valid_record(value, predicate):
            malformed.append({"path": relative, "reason": "record is incomplete or not passing"})
            continue
        if predicate in records:
            malformed.append({"path": relative, "reason": f"duplicate predicate {predicate}"})
            continue
        records[predicate] = {"source": path, "reference": reference}
    # Keep references to the trusted run's ordinary reports even though they
    # cannot attest a predicate.  This makes a BLOCKED receipt actionable and
    # identifies exactly which observations need a producer-side adapter.
    for candidate in (
        root / "journal.json", root / "commands" / "ci.json",
        root / "commands" / "http.json", root / "commands" / "g4.json",
        root / "commands" / "reproducibility.json",
        root / "build" / "ci" / "result.json",
        root / "http" / "http_report.json",
        root / "g4" / "comparison.json",
        root / "reproducibility" / "reproduction.json",
    ):
        if candidate.is_file() and not candidate.is_symlink():
            relative = candidate.relative_to(root).as_posix()
            if not any(row.get("path") == relative for row in sources):
                sources.append({"path": relative, "kind": "trusted_run_report",
                                "sha256": sha256_bytes(candidate.read_bytes())})
    missing = [p for p in names if p not in records]
    result: dict[str, Any] = {
        "schema_version": "1.0.0", "kind": "p10_gate_record_export",
        "status": "COMPLETE" if not missing and not malformed else "BLOCKED",
        "records": {p: records[p]["reference"] for p in sorted(records)},
        "missing_predicates": missing, "malformed_records": malformed,
        "source_refs": sources,
    }
    if output_root is not None and records:
        destination = Path(output_root).resolve()
        destination.mkdir(parents=True, exist_ok=True)
        for predicate, payload in records.items():
            target = destination / f"{predicate}.json"
            data = payload["source"].read_bytes()
            if target.exists() and target.read_bytes() != data:
                raise GateRecordError(f"refusing to overwrite differing gate record: {target}")
            if not target.exists():
                target.write_bytes(data)
            result["records"][predicate] = {"path": target.name, "sha256": sha256_bytes(data)}
    return result


__all__ = ["GateRecordError", "export_gate_records", "required_predicates", "sha256_bytes"]
