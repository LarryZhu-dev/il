"""Read-only P00 gate and the repository's dependency-free JSON Schema subset.

The validator supports the explicitly enumerated vocabulary used in repository
schemas; unsupported assertions fail closed. It is not a general JSON Schema
implementation. No command supplied by a task or document is executed here.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path


TARGET = "x86_64-unknown-linux-gnu"
DEPENDENCIES = {
    "P00": [], "P01": ["P00"], "P02": ["P00"], "P03": ["P02"],
    "P04": ["P03"], "P05": ["P01", "P03", "P04"], "P06": ["P05"],
    "P07": ["P06"], "P08": ["P02", "P03", "P07"],
    "P09": ["P07", "P08"], "P10": ["P09"],
    **{f"E{i:02}": ["P10"] for i in range(1, 6)},
}
DIRECTORIES = "spec rfc adr schema compiler runtime packages tools tests examples eval build release".split()
SPEC_FILES = "language types memory effects abi http errors versioning".split()
SCHEMA_FILES = "diagnostic evidence task repository_state".split()
HASH = re.compile(r"^sha256:[0-9a-f]{64}$")
ANNOTATIONS = {"$schema", "$id", "$defs", "$comment", "title", "description", "default", "examples", "deprecated", "readOnly", "writeOnly"}
ASSERTIONS = {"$ref", "type", "const", "enum", "allOf", "anyOf", "oneOf", "not", "if", "then", "else", "properties", "required", "additionalProperties", "patternProperties", "propertyNames", "dependentRequired", "items", "prefixItems", "contains", "minContains", "maxContains", "minItems", "maxItems", "uniqueItems", "minLength", "maxLength", "pattern", "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "multipleOf", "minProperties", "maxProperties"}


class ValidationError(ValueError):
    pass


def read_json(path: Path):
    def no_duplicate_keys(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValidationError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    def reject_constant(value):
        raise ValidationError(f"non-JSON numeric constant: {value}")

    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=no_duplicate_keys,
                      parse_constant=reject_constant)


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            hasher.update(chunk)
    return "sha256:" + hasher.hexdigest()


def _equal(left, right):
    # JSON booleans and integers are distinct, unlike Python's True == 1.
    return json.dumps(left, sort_keys=True, separators=(",", ":")) == json.dumps(right, sort_keys=True, separators=(",", ":"))


def validate_instance(value, schema, schema_path: Path, location="$", root_schema=None, depth=0):
    """Raise ValidationError on a violated constraint or unsupported vocabulary."""
    if depth > 128:
        raise ValidationError(f"{location}: schema recursion limit exceeded")
    if schema is True:
        return
    if schema is False:
        raise ValidationError(f"{location}: schema rejects value")
    if not isinstance(schema, dict):
        raise ValidationError(f"{location}: schema must be object or boolean")
    unknown = set(schema) - ANNOTATIONS - ASSERTIONS
    if unknown:
        raise ValidationError(f"{location}: unsupported schema keywords {sorted(unknown)}")
    root_schema = schema if root_schema is None else root_schema

    def child(item, rule, at=location):
        validate_instance(item, rule, schema_path, at, root_schema, depth + 1)

    def matches(rule, item=value):
        try:
            child(item, rule)
            return True
        except ValidationError:
            return False

    if "$ref" in schema:
        file_name, _, fragment = schema["$ref"].partition("#")
        if ":" in file_name or file_name.startswith(("/", "\\")):
            raise ValidationError(f"{location}: only local schema references are allowed")
        reference_path = (schema_path.parent / file_name).resolve() if file_name else schema_path
        reference_root = read_json(reference_path) if file_name else root_schema
        reference = reference_root
        if fragment and not fragment.startswith("/"):
            raise ValidationError(f"{location}: reference must use a JSON pointer")
        for part in fragment.split("/")[1:]:
            part = part.replace("~1", "/").replace("~0", "~")
            try:
                reference = reference[part]
            except (KeyError, TypeError) as error:
                raise ValidationError(f"{location}: unresolved reference {schema['$ref']}") from error
        validate_instance(value, reference, reference_path, location, reference_root, depth + 1)
    kinds = {
        "null": value is None, "boolean": isinstance(value, bool),
        "integer": isinstance(value, int) and not isinstance(value, bool),
        "number": isinstance(value, (int, float)) and not isinstance(value, bool),
        "string": isinstance(value, str), "array": isinstance(value, list), "object": isinstance(value, dict),
    }
    if "type" in schema:
        types = schema["type"] if isinstance(schema["type"], list) else [schema["type"]]
        if not any(kinds.get(kind, False) for kind in types):
            raise ValidationError(f"{location}: expected {types}")
    if "const" in schema and not _equal(value, schema["const"]):
        raise ValidationError(f"{location}: differs from required constant")
    if "enum" in schema and not any(_equal(value, option) for option in schema["enum"]):
        raise ValidationError(f"{location}: not an allowed value")
    for rule in schema.get("allOf", []):
        child(value, rule)
    for keyword, expected in (("anyOf", lambda count: count >= 1), ("oneOf", lambda count: count == 1)):
        if keyword in schema and not expected(sum(matches(rule) for rule in schema[keyword])):
            raise ValidationError(f"{location}: {keyword} alternatives not satisfied")
    if "not" in schema and matches(schema["not"]):
        raise ValidationError(f"{location}: forbidden schema matched")
    if "if" in schema:
        branch = "then" if matches(schema["if"]) else "else"
        if branch in schema:
            child(value, schema[branch])
    if isinstance(value, dict):
        missing = set(schema.get("required", [])) - set(value)
        if missing:
            raise ValidationError(f"{location}: missing fields {sorted(missing)}")
        for key, item in value.items():
            if "propertyNames" in schema:
                child(key, schema["propertyNames"], location + ".<key>")
            covered = False
            if key in schema.get("properties", {}):
                child(item, schema["properties"][key], location + "." + key)
                covered = True
            for pattern, rule in schema.get("patternProperties", {}).items():
                if re.search(pattern, key):
                    child(item, rule, location + "." + key)
                    covered = True
            if not covered:
                child(item, schema.get("additionalProperties", True), location + "." + key)
        for key, required in schema.get("dependentRequired", {}).items():
            if key in value and not set(required) <= set(value):
                raise ValidationError(f"{location}: dependencies missing for {key}")
        for bound, compare in (("minProperties", lambda n: len(value) >= n), ("maxProperties", lambda n: len(value) <= n)):
            if bound in schema and not compare(schema[bound]):
                raise ValidationError(f"{location}: violates {bound}")
    if isinstance(value, list):
        for index, item in enumerate(value):
            prefix = schema.get("prefixItems", [])
            rule = prefix[index] if index < len(prefix) else schema.get("items", True)
            child(item, rule, f"{location}[{index}]")
        for bound, compare in (("minItems", lambda n: len(value) >= n), ("maxItems", lambda n: len(value) <= n)):
            if bound in schema and not compare(schema[bound]):
                raise ValidationError(f"{location}: violates {bound}")
        if schema.get("uniqueItems"):
            serialized = [json.dumps(item, sort_keys=True, separators=(",", ":")) for item in value]
            if len(set(serialized)) != len(serialized):
                raise ValidationError(f"{location}: duplicate array item")
        if "contains" in schema:
            count = sum(matches(schema["contains"], item) for item in value)
            if count < schema.get("minContains", 1) or count > schema.get("maxContains", len(value)):
                raise ValidationError(f"{location}: violates contains cardinality")
    if isinstance(value, str):
        if "minLength" in schema and len(value) < schema["minLength"]:
            raise ValidationError(f"{location}: string too short")
        if "maxLength" in schema and len(value) > schema["maxLength"]:
            raise ValidationError(f"{location}: string too long")
        if "pattern" in schema and not re.search(schema["pattern"], value):
            raise ValidationError(f"{location}: invalid string pattern")
    if kinds["number"]:
        for bound, compare in (("minimum", lambda n: value >= n), ("maximum", lambda n: value <= n), ("exclusiveMinimum", lambda n: value > n), ("exclusiveMaximum", lambda n: value < n)):
            if bound in schema and not compare(schema[bound]):
                raise ValidationError(f"{location}: violates {bound}")
        if "multipleOf" in schema and value % schema["multipleOf"] != 0:
            raise ValidationError(f"{location}: violates multipleOf")


def validate_file(root: Path, value, schema_name: str):
    path = root / "schema" / (schema_name + ".schema.json")
    validate_instance(value, read_json(path), path)


def git(root: Path, *args: str) -> str:
    result = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, check=False)
    if result.returncode:
        raise ValidationError("git " + " ".join(args) + ": " + result.stderr.strip())
    return result.stdout.strip()


def safe_artifact(root: Path, relative: str) -> Path:
    path = (root / relative).resolve()
    if Path(relative).is_absolute() or root not in path.parents:
        raise ValidationError(f"artifact outside repository: {relative}")
    if not path.is_file():
        raise ValidationError(f"missing artifact: {relative}")
    return path


def check_evidence(root: Path, record, state, lock):
    validate_file(root, record, "evidence")
    if record["revision"] > state["head_revision"]:
        raise ValidationError("evidence references a future revision")
    if record["target"] != state["target"]:
        raise ValidationError("evidence target differs from state")
    if record["toolchain_lock_hash"] != digest(root / "toolchain.lock"):
        raise ValidationError("evidence toolchain lock hash differs")
    if record["host_compiler_hash"] != lock["host_compiler_hash"]:
        raise ValidationError("evidence host compiler differs from locked compiler")
    unavailable = {component for component in ("graph", "compiler", "runtime") if record[component + "_hash"] is None}
    if unavailable != set(record["unavailable_components"]):
        raise ValidationError("evidence unavailable components contradict hashes")
    if any(command["exit_code"] != 0 for command in record["commands"]):
        raise ValidationError("verification evidence contains a failed command")
    if any(not test["passed"] for test in record["tests"]):
        raise ValidationError("verification evidence contains a failing suite")
    if len({command["id"] for command in record["commands"]}) != len(record["commands"]):
        raise ValidationError("evidence command IDs are not unique")
    for artifact in record["artifacts"]:
        if digest(safe_artifact(root, artifact["path"])) != artifact["sha256"]:
            raise ValidationError("evidence artifact hash differs: " + artifact["path"])
    committed = json.loads(git(root, "show", record["git_commit"] + ":repository_state.json"))
    if committed["head_revision"] != record["revision"]:
        raise ValidationError("evidence revision differs from its Git commit")


def run_checks(root: Path, pre_commit: bool = False):
    checks = []
    state = None
    tasks = {}

    def check(name, action):
        try:
            action()
            checks.append({"id": name, "passed": True})
        except (OSError, ValueError, KeyError, TypeError, RecursionError) as error:
            checks.append({"id": name, "passed": False, "error": str(error)})

    def tree():
        absent = [item for item in DIRECTORIES if not (root / item).is_dir()]
        absent += [f"spec/{item}.yaml" for item in SPEC_FILES if not (root / "spec" / (item + ".yaml")).is_file()]
        absent += [item for item in ("il_execution_spec_v1.md", "toolchain.lock", "repository_state.json") if not (root / item).is_file()]
        if absent:
            raise ValidationError("missing paths: " + ", ".join(absent))
        if not any((root / ".github/workflows").glob("*.yml")) and not any((root / ".github/workflows").glob("*.yaml")):
            raise ValidationError("missing CI workflow")
        if not any("template" in path.name.lower() for path in (root / "rfc").iterdir()):
            raise ValidationError("missing RFC template")

    def schemas():
        for name in SCHEMA_FILES:
            path = root / "schema" / (name + ".schema.json")
            schema = read_json(path)
            if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
                raise ValidationError(f"{path.name}: wrong schema dialect")
            if schema.get("additionalProperties") is not False:
                raise ValidationError(f"{path.name}: root must reject unknown properties")
        # Validate real state/task instances, not merely the JSON syntax of schemas.
        validate_file(root, read_json(root / "repository_state.json"), "repository_state")
        for path in sorted((root / "eval/tasks").glob("*.json")):
            validate_file(root, read_json(path), "task")
        graph = read_json(root / "examples/bootstrap/graph.json")
        expected = {"project_id", "graph_version", "revision", "target", "modules", "types", "functions", "capabilities", "packages", "contracts"}
        if set(graph) != expected or graph["project_id"] != "il" or graph["graph_version"] != "1.0.0" or graph["revision"] != 0 or graph["target"] != TARGET:
            raise ValidationError("invalid bootstrap graph envelope")
        if any(graph[field] != [] for field in ("modules", "types", "functions", "capabilities", "packages", "contracts")):
            raise ValidationError("P00 bootstrap graph must remain empty")

    def repository_state():
        nonlocal state
        state = read_json(root / "repository_state.json")
        validate_file(root, state, "repository_state")
        if state["last_verified_revision"] > state["head_revision"]:
            raise ValidationError("last verified revision exceeds head revision")
        groups = [set(state[key]) for key in ("open_tasks", "blocked_tasks", "failed_tasks")]
        if any(left & right for index, left in enumerate(groups) for right in groups[index + 1:]):
            raise ValidationError("state task groups overlap")

    def task_graph():
        for task_id, dependencies in DEPENDENCIES.items():
            task = read_json(root / "eval/tasks" / (task_id + ".json"))
            validate_file(root, task, "task")
            if task["task_id"] != task_id or task["requires"] != dependencies:
                raise ValidationError(task_id + ": differs from locked dependency graph")
            if task["base_revision"] > state["head_revision"] or task["rollback_revision"] > state["head_revision"]:
                raise ValidationError(task_id + ": references future revision")
            tasks[task_id] = task
        for task_id, task in tasks.items():
            if task["status"] in ("READY", "RUNNING", "VERIFIED"):
                unverified = [dep for dep in task["requires"] if tasks[dep]["status"] != "VERIFIED"]
                if unverified:
                    raise ValidationError(task_id + ": dependencies not VERIFIED: " + ", ".join(unverified))
        for field, statuses in (("open_tasks", {"READY", "RUNNING"}), ("blocked_tasks", {"BLOCKED", "DESIGN_REQUIRED"}), ("failed_tasks", {"FAILED"})):
            expected = {task_id for task_id, task in tasks.items() if task["status"] in statuses}
            if set(state[field]) != expected:
                raise ValidationError(field + ": differs from task records")

    def lockfile():
        lock = read_json(root / "toolchain.lock")
        if lock.get("target") != TARGET or state["target"] != TARGET:
            raise ValidationError("first target must be Linux x86-64")
        for field in ("base_image_digest", "host_compiler_hash"):
            if not isinstance(lock.get(field), str) or not HASH.fullmatch(lock[field]):
                raise ValidationError("toolchain lock lacks real " + field)
        for field in ("rust", "llvm"):
            if not re.fullmatch(r"\d+\.\d+\.\d+", lock.get(field, "")):
                raise ValidationError("unlocked toolchain version: " + field)

    def forbidden_sources():
        suffixes = {".c", ".cc", ".cpp", ".cxx", ".rs", ".js", ".ts", ".py", ".java", ".class"}
        forbidden = [str(path.relative_to(root)) for path in (root / "build").rglob("*") if path.is_file() and path.suffix.lower() in suffixes]
        if forbidden:
            raise ValidationError("forbidden generated target source: " + ", ".join(forbidden[:20]))

    def clean_repository():
        if git(root, "status", "--porcelain", "--untracked-files=all"):
            raise ValidationError("repository has uncommitted or untracked files")

    def binding():
        commit = git(root, "rev-parse", "HEAD")
        tree_hash = git(root, "rev-parse", "HEAD^{tree}")
        metadata = Path(git(root, "rev-parse", "--git-dir"))
        if not metadata.is_absolute():
            metadata = root / metadata
        journal = read_json(metadata / "il/revision_bindings.json")
        if set(journal) != {"schema_version", "bindings"} or journal["schema_version"] != "1.0.0":
            raise ValidationError("STATE_INCONSISTENT: malformed binding journal")
        expected = {"revision": state["head_revision"], "git_commit": commit, "tree_hash": tree_hash}
        if expected not in journal["bindings"]:
            raise ValidationError("STATE_INCONSISTENT: HEAD has no matching revision/tree binding")
        if json.loads(git(root, "show", "HEAD:repository_state.json")) != state:
            raise ValidationError("STATE_INCONSISTENT: state differs from committed state")

    def evidence():
        lock = read_json(root / "toolchain.lock")
        records = list((root / "eval/evidence").rglob("*.json")) + list((root / "build").rglob("*evidence*.json"))
        verified = {task_id for task_id, task in tasks.items() if task["status"] == "VERIFIED"}
        accepted = set()
        failures = []
        for path in sorted(set(records)):
            record = read_json(path)
            if record.get("task_id") not in verified:
                continue
            try:
                check_evidence(root, record, state, lock)
                accepted.add(record["task_id"])
            except (OSError, ValueError, KeyError, TypeError) as error:
                failures.append(f"{path.name}: {error}")
        missing = verified - accepted
        if missing:
            raise ValidationError("missing valid evidence for " + ", ".join(sorted(missing)) + ("; " + "; ".join(failures) if failures else ""))

    check("repository_tree", tree)
    check("schemas_parse_and_validate_instances", schemas)
    check("repository_state", repository_state)
    if state is not None:
        check("task_dependencies_and_state", task_graph)
        check("locked_toolchain_and_linux_x86_64", lockfile)
    check("forbidden_target_languages_not_used", forbidden_sources)
    if not pre_commit:
        check("clean_repository", clean_repository)
        if state is not None:
            check("state_head_equals_git_head", binding)
    if tasks:
        check("verified_tasks_have_evidence", evidence)
    ok = all(item["passed"] for item in checks)
    return {"ok": ok, "tool": "bootstrap_check", "tool_version": "1.0.0", "phase": "pre_commit" if pre_commit else "verification", "ready_for_verification": ok and not pre_commit, "base_revision": state.get("head_revision") if isinstance(state, dict) else None, "checks": checks}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--pre-commit", action="store_true", help="Structural preview only; skips clean Git and HEAD binding gates.")
    args = parser.parse_args()
    result = run_checks(args.root.resolve(), args.pre_commit)
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
