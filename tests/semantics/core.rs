use il_checker::{check, check_with_capabilities, is_owned_type};
use il_graph::*;
use std::path::Path;

fn fixture(name: &str) -> Graph {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/semantics");
    Graph::parse(&std::fs::read(root.join(format!("{name}.json"))).unwrap()).unwrap()
}

fn operation(id: &str, opcode: Opcode, inputs: &[&str], outputs: &[(&str, &str)], attributes: Attributes) -> Operation {
    Operation { entity_id: id.into(), opcode, inputs: inputs.iter().map(|id| (*id).into()).collect(),
        outputs: outputs.iter().map(|(id, ty)| ValueDef { entity_id: (*id).into(), type_ref: (*ty).into() }).collect(),
        attributes, effects: vec![], consumes: vec![], produces: vec![] }
}

fn assert_code(graph: &Graph, code: &str) {
    let diagnostics = check(graph);
    assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == code), "expected {code}, got {diagnostics:?}");
}

#[test]
fn independent_fixture_contracts_all_pass() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/semantics");
    let cases: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("cases.json")).unwrap()).unwrap();
    for case in cases["cases"].as_array().unwrap() {
        let name = case["id"].as_str().unwrap();
        let bytes = std::fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
        let graph = match Graph::parse(&bytes) {
            Ok(graph) => graph,
            Err(_) if case["valid"] == false && case["diagnostic_code"] == "E_SCHEMA_INVALID" => continue,
            Err(error) => panic!("{name}: unexpected schema rejection: {error}"),
        };
        let diagnostics = check(&graph);
        assert_eq!(diagnostics, check(&graph), "unstable diagnostic IDs for {name}");
        if case["valid"] == true { assert!(diagnostics.is_empty(), "{name}: {diagnostics:?}"); }
        else { assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == case["diagnostic_code"].as_str().unwrap()), "{name}: {diagnostics:?}"); }
    }
}

#[test]
fn metadata_cannot_hide_owned_moves_or_owned_results() {
    let mut graph = fixture("valid_owned_move_return");
    graph.functions[0].blocks[0].operations[0].consumes.clear();
    assert_code(&graph, "E_OWNERSHIP_METADATA");
    graph = fixture("valid_owned_move_return");
    graph.functions[0].blocks[0].operations[0].produces.clear();
    assert_code(&graph, "E_OWNERSHIP_METADATA");
}

#[test]
fn opaque_empty_record_is_owned_and_cannot_be_constructed() {
    let mut graph = fixture("valid_record_and_field");
    let name = graph.types[0].entity_id.clone();
    graph.types[0].layout = Layout::Opaque;
    graph.types[0].fields.clear();
    assert!(is_owned_type(&graph, &name));
    assert_code(&graph, "E_TYPE_MISMATCH");
}

#[test]
fn shared_loan_prevents_move_until_ended() {
    let mut graph = fixture("valid_lexical_borrow");
    graph.functions[0].blocks[0].operations.pop();
    assert_code(&graph, "E_BORROW_CONFLICT");
    assert_code(&graph, "E_BORROW_ESCAPE");
}

#[test]
fn unique_loan_rejects_overlapping_shared_loan() {
    let mut graph = fixture("valid_lexical_borrow");
    graph.functions[0].blocks[0].operations[0].opcode = Opcode::BorrowMut;
    let borrow = operation("main.borrow_again", Opcode::Borrow, &["main.source"], &[("main.view2", "String")], Attributes::Empty {});
    graph.functions[0].blocks[0].operations.insert(1, borrow);
    assert_code(&graph, "E_BORROW_CONFLICT");
}

#[test]
fn capability_record_requires_identical_host_scope() {
    let graph = fixture("missing_host_capability");
    let injected = graph.capabilities.clone();
    assert!(check_with_capabilities(&graph, &injected).is_empty());
    let mut forged = injected;
    forged[0].entity_id = "different_grant".into();
    assert!(check_with_capabilities(&graph, &forged).iter().any(|diagnostic| diagnostic.code == "E_CAPABILITY_MISSING"));
}

#[test]
fn unknown_runtime_symbol_is_not_an_ffi_escape_hatch() {
    let mut graph = fixture("valid_integer_return");
    graph.functions[0].blocks[0].operations[0].opcode = Opcode::RuntimeCall;
    graph.functions[0].blocks[0].operations[0].attributes = Attributes::RuntimeCall { symbol: "shell_eval".into(), capability: None };
    assert_code(&graph, "E_UNSUPPORTED_FEATURE");
}

#[test]
fn stdout_runtime_cannot_discard_io_failure() {
    let mut graph = fixture("valid_integer_return");
    let input = graph.functions[0].blocks[0].operations[0].outputs[0].entity_id.clone();
    let mut print = operation("main.print", Opcode::RuntimeCall, &[&input], &[], Attributes::RuntimeCall { symbol: "print_i64".into(), capability: None });
    print.effects = vec![Effect::Process];
    graph.functions[0].effects = vec![Effect::Process];
    graph.functions[0].blocks[0].operations.push(print);
    assert_code(&graph, "E_TYPE_MISMATCH");
}

#[test]
fn full_integer_width_boundaries_are_checked_before_lowering() {
    for (ty, accepted, rejected) in [("I8", 127, 128), ("U8", 255, 256), ("I16", 32767, 32768), ("U16", 65535, 65536), ("I32", 2147483647, 2147483648), ("U32", 4294967295, 4294967296)] {
        let mut graph = fixture("valid_integer_return");
        graph.functions[0].result = ty.into();
        let constant = &mut graph.functions[0].blocks[0].operations[0];
        constant.outputs[0].type_ref = ty.into();
        constant.attributes = Attributes::Constant { value: Literal::Integer(accepted) };
        assert!(check(&graph).is_empty(), "{ty}");
        graph.functions[0].blocks[0].operations[0].attributes = Attributes::Constant { value: Literal::Integer(rejected) };
        assert_code(&graph, "E_INTEGER_OVERFLOW");
    }
}

#[test]
fn arbitrary_schema_and_graph_mutations_never_panic() {
    let source = serde_json::to_value(fixture("valid_integer_add")).unwrap();
    for seed in 0..512u64 {
        let mut value = source.clone();
        match seed % 8 {
            0 => value["functions"][0]["result"] = serde_json::Value::String(format!("missing_{seed}")),
            1 => value["functions"][0]["blocks"][0]["operations"][0]["outputs"] = serde_json::json!([]),
            2 => value["functions"][0]["blocks"][0]["operations"][0]["attributes"] = serde_json::json!({"value":seed}),
            3 => value["functions"][0]["blocks"][0]["terminator"]["inputs"] = serde_json::json!([]),
            4 => value["functions"][0]["parameters"] = serde_json::json!([]),
            5 => value["functions"][0]["blocks"] = serde_json::json!([]),
            6 => value["functions"][0]["blocks"][0]["operations"][0]["opcode"] = serde_json::Value::String(format!("unknown_{seed}")),
            _ => value["revision"] = serde_json::json!(seed),
        }
        if let Ok(graph) = serde_json::from_value::<Graph>(value) {
            let diagnostics = std::panic::catch_unwind(|| check(&graph));
            assert!(diagnostics.is_ok(), "checker panic for seed {seed}");
        }
    }
}

fn graph_with(function: Function, types: Vec<TypeDef>) -> Graph {
    let mut graph = Graph::empty();
    graph.modules.push(Module { entity_id: "app".into(), path: "app".into(), imports: vec![],
        declarations: types.iter().map(|ty| ty.entity_id.clone()).chain(std::iter::once(function.entity_id.clone())).collect(), visibility: Visibility::Public });
    graph.functions.push(function);
    graph.types = types;
    graph
}

fn owned_return(id: &str, value: &str) -> Operation {
    let mut operation = operation(id, Opcode::Return, &[value], &[], Attributes::Empty {});
    operation.consumes = vec![value.into()];
    operation
}

#[test]
fn mutually_exclusive_owned_edge_transfers_are_not_double_moves() {
    let mut branch = operation("main.choose", Opcode::CondBranch, &["main.condition"], &[], Attributes::CondBranch {
        then_block: "main.left".into(), else_block: "main.right".into(), then_arguments: vec!["main.source".into()], else_arguments: vec!["main.source".into()],
    });
    branch.consumes = vec!["main.source".into()];
    let function = Function { entity_id: "main".into(), name: "main".into(),
        parameters: vec![Parameter { entity_id: "main.source".into(), name: "source".into(), type_ref: "String".into() }, Parameter { entity_id: "main.condition".into(), name: "condition".into(), type_ref: "Bool".into() }],
        result: "String".into(), effects: vec![], capabilities: vec![], contracts: vec![], blocks: vec![
            Block { entity_id: "main.entry".into(), arguments: vec![], operations: vec![], terminator: branch },
            Block { entity_id: "main.left".into(), arguments: vec![ValueDef { entity_id: "main.left_value".into(), type_ref: "String".into() }], operations: vec![], terminator: owned_return("main.left_return", "main.left_value") },
            Block { entity_id: "main.right".into(), arguments: vec![ValueDef { entity_id: "main.right_value".into(), type_ref: "String".into() }], operations: vec![], terminator: owned_return("main.right_return", "main.right_value") },
        ] };
    let diagnostics = check(&graph_with(function, vec![]));
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn ownership_join_rejects_parameter_live_on_only_one_incoming_path() {
    let mut drop_value = operation("main.drop", Opcode::Drop, &["main.source"], &[], Attributes::Empty {});
    drop_value.consumes = vec!["main.source".into()];
    let function = Function { entity_id: "main".into(), name: "main".into(),
        parameters: vec![Parameter { entity_id: "main.source".into(), name: "source".into(), type_ref: "String".into() }, Parameter { entity_id: "main.condition".into(), name: "condition".into(), type_ref: "Bool".into() }],
        result: "Unit".into(), effects: vec![], capabilities: vec![], contracts: vec![], blocks: vec![
            Block { entity_id: "main.entry".into(), arguments: vec![], operations: vec![], terminator: operation("main.choose", Opcode::CondBranch, &["main.condition"], &[], Attributes::CondBranch { then_block: "main.left".into(), else_block: "main.right".into(), then_arguments: vec![], else_arguments: vec![] }) },
            Block { entity_id: "main.left".into(), arguments: vec![], operations: vec![drop_value], terminator: operation("main.left_branch", Opcode::Branch, &[], &[], Attributes::Branch { target: "main.join".into() }) },
            Block { entity_id: "main.right".into(), arguments: vec![], operations: vec![], terminator: operation("main.right_branch", Opcode::Branch, &[], &[], Attributes::Branch { target: "main.join".into() }) },
            Block { entity_id: "main.join".into(), arguments: vec![], operations: vec![], terminator: operation("main.return", Opcode::Return, &[], &[], Attributes::Empty {}) },
        ] };
    assert_code(&graph_with(function, vec![]), "E_OWNERSHIP_JOIN");
}

#[test]
fn sum_payload_requires_path_refinement_and_moves_whole_owned_sum() {
    let ty = TypeDef { entity_id: "Choice".into(), kind: TypeKind::Sum, parameters: vec![], layout: Layout::Inferred, integer: None, fields: vec![],
        variants: vec![Variant { name: "Text".into(), fields: vec!["String".into()] }, Variant { name: "Empty".into(), fields: vec![] }] };
    let mut switch = operation("main.switch", Opcode::Switch, &["main.choice"], &[], Attributes::Switch {
        cases: vec![SwitchCase { tag: "Text".into(), target: "main.text".into(), arguments: vec!["main.choice".into()] }, SwitchCase { tag: "Empty".into(), target: "main.empty".into(), arguments: vec![] }], default: "main.invalid".into(), default_arguments: vec![],
    });
    switch.consumes = vec!["main.choice".into()];
    let mut payload = operation("main.payload", Opcode::Payload, &["main.text_choice"], &[("main.text_value", "String")], Attributes::Empty {});
    payload.consumes = vec!["main.text_choice".into()];
    payload.produces = payload.outputs.clone();
    let mut drop_value = operation("main.drop", Opcode::Drop, &["main.text_value"], &[], Attributes::Empty {});
    drop_value.consumes = vec!["main.text_value".into()];
    let function = Function { entity_id: "main".into(), name: "main".into(), parameters: vec![Parameter { entity_id: "main.choice".into(), name: "choice".into(), type_ref: "Choice".into() }],
        result: "Unit".into(), effects: vec![], capabilities: vec![], contracts: vec![], blocks: vec![
            Block { entity_id: "main.entry".into(), arguments: vec![], operations: vec![], terminator: switch },
            Block { entity_id: "main.text".into(), arguments: vec![ValueDef { entity_id: "main.text_choice".into(), type_ref: "Choice".into() }], operations: vec![payload, drop_value], terminator: operation("main.text_return", Opcode::Return, &[], &[], Attributes::Empty {}) },
            Block { entity_id: "main.empty".into(), arguments: vec![], operations: vec![], terminator: operation("main.empty_return", Opcode::Return, &[], &[], Attributes::Empty {}) },
            Block { entity_id: "main.invalid".into(), arguments: vec![], operations: vec![], terminator: operation("main.trap", Opcode::Trap, &[], &[], Attributes::Trap { code: "E_SCHEMA_INVALID".into() }) },
        ] };
    let graph = graph_with(function, vec![ty]);
    let diagnostics = check(&graph);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let mut invalid = graph;
    let block = &mut invalid.functions[0].blocks[1];
    block.operations[0].outputs[0].type_ref = "I64".into();
    block.operations[0].produces.clear();
    assert_code(&invalid, "E_TYPE_MISMATCH");
}

#[test]
fn cross_module_calls_require_direct_public_imports() {
    let mut graph = fixture("valid_typed_call");
    let callee = graph.functions.iter().find(|function| function.entity_id != "main").unwrap().entity_id.clone();
    graph.modules[0].declarations.retain(|id| id != &callee);
    graph.modules.push(Module { entity_id: "library".into(), path: "library".into(), imports: vec![], declarations: vec![callee], visibility: Visibility::Public });
    assert_code(&graph, "E_NAME_NOT_FOUND");
    graph.modules[0].imports.push("library".into());
    assert!(check(&graph).is_empty());
    graph.modules[1].visibility = Visibility::Private;
    assert_code(&graph, "E_NAME_NOT_FOUND");
    graph.modules[1].visibility = Visibility::Public;
    graph.modules[0].imports = vec!["middle".into()];
    graph.modules.push(Module { entity_id: "middle".into(), path: "middle".into(), imports: vec!["library".into()], declarations: vec![], visibility: Visibility::Public });
    assert_code(&graph, "E_NAME_NOT_FOUND");
}

#[test]
fn nominal_type_references_obey_module_visibility_and_no_orphan_namespace() {
    let mut graph = fixture("valid_record_and_field");
    let record = graph.types[0].entity_id.clone();
    graph.modules[0].declarations.retain(|id| id != &record);
    assert_code(&graph, "E_NAME_NOT_FOUND");
    graph.modules.push(Module { entity_id: "data".into(), path: "data".into(), imports: vec![], declarations: vec![record], visibility: Visibility::Public });
    assert_code(&graph, "E_NAME_NOT_FOUND");
    graph.modules[0].imports.push("data".into());
    assert!(check(&graph).is_empty());
    graph.modules[0].declarations.clear();
    assert_code(&graph, "E_NAME_NOT_FOUND");
}

#[test]
fn opaque_resources_cannot_leak_fields_through_borrowed_projections() {
    let mut graph = fixture("valid_record_and_field");
    let record = graph.types[0].entity_id.clone();
    graph.types[0].layout = Layout::Opaque;
    let projection = graph.functions[0].blocks[0].operations.iter().find(|operation| operation.opcode == Opcode::Field).unwrap().clone();
    let input = projection.inputs[0].clone();
    graph.functions[0].parameters.push(Parameter { entity_id: input.clone(), name: "opaque".into(), type_ref: record });
    graph.functions[0].blocks[0].operations = vec![projection];
    assert_code(&graph, "E_UNSUPPORTED_FEATURE");
}

#[test]
fn duplicate_owned_call_arguments_cannot_duplicate_one_owner() {
    let mut graph = fixture("valid_owned_move_return");
    let mut callee = graph.functions[0].clone();
    callee.entity_id = "consume".into();
    callee.name = "consume".into();
    callee.parameters = vec![Parameter { entity_id: "consume.left".into(), name: "left".into(), type_ref: "String".into() }, Parameter { entity_id: "consume.right".into(), name: "right".into(), type_ref: "String".into() }];
    callee.result = "Unit".into();
    callee.blocks = vec![Block { entity_id: "consume.entry".into(), arguments: vec![], operations: vec![], terminator: operation("consume.return", Opcode::Return, &[], &[], Attributes::Empty {}) }];
    let owner = graph.functions[0].parameters[0].entity_id.clone();
    let mut call = operation("main.call", Opcode::Call, &[&owner, &owner], &[], Attributes::Call { callee: "consume".into() });
    call.consumes = vec![owner];
    graph.functions[0].result = "Unit".into();
    graph.functions[0].blocks[0].operations = vec![call];
    graph.functions[0].blocks[0].terminator = operation("main.return", Opcode::Return, &[], &[], Attributes::Empty {});
    graph.functions.push(callee);
    graph.modules[0].declarations.push("consume".into());
    assert_code(&graph, "E_USE_AFTER_MOVE");
}

#[test]
fn huge_shallow_type_graph_is_budgeted_without_recursive_traversal() {
    let mut graph = Graph::empty();
    graph.modules.push(Module { entity_id: "app".into(), path: "app".into(), imports: vec![], declarations: vec![], visibility: Visibility::Public });
    for index in 0..10_000 {
        let id = format!("type_{index}");
        graph.modules[0].declarations.push(id.clone());
        graph.types.push(TypeDef { entity_id: id, kind: TypeKind::Record, parameters: vec![], layout: Layout::Inferred, integer: None,
            fields: if index == 0 { vec![] } else { vec![Field { name: "next".into(), type_ref: format!("type_{}", index - 1) }] }, variants: vec![] });
    }
    assert_code(&graph, "E_RESOURCE_LIMIT");
}

#[test]
fn privileged_runtime_calls_select_exact_grants() {
    let mut graph = fixture("missing_host_capability");
    let grants = graph.capabilities.clone();
    let mut call = operation("main.clock", Opcode::RuntimeCall, &[], &[("main.now", "U64")],
        Attributes::RuntimeCall { symbol:"clock_now".into(), capability:Some("host.clock".into()) });
    call.effects = vec![Effect::Clock];
    graph.functions[0].blocks[0].operations.push(call);
    assert!(check_with_capabilities(&graph, &grants).is_empty());
    for selector in [None, Some("ungranted.clock".into())] {
        graph.functions[0].blocks[0].operations[0].attributes = Attributes::RuntimeCall { symbol:"clock_now".into(), capability:selector };
        assert!(check_with_capabilities(&graph, &grants).iter().any(|d|d.code=="E_CAPABILITY_MISSING"));
    }
}

#[test]
fn host_resource_nominal_layout_cannot_be_redefined() {
    let mut graph = fixture("valid_integer_return");
    graph.modules[0].declarations.push("core.File".into());
    graph.types.push(TypeDef { entity_id:"core.File".into(),kind:TypeKind::Record,layout:Layout::Opaque,
        parameters:vec![],integer:None,fields:vec![Field{name:"slot".into(),type_ref:"U64".into()},Field{name:"generation".into(),type_ref:"U64".into()}],variants:vec![] });
    assert!(check(&graph).is_empty());
    graph.types[0].fields[1].type_ref="U32".into();
    assert_code(&graph,"E_TYPE_MISMATCH");
    graph.types[0].fields[1].type_ref="U64".into();
    graph.types[0].layout=Layout::Inferred;
    assert_code(&graph,"E_TYPE_MISMATCH");
}
