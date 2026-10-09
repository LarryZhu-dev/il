use il_graph::*;
use serde_json::json;

fn fixture() -> Graph {
    serde_json::from_slice(include_bytes!("../fixtures/semantics/valid_typed_call.json")).unwrap()
}

#[test]
fn targeted_views_match_index_and_project_large_container_fields() {
    let graph = fixture();
    let references = entity_reference_index(&graph);
    for (id, expected) in entity_index(&graph).unwrap() {
        assert_eq!(references[&id], entity_references(&graph, &id));
        assert_eq!(entity_view(&graph, &id, None).unwrap(), expected);
        for (field, value) in expected.as_object().unwrap() {
            assert_eq!(entity_view(&graph, &id, Some(&[field.clone()])).unwrap(), json!({field: value}));
        }
    }
    assert_eq!(entity_view(&graph, "program", Some(&["revision".into(), "target".into()])).unwrap(),
        json!({"revision": 0, "target": TARGET}));
    assert_eq!(entity_view(&graph, "main", Some(&["name".into(), "result".into()])).unwrap(),
        json!({"name": "main", "result": "I64"}));
    assert_eq!(entity_view(&graph, "absent", None).unwrap_err().code, "E_NAME_NOT_FOUND");
    for fields in [vec!["unknown".into()], vec!["name".into(), "name".into()]] {
        assert_eq!(entity_view(&graph, "main", Some(&fields)).unwrap_err().code, "E_SCHEMA_INVALID");
    }
}

#[test]
fn queries_distinguish_external_dependencies_from_embedded_definitions() {
    let mut graph = fixture();
    graph.packages.push(Package { entity_id: "package".into(), name: "example".into(), version: "1".into(),
        modules: vec!["app".into()], capabilities: vec!["clock".into()], effects: vec![] });
    graph.capabilities.push(Capability { entity_id: "clock".into(), kind: CapabilityKind::ClockRead, scope: None });
    assert_eq!(entity_references(&graph, "package"), ["app", "clock"]);
    assert!(entity_references(&graph, "clock").is_empty());
    assert_eq!(entity_references(&graph, "main"), ["identity"]);
    assert_eq!(entity_references(&graph, "main.entry"), ["identity"]);
    assert_eq!(entity_references(&graph, "main.call"), ["identity", "main.answer"]);
    assert_eq!(entity_references(&graph, "identity.entry"), ["identity.value"]);
    assert!(entity_references(&graph, "identity").is_empty());
    graph.functions[0].parameters[0].type_ref = "Widget".into();
    assert_eq!(entity_references(&graph, "identity.value"), ["Widget"]);
    assert_eq!(entity_references(&graph, "identity"), ["Widget"]);
    graph.functions[1].blocks[0].arguments.push(ValueDef { entity_id: "argument".into(), type_ref: "Widget".into() });
    assert_eq!(entity_references(&graph, "argument"), ["Widget"]);
    assert_eq!(entity_references(&graph, "main"), ["Widget", "identity"]);
}

#[test]
fn operation_references_include_control_flow_types_and_runtime_authority() {
    let cases = [
        (Attributes::RuntimeCall { symbol: "clock_now".into(), capability: Some("clock".into()) }, vec!["clock"]),
        (Attributes::Cast { target_type: "Widget".into() }, vec!["Widget"]),
        (Attributes::Record { type_id: "Widget".into() }, vec!["Widget"]),
        (Attributes::Variant { type_id: "Widget".into(), variant: "A".into() }, vec!["Widget"]),
        (Attributes::Branch { target: "next".into() }, vec!["next"]),
        (Attributes::CondBranch { then_block: "yes".into(), else_block: "no".into(),
            then_arguments: vec!["a".into()], else_arguments: vec!["b".into()] }, vec!["a", "b", "no", "yes"]),
        (Attributes::Switch { cases: vec![SwitchCase { tag: "A".into(), target: "yes".into(), arguments: vec!["a".into()] }],
            default: "no".into(), default_arguments: vec!["b".into()] }, vec!["a", "b", "no", "yes"]),
    ];
    for (attributes, references) in cases {
        let mut graph = fixture();
        graph.functions[1].blocks[0].operations[0].attributes = attributes;
        assert_eq!(entity_references(&graph, "main.answer.op"), references);
        for (id, references) in entity_reference_index(&graph) {
            assert_eq!(references, entity_references(&graph, &id));
        }
    }
    let mut graph = fixture();
    let operation = &mut graph.functions[1].blocks[0].operations[0];
    operation.outputs[0].type_ref = "Widget".into();
    operation.consumes = vec!["owned".into()];
    assert_eq!(entity_references(&graph, "main.answer.op"), ["Widget", "owned"]);
    assert_eq!(entity_references(&graph, "main.answer"), ["Widget"]);
    assert_eq!(entity_references(&graph, "main"), ["Widget", "identity", "owned"]);
}
