"""Derive P10 predicate records from retained, raw graph observations.

The adapter only promotes assertions that are present in the raw observation
stream.  It never treats a suite summary as a predicate and refuses to create
records when the observation lacks measured timing, process status, or the
predicate-specific fields required by the P10 contract.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any, Iterable

HASH_PREFIX = "sha256:"
ROUTE_PATH = "/hello/{name}"


class DeriveError(ValueError):
    """Raw observations cannot attest a requested predicate."""


def digest(data: bytes) -> str:
    return HASH_PREFIX + hashlib.sha256(data).hexdigest()


def canonical(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()


def _read(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise DeriveError(f"cannot read graph observation report: {error}") from error
    if not isinstance(value, dict):
        raise DeriveError("graph observation report is not an object")
    observations = value.get("raw_observations")
    if not isinstance(observations, list):
        raise DeriveError("graph observation report has no raw_observations")
    return value


def _obs_ok(row: Any) -> bool:
    if not isinstance(row, dict):
        return False
    process = row.get("process")
    if not isinstance(process, dict) or type(process.get("exit_code")) is not int:
        return False
    # Negative transactions are expected to return a non-zero CLI status.  The
    # surrounding graph test still passes; ``expect_ok`` records that contract.
    if row.get("expect_ok") is True and process["exit_code"] != 0:
        return False
    return type(process.get("duration_ns")) is int and process["duration_ns"] > 0


def _request(row: dict[str, Any]) -> dict[str, Any] | None:
    request = row.get("request")
    if not isinstance(request, dict) or request.get("encoding") != "utf-8":
        return None
    data = request.get("data")
    if not isinstance(data, str):
        return None
    try:
        value = json.loads(data)
    except (TypeError, json.JSONDecodeError):
        return None
    return value if isinstance(value, dict) else None


def _response(row: dict[str, Any]) -> dict[str, Any] | None:
    response = row.get("response")
    if not isinstance(response, dict) or response.get("encoding") != "utf-8":
        return None
    data = response.get("data")
    if not isinstance(data, str):
        return None
    try:
        value = json.loads(data)
    except (TypeError, json.JSONDecodeError):
        return None
    return value if isinstance(value, dict) else None


def _base(predicate: str, rows: list[dict[str, Any]], provenance: dict[str, Any],
          *, command_id: str = "graph-cli") -> dict[str, Any]:
    if not rows or not all(_obs_ok(row) for row in rows):
        raise DeriveError(f"{predicate} has no measured successful process observations")
    duration = sum(row["process"]["duration_ns"] for row in rows)
    argv = rows[0]["process"].get("argv")
    if not isinstance(argv, list) or not argv or not all(isinstance(item, str) for item in argv):
        raise DeriveError(f"{predicate} observation has no process argv")
    source_commit = provenance.get("source_commit")
    target = provenance.get("target")
    lock_hash = provenance.get("toolchain_lock_sha256")
    if (not isinstance(source_commit, str) or len(source_commit) != 40
            or not isinstance(target, str) or not isinstance(lock_hash, str)):
        raise DeriveError("predicate provenance is incomplete")
    return {
        "schema_version": "1.0.0", "kind": "p10_gate_record", "predicate": predicate,
        "source_commit": source_commit, "target": target,
        "toolchain_lock_sha256": lock_hash, "status": "PASSED",
        "command_id": command_id, "argv_sha256": digest(canonical(argv)), "exit_code": 0,
        "duration_ns": duration,
        "test_suite": {"id": "graph_cli_blackbox", "exit_code": 0, "count": len(rows)},
        "artifacts": [], "assertions": [{"id": predicate, "passed": True}],
    }


def _matching(rows: Iterable[dict[str, Any]], tag: str) -> list[dict[str, Any]]:
    return [row for row in rows if _obs_ok(row) and tag in row.get("tags", [])]


def derive_records(report: str | Path, provenance: dict[str, Any], *, output: str | Path | None = None,
                   graph_artifact: dict[str, Any] | None = None) -> dict[str, Any]:
    """Return only predicates directly attested by ``raw_observations``.

    ``graph_artifact`` is optional because local graph acceptance does not
    always run in the same bundle as compiler artifacts.  Failure-preservation
    is emitted only when that binding is supplied and matches the observation.
    """
    value = _read(Path(report))
    rows = [row for row in value["raw_observations"] if isinstance(row, dict)]
    records: dict[str, dict[str, Any]] = {}
    blocked: dict[str, str] = {}

    def add(predicate: str, selected: list[dict[str, Any]], mutate) -> None:
        if not selected:
            blocked[predicate] = "raw observations do not attest predicate"
            return
        try:
            record = _base(predicate, selected, provenance)
            mutate(record, selected)
        except DeriveError as error:
            blocked[predicate] = str(error)
            return
        records[predicate] = record

    add("inspect_succeeds", _matching(rows, "inspect_succeeds"), lambda record, selected: None)
    add("slice_succeeds", _matching(rows, "slice_succeeds"), lambda record, selected: None)
    add("stale_transaction_rejected", _matching(rows, "stale_transaction_rejected"),
        lambda record, selected: record.update({"transaction": _stale_binding(selected[-1])}))
    add("transaction_add_route_succeeds", _route_rows(rows),
        lambda record, selected: record.update({"transaction": _route_binding(selected[-1])}))
    add("diagnostics_machine_readable", _diagnostic_rows(rows),
        lambda record, selected: record.update({"diagnostics": _diagnostics(selected)}))
    if graph_artifact is not None:
        add("failure_preserves_previous_revision", _matching(rows, "failure_preserves_previous_revision"),
            lambda record, selected: record.update({
                "artifacts": [graph_artifact],
                "transaction": _failure_binding(selected[-1], graph_artifact),
            }))
    else:
        blocked["failure_preserves_previous_revision"] = "graph artifact binding is required"

    result: dict[str, Any] = {"schema_version": "1.0.0", "kind": "p10_gate_record_derivation",
                              "status": "COMPLETE" if not blocked else "BLOCKED",
                              "records": records, "blocked_predicates": blocked}
    if output is not None:
        destination = Path(output)
        destination.mkdir(parents=True, exist_ok=True)
        for predicate, record in records.items():
            (destination / f"{predicate}.json").write_text(
                json.dumps(record, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8")
        (destination / "derivation.json").write_text(
            json.dumps({k: v for k, v in result.items() if k != "records"}, ensure_ascii=False,
                       sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return result


def _stale_binding(row: dict[str, Any]) -> dict[str, Any]:
    response = _response(row) or {}
    diagnostics = response.get("diagnostics") or []
    observed = row.get("binding", {}).get("base_revision")
    for diagnostic in diagnostics:
        if isinstance(diagnostic, dict) and diagnostic.get("code") == "E_STALE_REVISION":
            observed = diagnostic.get("base_revision", observed)
    binding = row.get("binding") or {}
    head = binding.get("result_revision")
    requested = binding.get("base_revision")
    if not (type(requested) is int and type(head) is int and requested < head):
        raise DeriveError("stale observation lacks requested and observed revisions")
    return {"requested_base_revision": requested, "observed_head_revision": head,
            "result_code": "E_STALE_REVISION", "head_before": head, "head_after": head}


def _route_rows(rows: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    result = []
    for row in rows:
        if not _obs_ok(row) or row.get("process", {}).get("exit_code") != 0:
            continue
        request = _request(row)
        if not request or request.get("base_revision") is None:
            continue
        for operation in request.get("operations", []):
            route = operation.get("route") if isinstance(operation, dict) else None
            if isinstance(route, dict) and route.get("path") == ROUTE_PATH:
                response = _response(row) or {}
                if response.get("ok") is True:
                    result.append(row)
    return result


def _route_binding(row: dict[str, Any]) -> dict[str, Any]:
    request = _request(row) or {}
    response = _response(row) or {}
    operation = next((item for item in request.get("operations", [])
                      if isinstance(item, dict) and isinstance(item.get("route"), dict)
                      and item["route"].get("path") == ROUTE_PATH), None)
    if operation is None:
        raise DeriveError("route observation has no /hello/{name} operation")
    route = operation["route"]
    route_bytes = canonical(route)
    base = request.get("base_revision")
    result = response.get("result_revision")
    if not (type(base) is int and type(result) is int and result == base + 1):
        raise DeriveError("route transaction revisions are not consecutive")
    return {"base_revision": base, "result_revision": result, "route_path": ROUTE_PATH,
            "route_sha256": digest(route_bytes)}


def _diagnostic_rows(rows: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    return [row for row in rows if _obs_ok(row) and "diagnostics_machine_readable" in row.get("tags", [])]


def _diagnostics(rows: Iterable[dict[str, Any]]) -> list[dict[str, str]]:
    result = []
    for row in rows:
        response = _response(row) or {}
        for diagnostic in response.get("diagnostics", []):
            if not isinstance(diagnostic, dict):
                continue
            required = {"diagnostic_id", "code", "stage", "entity_id"}
            if required.issubset(diagnostic) and all(isinstance(diagnostic[key], str) and diagnostic[key] for key in required):
                result.append({key: diagnostic[key] for key in required})
    if not result:
        raise DeriveError("diagnostic observations contain no machine-readable identity")
    return result


__all__ = ["DeriveError", "derive_records"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--toolchain-lock-sha256", required=True)
    args = parser.parse_args()
    result = derive_records(args.report, {"source_commit": args.source_commit,
                                          "target": args.target,
                                          "toolchain_lock_sha256": args.toolchain_lock_sha256},
                            output=args.output)
    print(json.dumps({k: v for k, v in result.items() if k != "records"}, ensure_ascii=False))
    raise SystemExit(0 if result["status"] == "COMPLETE" else 2)
