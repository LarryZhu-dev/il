mod protocol;
mod source;
mod strict_json;
mod execution;
mod native;

use il_graph::*;
use protocol::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::env;
use std::io::{self, Read};
use std::path::PathBuf;

const INPUT_LIMIT: usize = 262_144;
const OUTPUT_LIMIT: usize = 262_144;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn optional<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceText { source: String }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportText { op: String, source: String }

fn transaction_request(input: &str) -> Result<Transaction, Failure> {
    let mut value: Value = request(input)?;
    let revision = value.get("base_revision").and_then(Value::as_u64)
        .ok_or_else(|| Failure::input("base_revision must be uint64"))?;
    if let Some(operations) = value.get_mut("operations").and_then(Value::as_array_mut) {
        for operation in operations {
            if operation.get("op").and_then(Value::as_str) == Some("import_text") {
                let source: ImportText = serde_json::from_value(operation.clone()).map_err(Failure::json)?;
                debug_assert_eq!(source.op, "import_text");
                let graph = il_frontend::parse(&source.source, revision).map_err(Failure::diagnostics)?;
                *operation = serde_json::to_value(TransactionOperation::ReplaceProgram { graph }).map_err(Failure::json)?;
            }
        }
    }
    serde_json::from_value(value).map_err(Failure::json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inspect {
    entity_id: String,
    #[serde(default, deserialize_with = "optional")]
    revision: Option<u64>,
    #[serde(default, deserialize_with = "optional")]
    fields: Option<Vec<String>>,
    #[serde(default, deserialize_with = "optional")]
    budget: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Validate {
    graph_or_revision: Value,
    checks: Vec<Check>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Test {
    revision: u64,
    suite: execution::Suite,
    isolation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Build {
    revision: u64,
    target: String,
    profile: il_object_emitter::Profile,
    #[serde(default = "full_runtime")]
    runtime_profile: il_native_ir::RuntimeProfile,
    #[serde(default, deserialize_with = "optional")]
    exports: Option<Vec<String>>,
}

fn full_runtime() -> il_native_ir::RuntimeProfile { il_native_ir::RuntimeProfile::Full }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Diff {
    base_revision: u64,
    target_revision: u64,
    #[serde(default)]
    scope: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Restore {
    revision: u64,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Slice {
    root_entities: Vec<String>,
    #[serde(default, deserialize_with = "optional")]
    revision: Option<u64>,
    max_nodes: usize,
    max_tokens: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Relations {
    entity_id: String,
    #[serde(default, deserialize_with = "optional")]
    revision: Option<u64>,
    #[serde(default, deserialize_with = "optional")]
    budget: Option<usize>,
    #[serde(default, deserialize_with = "optional")]
    direction: Option<String>,
}

struct Options {
    repository: PathBuf,
    store: PathBuf,
    command: String,
    host_policy: il_runtime_startup::HostPolicy,
}

fn options() -> Result<(Options, Option<PathBuf>), Failure> {
    let mut args = env::args_os().skip(1);
    let mut repository = env::current_dir().map_err(Failure::io)?;
    let mut store = None;
    let mut command = None;
    let mut policy_path = None;
    let mut repository_seen = false;
    while let Some(argument) = args.next() {
        if command.is_some() { return Err(Failure::input("unexpected argument after command")); }
        match argument.to_str() {
            Some("--repository") if !repository_seen => {
                repository = PathBuf::from(args.next().ok_or_else(|| Failure::input("--repository requires a path"))?);
                repository_seen = true;
            }
            Some("--store") if store.is_none() => {
                store = Some(PathBuf::from(args.next().ok_or_else(|| Failure::input("--store requires a path"))?));
            }
            Some("--host-policy") if policy_path.is_none() => {
                policy_path = Some(PathBuf::from(args.next().ok_or_else(|| Failure::input("--host-policy requires a path"))?));
            }
            Some(value) if !value.starts_with('-') => command = Some(value.to_owned()),
            _ => return Err(Failure::input("unknown or duplicate option")),
        }
    }
    let command = command.ok_or_else(|| Failure::input("expected an il command"))?;
    let store = store.unwrap_or_else(|| repository.join(".il/project"));
    Ok((Options { repository, store, command, host_policy: il_runtime_startup::HostPolicy::empty() }, policy_path))
}

fn request<T: serde::de::DeserializeOwned>(input: &str) -> Result<T, Failure> {
    let value = strict_json::parse(input).map_err(Failure::json)?;
    if let Some(object) = value.as_object() {
        for key in ["revision", "fields", "budget", "direction"] {
            if object.get(key) == Some(&Value::Null) { return Err(Failure::input("optional request fields cannot be null")); }
        }
        if let Some(id) = object.get("entity_id").and_then(Value::as_str) {
            if !valid_id(id) { return Err(Failure::input("invalid entity identifier")); }
        }
        for key in ["scope", "root_entities"] {
            if let Some(ids) = object.get(key).and_then(Value::as_array) {
                if ids.iter().any(|id| !id.as_str().is_some_and(valid_id)) { return Err(Failure::input("invalid entity identifier list")); }
            }
        }
    }
    serde_json::from_value(value).map_err(Failure::json)
}

fn bounded(value: Value, budget: Option<usize>) -> Result<Value, Failure> {
    let budget = budget.unwrap_or(16_384);
    if budget == 0 || budget > 65_536 { return Err(Failure::input("budget must be between 1 and 65536")); }
    if serde_json::to_vec(&value).map_err(Failure::json)?.len() > budget {
        return Err(Failure::new("E_CONTEXT_INSUFFICIENT", "result exceeds conservative UTF-8 byte budget"));
    }
    Ok(value)
}

fn load(store: Option<&Store>, context: &source::Context, revision: u64) -> Result<Graph, Failure> {
    if let Some(store) = store { return Ok(store.load(revision)?); }
    let graph: Graph = serde_json::from_value(context.seed.clone()).map_err(Failure::json)?;
    if graph.revision != revision { return Err(Failure::new("E_NAME_NOT_FOUND", "requested revision does not exist")); }
    Ok(graph)
}

fn outcome(tool: &str, outcome: TransactionOutcome) -> Value {
    let mut response = envelope(tool, outcome.base_revision, outcome.result_revision,
        json!({"revision": outcome.result_revision, "graph_hash": outcome.graph_hash, "candidate": outcome.candidate}));
    response["ok"] = json!(outcome.ok);
    response["diagnostics"] = serde_json::to_value(outcome.diagnostics).unwrap();
    response
}

fn dispatch(options: &Options, context: &source::Context, input: &str, base: &mut u64) -> Result<Value, Failure> {
    let check = |graph: &Graph| il_checker::check_with_capabilities(graph, &options.host_policy.grants);
    let store = if options.store.join(".il/HEAD").exists() { Some(Store::open(&options.store)?) }
        else if options.store.join(".il").exists() && options.command != "transact" {
            return Err(Failure::new("E_STATE_INCONSISTENT", "store initialization is incomplete or HEAD is missing"));
        } else { None };
    if let Some(store) = &store {
        let provenance = store.provenance()?;
        source::verify_provenance(context, &provenance)?;
        *base = store.read_head()?.revision;
    } else {
        *base = context.seed["revision"].as_u64().ok_or_else(|| Failure::input("invalid seed revision"))?;
    }
    let current = *base;
    let tool = options.command.as_str();
    match tool {
        "state" => {
            let _: Empty = request(input)?;
            let graph = load(store.as_ref(), context, current)?;
            Ok(envelope(tool, current, current, json!({"head_revision": current, "initialized": store.is_some(),
                "graph_hash": graph.hash().map_err(Failure::json)?, "project_id": graph.project_id, "target": graph.target})))
        }
        "transact" => {
            let parsed = match transaction_request(input) {
                Err(error) if error.diagnostics.is_empty() => return Err(error),
                parsed => parsed,
            };
            let store = match store {
                Some(store) => store,
                None => {
                    let graph: Graph = serde_json::from_value(context.seed.clone()).map_err(Failure::json)?;
                    if graph != Graph::empty() { return Err(Failure::new("E_STATE_INCONSISTENT", "new application store requires the canonical empty seed")); }
                    match Store::initialize(&options.store, context.provenance.clone()) {
                        Ok(store) => store,
                        Err(error) if options.store.join(".il/HEAD").exists() => {
                            // Another first writer may have completed initialization while
                            // this process waited on the store's publication lock.
                            let opened = Store::open(&options.store)?;
                            let provenance = opened.provenance()?;
                            source::verify_provenance(context, &provenance).map_err(|_| Failure::from(error))?;
                            opened
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
            };
            match parsed {
                Ok(transaction) => Ok(outcome(tool, store.transact(&transaction, context.provenance.clone(), &check)?)),
                Err(error) => {
                    let request: Value = request(input)?;
                    let revision = request["base_revision"].as_u64().unwrap();
                    Ok(outcome(tool, store.retain_rejected_text(revision, &request, error.diagnostics)?))
                }
            }
        }
        "restore" => {
            let args: Restore = request(input)?;
            if args.reason.trim().is_empty() { return Err(Failure::input("restore reason cannot be empty")); }
            let store = store.ok_or_else(|| Failure::new("E_NAME_NOT_FOUND", "application store has not been initialized"))?;
            Ok(outcome(tool, store.restore(args.revision, &args.reason, context.provenance.clone(), &check)?))
        }
        "inspect" => {
            let args: Inspect = request(input)?;
            let revision = args.revision.unwrap_or(current);
            let graph = load(store.as_ref(), context, revision)?;
            let mut entity = if args.entity_id == "program" { json!(graph) } else {
                entity_index(&graph)?.remove(&args.entity_id)
                    .ok_or_else(|| Failure::new("E_NAME_NOT_FOUND", "entity not found at revision"))?
            };
            if let Some(fields) = args.fields {
                let object = entity.as_object().unwrap();
                let mut selected = serde_json::Map::new();
                for field in fields {
                    selected.insert(field.clone(), object.get(&field).ok_or_else(|| Failure::input("unknown entity field"))?.clone());
                }
                entity = Value::Object(selected);
            }
            Ok(envelope(tool, current, revision, bounded(json!({"entity": entity}), args.budget)?))
        }
        "validate" => {
            let args: Validate = request(input)?;
            if args.checks.iter().enumerate().any(|(index, check)| args.checks[..index].contains(check)) {
                return Err(Failure::input("checks must be unique"));
            }
            let graph = if let Some(revision) = args.graph_or_revision.as_u64() {
                load(store.as_ref(), context, revision)?
            } else { serde_json::from_value::<Graph>(args.graph_or_revision).map_err(Failure::json)? };
            let diagnostics = check(&graph);
            let mut response = envelope(tool, current, graph.revision,
                json!({"valid": diagnostics.is_empty(), "checks": ["schema", "names", "references", "types", "ownership", "effects", "capabilities", "contracts"]}));
            response["ok"] = json!(diagnostics.is_empty());
            response["diagnostics"] = serde_json::to_value(diagnostics).unwrap();
            Ok(response)
        }
        "schema-check" => {
            let args: SourceText = request(input)?;
            let graph = il_frontend::parse(&args.source, current).map_err(Failure::diagnostics)?;
            let diagnostics = check(&graph);
            if !diagnostics.is_empty() { return Err(Failure::diagnostics(diagnostics)); }
            let source = il_frontend::format(&graph).map_err(Failure::diagnostics)?;
            Ok(envelope(tool, current, current, json!({"graph": graph, "source": source})))
        }
        "test" => {
            let args: Test = request(input)?;
            if !["captured", "native_debug", "native_release"].contains(&args.isolation.as_str()) { return Err(Failure::input("unknown execution isolation")); }
            let graph = load(store.as_ref(), context, args.revision)?;
            let result = if args.isolation == "captured" { execution::run(&graph, args.suite, &options.host_policy)? }
                else { native::test(&graph, args.suite, &args.isolation, &options.store, context, &options.host_policy)? };
            let mut response = envelope(tool, current, args.revision, result);
            response["ok"] = json!(response["result"]["execution"]["status"] == "returned");
            response["diagnostics"] = response["result"]["execution"]["diagnostics"].clone();
            Ok(response)
        }
        "build" => {
            let args: Build = request(input)?;
            if args.target != TARGET { return Err(Failure::new("E_UNSUPPORTED_TARGET", "native compilation requires x86_64-unknown-linux-gnu")); }
            let graph = load(store.as_ref(), context, args.revision)?;
            let result = native::build(&graph, args.profile, args.runtime_profile, args.exports, &options.store, context, &options.host_policy)?;
            Ok(envelope(tool, current, args.revision, result))
        }
        "diff" => {
            let args: Diff = request(input)?;
            let left = entity_index(&load(store.as_ref(), context, args.base_revision)?)?;
            let right = entity_index(&load(store.as_ref(), context, args.target_revision)?)?;
            let ids: std::collections::BTreeSet<_> = left.keys().chain(right.keys()).collect();
            let (mut added, mut removed, mut modified) = (vec![], vec![], vec![]);
            for id in ids {
                if !args.scope.is_empty() && !args.scope.contains(id) { continue; }
                match (left.get(id), right.get(id)) {
                    (None, Some(_)) => added.push(id), (Some(_), None) => removed.push(id),
                    (Some(before), Some(after)) if before != after => modified.push(id), _ => {}
                }
            }
            Ok(envelope(tool, current, args.target_revision, json!({"added": added, "removed": removed,
                "modified": modified, "base_revision": args.base_revision, "target_revision": args.target_revision})))
        }
        "slice" => {
            let args: Slice = request(input)?;
            if args.max_nodes == 0 || args.max_nodes > 4096 || args.max_tokens == 0 || args.max_tokens > 65536 {
                return Err(Failure::input("slice budgets exceed supported limits"));
            }
            let revision = args.revision.unwrap_or(current);
            let entities = if let Some(store) = &store {
                let result = store.slice(&args.root_entities, revision, args.max_nodes, args.max_tokens)?;
                if result.truncated { return Err(Failure::new("E_CONTEXT_INSUFFICIENT", "slice exceeds requested context budget")); }
                if !result.missing.is_empty() { return Err(Failure::new("E_NAME_NOT_FOUND", "slice roots include unavailable entities")); }
                result.entities
            } else {
                load(None, context, revision)?;
                if !args.root_entities.is_empty() { return Err(Failure::new("E_NAME_NOT_FOUND", "empty graph has no entities")); }
                vec![]
            };
            Ok(envelope(tool, current, revision, bounded(json!({"node_count": entities.len(), "entities": entities, "revision": revision}), Some(args.max_tokens))?))
        }
        "callers" | "dependencies" => {
            let args: Relations = request(input)?;
            if tool == "callers" && args.direction.is_some() { return Err(Failure::input("callers does not accept direction")); }
            let direction = args.direction.as_deref().unwrap_or("forward");
            if !["forward", "reverse"].contains(&direction) { return Err(Failure::input("direction must be forward or reverse")); }
            let revision = args.revision.unwrap_or(current);
            let graph = load(store.as_ref(), context, revision)?;
            let index = entity_index(&graph)?;
            if !index.contains_key(&args.entity_id) { return Err(Failure::new("E_NAME_NOT_FOUND", "entity not found")); }
            let mut ids = if tool == "callers" {
                graph.functions.iter().filter(|function| function.blocks.iter().any(|block|
                    block.operations.iter().chain(std::iter::once(&block.terminator)).any(|op|
                        matches!(&op.attributes, Attributes::Call { callee } if *callee == args.entity_id))))
                    .map(|function| function.entity_id.clone()).collect()
            }
                else if direction == "forward" { entity_references(&graph, &args.entity_id) }
                else { index.keys().filter(|id| entity_references(&graph, id).contains(&args.entity_id)).cloned().collect() };
            ids.sort(); ids.dedup();
            Ok(envelope(tool, current, revision, bounded(json!({"entity_ids": ids, "revision": revision}), args.budget)?))
        }
        _ => Err(Failure::new("E_UNSUPPORTED_FEATURE", "command is not available in this tool version")),
    }
}

fn main() {
    let mut tool = "unknown".to_owned();
    let mut base_revision = 0;
    let result = (|| -> Result<Value, Failure> {
        let (mut options, policy_path) = options()?;
        tool = options.command.clone();
        if let Some(path) = policy_path {
            let mut bytes = Vec::new();
            std::fs::File::open(path).map_err(Failure::io)?.take(65_537).read_to_end(&mut bytes).map_err(Failure::io)?;
            options.host_policy = il_runtime_startup::HostPolicy::parse(&bytes).map_err(|error| Failure::new(error.code, &error.message))?;
        }
        let context = source::verify(&options.repository)?;
        let mut input = String::new();
        io::stdin().take((INPUT_LIMIT + 1) as u64).read_to_string(&mut input).map_err(Failure::io)?;
        if input.len() > INPUT_LIMIT { return Err(Failure::input("JSON request exceeds 262144 bytes")); }
        dispatch(&options, &context, &input, &mut base_revision)
    })();
    let mut response = match result {
        Ok(value) => value,
        Err(error) => failed(&tool, base_revision, &error),
    };
    let mut encoded = serde_json::to_string(&response).unwrap();
    if encoded.len() > OUTPUT_LIMIT {
        response = failed(&tool, base_revision, &Failure::new("E_CONTEXT_INSUFFICIENT", "response exceeds 262144 bytes"));
        encoded = serde_json::to_string(&response).unwrap();
    }
    println!("{encoded}");
    if response["ok"] != true { std::process::exit(1); }
}
