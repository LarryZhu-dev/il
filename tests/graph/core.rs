use il_graph::*;
use std::fs;
use std::sync::{Arc, Barrier};

mod query;

fn provenance() -> Provenance { Provenance { source_git_commit: "a".repeat(40), source_tree_hash: "b".repeat(40) } }
fn module(id: &str) -> Module {
    Module { entity_id: id.into(), path: id.into(), imports: vec![], declarations: vec![], visibility: Visibility::Private }
}
fn request(base: u64, id: &str) -> Transaction {
    Transaction { task_id: "P02-test".into(), base_revision: base, scope: vec![id.into()],
        operations: vec![TransactionOperation::AddModule { module: module(id) }], required_checks: vec![Check::Schema, Check::Names] }
}
fn initialized() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::initialize(directory.path(), provenance()).unwrap();
    (directory, store)
}
fn rejected(outcome: TransactionOutcome, code: &str, head: u64) {
    assert!(!outcome.ok);
    assert_eq!(outcome.result_revision, head);
    assert!(outcome.diagnostics.iter().any(|diagnostic| diagnostic.code == code), "{:?}", outcome.diagnostics);
    assert!(outcome.candidate.is_some());
}

#[test]
fn canonical_graph_matches_bootstrap_fixture_byte_for_byte() {
    let expected = include_bytes!("../../examples/bootstrap/graph.json");
    assert_eq!(Graph::empty().canonical_bytes().unwrap(), expected);
    let parsed = Graph::parse(expected).unwrap();
    assert_eq!(parsed.canonical_bytes().unwrap(), expected);
    assert_eq!(parsed.hash().unwrap(), hash_bytes(expected));
}

#[test]
fn unknown_graph_and_nested_module_fields_are_rejected() {
    let mut graph = serde_json::to_value(Graph::empty()).unwrap();
    graph["unreviewed"] = true.into();
    assert!(serde_json::from_value::<Graph>(graph).is_err());
    let raw = r#"{"entity_id":"app","path":"app","imports":[],"declarations":[],"visibility":"private","anything":true}"#;
    assert!(serde_json::from_str::<Module>(raw).is_err());
}

#[test]
fn optional_and_nullable_fields_follow_the_closed_graph_schema() {
    let record = r#"{"entity_id":"Empty","kind":"record","parameters":[],"layout":"inferred"}"#;
    assert!(serde_json::from_str::<TypeDef>(record).is_ok());
    let mut explicit_null: serde_json::Value = serde_json::from_str(record).unwrap();
    explicit_null["integer"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<TypeDef>(explicit_null).is_err());
    let integer = r#"{"entity_id":"Byte","kind":"int","parameters":[],"layout":"inferred","integer":{"signed":false,"bits":8}}"#;
    assert!(serde_json::from_str::<TypeDef>(integer).is_ok());
    let clock = r#"{"entity_id":"clock","kind":"ClockRead","scope":null}"#;
    assert!(serde_json::from_str::<Capability>(clock).is_ok());
    let missing_scope = r#"{"entity_id":"clock","kind":"ClockRead"}"#;
    assert!(serde_json::from_str::<Capability>(missing_scope).is_err());
    let listen = r#"{"entity_id":"listener","kind":"Listen","scope":"127.0.0.1:8080"}"#;
    assert!(serde_json::from_str::<Capability>(listen).is_ok());
}

#[test]
fn graph_schema_string_and_unique_array_constraints_are_enforced() {
    let base: Graph = serde_json::from_slice(include_bytes!("../fixtures/semantics/valid_typed_call.json")).unwrap();
    assert!(base.validate_structural().is_empty());
    let mut bad_parameter = base.clone();
    bad_parameter.functions[0].parameters[0].name = "bad name".into();
    let mut bad_package = base.clone();
    bad_package.packages.push(Package { entity_id: "pkg".into(), name: "".into(), version: "".into(), modules: vec![], effects: vec![], capabilities: vec![] });
    let mut duplicate_package = base.clone();
    duplicate_package.packages.push(Package { entity_id: "pkg".into(), name: "valid".into(), version: "1.0.0".into(), modules: vec!["app".into(), "app".into()], effects: vec![], capabilities: vec![] });
    let mut duplicate_effect = base.clone();
    duplicate_effect.functions[0].effects = vec![Effect::Alloc, Effect::Alloc];
    let mut bad_trap = base.clone();
    let terminator = &mut bad_trap.functions[0].blocks[0].terminator;
    terminator.opcode = Opcode::Trap;
    terminator.inputs.clear();
    terminator.attributes = Attributes::Trap { code: "".into() };
    for graph in [bad_parameter, bad_package, duplicate_package, duplicate_effect, bad_trap] {
        let diagnostics = graph.validate_structural();
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == "E_SCHEMA_INVALID"), "{diagnostics:?}");
    }
}

#[test]
fn unknown_opcode_and_wrong_attributes_are_rejected() {
    let raw = r#"{"entity_id":"op","opcode":"eval","inputs":[],"outputs":[],"attributes":{},"effects":[],"consumes":[],"produces":[]}"#;
    assert!(serde_json::from_str::<Operation>(raw).is_err());
    assert!(serde_json::from_str::<Operation>(&raw.replace("eval", "return").replace("\"attributes\":{}", "\"attributes\":{\"shell\":\"x\"}")).is_err());
    let operation: Operation = serde_json::from_str(&raw.replace("eval", "const")).unwrap();
    assert!(!operation.attributes_match());
}

#[test]
fn duplicate_and_dangling_entities_are_diagnosed() {
    let mut graph = Graph::empty();
    graph.modules = vec![module("app"), module("app")];
    assert!(graph.validate_structural().iter().any(|d| d.code == "E_DUPLICATE_NAME"));
    graph.modules.pop();
    graph.modules[0].imports.push("absent".into());
    assert!(graph.validate_structural().iter().any(|d| d.code == "E_NAME_NOT_FOUND"));
}

#[test]
fn valid_transaction_publishes_complete_immutable_revision() {
    let (directory, store) = initialized();
    let initial = fs::read(directory.path().join(".il/revisions/00000000000000000000/graph.json")).unwrap();
    let outcome = store.transact(&request(0, "app"), provenance(), &Graph::validate_structural).unwrap();
    assert!(outcome.ok);
    assert_eq!(outcome.result_revision, 1);
    assert_eq!(store.load(1).unwrap().modules[0].entity_id, "app");
    assert_eq!(fs::read(directory.path().join(".il/revisions/00000000000000000000/graph.json")).unwrap(), initial);
    assert_eq!(store.read_head().unwrap().graph_hash, store.load(1).unwrap().hash().unwrap());
}

#[test]
fn stale_transaction_preserves_head_and_retains_candidate() {
    let (directory, store) = initialized();
    store.transact(&request(0, "first"), provenance(), &Graph::validate_structural).unwrap();
    let head = store.read_head().unwrap();
    let outcome = store.transact(&request(0, "second"), provenance(), &Graph::validate_structural).unwrap();
    let path = outcome.candidate.clone().unwrap();
    rejected(outcome, "E_STALE_REVISION", 1);
    assert!(directory.path().join(path).join("diagnostics.json").is_file());
    assert_eq!(store.read_head().unwrap(), head);
}

#[test]
fn scope_is_exact_and_cannot_disable_validation() {
    let (_directory, store) = initialized();
    let mut transaction = request(0, "app");
    transaction.scope = vec!["another".into()];
    transaction.required_checks.clear();
    rejected(store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap(), "E_INVALID_SCOPE", 0);
    transaction.scope = vec!["app".into()];
    if let TransactionOperation::AddModule { module } = &mut transaction.operations[0] { module.imports.push("absent".into()); }
    rejected(store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap(), "E_NAME_NOT_FOUND", 0);
}

#[test]
fn duplicate_checks_fail_schema_without_publishing() {
    let (_directory, store) = initialized();
    let mut transaction = request(0, "app");
    transaction.required_checks.push(Check::Schema);
    rejected(store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap(), "E_SCHEMA_INVALID", 0);
    assert_eq!(store.read_head().unwrap().revision, 0);
}

#[test]
fn interrupted_experiment_is_completed_without_replacing_conflicting_bytes() {
    let (directory, store) = initialized();
    let mut transaction = request(0, "app");
    transaction.scope = vec!["other".into()];
    let first = store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap();
    let experiment = directory.path().join(first.candidate.unwrap());
    fs::remove_file(experiment.join("transaction.json")).unwrap();
    fs::remove_file(experiment.join("diagnostics.json")).unwrap();
    rejected(store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap(), "E_INVALID_SCOPE", 0);
    assert!(experiment.join("transaction.json").is_file());
    assert!(experiment.join("diagnostics.json").is_file());
    fs::write(experiment.join("graph.json"), b"tampered").unwrap();
    assert_eq!(store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap_err().code, "E_STATE_INCONSISTENT");
    assert_eq!(store.read_head().unwrap().revision, 0);
}

#[test]
fn failed_late_operation_never_partially_commits() {
    let (_directory, store) = initialized();
    let mut transaction = request(0, "app");
    transaction.scope.push("missing".into());
    transaction.operations.push(TransactionOperation::RenameModule { entity_id: "missing".into(), path: "other".into() });
    rejected(store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap(), "E_NAME_NOT_FOUND", 0);
    assert!(store.load(0).unwrap().modules.is_empty());
}

#[test]
fn duplicate_id_transaction_never_changes_head() {
    let (_directory, store) = initialized();
    store.transact(&request(0, "app"), provenance(), &Graph::validate_structural).unwrap();
    rejected(store.transact(&request(1, "app"), provenance(), &Graph::validate_structural).unwrap(), "E_DUPLICATE_NAME", 1);
}

#[test]
fn semantic_validation_is_mandatory_and_cannot_be_disabled_by_checks() {
    let (_directory, store) = initialized();
    let transaction = request(0, "app");
    let reject = |graph: &Graph| vec![Diagnostic::error("E_TYPE_MISMATCH", Some("app"), "injected mandatory semantic rejection", graph.revision)];
    rejected(store.transact(&transaction, provenance(), &reject).unwrap(), "E_TYPE_MISMATCH", 0);
    assert_eq!(store.read_head().unwrap().revision, 0);
    let mut requested = transaction.clone();
    requested.required_checks.push(Check::Types);
    assert!(store.transact(&requested, provenance(), &Graph::validate_structural).unwrap().ok);
    let mut invalid = request(1, "bad");
    if let TransactionOperation::AddModule { module } = &mut invalid.operations[0] { module.declarations.push("absent".into()); }
    rejected(store.transact(&invalid, provenance(), &Graph::validate_structural).unwrap(), "E_NAME_NOT_FOUND", 1);
    assert!(!store.restore(0, "recheck historical graph", provenance(), &reject).unwrap().ok);
    assert_eq!(store.read_head().unwrap().revision, 1);
}

#[test]
fn remove_with_inbound_import_fails_without_cascade() {
    let (_directory, store) = initialized();
    let mut transaction = request(0, "app");
    let mut importing = module("consumer");
    importing.imports.push("app".into());
    transaction.scope.push("consumer".into());
    transaction.operations.push(TransactionOperation::AddModule { module: importing });
    assert!(store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap().ok);
    transaction.base_revision = 1;
    transaction.operations = vec![TransactionOperation::RemoveModule { entity_id: "app".into() }];
    rejected(store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap(), "E_NAME_NOT_FOUND", 1);
    transaction.operations.insert(0, TransactionOperation::SetModuleImports { entity_id: "consumer".into(), imports: vec![] });
    assert!(store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap().ok);
    assert_eq!(store.load(2).unwrap().modules.len(), 1);
}

#[test]
fn rename_preserves_identity_and_diff_detects_modification() {
    let (_directory, store) = initialized();
    store.transact(&request(0, "app"), provenance(), &Graph::validate_structural).unwrap();
    let mut transaction = request(1, "app");
    transaction.operations = vec![TransactionOperation::RenameModule { entity_id: "app".into(), path: "renamed".into() }];
    assert!(store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap().ok);
    let module = store.inspect("app", 2).unwrap();
    assert_eq!(module["path"], "renamed");
    assert_eq!(module["entity_id"], "app");
    let changes = store.diff(1, 2, &[]).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].change, "modified");
}

#[test]
fn restore_always_creates_new_revision_without_erasing_history() {
    let (_directory, store) = initialized();
    store.transact(&request(0, "app"), provenance(), &Graph::validate_structural).unwrap();
    assert_eq!(store.restore(0, "restore initial graph", provenance(), &Graph::validate_structural).unwrap().result_revision, 2);
    assert!(store.load(2).unwrap().modules.is_empty());
    assert_eq!(store.load(1).unwrap().modules.len(), 1);
    assert_eq!(store.restore(2, "restore identical content", provenance(), &Graph::validate_structural).unwrap().result_revision, 3);
}

#[test]
fn each_interruption_point_preserves_previous_visible_head() {
    for fault in [FaultPoint::BeforeSnapshot, FaultPoint::AfterSnapshot, FaultPoint::BeforeHeadPublication] {
        let (directory, store) = initialized();
        let head = store.read_head().unwrap();
        assert!(store.transact_with_fault(&request(0, "failed"), provenance(), &Graph::validate_structural, Some(fault)).is_err());
        let reopened = Store::open(directory.path()).unwrap();
        assert_eq!(reopened.read_head().unwrap(), head);
        assert!(reopened.load(1).is_err());
        let completed = reopened.transact(&request(0, "success"), provenance(), &Graph::validate_structural).unwrap();
        assert!(completed.ok);
        assert_eq!(completed.result_revision, if fault == FaultPoint::BeforeSnapshot { 1 } else { 2 });
        assert_eq!(reopened.load(completed.result_revision).unwrap().modules[0].entity_id, "success");
    }
}

#[test]
fn concurrent_same_base_writers_have_one_winner() {
    let (_directory, store) = initialized();
    let barrier = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8).map(|index| {
        let store = store.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || { barrier.wait(); store.transact(&request(0, &format!("module_{index}")), provenance(), &Graph::validate_structural).unwrap() })
    }).collect();
    let outcomes: Vec<_> = workers.into_iter().map(|thread| thread.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|outcome| outcome.ok).count(), 1);
    for outcome in outcomes.into_iter().filter(|outcome| !outcome.ok) { rejected(outcome, "E_STALE_REVISION", 1); }
    assert_eq!(store.load(1).unwrap().modules.len(), 1);
}

#[test]
fn graph_and_ancestor_tampering_are_detected() {
    let (directory, store) = initialized();
    store.transact(&request(0, "app"), provenance(), &Graph::validate_structural).unwrap();
    let graph_path = directory.path().join(".il/revisions/00000000000000000000/graph.json");
    fs::write(&graph_path, b"{}").unwrap();
    assert_eq!(store.read_head().unwrap_err().code, "E_STATE_INCONSISTENT");
    let manifest_path = graph_path.with_file_name("manifest.json");
    let mut manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest.graph_hash = hash_bytes(b"{}");
    fs::write(manifest_path, canonical_bytes(&manifest).unwrap()).unwrap();
    assert_eq!(store.read_head().unwrap_err().code, "E_STATE_INCONSISTENT");
}

#[test]
fn interrupted_initialization_resumes_only_matching_missing_files() {
    let (directory, _store) = initialized();
    fs::remove_file(directory.path().join(".il/HEAD")).unwrap();
    fs::remove_file(directory.path().join(".il/revisions/00000000000000000000/manifest.json")).unwrap();
    let recovered = Store::initialize(directory.path(), provenance()).unwrap();
    assert_eq!(recovered.read_head().unwrap().revision, 0);
    assert_eq!(recovered.load(0).unwrap(), Graph::empty());
    fs::remove_file(directory.path().join(".il/HEAD")).unwrap();
    let other = Provenance { source_git_commit: "c".repeat(40), source_tree_hash: "d".repeat(40) };
    assert_eq!(Store::initialize(directory.path(), other).unwrap_err().code, "E_STATE_INCONSISTENT");
    assert!(!directory.path().join(".il/HEAD").exists());
}

#[test]
fn post_publication_failure_reports_committed_revision_without_rollback() {
    let (_directory, store) = initialized();
    let error = store.transact_with_fault(&request(0, "app"), provenance(), &Graph::validate_structural, Some(FaultPoint::AfterHeadPublication)).unwrap_err();
    assert_eq!(error.code, "E_COMMIT_DURABILITY_UNCERTAIN");
    assert_eq!(error.committed_revision, Some(1));
    assert_eq!(store.read_head().unwrap().revision, 1);
    assert_eq!(store.load(1).unwrap().modules[0].entity_id, "app");
    assert!(store.load(0).unwrap().modules.is_empty());
    assert!(!Diagnostic::error(&error.code, None, &error.message, 1).retryable);
}

#[test]
fn each_new_snapshot_uses_current_source_provenance_preserving_ancestry() {
    let (directory, store) = initialized();
    let next = Provenance { source_git_commit: "c".repeat(40), source_tree_hash: "d".repeat(40) };
    assert!(store.transact(&request(0, "app"), next.clone(), &Graph::validate_structural).unwrap().ok);
    assert_eq!(store.provenance().unwrap(), next);
    let initial: Manifest = serde_json::from_slice(&fs::read(directory.path().join(".il/revisions/00000000000000000000/manifest.json")).unwrap()).unwrap();
    assert_eq!(initial.provenance, provenance());
    store.restore(0, "restore contents using current toolchain", provenance(), &Graph::validate_structural).unwrap();
    assert_eq!(store.provenance().unwrap(), provenance());
    assert_eq!(store.load(1).unwrap().modules[0].entity_id, "app");
}

#[test]
fn slice_respects_limits_and_follows_direct_imports() {
    let (_directory, store) = initialized();
    let mut transaction = request(0, "library");
    let mut app = module("app");
    app.imports.push("library".into());
    transaction.scope.push("app".into());
    transaction.operations.push(TransactionOperation::AddModule { module: app });
    assert!(store.transact(&transaction, provenance(), &Graph::validate_structural).unwrap().ok);
    let full = store.slice(&["app".into()], 1, 10, 10000).unwrap();
    assert_eq!(full.entities.len(), 2);
    assert!(!full.truncated);
    let limited = store.slice(&["app".into()], 1, 1, 10000).unwrap();
    assert_eq!(limited.entities.len(), 1);
    assert!(limited.truncated);
    let tiny = store.slice(&["app".into()], 1, 10, 1).unwrap();
    assert!(tiny.entities.is_empty());
    assert!(tiny.truncated);
}
