"""Build a fail-closed P10 seed from a trusted run's captured reports.

This module is intentionally a small adapter.  A command exit status, test
count, or ordinary gate summary is not a P10 predicate.  A seed is emitted
only when the CI command itself captured an explicit ``p10_acceptance_bundle``
manifest (or a nested ``manifest`` object); every referenced byte is checked
against the trusted run before it is returned.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
from typing import Any

HASH_PREFIX = "sha256:"
TARGET = "x86_64-unknown-linux-gnu"
COMMANDS = ("ci", "http", "g4", "reproducibility")


class SeedManifestError(ValueError):
    """The trusted run cannot provide a safe seed."""


def sha256(data: bytes) -> str:
    return HASH_PREFIX + hashlib.sha256(data).hexdigest()


def _hash(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 71 and value.startswith(HASH_PREFIX) and all(
        c in "0123456789abcdef" for c in value[7:]
    )


def _json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise SeedManifestError(f"invalid JSON report {path}: {exc}") from exc


def _relative(value: Any) -> str:
    if not isinstance(value, str) or not value or "\\" in value or ":" in value:
        raise SeedManifestError("evidence path is not a canonical relative POSIX path")
    p = PurePosixPath(value)
    if p.is_absolute() or any(part in {"", ".", ".."} for part in value.split("/")):
        raise SeedManifestError(f"evidence path escapes trusted run: {value!r}")
    return p.as_posix()


def _check_ref(root: Path, ref: Any, label: str) -> dict[str, str]:
    if not isinstance(ref, dict) or set(ref) != {"path", "sha256"} or not _hash(ref.get("sha256")):
        raise SeedManifestError(f"{label} is not a record reference")
    relative = _relative(ref["path"])
    candidate = root / Path(*relative.split("/"))
    try:
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(root.resolve(strict=True))
    except (OSError, ValueError) as exc:
        raise SeedManifestError(f"{label} is outside trusted run") from exc
    if candidate.is_symlink() or not resolved.is_file():
        raise SeedManifestError(f"{label} is not a regular file")
    actual = sha256(resolved.read_bytes())
    if actual != ref["sha256"]:
        raise SeedManifestError(f"{label} hash mismatch")
    return {"path": relative, "sha256": actual}


def _contract(contract: dict[str, Any] | None) -> dict[str, Any]:
    if contract is None:
        contract = _json(Path(__file__).with_name("contract.json"))
    if not isinstance(contract, dict) or not isinstance(contract.get("gates"), dict):
        raise SeedManifestError("P10 contract has no gate matrix")
    return contract


def _predicates(contract: dict[str, Any]) -> list[str]:
    result: list[str] = []
    for rows in contract["gates"].values():
        if not isinstance(rows, list) or not all(isinstance(row, str) and row for row in rows):
            raise SeedManifestError("P10 contract gate matrix is malformed")
        result.extend(rows)
    if len(result) != len(set(result)):
        raise SeedManifestError("P10 contract repeats a predicate")
    return result


def _ci_report(root: Path) -> tuple[Path | None, Any]:
    path = root / "build" / "ci" / "result.json"
    if not path.is_file():
        command = root / "commands" / "ci.json"
        if command.is_file():
            observation = _json(command)
            for row in observation.get("outputs", []) if isinstance(observation, dict) else []:
                if isinstance(row, dict) and row.get("present") is True and row.get("path", "").endswith("result.json"):
                    path = root / Path(*row["path"].split("/"))
                    break
    return (path, _json(path)) if path.is_file() else (None, None)


def _report_refs(root: Path) -> list[dict[str, str]]:
    refs = []
    for relative in (
        "journal.json", "commands/ci.json", "commands/http.json", "commands/g4.json",
        "commands/reproducibility.json", "build/ci/result.json", "http/http_report.json",
        "g4/comparison.json", "reproducibility/reproduction.json",
    ):
        path = root / Path(*relative.split("/"))
        if path.is_file() and not path.is_symlink():
            refs.append({"path": relative, "sha256": sha256(path.read_bytes())})
    return refs


def build_seed(run_root: str | Path, *, contract: dict[str, Any] | None = None) -> dict[str, Any]:
    """Return ``COMPLETE`` with a verified explicit seed, otherwise ``BLOCKED``.

    The blocked result is deliberately useful to producers: it identifies all
    predicates and inventory sections that still need explicit evidence.
    """
    root = Path(run_root).resolve()
    contract = _contract(contract)
    names = _predicates(contract)
    missing = list(names)
    reasons: list[str] = []
    candidate: Any = None
    ci_path, ci = _ci_report(root)
    if ci_path is None:
        reasons.append("CI raw report build/ci/result.json is missing")
    elif isinstance(ci, dict) and ci.get("kind") == "p10_acceptance_bundle":
        candidate = ci
    elif isinstance(ci, dict) and isinstance(ci.get("manifest"), dict) and ci["manifest"].get("kind") == "p10_acceptance_bundle":
        candidate = ci["manifest"]
    else:
        reasons.append("CI report contains only a summary; no explicit p10_acceptance_bundle manifest")

    if candidate is None:
        return {
            "schema_version": "1.0.0", "kind": "p10_seed_manifest", "status": "BLOCKED",
            "run": str(root), "blocked_predicates": missing,
            "blocked_sections": ["executables", "artifacts", "comparison"],
            "reasons": reasons, "source_refs": _report_refs(root),
        }
    try:
        required = {"schema_version", "kind", "source_commit", "target", "toolchain_lock_sha256", "executables", "predicates", "artifacts", "comparison"}
        if set(candidate) != required or candidate.get("schema_version") != "1.0.0" or candidate.get("target") != contract.get("target", TARGET):
            raise SeedManifestError("explicit seed manifest has unsupported fields")
        predicates = candidate["predicates"]
        if set(predicates) != set(names):
            missing = sorted(set(names) - set(predicates))
            raise SeedManifestError(f"explicit seed is missing predicates: {missing}")
        checked_predicates = {name: _check_ref(root, predicates[name], f"predicate {name}") for name in names}
        artifacts = candidate["artifacts"]
        if not isinstance(artifacts, list) or not artifacts:
            raise SeedManifestError("explicit seed artifact inventory is empty")
        checked_artifacts = []
        for index, item in enumerate(artifacts):
            if not isinstance(item, dict) or set(item) != {"role", "path", "sha256"}:
                raise SeedManifestError(f"artifact {index} is malformed")
            checked = _check_ref(root, {"path": item["path"], "sha256": item["sha256"]}, f"artifact {index}")
            checked_artifacts.append({"role": item["role"], **checked})
        comparison = candidate["comparison"]
        trials = comparison.get("trials") if isinstance(comparison, dict) else None
        if not isinstance(trials, list) or len(trials) != 18:
            raise SeedManifestError("explicit seed comparison does not contain 18 trial references")
        checked_trials = [_check_ref(root, ref, f"G4 trial {i}") for i, ref in enumerate(trials)]
        seed = dict(candidate)
        seed["predicates"] = checked_predicates
        seed["artifacts"] = checked_artifacts
        seed["comparison"] = dict(comparison, trials=checked_trials)
        return {"schema_version": "1.0.0", "kind": "p10_seed_manifest", "status": "COMPLETE", "manifest": seed, "source_refs": _report_refs(root)}
    except SeedManifestError as exc:
        return {
            "schema_version": "1.0.0", "kind": "p10_seed_manifest", "status": "BLOCKED",
            "run": str(root), "blocked_predicates": missing,
            "blocked_sections": ["executables", "artifacts", "comparison"],
            "reasons": [str(exc)], "source_refs": _report_refs(root),
        }


def main() -> int:
    parser = argparse.ArgumentParser(description="derive a fail-closed P10 seed manifest")
    parser.add_argument("--run", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    options = parser.parse_args()
    result = build_seed(options.run)
    data = (json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()
    if options.output:
        options.output.parent.mkdir(parents=True, exist_ok=True)
        options.output.write_bytes(data)
    else:
        print(data.decode(), end="")
    return 0 if result["status"] == "COMPLETE" else 2


if __name__ == "__main__":
    raise SystemExit(main())

