//! Immutable, content-addressed tool receipts. Client JSON never supplies paths.
use crate::{protocol::{Failure, failed}, source::Context};
use il_graph::{canonical_bytes, hash_bytes, Graph, Store};
use il_runtime_startup::HostPolicy;
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, io::{Read, Write}, path::{Path, PathBuf}};

const RECORD_LIMIT: u64 = 32 * 1024 * 1024;
pub const COMMANDS: &[&str] = &["state", "schema-check", "inspect", "callers", "dependencies", "slice", "validate", "transact", "diff", "restore", "build", "test", "blackbox", "explain", "evidence"];
fn invalid(message: impl AsRef<str>) -> Failure { Failure::new("E_STATE_INCONSISTENT", message.as_ref()) }
fn root(store: &Path) -> PathBuf { store.join(".il-tools") }
fn safe_path(store: &Path, path: &Path) -> Result<(), Failure> {
    let relative = path.strip_prefix(store).map_err(|_|invalid("path is outside application store"))?;
    let mut cursor = store.to_path_buf();
    for component in relative.components() {
        if !matches!(component,std::path::Component::Normal(_)) { return Err(invalid("invalid journal path component")); }
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(invalid("journal paths cannot contain symlinks")),
            Ok(_) => {},
            Err(error) if error.kind()==std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(Failure::io(error)),
        }
    }
    Ok(())
}
fn read_bytes(path: &Path, maximum: u64) -> Result<Vec<u8>, Failure> {
    let mut data = vec![];
    fs::File::open(path).map_err(Failure::io)?.take(maximum + 1).read_to_end(&mut data).map_err(Failure::io)?;
    if data.len() as u64 > maximum { return Err(Failure::new("E_RESOURCE_LIMIT", "journal artifact exceeds its byte limit")); }
    Ok(data)
}
fn publish(store: &Path, path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    let parent = path.parent().ok_or_else(|| invalid("missing journal parent"))?;
    fs::create_dir_all(parent).map_err(Failure::io)?;
    #[cfg(unix)]
    {
        // Persist newly created journal directory entries as well as file bytes.
        let mut directory=parent;
        while let Some(ancestor)=directory.parent() {
            fs::File::open(ancestor).and_then(|file|file.sync_all()).map_err(Failure::io)?;
            if ancestor==store { break; }
            directory=ancestor;
        }
    }
    if path.exists() {
        if read_bytes(path, bytes.len() as u64)? != bytes { return Err(invalid("immutable journal content differs")); }
        return Ok(());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(Failure::io)?;
    temporary.write_all(bytes).map_err(Failure::io)?;
    temporary.as_file().sync_all().map_err(Failure::io)?;
    match temporary.persist_noclobber(path) {
        Ok(_) => {},
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            if read_bytes(path, bytes.len() as u64)? != bytes { return Err(invalid("journal identity collision")); }
        }
        Err(error) => return Err(Failure::io(error.error)),
    }
    #[cfg(unix)]
    fs::File::open(parent).and_then(|file| file.sync_all()).map_err(Failure::io)?;
    Ok(())
}
fn blob(store: &Path, role: &str, data: &[u8]) -> Result<Value, Failure> {
    let sha = hash_bytes(data);
    let relative = format!("blobs/{}", &sha[7..]);
    let path=root(store).join(&relative);
    safe_path(store,&path)?;
    publish(store,&path, data)?;
    Ok(json!({"role":role,"path":relative,"sha256":sha}))
}
fn identifier(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|suffix| suffix.len() == 64 && suffix.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}
pub fn artifact_bytes(store: &Path, artifact: &Value) -> Result<Vec<u8>, Failure> {
    let sha = artifact["sha256"].as_str().ok_or_else(|| invalid("artifact hash missing"))?;
    if !identifier(sha, "sha256:") { return Err(invalid("invalid artifact identity")); }
    let expected = format!("blobs/{}", &sha[7..]);
    if artifact["path"] != expected { return Err(invalid("artifact locator differs from its content identity")); }
    safe_path(store,&root(store).join(&expected))?;
    let directory = root(store).canonicalize().map_err(Failure::io)?;
    let path = directory.join(expected);
    let resolved = path.canonicalize().map_err(Failure::io)?;
    if !resolved.starts_with(&directory) || fs::symlink_metadata(&path).map_err(Failure::io)?.file_type().is_symlink() { return Err(invalid("journal path escapes its store")); }
    let data = read_bytes(&resolved, 256 * 1024 * 1024)?;
    if hash_bytes(&data) != sha { return Err(invalid("journal artifact hash differs")); }
    Ok(data)
}
pub fn read_run(store: &Path, id: &str) -> Result<Value, Failure> {
    if !identifier(id, "run_") { return Err(Failure::input("run_id must be a full content identity")); }
    let path = root(store).join("runs").join(format!("{id}.json"));
    safe_path(store,&path)?;
    if !path.exists() { return Err(Failure::new("E_NAME_NOT_FOUND", "tool run does not exist")); }
    let directory = root(store).canonicalize().map_err(Failure::io)?;
    if !path.canonicalize().map_err(Failure::io)?.starts_with(&directory) || fs::symlink_metadata(&path).map_err(Failure::io)?.file_type().is_symlink() { return Err(invalid("run path escapes journal")); }
    let data = read_bytes(&path, RECORD_LIMIT)?;
    if &hash_bytes(&data)[7..] != &id[4..] { return Err(invalid("tool receipt was modified")); }
    crate::strict_json::parse(std::str::from_utf8(&data).map_err(|_| invalid("receipt is not UTF-8"))?).map_err(Failure::json)
}
pub fn role<'a>(run: &'a Value, name: &str) -> Result<&'a Value, Failure> {
    let values = run["artifacts"].as_array().ok_or_else(|| invalid("receipt artifact list missing"))?;
    let mut found = values.iter().filter(|value| value["role"] == name);
    let value = found.next().ok_or_else(|| Failure::new("E_NAME_NOT_FOUND", "artifact role is not registered by this run"))?;
    if found.next().is_some() { return Err(invalid("duplicate artifact role")); }
    Ok(value)
}
fn snapshot(store: &Path, context: &Context, revision: u64) -> Result<Graph, Failure> {
    if store.join(".il/HEAD").is_file() { return Store::open(store)?.load(revision).map_err(Into::into); }
    let graph: Graph = serde_json::from_value(context.seed.clone()).map_err(Failure::json)?;
    if graph.revision != revision { return Err(Failure::new("E_NAME_NOT_FOUND", "revision is unavailable")); }
    Ok(graph)
}
pub fn subject(tool: &str, request: &Value) -> Option<u64> {
    match tool {
        "transact" => request["base_revision"].as_u64(),
        "diff" => request["target_revision"].as_u64(),
        "validate" => request["graph_or_revision"].as_u64().or_else(|| request["graph_or_revision"]["revision"].as_u64()),
        "restore" | "state" | "schema-check" | "explain" => None,
        _ => request["revision"].as_u64(),
    }
}
pub fn bind_response(tool: &str, request: &Value, response: &mut Value, observed: u64) {
    if tool=="transact" { response["base_revision"]=json!(request["base_revision"].as_u64().unwrap_or(observed)); }
    else { response["base_revision"]=json!(observed); }
    if !matches!(tool,"transact"|"restore") {
        if let Some(revision)=subject(tool,request) { response["result_revision"]=json!(revision); }
    }
}
fn copy_reference(store: &Path, role_name: &str, reference: &Value) -> Result<Value, Failure> {
    let path = Path::new(reference["path"].as_str().ok_or_else(|| invalid("build artifact path missing"))?);
    let resolved = path.canonicalize().map_err(Failure::io)?;
    let directory = store.canonicalize().map_err(Failure::io)?;
    if !resolved.starts_with(&directory) || fs::symlink_metadata(path).map_err(Failure::io)?.file_type().is_symlink() { return Err(invalid("build artifact escapes application store")); }
    safe_path(&directory,path)?;
    let data = read_bytes(&resolved, 256 * 1024 * 1024)?;
    if reference["sha256"] != hash_bytes(&data) { return Err(invalid("build artifact hash differs")); }
    blob(store, role_name, &data)
}
pub fn record(store: &Path, context: &Context, policy: &HostPolicy, tool: &str, request: &Value, response: &Value, delivery: &Value, observed: u64) -> Result<String, Failure> {
    let bytes = canonical_bytes(response).map_err(Failure::json)?;
    if bytes.len() as u64 > RECORD_LIMIT / 2 { return Err(Failure::new("E_RESOURCE_LIMIT", "tool response exceeds retained receipt budget")); }
    let revision = response["result_revision"].as_u64().unwrap_or(observed);
    let graph = match snapshot(store, context, revision) {
        Ok(graph)=>Some(graph),
        Err(error) if error.code=="E_NAME_NOT_FOUND" && (response["ok"]!=true || tool=="validate") => None,
        Err(error)=>return Err(error),
    };
    let graph_hash = graph.as_ref().map(Graph::hash).transpose().map_err(Failure::json)?;
    let compiler_path = std::env::current_exe().map_err(Failure::io)?;
    let compiler = read_bytes(&compiler_path, 256 * 1024 * 1024)?;
    let lock = fs::read(context.repository.join("toolchain.lock")).map_err(Failure::io)?;
    let policy_bytes = canonical_bytes(&serde_json::to_value(policy).map_err(Failure::json)?).map_err(Failure::json)?;
    let mut artifacts = vec![blob(store,"response",&bytes)?, blob(store,"policy",&policy_bytes)?, blob(store,"toolchain_lock",&lock)?, blob(store,"compiler",&compiler)?];
    if let Some(graph) = &graph { artifacts.push(blob(store,"graph",&graph.canonical_bytes().map_err(Failure::json)?)?); }
    let candidate = if tool == "validate" && request["graph_or_revision"].is_object() { Some(canonical_bytes(&request["graph_or_revision"]).map_err(Failure::json)?) }
        else if tool == "schema-check" && response["result"]["graph"].is_object() { Some(canonical_bytes(&response["result"]["graph"]).map_err(Failure::json)?) }
        else if tool == "transact" {
            if let Some(relative)=response["result"]["candidate"].as_str() {
                let directory=store.join(relative);
                safe_path(store,&directory)?;
                let retained_diagnostics=read_bytes(&directory.join("diagnostics.json"),RECORD_LIMIT)?;
                let diagnostics:Value=serde_json::from_slice(&retained_diagnostics).map_err(Failure::json)?;
                if diagnostics!=response["diagnostics"] { return Err(invalid("retained diagnostics differ")); }
                let path=directory.join("graph.json");
                let mut identity;
                let candidate=if path.exists() {
                    safe_path(store,&path)?;
                    identity=read_bytes(&directory.join("transaction.json"),RECORD_LIMIT)?;
                    let data=read_bytes(&path,RECORD_LIMIT)?;
                    identity.extend(&data);
                    Some(data)
                } else {
                    identity=read_bytes(&directory.join("request.json"),RECORD_LIMIT)?;
                    if serde_json::from_slice::<Value>(&identity).map_err(Failure::json)?!=*request { return Err(invalid("retained request differs")); }
                    artifacts.push(blob(store,"candidate_request",&identity)?);
                    None
                };
                identity.extend(&retained_diagnostics);
                if directory.file_name().and_then(|name|name.to_str())!=Some(&hash_bytes(&identity)[7..]) { return Err(invalid("retained candidate identity differs")); }
                candidate
            } else { None }
        } else { None };
    let candidate = candidate.map(|bytes| -> Result<Vec<u8>,Failure> {
        // Valid graph candidates use the graph's canonical field order. Malformed
        // schema candidates remain exact JSON data and are not claimed as graphs.
        match serde_json::from_slice::<Graph>(&bytes) {
            Ok(graph)=>graph.canonical_bytes().map_err(Failure::json),
            Err(_)=>Ok(bytes),
        }
    }).transpose()?;
    let candidate_hash=candidate.as_ref().map(|bytes|hash_bytes(bytes));
    if let Some(bytes)=candidate { artifacts.push(blob(store,"candidate",&bytes)?); }
    let mut commands = vec![];
    let mut tests = vec![];
    let mut native_builds = vec![];
    if response["result"]["native"].is_object() { native_builds.push((String::new(), &response["result"]["native"])); }
    if tool == "blackbox" {
        if let Some(profiles) = response["result"]["profiles"].as_array() {
            for profile in profiles {
                let prefix=format!("{}.",profile["profile"].as_str().ok_or_else(||invalid("blackbox profile missing"))?);
                for name in ["stdout","stderr"] { if profile["process"][name].is_object() { artifacts.push(copy_reference(store,&format!("{prefix}{name}"),&profile["process"][name])?); } }
                if profile["process_record"].is_object() { artifacts.push(copy_reference(store,&format!("{prefix}process_record"),&profile["process_record"])?); }
                native_builds.push((prefix, &profile["build"]["native"]));
            }
        }
        if response["result"]["contract_artifact"].is_object() { artifacts.push(copy_reference(store,"http_contract",&response["result"]["contract_artifact"])?); }
        if response["result"]["report"].is_object() { artifacts.push(copy_reference(store,"blackbox_report",&response["result"]["report"])?); }
    }
    let mut runtime_hash = Value::Null;
    for (prefix,native) in native_builds {
        if let Some(values) = native["artifacts"].as_object() { for (name,value) in values { if value.is_object() { artifacts.push(copy_reference(store,&format!("{prefix}{name}"),value)?); } } }
        let build_ref = copy_reference(store,&format!("{prefix}build_record"),&native["build_record"])?;
        let build: Value = serde_json::from_slice(&artifact_bytes(store,&build_ref)?).map_err(Failure::json)?;
        if build["input"]["policy_hash"]!=hash_bytes(&policy_bytes) { return Err(invalid("native build policy differs from receipt")); }
        let driver_ref = copy_reference(store,&format!("{prefix}driver_record"),&build["driver_record"])?;
        let driver: Value = serde_json::from_slice(&artifact_bytes(store,&driver_ref)?).map_err(Failure::json)?;
        artifacts.push(copy_reference(store,&format!("{prefix}optimized_ir"),&driver["optimized_ir"])?);
        commands.extend(driver["commands"].as_array().cloned().unwrap_or_default());
        runtime_hash = build["input"]["runtime_hash"].clone();
        if let Some(expected) = runtime_hash.as_str() {
            let name = if build["input"]["runtime_profile"] == "minimal" { "libil_minimal_runtime.a" } else { "libil_native_runtime.a" };
            let runtime = read_bytes(&compiler_path.parent().unwrap().join(name), 256*1024*1024)?;
            if hash_bytes(&runtime) != expected { return Err(invalid("runtime changed since compilation")); }
            if !artifacts.iter().any(|item|item["role"]=="runtime") { artifacts.push(blob(store,"runtime",&runtime)?); }
        }
        artifacts.extend([build_ref,driver_ref]);
    }
    if tool == "test" && response["result"]["execution"].is_object() {
        artifacts.push(blob(store,"execution_input",&canonical_bytes(&response["result"]["execution_input"]).map_err(Failure::json)?)?);
        artifacts.push(blob(store,"execution_report",&canonical_bytes(&response["result"]["execution"]).map_err(Failure::json)?)?);
        tests.push(json!({"suite":request["suite"]["entry"],"passed":response["ok"]==true,"count":1,"contract_hash":null}));
    }
    if tool == "blackbox" { tests = response["result"]["tests"].as_array().cloned().unwrap_or_default(); }
    let comparison = if tool == "diff" { if let Some(base)=request["base_revision"].as_u64() {
        match snapshot(store,context,base) { Ok(graph)=>Some(graph.hash().map_err(Failure::json)?), Err(error) if error.code=="E_NAME_NOT_FOUND" && response["ok"]!=true=>None, Err(error)=>return Err(error) }
    } else { None } } else { None };
    let receipt = json!({"schema_version":"1.0.0","tool":tool,"request":request,"request_hash":hash_bytes(&canonical_bytes(request).map_err(Failure::json)?),
        "source_git_commit":context.provenance.source_git_commit,"source_tree_hash":context.provenance.source_tree_hash,
        "binding":{"base_revision":response["base_revision"],"result_revision":revision,"observed_head_revision":observed,"graph_hash":graph_hash,"candidate_hash":candidate_hash,"comparison_graph_hash":comparison},
        "compiler_hash":hash_bytes(&compiler),"runtime_hash":runtime_hash,"toolchain_lock_hash":hash_bytes(&lock),"policy_hash":hash_bytes(&policy_bytes),"response":response,"delivery":delivery,"commands":commands,"artifacts":artifacts,"tests":tests});
    let bytes = canonical_bytes(&receipt).map_err(Failure::json)?;
    if bytes.len() as u64 > RECORD_LIMIT { return Err(Failure::new("E_RESOURCE_LIMIT", "tool receipt exceeds 32 MiB")); }
    let id = format!("run_{}", &hash_bytes(&bytes)[7..]);
    let path=root(store).join("runs").join(format!("{id}.json"));
    safe_path(store,&path)?;
    publish(store,&path, &bytes)?;
    Ok(id)
}
pub fn explain(store: &Path, run_id: &str, diagnostic_id: &str) -> Result<(u64, Value), Failure> {
    let run = read_run(store,run_id)?;
    for reference in run["artifacts"].as_array().ok_or_else(||invalid("receipt artifacts missing"))? { artifact_bytes(store,reference)?; }
    let diagnostics = run["response"]["diagnostics"].as_array().ok_or_else(|| invalid("receipt diagnostics missing"))?;
    let delivered = run["delivery"]["diagnostics"].as_array().ok_or_else(|| invalid("receipt delivery diagnostics missing"))?;
    let matches: Vec<_> = diagnostics.iter().chain(delivered).filter(|d|d["diagnostic_id"]==diagnostic_id).collect();
    let diagnostic = matches.first().ok_or_else(||Failure::new("E_NAME_NOT_FOUND","diagnostic is not part of selected run"))?;
    if matches.iter().any(|other| other != diagnostic) { return Err(invalid("diagnostic identity is ambiguous in selected run")); }
    let revision = diagnostic["base_revision"].as_u64().ok_or_else(||invalid("diagnostic revision missing"))?;
    let candidate = role(&run,"candidate").or_else(|_|role(&run,"graph"));
    let mut related = vec![];
    if let Ok(reference) = candidate {
        let data=artifact_bytes(store,reference)?;
        let graph=serde_json::from_slice::<Graph>(&data).ok();
        let ids: BTreeSet<_> = diagnostic["entity_id"].as_str().into_iter().chain(diagnostic["related_entities"].as_array().into_iter().flatten().filter_map(Value::as_str)).collect();
        for id in ids {
            let available=if let Some(graph)=&graph {
                match il_graph::entity_view(graph,id,Some(&[])) { Ok(_)=>true,Err(error) if error.code=="E_NAME_NOT_FOUND"=>false,Err(error)=>return Err(error.into()) }
            } else { false };
            related.push(json!({"entity_id":id,"available":available}));
        }
    }
    Ok((revision,json!({"subject_run_id":run_id,"diagnostic":diagnostic,"binding":run["binding"],"related_entities":related,
        "rules":crate::diagnostic_rules::rules(diagnostic["code"].as_str().unwrap_or("")),
        "suggested_operations":diagnostic["suggested_operations"]})))
}
pub fn delivery(tool: &str, request: &Value, response: &Value) -> Value {
    let budget = match tool {
        "inspect"|"callers"|"dependencies" => request["budget"].as_u64().unwrap_or(16_384),
        "slice" => request["max_tokens"].as_u64().unwrap_or(16_384),
        "explain" => request["context_budget"].as_u64().unwrap_or(16_384),
        _ => 262_144,
    };
    // Native execution reports can contain thousands of lifecycle moves. The
    // complete report is retained in the receipt and execution_report blob;
    // the wire response may use the bounded execution projection below.
    let mut delivered = response.clone();
    let full_required = serde_json::to_vec(&deliver(delivered.clone(),&format!("run_{}","0".repeat(64)))).unwrap().len() as u64;
    if full_required > budget && tool == "test"
        && response["ok"] == true
        && response["result"]["execution"]["status"] == "returned"
    {
        let execution = delivered["result"]["execution"].as_object_mut();
        if let Some(execution) = execution {
            // Lifecycle is detailed provenance, not a semantic result. Keep
            // the schema-required field present while retaining the exact
            // unbounded sequence in the receipt artifact.
            execution.insert("lifecycle".into(), json!([]));
            delivered["result"]["execution_report_artifact"] = json!("execution_report");
        }
        let projected_required = serde_json::to_vec(&deliver(delivered.clone(),&format!("run_{}","0".repeat(64)))).unwrap().len() as u64;
        if projected_required <= budget { return delivered; }
    }
    // Content-addressed IDs have a fixed width. Account for the eventual ID
    // before hashing the receipt, so budget diagnostics are themselves durable.
    let required = full_required;
    if required > budget {
        let mut error = failed(tool,subject(tool,request).unwrap_or_else(||response["result_revision"].as_u64().unwrap_or(0)),&Failure::new("E_CONTEXT_INSUFFICIENT","complete response exceeds context budget; retained receipt is available"));
        error["base_revision"]=response["base_revision"].clone();
        error["result_revision"] = response["result_revision"].clone();
        let (missing,source)=if let Some(missing)=response["result"]["missing"].as_array() { (missing.clone(),"receipt.response.result.missing") }
            else { (request["root_entities"].as_array().cloned().unwrap_or_else(||request["entity_id"].as_str().map(|id|vec![json!(id)]).unwrap_or_default()),"receipt.request") };
        let mut used=0;
        let prefix:Vec<_>=missing.iter().take(16).take_while(|id|{used+=serde_json::to_vec(id).unwrap().len();used<=1024}).cloned().collect();
        let truncated=prefix.len()!=missing.len();
        error["result"] = json!({"status":"BLOCKED","missing":prefix,"missing_truncated":truncated,"missing_source":source,"budget":budget,"required":required});
        if response["ok"]==true && matches!(tool,"transact"|"restore") { error["result"]["committed"]=json!(true); error["result"]["revision"]=response["result_revision"].clone(); }
        error
    } else { response.clone() }
}
pub fn deliver(mut response: Value, id: &str) -> Value {
    if !response["result"].is_object() { response["result"]=json!({}); }
    response["result"]["run_id"]=json!(id);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_entity_id_keeps_a_bounded_explainable_delivery() {
        let request=json!({"entity_id":"a".repeat(260000),"revision":2,"budget":1});
        let response=failed("inspect",2,&Failure::new("E_NAME_NOT_FOUND","entity missing"));
        let delivery=delivery("inspect",&request,&response);
        assert_eq!(delivery["result"]["missing_truncated"],true);
        assert_eq!(delivery["result"]["missing"],json!([]));
        let delivered=deliver(delivery.clone(),&format!("run_{}","a".repeat(64)));
        assert!(serde_json::to_vec(&delivered).unwrap().len()<2048);
        assert_eq!(delivered["diagnostics"],delivery["diagnostics"]);
    }
    #[test]
    fn historical_failure_preserves_observed_and_subject_revisions() {
        let request=json!({"revision":2});
        let mut response=failed("inspect",2,&Failure::new("E_NAME_NOT_FOUND","entity missing"));
        bind_response("inspect",&request,&mut response,9);
        assert_eq!(response["base_revision"],9);
        assert_eq!(response["result_revision"],2);
        assert_eq!(response["diagnostics"][0]["base_revision"],2);
    }
    #[test]
    #[cfg(unix)]
    fn journal_refuses_a_symlinked_output_directory() {
        let store=tempfile::tempdir().unwrap();
        let other=tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(),store.path().join(".il-tools")).unwrap();
        assert!(blob(store.path(),"response",b"{}").is_err());
        assert_eq!(fs::read_dir(other.path()).unwrap().count(),0);
    }
}
