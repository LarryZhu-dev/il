#!/usr/bin/env python3
"""Assemble a successful trusted P10 run into a portable evidence bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import tempfile
from typing import Any


HASH_PREFIX = "sha256:"
TARGET = "x86_64-unknown-linux-gnu"
REQUIRED_COMMANDS = {"ci", "http", "g4", "reproducibility"}
MAX_BUNDLE_BYTES = 536_870_912


class AssembleError(RuntimeError):
    pass


def digest(data: bytes) -> str:
    return HASH_PREFIX + hashlib.sha256(data).hexdigest()


def is_hash(value: Any) -> bool:
    return (isinstance(value, str) and len(value) == 71 and value.startswith(HASH_PREFIX)
            and all(char in "0123456789abcdef" for char in value[7:]))


def is_commit(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 40 and all(char in "0123456789abcdef" for char in value)


def load_json(path: Path) -> tuple[Any, bytes]:
    try:
        raw = path.read_bytes()

        def pairs(items: list[tuple[str, Any]]) -> dict[str, Any]:
            result: dict[str, Any] = {}
            for key, value in items:
                if key in result:
                    raise AssembleError(f"duplicate JSON key in {path}: {key}")
                result[key] = value
            return result

        def reject_constant(value: str) -> None:
            raise AssembleError(f"invalid JSON constant in {path}: {value}")

        return json.loads(raw.decode("utf-8"), object_pairs_hook=pairs, parse_constant=reject_constant), raw
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AssembleError(f"invalid JSON {path}: {error}") from error


def canonical_relative(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value or "\\" in value or ":" in value:
        raise AssembleError(f"{field} must be a canonical relative POSIX path")
    pure = PurePosixPath(value)
    if pure.is_absolute() or any(part in {"", ".", ".."} for part in value.split("/")):
        raise AssembleError(f"{field} escapes its root: {value!r}")
    return pure.as_posix()


def regular_file(root: Path, candidate: Path, label: str) -> Path:
    try:
        root = root.resolve(strict=True)
        if candidate.is_symlink():
            raise AssembleError(f"{label} is a symbolic link: {candidate}")
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(root)
        if resolved.is_symlink() or not resolved.is_file():
            raise AssembleError(f"{label} is not a regular file: {candidate}")
        return resolved
    except (OSError, ValueError) as error:
        raise AssembleError(f"{label} escapes trusted run: {candidate}") from error


def atomic_write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".pending", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def read_journal(path: Path) -> tuple[Path, dict[str, Any], bytes]:
    journal_path = path.resolve()
    if journal_path.name != "journal.json" or journal_path.is_symlink() or not journal_path.is_file():
        raise AssembleError("trusted run journal is missing or is a symbolic link")
    run = journal_path.parent.resolve()
    journal, raw = load_json(journal_path)
    if not isinstance(journal, dict) or journal.get("schema_version") != "1.0.0" or journal.get("kind") != "p10_trusted_run":
        raise AssembleError("unsupported trusted run journal")
    if journal.get("status") != "COMPLETE":
        raise AssembleError("only a COMPLETE trusted run may be assembled")
    gate_export = journal.get("gate_records")
    if isinstance(gate_export, dict):
        if gate_export.get("status") not in {"COMPLETE", "BLOCKED"}:
            raise AssembleError("trusted gate-record export has an invalid status")
        export_path = gate_export.get("path")
        export_hash = gate_export.get("sha256")
        if not isinstance(export_path, str) or not is_hash(export_hash):
            raise AssembleError("trusted gate-record export reference is malformed")
        canonical_relative(export_path, "gate-record export path")
        export_file = regular_file(run, run / export_path, "gate-record export")
        if digest(export_file.read_bytes()) != export_hash:
            raise AssembleError("trusted gate-record export hash mismatch")
        export, _ = load_json(export_file)
        if (not isinstance(export, dict) or export.get("kind") != "p10_gate_record_export"
                or export.get("status") != gate_export["status"]
                or not isinstance(export.get("records"), dict)
                or not isinstance(export.get("missing_predicates"), list)):
            raise AssembleError("trusted gate-record export contents differ from the journal")
        if export.get("missing_predicates") != gate_export.get("missing_predicates"):
            raise AssembleError("trusted gate-record missing-predicate list differs from the journal")
        if gate_export["status"] == "COMPLETE" and export["missing_predicates"]:
            raise AssembleError("complete trusted gate-record export lists missing predicates")
    source = journal.get("source")
    if (not isinstance(source, dict) or not is_commit(source.get("commit"))
            or not is_commit(source.get("tree")) or source.get("dirty") is True):
        raise AssembleError("journal source identity is incomplete or dirty")
    lock = (journal.get("toolchain") or {}).get("lock")
    if not isinstance(lock, dict) or not is_hash(lock.get("sha256")):
        raise AssembleError("journal toolchain lock identity is missing")
    commands = journal.get("commands")
    if (not isinstance(commands, list) or len(commands) != len(REQUIRED_COMMANDS)
            or {row.get("command_id") for row in commands if isinstance(row, dict)} != REQUIRED_COMMANDS):
        raise AssembleError("trusted journal must contain exactly ci, http, g4 and reproducibility commands")
    for row in commands:
        if not isinstance(row, dict) or set(row) != {"command_id", "path", "sha256"}:
            raise AssembleError("invalid command observation reference")
        canonical_relative(row["path"], "command observation path")
        if not is_hash(row["sha256"]):
            raise AssembleError("invalid command observation hash")
        observation_path = regular_file(run, run / row["path"], "command observation")
        if digest(observation_path.read_bytes()) != row["sha256"]:
            raise AssembleError(f"command observation hash mismatch: {row['command_id']}")
        observation, _ = load_json(observation_path)
        if (not isinstance(observation, dict) or observation.get("kind") != "command_observation"
                or observation.get("command_id") != row["command_id"] or observation.get("status") != "SUCCESS"):
            raise AssembleError(f"command {row['command_id']} is not a successful trusted observation")
    plan = journal.get("plan")
    planned = {row.get("command_id") for row in plan.get("commands", []) if isinstance(row, dict)} if isinstance(plan, dict) else set()
    if planned != REQUIRED_COMMANDS:
        raise AssembleError("journal command plan does not match the required P10 commands")
    return run, journal, raw


def _output_paths(run: Path, command_id: str) -> list[Path]:
    """Return files captured by one successful command observation."""
    observation = run / "commands" / f"{command_id}.json"
    value, _ = load_json(regular_file(run, observation, "command observation"))
    rows = value.get("outputs") if isinstance(value, dict) else None
    if not isinstance(rows, list):
        raise AssembleError(f"command {command_id} has no captured output inventory")
    result: list[Path] = []
    for row in rows:
        if not isinstance(row, dict) or row.get("present") is not True:
            continue
        path = row.get("path")
        canonical_relative(path, f"{command_id} output path")
        candidate = regular_file(run, run / Path(*path.split("/")), f"{command_id} output")
        if digest(candidate.read_bytes()) != row.get("sha256"):
            raise AssembleError(f"{command_id} output hash mismatch: {path}")
        result.append(candidate)
    return result


def _find_json(run: Path, command_id: str, *names: str) -> Path:
    candidates = []
    for path in _output_paths(run, command_id):
        if path.name in names:
            candidates.append(path)
    if len(candidates) != 1:
        raise AssembleError(f"trusted {command_id} output must contain exactly one of {', '.join(names)}")
    return candidates[0]


def validate_seed(manifest: Any, journal: dict[str, Any]) -> dict[str, Any]:
    required = {"schema_version", "kind", "source_commit", "target", "toolchain_lock_sha256",
                "executables", "predicates", "artifacts", "comparison"}
    if (not isinstance(manifest, dict) or set(manifest) != required
            or manifest.get("schema_version") != "1.0.0"
            or manifest.get("kind") != "p10_acceptance_bundle" or manifest.get("target") != TARGET):
        raise AssembleError("unsupported P10 acceptance manifest")
    if manifest.get("source_commit") != journal["source"]["commit"]:
        raise AssembleError("P10 manifest source commit differs from trusted journal")
    if manifest.get("toolchain_lock_sha256") != journal["toolchain"]["lock"]["sha256"]:
        raise AssembleError("P10 manifest toolchain lock differs from trusted journal")
    comparison = manifest.get("comparison")
    if not isinstance(comparison, dict) or len(comparison.get("trials", [])) != 18:
        raise AssembleError("P10 manifest must contain exactly 18 G4 trial references")
    return manifest


def build_manifest(run: Path, journal: dict[str, Any], rebasing: "Rebasing") -> tuple[dict[str, Any], Path]:
    """Build the bundle manifest from command-produced raw reports.

    Raw reports are evidence, not claims supplied to this function.  The CI
    report must expose the complete manifest fields (or a nested ``manifest``
    object); the other three command reports are required and are checked for
    their locked matrix before their references are copied into the bundle.
    """
    ci_path = _find_json(run, "ci", "result.json", "manifest.json", "p10_manifest.json")
    ci, _ = load_json(ci_path)
    if isinstance(ci, dict) and ci.get("kind") == "p10_acceptance_bundle":
        # A complete CI manifest is accepted only as an output of the fixed
        # CI command.  The normal producer emits raw reports and therefore
        # follows the branch below.
        seed = validate_seed(ci, journal)
    else:
        seed = ci.get("manifest") if isinstance(ci, dict) else None
        if not isinstance(seed, dict):
            export_ref = journal.get("gate_records")
            exported_records: set[str] = set()
            missing_predicates: list[str] = []
            source_paths = [ci_path.relative_to(run).as_posix()]
            if isinstance(export_ref, dict):
                export_path = canonical_relative(export_ref.get("path"), "gate-record export path")
                export_file = regular_file(run, run / export_path, "gate-record export")
                export, _ = load_json(export_file)
                if digest(export_file.read_bytes()) != export_ref.get("sha256"):
                    raise AssembleError("trusted gate-record export hash mismatch")
                records = export.get("records") if isinstance(export, dict) else None
                missing = export.get("missing_predicates") if isinstance(export, dict) else None
                if isinstance(records, dict):
                    for predicate, reference in records.items():
                        if not isinstance(predicate, str) or not isinstance(reference, dict):
                            raise AssembleError("trusted gate-record export contains a malformed record reference")
                        record_path = canonical_relative(reference.get("path"), f"{predicate} gate-record path")
                        record_file = regular_file(run, run / record_path, f"{predicate} gate record")
                        raw = record_file.read_bytes()
                        if not is_hash(reference.get("sha256")) or digest(raw) != reference["sha256"]:
                            raise AssembleError(f"trusted gate-record hash mismatch: {predicate}")
                        gate_record, _ = load_json(record_file)
                        if (not isinstance(gate_record, dict) or gate_record.get("kind") != "p10_gate_record"
                                or gate_record.get("predicate") != predicate):
                            raise AssembleError(f"trusted gate-record identity mismatch: {predicate}")
                        if (gate_record.get("status") != "PASSED"
                                or gate_record.get("source_commit") != journal["source"]["commit"]
                                or gate_record.get("target") != TARGET
                                or gate_record.get("toolchain_lock_sha256") != journal["toolchain"]["lock"]["sha256"]):
                            raise AssembleError(f"trusted gate-record provenance/status mismatch: {predicate}")
                        exported_records.add(predicate)
                if isinstance(missing, list) and all(isinstance(item, str) for item in missing):
                    missing_predicates = sorted(set(missing))
                source_paths.append(export_path)
            else:
                missing_predicates = sorted(
                    predicate
                    for group in json.loads((Path(__file__).with_name("contract.json")).read_text(encoding="utf-8"))["gates"].values()
                    for predicate in group
                )

            for command_id, names in (("http", ("http_report.json",)),
                                      ("g4", ("comparison.json",)),
                                      ("reproducibility", ("reproduction.json",))):
                report_path = _find_json(run, command_id, *names)
                source_paths.append(report_path.relative_to(run).as_posix())

            contract = json.loads(Path(__file__).with_name("contract.json").read_text(encoding="utf-8"))
            expected_predicates = {item for group in contract["gates"].values() for item in group}
            missing_predicates = sorted(set(missing_predicates) | (expected_predicates - exported_records))
            missing_sections = ["executables", "artifacts"]
            if not (run / "g4" / "comparison.json").is_file():
                missing_sections.append("comparison")
            raise AssembleError(
                "P10 bundle BLOCKED: no explicit p10_acceptance_bundle manifest; "
                f"missing predicates={missing_predicates}; "
                f"missing inventory sections={missing_sections}; "
                f"observed raw evidence sources={sorted(set(source_paths))}"
            )
        if not isinstance(ci, dict) or ci.get("status") not in {"PASSED", "COMPLETE"}:
            raise AssembleError("trusted CI output is not a passed report")
        validate_seed(seed, journal)

    http_path = _find_json(run, "http", "http_report.json")
    g4_path = _find_json(run, "g4", "comparison.json")
    repro_path = _find_json(run, "reproducibility", "reproduction.json")
    http, _ = load_json(http_path)
    g4, _ = load_json(g4_path)
    repro, _ = load_json(repro_path)
    if not isinstance(http, dict) or http.get("state", http.get("status")) != "PASSED":
        raise AssembleError("trusted HTTP report is not passed")
    if not isinstance(g4, dict) or g4.get("status") not in {"COMPLETE", "PASSED"}:
        raise AssembleError("trusted G4 report is not complete")
    trials = g4.get("trials")
    if not isinstance(trials, list) or len(trials) != 18:
        raise AssembleError("trusted G4 report must contain exactly 18 trials")
    trial_refs = []
    seen_trials: set[tuple[Any, Any, Any]] = set()
    for row in trials:
        if not isinstance(row, dict):
            raise AssembleError("malformed G4 trial row")
        reference = row.get("g4_trial")
        if not isinstance(reference, dict):
            raise AssembleError("G4 trial row has no g4_trial reference")
        trial, _ = rebasing.read_reference_json(reference, g4_path.parent)
        key = (trial.get("task_id"), trial.get("implementation"), trial.get("trial_index"))
        if key in seen_trials:
            raise AssembleError("duplicate G4 task/implementation/trial")
        seen_trials.add(key)
        trial_refs.append(reference)
    if not isinstance(repro, dict) or repro.get("status") != "PASSED":
        raise AssembleError("trusted reproducibility report is not passed")
    comparison = dict(seed["comparison"])
    comparison["trials"] = trial_refs
    result = dict(seed)
    result["comparison"] = comparison
    return result, ci_path


class Rebasing:
    """Maintain a lazy mapping from referenced run files to bundle files.

    Trusted runs retain command working directories and temporary binaries.  A
    full recursive copy can therefore be gigabytes even when the acceptance
    manifest references only a small evidence closure.  Mapping is allocated
    when a reference is actually read, so unreferenced files never enter the
    published bundle.
    """

    def __init__(self, run: Path, destination: Path, report: Path):
        self.run = run
        self.destination = destination
        self.report = report.resolve()
        self.mapping: dict[Path, Path] = {}
        self.cache: dict[Path, bytes] = {}
        self.copied_bytes = 0

    def _register(self, source: Path) -> Path:
        source = regular_file(self.run, source, "evidence file")
        target = self.mapping.get(source)
        if target is None:
            target = self.destination / ("manifest.json" if source == self.report else "records" / source.relative_to(self.run))
            self.mapping[source] = target
        return target

    def resolve_reference(self, base: Path, text: str) -> Path:
        if os.path.isabs(text) or PurePosixPath(text.replace("\\", "/")).is_absolute():
            candidate = Path(text)
            # Reports are produced in a locked container and may retain its
            # absolute `/workspace/.../<run-name>/...` spelling.  Rebase only
            # that path form to this trusted local run; arbitrary absolute
            # paths remain rejected by regular_file below.
            parts = PurePosixPath(text.replace("\\", "/")).parts
            run_name = self.run.name
            if run_name in parts:
                suffix = parts[parts.index(run_name) + 1:]
                candidate = self.run.joinpath(*suffix)
        else:
            canonical_relative(text, "evidence reference path")
            candidate = base / Path(*text.split("/"))
            if not candidate.exists():
                candidate = self.run / Path(*text.split("/"))
        source = regular_file(self.run, candidate, "evidence reference")
        self._register(source)
        return source

    def read_reference_json(self, reference: Any, base: Path) -> tuple[dict[str, Any], Path]:
        if not isinstance(reference, dict) or not isinstance(reference.get("path"), str) or not is_hash(reference.get("sha256")):
            raise AssembleError("invalid evidence reference")
        source = self.resolve_reference(base, reference["path"])
        original = source.read_bytes()
        if digest(original) != reference["sha256"]:
            raise AssembleError(f"referenced evidence hash mismatch: {reference['path']}")
        value, _ = load_json(source)
        if not isinstance(value, dict):
            raise AssembleError(f"referenced evidence is not a JSON object: {reference['path']}")
        return value, source

    def write_source(self, source: Path) -> bytes:
        source = regular_file(self.run, source, "evidence file")
        target = self._register(source)
        if source in self.cache:
            return self.cache[source]
        if source.suffix.lower() == ".json":
            value, _ = load_json(source)
            value = self.rewrite(value, source.parent)
            data = (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
        else:
            data = source.read_bytes()
        projected = self.copied_bytes + len(data)
        if projected > MAX_BUNDLE_BYTES:
            raise AssembleError(
                f"assembled evidence exceeds {MAX_BUNDLE_BYTES} byte budget"
            )
        self.copied_bytes = projected
        self.cache[source] = data
        atomic_write(target, data)
        return data

    def rewrite(self, value: Any, base: Path) -> Any:
        if isinstance(value, list):
            return [self.rewrite(item, base) for item in value]
        if not isinstance(value, dict):
            return value
        result = {key: self.rewrite(item, base) for key, item in value.items()}
        if isinstance(value.get("path"), str) and is_hash(value.get("sha256")):
            source = self.resolve_reference(base, value["path"])
            original = source.read_bytes()
            if digest(original) != value["sha256"]:
                raise AssembleError(f"referenced evidence hash mismatch: {value['path']}")
            data = self.write_source(source)
            result["path"] = self._register(source).relative_to(self.destination).as_posix()
            result["sha256"] = digest(data)
        return result


def assemble(journal: str | Path, output: str | Path) -> dict[str, Any]:
    run, journal_value, journal_bytes = read_journal(Path(journal))
    destination = Path(output).resolve()
    marker = digest(journal_bytes)
    if destination.exists():
        marker_path = destination / ".assemble-source"
        if (marker_path.is_file() and marker_path.read_text(encoding="ascii").strip() == marker
                and (destination / "manifest.json").is_file()):
            value, _ = load_json(destination / "manifest.json")
            return value
        raise AssembleError(f"refusing to overwrite existing bundle: {destination}")
    destination.mkdir(parents=True)
    try:
        # Only the evidence closure is copied below records/.  The raw CI
        # report is an input to assembly and is replaced by the canonical
        # manifest, so it is not retained as a second copy.
        rebasing = Rebasing(run, destination, run / "ci" / "result.json")
        seed, report = build_manifest(run, journal_value, rebasing)
        final = rebasing.rewrite(seed, report.parent)
        if not isinstance(final, dict):
            raise AssembleError("assembled manifest is not an object")
        data = (json.dumps(final, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
        if rebasing.copied_bytes + len(data) > MAX_BUNDLE_BYTES:
            raise AssembleError(f"assembled evidence exceeds {MAX_BUNDLE_BYTES} byte budget")
        atomic_write(destination / "manifest.json", data)
        atomic_write(destination / ".assemble-source", (marker + "\n").encode("ascii"))
        return final
    except BaseException:
        shutil.rmtree(destination, ignore_errors=True)
        raise


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--journal", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        result = assemble(args.journal, args.output)
    except AssembleError as error:
        print(json.dumps({"ok": False, "status": "BLOCKED", "error": str(error)}, ensure_ascii=False))
        return 2
    print(json.dumps({"ok": True, "status": "ASSEMBLED", "manifest": str(args.output / "manifest.json"), "source_commit": result["source_commit"]}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
