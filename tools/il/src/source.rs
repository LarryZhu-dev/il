use crate::protocol::Failure;
use il_graph::{Graph, Provenance, TARGET};
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Context { pub repository: PathBuf, pub provenance: Provenance, pub seed: Value }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryState {
    project_id: String,
    schema_version: String,
    head_revision: u64,
    last_verified_revision: u64,
    #[serde(deserialize_with = "required_nullable_string")]
    compiler_version: Option<String>,
    #[serde(deserialize_with = "required_nullable_string")]
    runtime_version: Option<String>,
    target: String,
    open_tasks: Vec<String>,
    blocked_tasks: Vec<String>,
    failed_tasks: Vec<String>,
    locks: std::collections::BTreeMap<String, RepositoryLock>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryLock { owner: String, revision: u64 }

fn required_nullable_string<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

fn valid_task_ids(tasks: &[String]) -> bool {
    let unique: std::collections::BTreeSet<_> = tasks.iter().collect();
    unique.len() == tasks.len() && tasks.iter().all(|task| {
        matches!(task.as_str(), "P00" | "P01" | "P02" | "P03" | "P04" | "P05" | "P06" | "P07" | "P08" | "P09" | "P10" | "E01" | "E02" | "E03" | "E04" | "E05")
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding { revision: u64, git_commit: String, tree_hash: String }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bindings { schema_version: String, bindings: Vec<Binding> }

fn inconsistent(message: &str) -> Failure { Failure::new("E_STATE_INCONSISTENT", message) }

fn git(repository: &Path, args: &[&str]) -> Result<Vec<u8>, Failure> {
    let output = Command::new("git").arg("-C").arg(repository).args(args).output().map_err(Failure::io)?;
    if !output.status.success() { return Err(inconsistent("cannot resolve committed repository identity")); }
    Ok(output.stdout)
}

fn text(repository: &Path, args: &[&str]) -> Result<String, Failure> {
    String::from_utf8(git(repository, args)?).map(|text| text.trim().to_owned())
        .map_err(|_| inconsistent("Git returned an invalid UTF-8 identity"))
}

pub fn verify(repository: &Path) -> Result<Context, Failure> {
    let repository = repository.canonicalize().map_err(Failure::io)?;
    let commit = text(&repository, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let tree = text(&repository, &["rev-parse", "HEAD^{tree}"])?;
    let mut state = Value::Null;
    let mut seed = Value::Null;
    for path in ["repository_state.json", "examples/bootstrap/graph.json", "toolchain.lock"] {
        let working = fs::read(repository.join(path)).map_err(|_| inconsistent("required source context file is missing"))?;
        let committed = git(&repository, &["show", &format!("{commit}:{path}")])?;
        if working != committed { return Err(inconsistent(&format!("working {path} differs from committed context"))); }
        let value = crate::strict_json::parse(std::str::from_utf8(&working).map_err(|_| inconsistent("source context must be UTF-8"))?)
            .map_err(|_| inconsistent("source context contains invalid JSON"))?;
        if path == "repository_state.json" { state = value; }
        else if path == "examples/bootstrap/graph.json" { seed = value; }
    }
    for key in ["project_id", "schema_version", "head_revision", "last_verified_revision", "compiler_version", "runtime_version", "target", "open_tasks", "blocked_tasks", "failed_tasks", "locks"] {
        if state.get(key).is_none() { return Err(inconsistent("required repository state field is missing")); }
    }
    let state: RepositoryState = serde_json::from_value(state).map_err(|_| inconsistent("source repository state violates its closed schema"))?;
    let revision = state.head_revision;
    if state.project_id != "il" || state.schema_version != "1.0.0" || state.target != TARGET || state.last_verified_revision > revision {
        return Err(inconsistent("unsupported repository identity, schema, target, or verified revision"));
    }
    if [&state.compiler_version, &state.runtime_version].iter().any(|version| version.as_ref().is_some_and(String::is_empty))
        || [&state.open_tasks, &state.blocked_tasks, &state.failed_tasks].iter().any(|tasks| !valid_task_ids(tasks))
        || state.locks.values().any(|lock| lock.owner.is_empty() || lock.revision > revision)
    { return Err(inconsistent("repository versions, tasks, or locks are invalid")); }
    let tasks: Vec<_> = state.open_tasks.iter().chain(&state.blocked_tasks).chain(&state.failed_tasks).collect();
    if tasks.iter().collect::<std::collections::BTreeSet<_>>().len() != tasks.len() {
        return Err(inconsistent("task cannot appear in multiple repository status lists"));
    }
    let graph: Graph = serde_json::from_value(seed.clone()).map_err(|_| inconsistent("source seed violates the graph schema"))?;
    if !graph.validate_structural().is_empty() { return Err(inconsistent("source seed graph is structurally invalid")); }
    if seed["revision"] != revision || seed["project_id"] != state.project_id || seed["target"] != state.target {
        return Err(inconsistent("source state and seed graph disagree"));
    }
    let location = PathBuf::from(text(&repository, &["rev-parse", "--git-path", "il/revision_bindings.json"])?);
    let location = if location.is_absolute() { location } else { repository.join(location) };
    let journal: Bindings = serde_json::from_slice(&fs::read(location).map_err(|_| inconsistent("Git revision binding is missing"))?)
        .map_err(|_| inconsistent("Git revision binding is invalid"))?;
    if journal.schema_version != "1.0.0" || !journal.bindings.iter().any(|binding|
        binding.revision == revision && binding.git_commit == commit && binding.tree_hash == tree
    ) { return Err(inconsistent("no binding matches current Git commit, tree, and revision")); }
    Ok(Context { repository, provenance: Provenance { source_git_commit: commit, source_tree_hash: tree }, seed })
}

pub fn verify_provenance(context: &Context, provenance: &Provenance) -> Result<(), Failure> {
    for identity in [&provenance.source_git_commit, &provenance.source_tree_hash] {
        if identity.len() != 40 || !identity.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) {
            return Err(inconsistent("snapshot source identity is invalid"));
        }
    }
    let tree = text(&context.repository, &["rev-parse", &format!("{}^{{tree}}", provenance.source_git_commit)])?;
    if tree != provenance.source_tree_hash { return Err(inconsistent("snapshot source tree does not match its Git commit")); }
    git(&context.repository, &["merge-base", "--is-ancestor", &provenance.source_git_commit, &context.provenance.source_git_commit])?;
    Ok(())
}
