"""Derive explicit P10 HTTP gate evidence from a successful trusted report.

This adapter never treats suite counts as case evidence. It verifies every
receipt and referenced byte before returning a gate record; incomplete input
returns a BLOCKED diagnostic with no records.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path, PurePosixPath
from typing import Any

from tools.p10 import runner


ROOT = Path(__file__).resolve().parents[2]


def _digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


class _RunReader:
    def __init__(self, root: Path):
        self.root = root.resolve(strict=True)

    def resolve(self, text: Any) -> Path:
        if not isinstance(text, str) or not text:
            raise ValueError("reference path is empty or not a string")
        candidate = Path(text)
        if not candidate.is_absolute():
            if "\\" in text or ":" in text:
                raise ValueError(f"reference path is not a relative POSIX path: {text}")
            pure = PurePosixPath(text)
            if pure.is_absolute() or any(part in {"", ".", ".."} for part in text.split("/")):
                raise ValueError(f"reference path is not canonical: {text}")
            candidate = self.root.joinpath(*pure.parts)
        try:
            lexical = candidate.absolute().relative_to(self.root)
        except ValueError as error:
            raise ValueError(f"reference escapes trusted run: {text}") from error
        cursor = self.root
        for part in lexical.parts:
            cursor = cursor / part
            if cursor.is_symlink():
                raise ValueError(f"reference traverses a symbolic link: {text}")
        try:
            resolved = candidate.resolve(strict=True)
            resolved.relative_to(self.root)
        except (OSError, ValueError) as error:
            raise ValueError(f"reference is missing or outside trusted run: {text}") from error
        if not resolved.is_file():
            raise ValueError(f"reference is not a regular file: {text}")
        return resolved

    def read_ref(self, reference: Any, *, role: str | None = None):
        allowed = {"path", "sha256"} if role is None else {"path", "sha256", "role"}
        if not isinstance(reference, dict) or set(reference) != allowed:
            raise ValueError("byte reference fields are malformed")
        path = self.resolve(reference["path"])
        data = path.read_bytes()
        if reference["sha256"] != _digest(data):
            raise ValueError(f"reference hash mismatch: {reference['path']}")
        if role is not None and reference["role"] != role:
            raise ValueError(f"artifact role mismatch: {reference['path']}")
        normalized = {"path": path.relative_to(self.root).as_posix(), "sha256": _digest(data)}
        return data, normalized

    def read_json_ref(self, reference: Any, *, role: str | None = None):
        data, normalized = self.read_ref(reference, role=role)
        try:
            value = json.loads(data.decode("utf-8"), object_pairs_hook=_unique_object)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValueError(f"referenced record is not valid UTF-8 JSON: {normalized['path']}") from error
        return value, normalized


def _unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON field: {key}")
        value[key] = item
    return value


def _normalize_refs(value: Any, reader: _RunReader) -> Any:
    if isinstance(value, dict):
        result = {key: _normalize_refs(item, reader) for key, item in value.items()}
        if set(result) == {"path", "sha256", "size_bytes"}:
            data, normalized = reader.read_ref({"path": result["path"], "sha256": result["sha256"]})
            if type(result["size_bytes"]) is not int or result["size_bytes"] != len(data):
                raise ValueError(f"byte reference size mismatch: {result['path']}")
            result["path"] = normalized["path"]
        elif set(result) == {"path", "sha256"}:
            reader.read_ref(result)
            result["path"] = reader.resolve(result["path"]).relative_to(reader.root).as_posix()
        return result
    if isinstance(value, list):
        return [_normalize_refs(item, reader) for item in value]
    return value


def _reference(value: Any, reader: _RunReader) -> dict[str, str]:
    if not isinstance(value, dict) or set(value) != {"path", "sha256"}:
        raise ValueError("receipt reference must contain exactly path and sha256")
    data, normalized = reader.read_ref(value)
    return {"path": normalized["path"], "sha256": _digest(data)}


def _validate_http_report(report: dict[str, Any], reader: _RunReader, contract: dict[str, Any], contract_sha: str):
    if report.get("suite") != "http_external_blackbox" or report.get("state") != "PASSED" or report.get("passed") is not True:
        raise ValueError("HTTP report is not a passed external blackbox run")
    if report.get("contract_sha256") != contract_sha:
        raise ValueError("HTTP report is not bound to the committed spec/http.yaml")
    http = report.get("http")
    if not isinstance(http, dict) or http.get("client") != "external_tcp" or not isinstance(http.get("cases"), list):
        raise ValueError("HTTP report has no explicit external TCP receipt matrix")

    expected = {(case, profile) for case in contract["http_cases"] for profile in contract["required_http_profiles"]}
    seen: set[tuple[str, str]] = set()
    cases = []
    binaries: dict[str, dict[str, str]] = {}
    for row in http["cases"]:
        if not isinstance(row, dict) or set(row) != set(contract["http_case_fields"]):
            raise ValueError("HTTP case row fields differ from the locked contract")
        identity, profile = row.get("id"), row.get("profile")
        key = (identity, profile)
        if key not in expected or key in seen:
            raise ValueError(f"unknown or duplicate HTTP case/profile: {key}")
        if row.get("exit_code") != 0 or row.get("count") != 1:
            raise ValueError(f"HTTP case receipt did not pass: {identity}/{profile}")
        receipt_ref = _reference(row.get("receipt"), reader)
        receipt, _ = reader.read_json_ref(receipt_ref)
        receipt = runner.exact_fields(receipt, set(contract["http_receipt_fields"]), path=f"http.{identity}.{profile}")
        if (receipt["schema_version"] != "1.0.0" or receipt["kind"] != "http_exchange"
                or receipt["case_id"] != identity or receipt["client"] != "external_tcp"
                or receipt["profile"] != profile or receipt["contract_sha256"] != contract_sha):
            raise ValueError(f"HTTP receipt identity/contract binding mismatch: {identity}/{profile}")
        role = receipt["executable_role"]
        if role != "application_" + profile:
            raise ValueError(f"HTTP receipt executable role mismatch: {identity}/{profile}")

        exchange, _ = reader.read_json_ref(receipt["exchange"])
        process, _ = reader.read_json_ref(receipt["process"])
        policy_data, policy_ref = reader.read_ref(receipt["policy"])
        if not isinstance(exchange, dict) or not isinstance(process, dict):
            raise ValueError(f"HTTP exchange/process receipt is malformed: {identity}/{profile}")
        normalized_exchange = _normalize_refs(exchange, reader)
        # Validate wire bytes and timing with the same locked checker consumed by the final bundle verifier.
        runner.check_http_exchange(reader, normalized_exchange, path=f"http.{identity}.{profile}.exchange", contract=contract)
        executable = reader.resolve(process.get("argv", [None])[0] if isinstance(process.get("argv"), list) and process.get("argv") else None)
        executable_bytes = executable.read_bytes()
        runner.require_elf_x86_64(executable_bytes, path=executable.relative_to(reader.root).as_posix())
        executable_hash = _digest(executable_bytes)
        process = _normalize_refs(process, reader)
        runner.check_http_process(reader, process, path=f"http.{identity}.{profile}.process", executable_hash=executable_hash)
        if process["profile"] != profile or process["policy_sha256"] != policy_ref["sha256"]:
            raise ValueError(f"HTTP process is not bound to the receipt policy/profile: {identity}/{profile}")
        if role in binaries and binaries[role]["sha256"] != executable_hash:
            raise ValueError(f"HTTP profile is served by multiple executable hashes: {profile}")
        binaries[role] = {"path": executable.relative_to(reader.root).as_posix(), "sha256": executable_hash}

        assertions = receipt["assertions"]
        if not isinstance(assertions, list) or not assertions:
            raise ValueError(f"HTTP receipt has no assertions: {identity}/{profile}")
        assertion_ids = set()
        for assertion in assertions:
            assertion = runner.exact_fields(assertion, {"id", "passed"}, path=f"http.{identity}.{profile}.assertions")
            if not isinstance(assertion["id"], str) or not assertion["id"] or assertion["passed"] is not True:
                raise ValueError(f"HTTP assertion failed: {identity}/{profile}")
            assertion_ids.add(assertion["id"])
        if identity == "partial_write":
            if (exchange.get("mode") != "slow_reader" or exchange.get("response_read") is not False
                    or exchange.get("terminal") != "client_close_without_read"):
                raise ValueError(f"partial_write lacks a bound slow-reader exchange: {profile}")
            stderr, _ = runner.read_byte_ref(reader, process["stderr"], path=f"http.{identity}.{profile}.process.stderr")
            required_assertions = {"slow_reader_closed_without_response_read", "server_write_timeout_observed"}
            if b"E_HTTP_WRITE_TIMEOUT" not in stderr or not required_assertions.issubset(assertion_ids):
                raise ValueError(f"partial_write lacks process timeout evidence/assertions: {profile}")

        # Rebase receipt and nested references under the trusted run for bundle assembly.
        receipt_value = dict(receipt)
        receipt_value["exchange"] = _reference(receipt["exchange"], reader)
        receipt_value["process"] = _reference(receipt["process"], reader)
        receipt_value["policy"] = {"path": policy_ref["path"], "sha256": policy_ref["sha256"]}
        cases.append({"id": identity, "profile": profile, "exit_code": 0, "count": 1,
                      "receipt": receipt_ref, "receipt_record": receipt_value,
                      "exchange_record": normalized_exchange, "process_record": process,
                      "policy_bytes_sha256": _digest(policy_data)})
        seen.add(key)
    missing = sorted(expected - seen)
    if missing:
        raise ValueError(f"HTTP blackbox evidence omits case/profile receipts: {missing}")
    return cases, binaries


def derive_http_gate(report_path: str | Path, *, run_root: str | Path,
                     provenance: dict[str, Any], command: dict[str, Any]) -> dict[str, Any]:
    """Return a P10 HTTP gate record only after validating all 30 raw receipts.

    ``command`` must be the measured supervisor command observation; this
    function requires its actual argv, status and duration rather than filling
    those fields from a report summary.
    """
    blocked = {"schema_version": "1.0.0", "kind": "p10_http_gate_derivation",
               "status": "BLOCKED", "code": "E_EVIDENCE_INCOMPLETE", "records": {},
               "blocked_reasons": []}
    try:
        root = Path(run_root).resolve(strict=True)
        reader = _RunReader(root)
        report_file = reader.resolve(str(Path(report_path).resolve()))
        report = json.loads(report_file.read_bytes().decode("utf-8"), object_pairs_hook=_unique_object)
        if not isinstance(report, dict):
            raise ValueError("HTTP report is not a JSON object")
        contract, contract_bytes = runner.load_contract()
        spec_path = ROOT / "spec" / "http.yaml"
        contract_sha = _digest(spec_path.read_bytes())
        cases, binaries = _validate_http_report(report, reader, contract, contract_sha)
        required_provenance = {"source_commit", "target", "toolchain_lock_sha256"}
        if not isinstance(provenance, dict) or set(provenance) != required_provenance:
            raise ValueError("explicit source commit, target, and toolchain lock provenance are required")
        if (not runner.is_commit(provenance["source_commit"]) or provenance["target"] != contract["target"]
                or not runner.is_sha256(provenance["toolchain_lock_sha256"])):
            raise ValueError("P10 HTTP provenance is invalid or differs from locked target")
        if (not isinstance(command, dict) or set(command) != {"command_id", "argv", "exit_code", "duration_ns"}
                or not isinstance(command["command_id"], str) or not command["command_id"]
                or not isinstance(command["argv"], list) or not command["argv"]
                or not all(isinstance(arg, str) for arg in command["argv"])
                or command["exit_code"] != 0 or type(command["duration_ns"]) is not int
                or command["duration_ns"] <= 0):
            raise ValueError("measured successful supervisor command observation is required")
        argv_bytes = json.dumps(command["argv"], ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
        record = {
            "schema_version": "1.0.0", "kind": "p10_gate_record",
            "predicate": "blackbox_tests_external", **provenance, "status": "PASSED",
            "command_id": command["command_id"], "argv_sha256": _digest(argv_bytes),
            "exit_code": 0, "duration_ns": command["duration_ns"],
            "test_suite": {"id": "http_external_blackbox", "exit_code": 0, "count": len(cases)},
            "artifacts": [], "assertions": [{"id": "blackbox_tests_external", "passed": True}],
            "http": {"client": "external_tcp", "cases": [
                {key: row[key] for key in ("id", "profile", "exit_code", "count", "receipt")}
                for row in cases]},
        }
        return {"schema_version": "1.0.0", "kind": "p10_http_gate_derivation", "status": "COMPLETE",
                "code": None, "records": {"blackbox_tests_external": record},
                "blocked_reasons": [], "case_evidence": cases, "executables": binaries,
                "http_contract_sha256": contract_sha}
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError, KeyError, TypeError,
            runner.EvidenceError) as error:
        blocked["blocked_reasons"].append({"code": "E_EVIDENCE_INCOMPLETE", "message": str(error)})
        return blocked


__all__ = ["derive_http_gate"]
