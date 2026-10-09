use il_execution_model::{ExecutionStatus, Limits, Value, ValueData, VariantValue};
use il_frontend::{format, parse};
use il_graph::*;
use il_runtime_startup::HostPolicy;
use std::sync::OnceLock;

fn source() -> String {
    [include_str!("../../packages/core/lib.il"), include_str!("../../packages/alloc/lib.il"),
        include_str!("../../packages/io/lib.il"), include_str!("../../packages/time/lib.il"),
        include_str!("../../packages/net/lib.il"), include_str!("../../packages/json/lib.il"),
        include_str!("../../packages/http/lib.il"), include_str!("../../packages/test/lib.il"),
        include_str!("../../packages/tracing/lib.il"), include_str!("../../examples/http_demo/main.il")].join("\n")
}

fn graph() -> Graph {
    static GRAPH: OnceLock<Graph> = OnceLock::new();
    GRAPH.get_or_init(|| {
        let graph = parse(&source(), 0).unwrap_or_else(|d| panic!("HTTP source parse failed: {d:#?}"));
        let diagnostics = check(&graph);
        assert!(diagnostics.is_empty(), "HTTP static check failed: {diagnostics:#?}");
        graph
    }).clone()
}

fn check(graph: &Graph) -> Vec<Diagnostic> { il_checker::check_with_capabilities(graph, &graph.capabilities) }
fn dispatcher(graph: &Graph) -> &Function { graph.functions.iter().find(|f| f.entity_id == "demo.dispatch").unwrap() }
fn routes(graph: &mut Graph) -> &mut Vec<HttpRoute> {
    let contract = graph.contracts.iter_mut().find(|c| c.entity_id == "demo.server").unwrap();
    let ContractPredicate::HttpServer { routes, .. } = &mut contract.predicates[0] else { panic!("HTTP contract required") }; routes
}

fn variant(ty: &str, tag: &str, fields: Vec<Value>) -> Value {
    Value { type_ref: ty.into(), data: ValueData::Variant(VariantValue { tag: tag.into(), fields }) }
}
fn bytes(value: &str) -> Value { Value { type_ref: "Bytes".into(), data: ValueData::Bytes(value.as_bytes().to_vec()) } }
fn response(content_type: &str, body: &str) -> Value {
    variant("http.ResponseResult", "Ok", vec![variant("http.Response", "Response", vec![Value::integer("U16", 200), Value::string(content_type), bytes(body)])])
}
fn protocol(status: u16) -> Value {
    variant("http.ResponseResult", "Err", vec![variant("http.HttpError", "Protocol", vec![Value::integer("U16", status.into())])])
}

fn lower(graph: &Graph) -> il_mir::Program {
    let hir = il_hir::lower_with_capabilities(graph, &graph.capabilities).unwrap_or_else(|d| panic!("HIR: {d:#?}"));
    il_mir::lower_with_capabilities(&hir, &graph.capabilities).unwrap_or_else(|d| panic!("MIR: {d:#?}"))
}

fn dispatch(program: &il_mir::Program, method: &str, path: &str) -> Value {
    let policy = HostPolicy { grants: program.capabilities.clone(), ..HostPolicy::empty() };
    let execution = il_interpreter::execute_with_policy(program, "demo.dispatch", &[bytes(method), bytes(path)], Limits::default(), &policy);
    assert_eq!(execution.status, ExecutionStatus::Returned, "{execution:#?}");
    assert!(execution.diagnostics.is_empty());
    fn returned_allocations(value: &Value) -> u64 {
        match &value.data {
            ValueData::String(_) | ValueData::Bytes(_) => 1,
            ValueData::Record(fields) | ValueData::Tuple(fields) => fields.iter().map(returned_allocations).sum(),
            ValueData::Variant(value) => value.fields.iter().map(returned_allocations).sum(),
            _ => 0,
        }
    }
    assert_eq!(execution.live_allocations, returned_allocations(execution.value.as_ref().unwrap()), "only returned buffers may remain live in the execution snapshot");
    assert_eq!(execution.live_handles, 0);
    assert!(execution.handle_events.is_empty(), "pure dispatch must not open sockets");
    execution.value.unwrap()
}

#[test]
fn bodyless_dispatcher_elaborates_and_canonical_text_roundtrips() {
    let source = source();
    assert!(source.contains("contracts [demo.server];"), "demo must use the bodyless implementation contract");
    let graph = graph();
    assert!(dispatcher(&graph).blocks.len() > 1);
    let formatted = format(&graph).unwrap();
    assert_eq!(parse(&formatted, 0).unwrap_or_else(|d| panic!("canonical text ({} bytes): {d:#?}", formatted.len())), graph);
    assert_eq!(format(&parse(&formatted, 0).unwrap()).unwrap(), formatted);
    for source in ["module app { fn f()->I32; }", "module app { fn f()->I32 contracts[missing]; }"] {
        assert!(parse(source, 0).unwrap_err().iter().any(|d| d.code == "E_MISSING_RETURN"));
    }
}

#[test]
fn dispatcher_executes_real_handlers_and_protocol_failures_without_network() {
    let program = lower(&graph());
    for (method, path, expected) in [
        ("GET", "/health", response("text/plain; charset=utf-8", "ok")),
        ("GET", "/hello/世界", response("application/json", "{\"message\":\"Hello, 世界\"}")),
        ("GET", "/hello/\"x\\", response("application/json", "{\"message\":\"Hello, \\\"x\\\\\"}")),
        ("GET", "/missing", protocol(404)),
        ("POST", "/health", protocol(405)),
        ("POST", "/missing", protocol(404)),
    ] { assert_eq!(dispatch(&program, method, path), expected, "{method} {path}"); }
}

#[test]
fn declaration_and_raw_graph_tampering_fail_shared_checks() {
    let graph = graph();
    let mut changed = graph.clone();
    changed.functions.iter_mut().find(|f| f.entity_id == "demo.dispatch").unwrap().blocks[0].operations[1].attributes = Attributes::Constant { value: Literal::Bytes(b"/forged".to_vec()) };
    assert!(check(&changed).iter().any(|d| d.code == "E_HTTP_DECLARATION"));
    let mut changed = graph.clone();
    routes(&mut changed)[0].path = "/hello/world".into();
    assert!(http::elaborate(&mut changed).iter().any(|d| d.code == "E_ROUTE_COLLISION"));
    let mut changed = graph.clone();
    routes(&mut changed)[0].handler = "demo.hello".into();
    assert!(http::elaborate(&mut changed).iter().any(|d| d.code == "E_TYPE_MISMATCH"));
    let mut changed = graph.clone();
    changed.capabilities.iter_mut().find(|c| c.entity_id == "demo.listen").unwrap().scope = Some("127.0.0.1:8081".into());
    assert!(check(&changed).iter().any(|d| d.code == "E_CAPABILITY_MISSING"));
    let mut changed = graph.clone();
    changed.functions.iter_mut().find(|f| f.entity_id == "demo.dispatch").unwrap().effects.clear();
    assert!(check(&changed).iter().any(|d| d.code == "E_EFFECT_UNDECLARED"));
    assert!(il_checker::check(&graph).iter().any(|d| d.code == "E_CAPABILITY_MISSING"));
}

fn provenance() -> Provenance { Provenance { source_git_commit: "a".repeat(40), source_tree_hash: "b".repeat(40) } }
fn replace(store: &Store, graph: Graph, task: &str) -> TransactionOutcome {
    store.transact(&Transaction { task_id: task.into(), base_revision: graph.revision, scope: vec!["program".into()],
        operations: vec![TransactionOperation::ReplaceProgram { graph }], required_checks: vec![Check::Contracts, Check::Ownership] }, provenance(), &check).unwrap()
}

#[test]
fn graph_route_transactions_publish_new_revisions_and_restore_old_behavior() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::initialize(directory.path(), provenance()).unwrap();
    let imported = replace(&store, graph(), "import");
    assert!(imported.ok, "{:?}", imported.diagnostics);
    let original = store.load(imported.result_revision).unwrap();
    let mut candidate = original.clone();
    routes(&mut candidate).push(HttpRoute { entity_id: "demo.route.ready".into(), method: "GET".into(), path: "/ready".into(), handler: "demo.health".into(), parameters: vec![] });
    let added = replace(&store, candidate, "add_route");
    assert!(added.ok, "{:?}", added.diagnostics);
    assert!(added.result_revision > original.revision);
    assert_eq!(store.inspect("demo.route.ready", added.result_revision).unwrap()["path"], "/ready");
    let added_graph = store.load(added.result_revision).unwrap();
    assert_ne!(dispatcher(&added_graph), dispatcher(&original));
    assert_eq!(dispatch(&lower(&added_graph), "GET", "/ready"), response("text/plain; charset=utf-8", "ok"));

    let mut candidate = added_graph;
    routes(&mut candidate).iter_mut().find(|r| r.entity_id == "demo.route.ready").unwrap().path = "/ready-now".into();
    let renamed = replace(&store, candidate, "rename_route");
    assert!(renamed.ok, "{:?}", renamed.diagnostics);
    let renamed_graph = store.load(renamed.result_revision).unwrap();
    let program = lower(&renamed_graph);
    assert_eq!(dispatch(&program, "GET", "/ready"), protocol(404));
    assert_eq!(dispatch(&program, "GET", "/ready-now"), response("text/plain; charset=utf-8", "ok"));
    let changes = store.diff(added.result_revision, renamed.result_revision, &["demo.route.ready".into(), "demo.dispatch".into()]).unwrap();
    assert_eq!(changes.len(), 2);

    let mut candidate = renamed_graph;
    routes(&mut candidate).retain(|r| r.entity_id != "demo.route.ready");
    let removed = replace(&store, candidate, "remove_route");
    assert!(removed.ok, "{:?}", removed.diagnostics);
    assert!(store.inspect("demo.route.ready", removed.result_revision).is_err());
    assert_eq!(dispatch(&lower(&store.load(removed.result_revision).unwrap()), "GET", "/ready-now"), protocol(404));

    let restored = store.restore(added.result_revision, "restore_added_route", provenance(), &check).unwrap();
    assert!(restored.ok, "{:?}", restored.diagnostics);
    assert!(restored.result_revision > removed.result_revision);
    let restored_graph = store.load(restored.result_revision).unwrap();
    assert_eq!(store.inspect("demo.route.ready", restored.result_revision).unwrap()["path"], "/ready");
    assert_eq!(dispatch(&lower(&restored_graph), "GET", "/ready"), response("text/plain; charset=utf-8", "ok"));
    assert_eq!(store.load(original.revision).unwrap(), original, "historical snapshot must remain immutable");

    let mut invalid = restored_graph;
    routes(&mut invalid)[0].path = "/hello/collision".into();
    let rejected = replace(&store, invalid, "reject_collision");
    assert!(!rejected.ok);
    assert_eq!(rejected.result_revision, restored.result_revision);
    assert!(rejected.candidate.is_some());
    assert_eq!(store.read_head().unwrap().revision, restored.result_revision);
}
