"""Derive P10 G0 gate records from explicit CI test observations.

This module consumes the fixed CI runner's hashed logs and reports. A green
summary alone never creates a predicate: each record requires named passing
test rows belonging to its predicate-specific source suite.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
from typing import Any


class CiRecordError(ValueError):
    pass


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def canonical(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def _read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise CiRecordError(f"cannot read {path}: {error}") from error


def _gate(ci: dict[str, Any], name: str, base: Path) -> tuple[dict[str, Any], str]:
    gates = ci.get("gates")
    row = next((item for item in gates or [] if isinstance(item, dict) and item.get("name") == name), None)
    if (not isinstance(row, dict) or row.get("status") != "PASSED" or row.get("exit_code") != 0
            or type(row.get("elapsed_seconds")) not in (int, float) or row["elapsed_seconds"] <= 0
            or not isinstance(row.get("command"), list) or not row["command"]
            or not all(isinstance(item, str) for item in row["command"])):
        raise CiRecordError(f"CI gate {name} is not a measured successful command")
    log_name, expected_hash = row.get("log"), row.get("log_sha256")
    if not isinstance(log_name, str) or not isinstance(expected_hash, str):
        raise CiRecordError(f"CI gate {name} does not bind its raw log")
    path = (base / log_name).resolve()
    try:
        path.relative_to(base.resolve())
        raw = path.read_bytes()
    except (OSError, ValueError) as error:
        raise CiRecordError(f"CI gate {name} raw log is unavailable or outside report root") from error
    if hashlib.sha256(raw).hexdigest() != expected_hash:
        raise CiRecordError(f"CI gate {name} raw log hash mismatch")
    return row, raw.decode("utf-8", errors="strict")


def _passed_rust_tests(log: str) -> set[str]:
    passed: set[str] = set()
    for line in log.splitlines():
        match = re.fullmatch(r"test ([A-Za-z0-9_:]+) \.\.\. ok", line.strip())
        if match:
            passed.add(match.group(1).rsplit("::", 1)[-1])
    return passed


G0_RUST_TESTS = {
    "graph_schema_valid": {
        "optional_and_nullable_fields_follow_the_closed_graph_schema",
        "graph_schema_string_and_unique_array_constraints_are_enforced",
        "duplicate_checks_fail_schema_without_publishing",
    },
    "ownership_checker_verified": {
        "shared_loan_prevents_move_until_ended",
        "ownership_join_rejects_parameter_live_on_only_one_incoming_path",
        "metadata_cannot_hide_owned_moves_or_owned_results",
    },
    "MIR_verifier_verified": {
        "positive_independent_semantic_fixtures_lower_to_verified_mir",
        "mir_borrow_escape_and_forged_move_metadata_are_rejected",
        "malformed_mir_mutations_never_panic",
    },
    "interpreter_semantic_suite_passed": {
        "independent_fixture_contracts_all_pass",
        "structured_examples_execute_and_roundtrip",
        "dispatcher_executes_real_handlers_and_protocol_failures_without_network",
        "copy_drop_is_non_consuming_and_unit_calls_have_no_output",
        "owned_return_and_explicit_cleanup_order",
    },
}


def _semantic_cases(report: Any) -> set[str]:
    if (not isinstance(report, dict) or report.get("suite") != "semantic_cli_blackbox"
            or report.get("passed") is not True or type(report.get("count")) is not int
            or report["count"] <= 0 or not isinstance(report.get("cases"), list)):
        raise CiRecordError("semantic CLI report is not a completed passing case report")
    rows = report["cases"]
    if len(rows) != report["count"] or any(not isinstance(row, dict) or row.get("passed") is not True for row in rows):
        raise CiRecordError("semantic CLI case rows are incomplete or failing")
    return {row.get("id") for row in rows if isinstance(row.get("id"), str)}


def _record(predicate: str, gate: dict[str, Any], *, source_commit: str,
            target: str, toolchain_lock_sha256: str, suite_id: str,
            case_count: int) -> dict[str, Any]:
    return {
        "schema_version": "1.0.0", "kind": "p10_gate_record", "predicate": predicate,
        "source_commit": source_commit, "target": target,
        "toolchain_lock_sha256": toolchain_lock_sha256, "status": "PASSED",
        "command_id": gate["name"], "argv_sha256": digest(canonical(gate["command"])),
        "exit_code": 0, "duration_ns": round(gate["elapsed_seconds"] * 1_000_000_000),
        "test_suite": {"id": suite_id, "exit_code": 0, "count": case_count},
        "artifacts": [], "assertions": [{"id": predicate, "passed": True}],
    }


def derive_g0(ci_report: str | Path, *, provenance: dict[str, str],
              output: str | Path | None = None) -> dict[str, Any]:
    """Create only G0 records whose named passing observations are present."""
    ci_path = Path(ci_report).resolve()
    root = ci_path.parent.parent.parent
    ci = _read_json(ci_path)
    if not isinstance(ci, dict):
        raise CiRecordError("CI report must be an object")
    for field in ("source_commit", "target"):
        if not isinstance(provenance.get(field), str) or not provenance[field]:
            raise CiRecordError(f"missing {field} provenance")
    lock_hash = provenance.get("toolchain_lock_sha256")
    if not isinstance(lock_hash, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", lock_hash):
        raise CiRecordError("invalid toolchain lock hash")
    records: dict[str, dict[str, Any]] = {}
    blocked: dict[str, str] = {}
    try:
        rust_gate, rust_log = _gate(ci, "rust-tests", root)
        passed = _passed_rust_tests(rust_log)
    except (CiRecordError, UnicodeDecodeError) as error:
        rust_gate, passed = None, set()
        rust_error = str(error)
    else:
        rust_error = ""
    for predicate, required in G0_RUST_TESTS.items():
        if rust_gate is None:
            blocked[predicate] = rust_error
            continue
        missing = sorted(required - passed)
        if missing:
            blocked[predicate] = f"rust-tests raw log lacks named passing tests: {missing}"
            continue
        records[predicate] = _record(predicate, rust_gate, source_commit=provenance["source_commit"],
                                     target=provenance["target"], toolchain_lock_sha256=lock_hash,
                                     suite_id="rust_tests:" + predicate, case_count=len(required))
    predicate = "type_checker_verified"
    try:
        semantic_gate, _ = _gate(ci, "semantic-cli", root)
        semantic_name = next((item for item in semantic_gate["command"] if item.endswith("semantic_cli.py")), None)
        if semantic_name is None:
            raise CiRecordError("semantic CLI command does not name tests/semantic_cli.py")
        report_path = (root / "build" / "semantic_cli_report.json").resolve()
        semantic_cases = _semantic_cases(_read_json(report_path))
        required_cases = {"valid_integer_return", "valid_typed_call", "wrong_return_type",
                          "mixed_integer_add", "unknown_value", "integer_literal_out_of_range"}
        missing = sorted(required_cases - semantic_cases)
        if missing:
            raise CiRecordError(f"semantic report lacks passing checker cases: {missing}")
        records[predicate] = _record(predicate, semantic_gate, source_commit=provenance["source_commit"],
                                     target=provenance["target"], toolchain_lock_sha256=lock_hash,
                                     suite_id="semantic_cli_blackbox", case_count=len(required_cases))
    except (CiRecordError, OSError, UnicodeError, json.JSONDecodeError) as error:
        blocked[predicate] = str(error)
    # This predicate is attested by the exact independent graph CLI test rows,
    # not by its suite count. It uses the same adapter as all graph transaction
    # evidence so those records retain the raw request, response, process and
    # snapshot bindings.
    try:
        graph_path = (root / "build" / "graph_cli_report.json").resolve()
        graph_report = _read_json(graph_path)
        graph_tests = graph_report.get("raw_observations") if isinstance(graph_report, dict) else None
        graph_gate, _ = _gate(ci, "graph-cli", root)
        if graph_report.get("passed") is not True or not isinstance(graph_tests, list):
            raise CiRecordError("graph CLI report lacks passed raw observations")
        required_test = "test_validate_and_slice_use_structural_contract"
        observed = [row for row in graph_tests if isinstance(row, dict)
                    and row.get("test_case", "").endswith(required_test)
                    and isinstance(row.get("process"), dict)
                    and row["process"].get("exit_code") == 0
                    and type(row["process"].get("duration_ns")) is int
                    and row["process"]["duration_ns"] > 0
                    and isinstance(row.get("response"), dict)
                    and row["response"].get("encoding") == "utf-8"
                    and isinstance(row["response"].get("data"), str)
                    and json.loads(row["response"]["data"]).get("ok") is True]
        if not observed:
            raise CiRecordError("raw graph observations do not include the schema/validation test")
        graph_record = _record("graph_schema_valid", graph_gate,
                               source_commit=provenance["source_commit"], target=provenance["target"],
                               toolchain_lock_sha256=lock_hash, suite_id="graph_cli_blackbox",
                               case_count=len(observed))
        records["graph_schema_valid"] = graph_record
        blocked.pop("graph_schema_valid", None)
    except (CiRecordError, OSError, UnicodeError, json.JSONDecodeError, ValueError, TypeError) as error:
        blocked["graph_schema_valid"] = str(error)
    result = {"schema_version": "1.0.0", "kind": "p10_ci_record_derivation",
              "status": "COMPLETE" if not blocked else "BLOCKED",
              "records": records, "blocked_predicates": blocked}
    if output is not None:
        destination = Path(output)
        destination.mkdir(parents=True, exist_ok=True)
        for name, record in records.items():
            path = destination / f"{name}.json"
            data = json.dumps(record, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8") + b"\n"
            if path.exists() and path.read_bytes() != data:
                raise CiRecordError(f"refusing to replace differing candidate record: {path}")
            if not path.exists():
                path.write_bytes(data)
        status = {key: value for key, value in result.items() if key != "records"}
        (destination / "ci_derivation.json").write_text(
            json.dumps(status, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return result


def derive_graph_transactions(graph_report: str | Path, *, provenance: dict[str, str],
                              graph_artifact: dict[str, Any] | None = None,
                              ai_report: str | Path | None = None,
                              ci_report: str | Path | None = None,
                              output: str | Path | None = None) -> dict[str, Any]:
    """Derive G2 transaction predicates from the graph suite's raw observations."""
    from tools.p10.derive_records import derive_records

    result = derive_records(graph_report, provenance, graph_artifact=graph_artifact,
                            ai_report=ai_report, ci_report=ci_report, output=output)
    wanted = {"inspect_succeeds", "slice_succeeds", "transaction_add_route_succeeds",
              "stale_transaction_rejected", "failure_preserves_previous_revision",
              "diagnostics_machine_readable"}
    return {"schema_version": "1.0.0", "kind": "p10_graph_transaction_derivation",
            "status": "COMPLETE" if wanted.isdisjoint(result["blocked_predicates"]) else "BLOCKED",
            "records": {name: result["records"][name] for name in wanted if name in result["records"]},
            "blocked_predicates": {name: result["blocked_predicates"][name]
                                   for name in wanted if name in result["blocked_predicates"]}}


__all__ = ["CiRecordError", "derive_g0", "derive_graph_transactions", "G0_RUST_TESTS"]
