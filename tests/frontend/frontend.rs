use il_frontend::{format, parse};
use il_graph::*;

const HELLO: &str = include_str!("../../examples/core/hello.il");

#[test]
fn annotated_readable_program_has_stable_graph_identity() {
    let graph = parse(HELLO, 7).unwrap();
    assert_eq!(graph.revision, 7);
    assert_eq!(graph.modules[0].declarations, ["app.main"]);
    let function = &graph.functions[0];
    assert_eq!(function.blocks[0].operations[0].entity_id, "app.main.zero");
    assert_eq!(function.blocks[0].operations[0].outputs[0].entity_id, "app.main.zero.value");
    assert_eq!(function.blocks[0].terminator.inputs, ["app.main.zero.value"]);
    assert!(graph.validate_structural().is_empty());
}

#[test]
fn formatting_is_idempotent_and_preserves_revision() {
    let graph = parse(HELLO, 19).unwrap();
    let projected = format(&graph).unwrap();
    let reparsed = parse(&projected, 19).unwrap();
    assert_eq!(graph, reparsed);
    assert_eq!(format(&reparsed).unwrap(), projected);
}

#[test]
fn arithmetic_precedence_and_calls_resolve_to_stable_ids() {
    let graph = parse(include_str!("../../examples/core/arithmetic.il"), 0).unwrap();
    assert!(graph.validate_structural().is_empty(), "{:?}", graph.validate_structural());
    let call = graph.functions[1].blocks[0].operations.iter().find(|op| op.opcode == Opcode::Call).unwrap();
    assert_eq!(call.attributes, Attributes::Call { callee: "math.add".into() });
    let source = "module m { fn main() -> I64 { let n:I64 = 2 + 3 * 4; return n; } }";
    let graph = parse(source, 0).unwrap();
    let ops = &graph.functions[0].blocks[0].operations;
    let multiply = ops.iter().position(|op| op.opcode == Opcode::Mul).unwrap();
    let add = ops.iter().position(|op| op.opcode == Opcode::Add).unwrap();
    assert!(multiply < add);
    assert!(graph.validate_structural().is_empty());
}

#[test]
fn explicit_narrowing_cast_keeps_the_source_literal_width() {
    let graph = parse("module app { fn f()->I8 { let value:I8 = cast<I8>(1000); return value; } }", 0).unwrap();
    let operations = &graph.functions[0].blocks[0].operations;
    assert_eq!(operations[0].opcode, Opcode::Const);
    assert_eq!(operations[0].outputs[0].type_ref, "I64");
    assert_eq!(operations[1].opcode, Opcode::Cast);
    assert_eq!(operations[1].outputs[0].type_ref, "I8");
    assert!(graph.validate_structural().is_empty());
}

#[test]
fn types_and_nominal_references_survive_projection() {
    let graph = parse(include_str!("../../examples/core/types.il"), 0).unwrap();
    assert!(graph.validate_structural().is_empty(), "{:?}", graph.validate_structural());
    assert_eq!(graph.types[1].parameters, ["model.Point"]);
    assert_eq!(graph.types[2].parameters, ["model.Point", "I32"]);
    assert_eq!(parse(&format(&graph).unwrap(), 0).unwrap(), graph);
}

#[test]
fn generated_ids_are_deterministic_and_all_are_formatted() {
    let source = "module app { fn main() -> I32 { let answer:I32 = 42; return answer; } }";
    let left = parse(source, 4).unwrap();
    assert_eq!(left, parse(source, 4).unwrap());
    assert_ne!(left.modules[0].entity_id, parse(source, 5).unwrap().modules[0].entity_id);
    assert_eq!(parse(&format(&left).unwrap(), 4).unwrap(), left);
}

#[test]
fn complete_metadata_and_ownership_roundtrip_without_inference() {
    let source = r#"
@id("app") module app visibility public imports [] declarations ["app.main"] {}
@id("app.main") fn main() -> I32 effects [alloc,clock] capabilities ["clock"] contracts ["returns"] {
  @id("entry") block entry {
    @id("text") op const inputs [] outputs [@id("text.value") String] attributes {"value":"hello世界"} effects [alloc] consumes [] produces [@id("text.value") String];
    @id("drop") op drop inputs ["text.value"] outputs [] attributes {} effects [] consumes ["text.value"] produces [];
    @id("zero") op const inputs [] outputs [@id("zero.value") I32] attributes {"value":0} effects [] consumes [] produces [];
    @id("return") op return inputs ["zero.value"] outputs [] attributes {} effects [] consumes [] produces [];
  }
}
@id("clock") capability clock: ClockRead scope null;
@id("core") package core version "1.0.0" modules ["app"] effects [clock] capabilities ["clock"];
@id("returns") contract subject "app.main" predicates [{"kind":"returns","type_ref":"I32"}];
"#;
    let graph = parse(source, 0).unwrap();
    assert!(graph.validate_structural().is_empty(), "{:?}", graph.validate_structural());
    assert_eq!(parse(&format(&graph).unwrap(), 0).unwrap(), graph);
    assert_eq!(graph.functions[0].blocks[0].operations[0].produces[0].entity_id, "text.value");
}

#[test]
fn duplicate_display_names_in_distinct_modules_roundtrip() {
    let source = "@id(\"a\") module a { @id(\"a.f\") fn same()->I32 { return 0; } } @id(\"b\") module b { @id(\"b.f\") fn same()->I32 { return 1; } }";
    let graph = parse(source, 0).unwrap();
    assert_eq!(parse(&format(&graph).unwrap(), 0).unwrap(), graph);
}

#[test]
fn duplicate_attribute_keys_are_rejected_recursively() {
    let source = "module a { fn f()->I32 { op const inputs [] outputs [@id(\"x\") I32] attributes {\"value\":0,\"value\":1} effects [] consumes [] produces []; return x; } }";
    assert_eq!(parse(source, 0).unwrap_err()[0].code, "E_SCHEMA_INVALID");
}

#[test]
fn errors_are_structured_and_stable() {
    for source in ["module {", "module a { fn f()->I32 { let x:I32=1 return x; } }", "module a { fn f()->I32 { let x:I32=1; } }"] {
        let first = parse(source, 0).unwrap_err();
        assert_eq!(first, parse(source, 0).unwrap_err());
        assert_eq!(first[0].stage, "text_parse");
    }
    assert_eq!(parse("module a { fn f()->I32 { eval(\"forbidden\"); } }", 0).unwrap_err()[0].code, "E_UNSUPPORTED_FEATURE");
}

#[test]
fn malformed_string_escapes_and_excessive_nesting_reject() {
    assert!(parse("module a { fn f()->String { return \"\\uD800\"; } }", 0).is_err());
    let nested = format!("module a {{ fn f()->I32 {{ return {}0{}; }} }}", "(".repeat(140), ")".repeat(140));
    assert_eq!(parse(&nested, 0).unwrap_err()[0].code, "E_RESOURCE_LIMIT");
    assert!(parse(&" ".repeat(1_048_577), 0).is_err());
}

#[test]
fn deterministic_malformed_source_fuzz_never_panics() {
    let alphabet = b"abc012@(){}[],;:\"\\+-*/\n ";
    let mut seed = 42u64;
    for case in 0..512 {
        let mut source = String::new();
        for _ in 0..(case % 257) {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            source.push(alphabet[(seed >> 32) as usize % alphabet.len()] as char);
        }
        assert!(std::panic::catch_unwind(|| parse(&source, 0)).is_ok(), "input {source:?}");
    }
}
