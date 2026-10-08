use il_graph::*;
use il_mir::{Terminator, DropAction};
use std::path::Path;

fn operation(id: &str, opcode: Opcode, inputs: &[&str], outputs: &[(&str, &str)], attributes: Attributes) -> Operation {
    Operation { entity_id: id.into(), opcode, inputs: inputs.iter().map(|id| (*id).into()).collect(), outputs: outputs.iter().map(|(id, ty)| ValueDef { entity_id: (*id).into(), type_ref: (*ty).into() }).collect(), attributes, effects: vec![], consumes: vec![], produces: vec![] }
}

fn cleanup_graph() -> Graph {
    let mut constant = operation("main.constant", Opcode::Const, &[], &[("main.local", "String")], Attributes::Constant { value: Literal::String("local".into()) });
    constant.effects = vec![Effect::Alloc];
    constant.produces = constant.outputs.clone();
    let mut graph = Graph::empty();
    graph.modules = vec![Module { entity_id: "app".into(), path: "app".into(), imports: vec![], declarations: vec!["main".into()], visibility: Visibility::Public }];
    graph.functions = vec![Function { entity_id: "main".into(), name: "main".into(), parameters: vec![
        Parameter { entity_id: "main.first".into(), name: "first".into(), type_ref: "String".into() },
        Parameter { entity_id: "main.second".into(), name: "second".into(), type_ref: "String".into() },
    ], result: "Unit".into(), effects: vec![Effect::Alloc], capabilities: vec![], contracts: vec![], blocks: vec![Block {
        entity_id: "main.entry".into(), arguments: vec![], operations: vec![constant], terminator: operation("main.return", Opcode::Return, &[], &[], Attributes::Empty {}),
    }] }];
    graph
}

fn lower(graph: &Graph) -> il_mir::Program { il_mir::lower(&il_hir::lower(graph).unwrap()).unwrap() }

fn cleanup(program: &mut il_mir::Program) -> &mut Vec<DropAction> {
    match &mut program.functions[0].blocks[0].terminator { Terminator::Return { cleanup, .. } => cleanup, _ => panic!("expected return") }
}

fn assert_code(program: &il_mir::Program, code: &str) {
    let diagnostics = il_mir::verify(program);
    assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == code), "expected {code}, got {diagnostics:?}");
}

#[test]
fn positive_independent_semantic_fixtures_lower_to_verified_mir() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/semantics");
    let cases: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("cases.json")).unwrap()).unwrap();
    for case in cases["cases"].as_array().unwrap().iter().filter(|case| case["valid"] == true) {
        let graph = Graph::parse(&std::fs::read(root.join(case["file"].as_str().unwrap())).unwrap()).unwrap();
        let hir = il_hir::lower(&graph).unwrap();
        let mir = il_mir::lower(&hir).unwrap_or_else(|diagnostics| panic!("{}: {diagnostics:?}", case["id"]));
        assert!(il_mir::verify(&mir).is_empty());
    }
}

#[test]
fn cleanup_is_complete_in_reverse_initialization_order() {
    let mut mir = lower(&cleanup_graph());
    let ids: Vec<_> = cleanup(&mut mir).iter().map(|action| action.value_id.as_str()).collect();
    assert_eq!(ids, ["main.local", "main.second", "main.first"]);
}

#[test]
fn missing_drop_is_rejected_without_trusting_source_graph_or_metadata() {
    let mut mir = lower(&cleanup_graph());
    cleanup(&mut mir).pop();
    assert_code(&mir, "E_MIR_MISSING_DROP");
}

#[test]
fn double_cleanup_and_wrong_order_are_rejected() {
    let mut mir = lower(&cleanup_graph());
    let duplicate = cleanup(&mut mir)[0].clone();
    cleanup(&mut mir).push(duplicate);
    assert_code(&mir, "E_DOUBLE_DROP");
    let mut mir = lower(&cleanup_graph());
    cleanup(&mut mir).swap(0, 1);
    assert_code(&mir, "E_MIR_CLEANUP_ORDER");
}

#[test]
fn wrong_cleanup_type_and_unknown_cleanup_value_are_rejected() {
    let mut mir = lower(&cleanup_graph());
    cleanup(&mut mir)[0].type_ref = "Bytes".into();
    assert_code(&mir, "E_TYPE_MISMATCH");
    let mut mir = lower(&cleanup_graph());
    cleanup(&mut mir).push(DropAction { value_id: "unknown".into(), type_ref: "String".into() });
    assert_code(&mir, "E_MIR_INVALID_CLEANUP");
}

#[test]
fn returned_owner_is_transferred_before_cleaning_remaining_locals() {
    let mut graph = cleanup_graph();
    graph.functions[0].result = "String".into();
    let terminator = &mut graph.functions[0].blocks[0].terminator;
    terminator.inputs = vec!["main.local".into()];
    terminator.consumes = terminator.inputs.clone();
    let mut mir = lower(&graph);
    assert_eq!(cleanup(&mut mir).len(), 2);
    cleanup(&mut mir).insert(0, DropAction { value_id: "main.local".into(), type_ref: "String".into() });
    assert_code(&mir, "E_MIR_INVALID_CLEANUP");
}

#[test]
fn explicit_drop_is_not_repeated_by_implicit_cleanup() {
    let mut graph = cleanup_graph();
    let mut drop = operation("main.drop", Opcode::Drop, &["main.local"], &[], Attributes::Empty {});
    drop.consumes = vec!["main.local".into()];
    graph.functions[0].blocks[0].operations.push(drop);
    let mut mir = lower(&graph);
    assert_eq!(cleanup(&mut mir).len(), 2);
    cleanup(&mut mir).push(DropAction { value_id: "main.local".into(), type_ref: "String".into() });
    assert_code(&mir, "E_DOUBLE_DROP");
}

#[test]
fn traps_do_not_insert_unwinding_cleanup() {
    let mut graph = cleanup_graph();
    graph.functions[0].blocks[0].terminator = operation("main.trap", Opcode::Trap, &[], &[], Attributes::Trap { code: "E_RESOURCE_LIMIT".into() });
    let mir = lower(&graph);
    assert!(matches!(mir.functions[0].blocks[0].terminator, Terminator::Trap { .. }));
    assert!(il_mir::verify(&mir).is_empty());
}

#[test]
fn branch_cleanup_is_specific_to_the_selected_transfer_set() {
    let mut graph = cleanup_graph();
    let function = &mut graph.functions[0];
    function.parameters.clear();
    function.blocks[0].operations.push(operation("main.condition", Opcode::Const, &[], &[("main.flag", "Bool")], Attributes::Constant { value: Literal::Bool(true) }));
    let mut branch = operation("main.choose", Opcode::CondBranch, &["main.flag"], &[], Attributes::CondBranch {
        then_block: "main.left".into(), else_block: "main.right".into(), then_arguments: vec!["main.local".into()], else_arguments: vec![],
    });
    branch.consumes = vec!["main.local".into()];
    function.blocks[0].terminator = branch;
    function.blocks.push(Block { entity_id: "main.left".into(), arguments: vec![ValueDef { entity_id: "main.transferred".into(), type_ref: "String".into() }], operations: vec![], terminator: operation("main.left_return", Opcode::Return, &[], &[], Attributes::Empty {}) });
    function.blocks.push(Block { entity_id: "main.right".into(), arguments: vec![], operations: vec![], terminator: operation("main.right_return", Opcode::Return, &[], &[], Attributes::Empty {}) });
    let mut mir = lower(&graph);
    if let Terminator::CondBranch { then_edge, else_edge, .. } = &mut mir.functions[0].blocks[0].terminator {
        assert!(then_edge.cleanup.is_empty());
        assert_eq!(else_edge.cleanup[0].value_id, "main.local");
        std::mem::swap(&mut then_edge.cleanup, &mut else_edge.cleanup);
    } else { panic!("expected conditional"); }
    assert_code(&mir, "E_MIR_INVALID_CLEANUP");
    assert_code(&mir, "E_MIR_MISSING_DROP");
}

#[test]
fn stage_hashes_bind_actual_canonical_inputs_and_outputs() {
    let graph = cleanup_graph();
    let hir = il_hir::lower(&graph).unwrap();
    let mir = il_mir::lower(&hir).unwrap();
    let hir_record = hir.stage_record().unwrap();
    let mir_record = mir.stage_record().unwrap();
    assert_eq!(hir_record.input_hash, graph.hash().unwrap());
    assert_eq!(hir_record.output_hash, hash_bytes(&hir.canonical_bytes().unwrap()));
    assert_eq!(mir_record.input_hash, hir_record.output_hash);
    assert_eq!(mir_record.output_hash, hash_bytes(&mir.canonical_bytes().unwrap()));
    assert_eq!(mir, lower(&graph));
}

#[test]
fn tampered_hir_does_not_skip_checking_or_source_hash_validation() {
    let mut hir = il_hir::lower(&cleanup_graph()).unwrap();
    hir.graph.functions[0].effects.clear();
    assert!(il_mir::lower(&hir).is_err());
    hir.input_hash = hir.graph.hash().unwrap();
    assert!(il_mir::lower(&hir).is_err());
}

#[test]
fn malformed_mir_mutations_never_panic() {
    let source = serde_json::to_value(lower(&cleanup_graph())).unwrap();
    for index in 0..256 {
        let mut value = source.clone();
        match index % 6 {
            0 => value["functions"][0]["blocks"][0]["terminator"]["cleanup"] = serde_json::json!([]),
            1 => value["functions"][0]["blocks"][0]["operations"][0]["outputs"] = serde_json::json!([]),
            2 => value["functions"][0]["blocks"] = serde_json::json!([]),
            3 => value["input_hash"] = serde_json::Value::String("a".repeat(index)),
            4 => value["functions"][0]["result"] = serde_json::json!("missing"),
            _ => value["functions"][0]["blocks"][0]["terminator"]["value"] = serde_json::json!("main.first"),
        }
        if let Ok(mir) = serde_json::from_value::<il_mir::Program>(value) {
            assert!(std::panic::catch_unwind(|| il_mir::verify(&mir)).is_ok());
        }
    }
}

#[test]
fn mir_capability_declarations_never_grant_themselves_authority() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/semantics/missing_host_capability.json");
    let graph = Graph::parse(&std::fs::read(path).unwrap()).unwrap();
    let trusted = graph.capabilities.clone();
    assert!(il_hir::lower(&graph).is_err());
    let hir = il_hir::lower_with_capabilities(&graph, &trusted).unwrap();
    assert!(il_mir::lower(&hir).is_err());
    let mir = il_mir::lower_with_capabilities(&hir, &trusted).unwrap();
    assert_code(&mir, "E_CAPABILITY_MISSING");
    assert!(il_mir::verify_with_capabilities(&mir, &trusted).is_empty());
}

#[test]
fn mir_borrow_escape_and_forged_move_metadata_are_rejected() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/semantics/valid_lexical_borrow.json");
    let graph = Graph::parse(&std::fs::read(path).unwrap()).unwrap();
    let mut mir = lower(&graph);
    mir.functions[0].blocks[0].operations.retain(|operation| operation.opcode != Opcode::EndBorrow);
    assert_code(&mir, "E_BORROW_ESCAPE");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/semantics/valid_owned_move_return.json");
    let graph = Graph::parse(&std::fs::read(path).unwrap()).unwrap();
    let mut mir = lower(&graph);
    mir.functions[0].blocks[0].operations[0].consumes.clear();
    assert_code(&mir, "E_OWNERSHIP_METADATA");
}

#[test]
fn nullable_return_value_is_required_by_the_closed_mir_schema() {
    let mut value = serde_json::to_value(lower(&cleanup_graph())).unwrap();
    assert_eq!(value["functions"][0]["blocks"][0]["terminator"]["value"], serde_json::Value::Null);
    value["functions"][0]["blocks"][0]["terminator"].as_object_mut().unwrap().remove("value");
    assert!(serde_json::from_value::<il_mir::Program>(value).is_err());
}

#[test]
fn synthetic_type_view_cannot_hide_invalid_mir_display_names() {
    for invalid in ["", "bad name", "../shell"] {
        let mut mir = lower(&cleanup_graph());
        mir.functions[0].name = invalid.into();
        assert_code(&mir, "E_SCHEMA_INVALID");
    }
}
