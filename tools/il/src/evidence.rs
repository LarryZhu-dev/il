//! Evidence only accepts immutable run identities, never client success claims.
use crate::{journal, protocol::Failure, source::Context};
use il_graph::{canonical_bytes, hash_bytes, Graph};
use il_runtime_startup::HostPolicy;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::{BTreeMap, BTreeSet}, fs, io::{Read, Write}, path::{Path, PathBuf}};
use sha2::{Digest, Sha256};

const BUNDLE_LIMIT: u64 = 512 * 1024 * 1024;
const ARTIFACT_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection { pub run_id: String, pub role: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request { pub revision: u64, pub task_id: String, pub artifacts: Vec<Selection>, pub tests: Vec<String> }
fn failure(message: &str) -> Failure { Failure::new("E_EVIDENCE_INCOMPLETE", message) }
fn budget() -> Failure { Failure::new("E_RESOURCE_LIMIT", "application evidence exceeds its 512 MiB content budget") }
fn charge(used: &mut u64, size: u64) -> Result<(), Failure> {
    *used = used.checked_add(size).filter(|value| *value <= BUNDLE_LIMIT).ok_or_else(budget)?;
    Ok(())
}
fn hash_file(path: &Path) -> Result<String, Failure> {
    let mut file = fs::File::open(path).map_err(Failure::io)?;
    let mut hash = Sha256::new(); let mut used = 0;
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(Failure::io)?;
        if count == 0 { break; }
        used += count as u64;
        if used > ARTIFACT_LIMIT { return Err(budget()); }
        hash.update(&buffer[..count]);
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}
fn checked_path(root: &Path, path: &Path, directory: bool) -> Result<PathBuf, Failure> {
    let relative = path.strip_prefix(root).map_err(|_|failure("evidence path is outside its root"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else { return Err(failure("invalid evidence path component")); };
        current.push(name);
        if fs::symlink_metadata(&current).map_err(Failure::io)?.file_type().is_symlink() { return Err(failure("evidence path contains a symbolic link")); }
    }
    let resolved = path.canonicalize().map_err(Failure::io)?;
    if !resolved.starts_with(root) || (directory && !resolved.is_dir()) || (!directory && !resolved.is_file()) {
        return Err(failure("evidence path escapes its root or has the wrong file type"));
    }
    Ok(resolved)
}
fn directory(root: &Path, path: &Path) -> Result<(), Failure> {
    match fs::create_dir(path) {
        Ok(()) => {},
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
        Err(error) => return Err(Failure::io(error)),
    }
    checked_path(root, path, true)?;
    Ok(())
}
fn verify_bundle(root: &Path, destination: &Path, manifest: &[u8], blobs: &BTreeMap<String, Vec<u8>>, runs: &BTreeMap<String, Vec<u8>>) -> Result<(), Failure> {
    checked_path(root, destination, true)?;
    let mut expected = BTreeMap::new();
    expected.insert(PathBuf::from("manifest.json"), hash_bytes(manifest));
    for (hash, data) in blobs { expected.insert(PathBuf::from("blobs").join(&hash[7..]), hash_bytes(data)); }
    for (id, data) in runs { expected.insert(PathBuf::from("runs").join(format!("{id}.json")), hash_bytes(data)); }
    for directory_name in ["blobs", "runs"] { checked_path(root, &destination.join(directory_name), true)?; }
    let top: BTreeSet<_> = fs::read_dir(destination).map_err(Failure::io)?.map(|entry|entry.map(|e|e.file_name())).collect::<Result<_,_>>().map_err(Failure::io)?;
    if top != ["manifest.json", "blobs", "runs"].into_iter().map(std::ffi::OsString::from).collect() { return Err(failure("existing evidence contains unexpected entries")); }
    for name in ["blobs", "runs"] {
        let actual: BTreeSet<_> = fs::read_dir(destination.join(name)).map_err(Failure::io)?.map(|entry|entry.map(|e|PathBuf::from(name).join(e.file_name()))).collect::<Result<_,_>>().map_err(Failure::io)?;
        let wanted = expected.keys().filter(|path|path.starts_with(name)).cloned().collect();
        if actual != wanted { return Err(failure("existing evidence file set differs")); }
    }
    for (relative, expected_hash) in expected {
        let path = checked_path(root, &destination.join(relative), false)?;
        if hash_file(&path)? != expected_hash { return Err(failure("existing evidence content differs")); }
    }
    Ok(())
}
fn verify_run_artifacts(run: &Value, blobs: &BTreeMap<String, Vec<u8>>, target: &str) -> Result<(), Failure> {
    for (role, expected) in [("graph",&run["binding"]["graph_hash"]),("compiler",&run["compiler_hash"]),
        ("policy",&run["policy_hash"]),("toolchain_lock",&run["toolchain_lock_hash"])] {
        if journal::role(run,role)?["sha256"] != *expected { return Err(failure("receipt identity differs from its registered input artifact")); }
    }
    if !run["runtime_hash"].is_null() && journal::role(run,"runtime")?["sha256"] != run["runtime_hash"] { return Err(failure("runtime artifact differs from receipt identity")); }
    if journal::role(run,"response")?["sha256"] != hash_bytes(&canonical_bytes(&run["response"]).map_err(Failure::json)?) { return Err(failure("response artifact differs from receipt response")); }
    for artifact in run["artifacts"].as_array().ok_or_else(||failure("receipt artifacts missing"))? {
        let role = artifact["role"].as_str().ok_or_else(||failure("artifact role missing"))?;
        let Some(prefix) = role.strip_suffix("build_record") else { continue; };
        if !prefix.is_empty() && !prefix.ends_with('.') { continue; }
        let hash = artifact["sha256"].as_str().ok_or_else(||failure("build record identity missing"))?;
        let data = blobs.get(hash).ok_or_else(||failure("build record blob missing"))?;
        let build: Value = serde_json::from_slice(data).map_err(Failure::json)?;
        if build["status"]!="VERIFIED" || build["scope"]!="native_compilation" || build["revision"]!=run["binding"]["result_revision"] || build["input"]["target"]!=target {
            return Err(failure("native build record has different status, revision or target"));
        }
        for field in ["compiler_hash","runtime_hash","toolchain_lock_hash","policy_hash"] {
            if build["input"][field]!=run[field] { return Err(failure("native build inputs differ from receipt identities")); }
        }
        if build["input"]["graph_hash"]!=run["binding"]["graph_hash"] || build["source"]["source_git_commit"]!=run["source_git_commit"] || build["source"]["source_tree_hash"]!=run["source_tree_hash"] {
            return Err(failure("native build has different graph or source provenance"));
        }
        let driver = journal::role(run,&format!("{prefix}driver_record"))?;
        if driver["sha256"]!=build["driver_record"]["sha256"] { return Err(failure("driver receipt differs from native build record")); }
        for (name, reference) in build["artifacts"].as_object().ok_or_else(||failure("native build artifacts missing"))? {
            if reference.is_null() { continue; }
            if journal::role(run,&format!("{prefix}{name}"))?["sha256"]!=reference["sha256"] { return Err(failure("native artifact differs from build record")); }
        }
    }
    Ok(())
}
fn write(path: &Path, data: &[u8]) -> Result<(),Failure> {
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(Failure::io)?;
    file.write_all(data).and_then(|_|file.sync_all()).map_err(Failure::io)
}
pub fn emit(store: &Path, context: &Context, policy: &HostPolicy, graph: &Graph, args: &Request) -> Result<Value,Failure> {
    if !il_graph::valid_id(&args.task_id) || args.artifacts.is_empty() || args.tests.is_empty() || args.artifacts.len()>64 || args.tests.len()>64 { return Err(Failure::input("evidence requires a task ID and bounded nonempty artifact/test selectors")); }
    let mut selectors = BTreeSet::new();
    for item in &args.artifacts { if !selectors.insert((&item.run_id,&item.role)) { return Err(Failure::input("duplicate artifact selector")); } }
    if args.tests.iter().collect::<BTreeSet<_>>().len()!=args.tests.len() { return Err(Failure::input("duplicate test run")); }
    let graph_hash = graph.hash().map_err(Failure::json)?;
    let store_root = store.canonicalize().map_err(Failure::io)?;
    let store = store_root.as_path();
    checked_path(store,&store.join(".il-tools"),true)?;
    let compiler_hash = hash_file(&std::env::current_exe().map_err(Failure::io)?)?;
    let lock_hash = hash_file(&context.repository.join("toolchain.lock"))?;
    let policy_hash = hash_bytes(&canonical_bytes(&serde_json::to_value(policy).map_err(Failure::json)?).map_err(Failure::json)?);
    let mut runs = BTreeMap::new();
    let mut run_bytes = BTreeMap::new();
    let mut used = 0;
    for id in args.artifacts.iter().map(|s|&s.run_id).chain(&args.tests) {
        if runs.contains_key(id) { continue; }
        let run = journal::read_run(store,id)?;
        let bytes = canonical_bytes(&run).map_err(Failure::json)?;
        charge(&mut used, bytes.len() as u64)?;
        if run["binding"]["result_revision"]!=args.revision || run["binding"]["graph_hash"]!=graph_hash || !run["binding"]["candidate_hash"].is_null() { return Err(failure("selected run is not bound to this published graph")); }
        if run["compiler_hash"]!=compiler_hash || run["toolchain_lock_hash"]!=lock_hash || run["policy_hash"]!=policy_hash || run["source_git_commit"]!=context.provenance.source_git_commit || run["source_tree_hash"]!=context.provenance.source_tree_hash { return Err(failure("selected run has different compiler, source, toolchain or authority")); }
        if run["response"]["ok"]!=true { return Err(failure("failed run cannot be promoted as passing evidence")); }
        run_bytes.insert(id.clone(), bytes);
        runs.insert(id.clone(),run);
    }
    let mut selected = vec![];
    let mut has_build = false;
    for selector in &args.artifacts {
        let run = &runs[&selector.run_id];
        if !matches!(run["tool"].as_str(),Some("build"|"test"|"blackbox")) { return Err(failure("artifact selector must identify an actual build or execution run")); }
        let artifact = journal::role(run,&selector.role)?;
        has_build |= selector.role == "executable" || selector.role.ends_with(".executable") || selector.role == "object";
        selected.push(json!({"run_id":selector.run_id,"role":selector.role,"sha256":artifact["sha256"]}));
    }
    if !has_build { return Err(failure("evidence must include an actual native object or executable")); }
    let mut tests = vec![];
    for id in &args.tests {
        let run = &runs[id];
        if !matches!(run["tool"].as_str(),Some("test"|"blackbox")) { return Err(failure("test selector must identify an executed test or blackbox run")); }
        let executed = run["tests"].as_array().ok_or_else(||failure("run test list missing"))?;
        if executed.is_empty() || executed.iter().any(|test|test["passed"]!=true || test["count"].as_u64().unwrap_or(0)==0) { return Err(failure("test receipt does not attest successful executions")); }
        for test in executed { tests.push(json!({"run_id":id,"test":test})); }
    }
    let mut blobs = BTreeMap::new();
    let mut runtime_hash = None;
    for run in runs.values() {
        if let Some(hash) = run["runtime_hash"].as_str() {
            if runtime_hash.as_ref().is_some_and(|prior|prior!=hash) { return Err(failure("selected native runs use different runtimes")); }
            runtime_hash=Some(hash.to_owned());
        }
        let mut roles = BTreeSet::new();
        for artifact in run["artifacts"].as_array().ok_or_else(||failure("run artifact list missing"))? {
            let role = artifact["role"].as_str().ok_or_else(||failure("artifact role missing"))?;
            if !roles.insert(role) { return Err(failure("duplicate artifact role")); }
            let hash=artifact["sha256"].as_str().ok_or_else(||failure("artifact hash missing"))?.to_owned();
            if hash.len()!=71 || !hash.starts_with("sha256:") || !hash[7..].bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) { return Err(failure("invalid artifact hash")); }
            if artifact["path"] != format!("blobs/{}",&hash[7..]) { return Err(failure("artifact locator differs from hash")); }
            if !blobs.contains_key(&hash) {
                let path = store.join(".il-tools").join(artifact["path"].as_str().unwrap());
                let size = fs::metadata(&path).map_err(Failure::io)?.len();
                if size > ARTIFACT_LIMIT || size > BUNDLE_LIMIT.saturating_sub(used) { return Err(budget()); }
                let data = journal::artifact_bytes(store,artifact)?;
                charge(&mut used, data.len() as u64)?;
                blobs.insert(hash,data);
            }
        }
    }
    for run in runs.values() { verify_run_artifacts(run,&blobs,&graph.target)?; }
    let manifest = json!({"schema_version":"1.0.0","kind":"application_evidence","task_id":args.task_id,"revision":args.revision,"graph_hash":graph_hash,
        "source_git_commit":context.provenance.source_git_commit,"source_tree_hash":context.provenance.source_tree_hash,"compiler_hash":compiler_hash,"runtime_hash":runtime_hash,
        "target":graph.target,"toolchain_lock_hash":lock_hash,"policy_hash":policy_hash,"runs":runs.keys().collect::<Vec<_>>(),"selected_artifacts":selected,"tests":tests,
        "blobs":blobs.keys().collect::<Vec<_>>(),"known_limits":["Application execution evidence does not change development task status.","Rebuild requires the recorded LLVM tools and target host."]});
    let bytes=canonical_bytes(&manifest).map_err(Failure::json)?;
    charge(&mut used, bytes.len() as u64)?;
    let id=format!("ev_{}",&hash_bytes(&bytes)[7..]);
    let journal_root=store.join(".il-tools");directory(store,&journal_root)?;
    let evidence_root=journal_root.join("evidence");directory(store,&evidence_root)?;
    let destination=evidence_root.join(&id);
    if destination.exists() {
        verify_bundle(store,&destination,&bytes,&blobs,&run_bytes)?;
    } else {
        let owner=tempfile::Builder::new().prefix("pending-").tempdir_in(&evidence_root).map_err(Failure::io)?;
        let temporary=owner.path();
        fs::create_dir(temporary.join("blobs")).map_err(Failure::io)?;
        fs::create_dir(temporary.join("runs")).map_err(Failure::io)?;
        for(hash,data)in &blobs { write(&temporary.join("blobs").join(&hash[7..]),data)?; }
        for(run_id,data)in &run_bytes { write(&temporary.join("runs").join(format!("{run_id}.json")),data)?; }
        write(&temporary.join("manifest.json"),&bytes)?;
        #[cfg(unix)]
        for path in [temporary.to_path_buf(),temporary.join("blobs"),temporary.join("runs")] { fs::File::open(path).and_then(|f|f.sync_all()).map_err(Failure::io)?; }
        if let Err(error) = fs::rename(temporary,&destination) {
            if destination.exists() { verify_bundle(store,&destination,&bytes,&blobs,&run_bytes)?; }
            else { return Err(Failure::io(error)); }
        }
        #[cfg(unix)]
        fs::File::open(&evidence_root).and_then(|f|f.sync_all()).map_err(Failure::io)?;
    }
    Ok(json!({"evidence_id":id,"revision":args.revision,"graph_hash":graph_hash,"artifacts":[{"path":destination.join("manifest.json").canonicalize().map_err(Failure::io)?,"sha256":hash_bytes(&bytes)}]}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success<T>(result: Result<T,Failure>) -> T { result.unwrap_or_else(|error| panic!("{}: {}",error.code,error.message)) }

    #[test]
    fn bundle_reuse_detects_changed_bytes_and_unregistered_files() {
        let owner = tempfile::tempdir().unwrap();
        let root = owner.path().canonicalize().unwrap();
        let destination = root.join("evidence");
        fs::create_dir(&destination).unwrap();
        fs::create_dir(destination.join("blobs")).unwrap();
        fs::create_dir(destination.join("runs")).unwrap();
        let data = b"verified native object".to_vec();
        let hash = hash_bytes(&data);
        let blobs = BTreeMap::from([(hash.clone(),data.clone())]);
        let run_id = format!("run_{}", "a".repeat(64));
        let runs = BTreeMap::from([(run_id.clone(),b"receipt".to_vec())]);
        success(write(&destination.join("manifest.json"),b"manifest"));
        success(write(&destination.join("blobs").join(&hash[7..]),&data));
        success(write(&destination.join("runs").join(format!("{run_id}.json")),b"receipt"));
        success(verify_bundle(&root,&destination,b"manifest",&blobs,&runs));
        fs::write(destination.join("blobs").join(&hash[7..]),b"changed native object").unwrap();
        assert!(verify_bundle(&root,&destination,b"manifest",&blobs,&runs).is_err());
        fs::write(destination.join("blobs").join(&hash[7..]),data).unwrap();
        fs::write(destination.join("runs/unregistered.json"),b"{}").unwrap();
        assert!(verify_bundle(&root,&destination,b"manifest",&blobs,&runs).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn bundle_paths_reject_symlinks_even_for_identical_content() {
        use std::os::unix::fs::symlink;
        let owner = tempfile::tempdir().unwrap();
        let root = owner.path().canonicalize().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("native"),b"trusted").unwrap();
        symlink(outside.path(),root.join("evidence")).unwrap();
        assert!(checked_path(&root,&root.join("evidence"),true).is_err());
        assert!(checked_path(&root,&root.join("evidence/native"),false).is_err());
        fs::create_dir(root.join("blobs")).unwrap();
        symlink(outside.path().join("native"),root.join("blobs/native")).unwrap();
        assert!(checked_path(&root,&root.join("blobs/native"),false).is_err());
    }

    #[test]
    fn total_evidence_budget_rejects_aggregate_and_integer_overflow() {
        let mut used = BUNDLE_LIMIT - 1;
        success(charge(&mut used,1));
        assert_eq!(charge(&mut used,1).unwrap_err().code,"E_RESOURCE_LIMIT");
        let mut used = 1;
        assert_eq!(charge(&mut used,u64::MAX).unwrap_err().code,"E_RESOURCE_LIMIT");
    }

    #[test]
    fn receipt_cannot_substitute_another_input_artifact() {
        let identity = hash_bytes(b"actual input");
        let response = json!({"ok":true});
        let mut run = json!({"binding":{"graph_hash":identity},"compiler_hash":identity,"policy_hash":identity,
            "toolchain_lock_hash":identity,"runtime_hash":null,"response":response,"artifacts":[]});
        for role in ["graph","compiler","policy","toolchain_lock"] {
            run["artifacts"].as_array_mut().unwrap().push(json!({"role":role,"sha256":identity}));
        }
        run["artifacts"].as_array_mut().unwrap().push(json!({"role":"response","sha256":hash_bytes(&canonical_bytes(&response).unwrap())}));
        success(verify_run_artifacts(&run,&BTreeMap::new(),il_graph::TARGET));
        run["artifacts"][0]["sha256"] = json!(hash_bytes(b"substituted graph"));
        assert_eq!(verify_run_artifacts(&run,&BTreeMap::new(),il_graph::TARGET).unwrap_err().code,"E_EVIDENCE_INCOMPLETE");
    }
}
