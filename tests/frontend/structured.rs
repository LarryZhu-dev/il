use il_frontend::{format, parse};
use il_interpreter::{execute, Execution, ExecutionStatus, Limits, ValueData};

fn run(source: &str, limits: Limits) -> Execution {
    let graph = parse(source, 0).unwrap_or_else(|errors| panic!("parse: {errors:#?}"));
    let entry = graph.functions.iter().find(|function| function.name == "main").unwrap().entity_id.clone();
    let source = format(&graph).unwrap_or_else(|errors| panic!("format: {errors:#?}"));
    assert_eq!(parse(&source, 0).unwrap(), graph, "structured graph must round-trip losslessly");
    let hir = il_hir::lower(&graph).unwrap_or_else(|errors| panic!("check: {errors:#?}"));
    let mir = il_mir::lower(&hir).unwrap_or_else(|errors| panic!("MIR: {errors:#?}"));
    let execution = execute(&mir, &entry, &[], limits);
    assert_eq!(execution.status, ExecutionStatus::Returned, "{execution:#?}");
    execution
}

fn integer(source: &str, expected: i64) {
    let execution = run(source, Limits::default());
    assert_eq!(execution.value.unwrap().data, ValueData::Integer(expected.to_string()));
    assert_eq!(execution.live_allocations, 0);
}

#[test]
fn forward_call_uses_declared_i32_literal_context() {
    integer("module app { fn main()->I32 { return later(20,22); } fn later(a:I32,b:I32)->I32 { return a+b; } }", 42);
}

#[test]
fn if_else_joins_assignment_and_else_if() {
    integer("module app { fn main()->I32 { let mut n:I32=1; if false { n=3; } else if true { n=42; } else { n=5; } return n; } }", 42);
}

#[test]
fn while_mutable_state_break_and_continue() {
    integer("module app { fn main()->I32 { let mut n:I32=0; let mut total:I32=0; while n<10 { n=n+1; if n==3 { continue; } if n==7 { break; } total=total+n; } return total; } }", 18);
}

#[test]
fn zero_iteration_loop_keeps_initial_value() {
    integer("module app { fn main()->I32 { let mut n:I32=42; while false { n=0; } return n; } }", 42);
}

#[test]
fn owned_parameter_prologue_and_branch_cleanup() {
    integer("module app { fn main()->I32 effects [alloc] { return choose(true,\"owned\"); } fn choose(flag:Bool,text:String)->I32 { if flag { drop(text); return 42; } else { drop(text); return 0; } } }", 42);
}

#[test]
fn owned_value_crosses_loop_edges_without_copy_or_leak() {
    integer("module app { fn main()->I32 effects [alloc] { let text:String=\"abc\"; let mut n:I32=0; while n<3 { let len:Usize=runtime.string_len(text); n=n+1; } return n; } }", 3);
}

#[test]
fn match_decomposes_sum_with_exhaustive_cases() {
    integer("module app { type Maybe=Option<I32>; fn main()->I32 { let value:Maybe=Some(42); match value { Some(number)=>{return number;} None=>{return 0;} } } }", 42);
}

#[test]
fn wildcard_match_discards_owned_payload() {
    integer("module app { type Maybe=Option<String>; fn main()->I32 effects [alloc] { let value:Maybe=Some(\"owned\"); match value { None=>{return 0;} _=>{return 42;} } } }", 42);
}

#[test]
fn question_success_preserves_partial_expression_values() {
    let execution = run("module app { type Out=Result<I32,I32>; fn make()->Out{return Ok(40);} fn main()->Out { let n:I32=2+make()?; return Ok(n); } }", Limits::default());
    let ValueData::Variant(value) = execution.value.unwrap().data else { panic!("expected Result"); };
    assert_eq!(value.tag, "Ok"); assert_eq!(value.fields[0].data, ValueData::Integer("42".into()));
}

#[test]
fn question_error_cleans_live_owned_locals() {
    let execution = run("module app { type Out=Result<I32,I32>; fn fail()->Out{return Err(9);} fn main()->Out effects [alloc] { let owner:String=\"alive\"; let n:I32=fail()?; return Ok(n); } }", Limits::default());
    let ValueData::Variant(value) = execution.value.unwrap().data else { panic!("expected Result"); };
    assert_eq!(value.tag, "Err"); assert_eq!(value.fields[0].data, ValueData::Integer("9".into()));
    assert_eq!(execution.live_allocations, 0);
}

#[test]
fn template_is_fallible_string_only_and_cleans_temporaries() {
    let source = r#"module app { type Out=Result<I64,core.AllocError>; fn main()->Out effects [alloc] { let name:String="World"; let text:String=f"Hello ${name}"?; let n:Usize=runtime.string_len(text); return Ok(cast<I64>(n)); } }"#;
    let execution = run(source, Limits::default());
    let ValueData::Variant(value) = execution.value.unwrap().data else { panic!("expected Result"); };
    assert_eq!(value.tag, "Ok"); assert_eq!(value.fields[0].data, ValueData::Integer("11".into()));
    assert_eq!(execution.live_allocations, 0);
}

#[test]
fn template_allocation_failure_returns_err_without_leaking_inputs() {
    let source = r#"module app { type Out=Result<String,core.AllocError>; fn main()->Out effects [alloc] { let name:String="12345678"; return f"abcdefgh${name}"; } }"#;
    let execution = run(source, Limits { max_heap_bytes: 20, ..Limits::default() });
    let ValueData::Variant(value) = execution.value.unwrap().data else { panic!("expected Result"); };
    assert_eq!(value.tag, "Err"); assert_eq!(execution.live_allocations, 0);
}

#[test]
fn required_surface_type_and_scope_errors_are_explicit() {
    for (source, code) in [
        ("module app { fn main()->I32 { let x:I32=1; x=2; return x; } }", "E_SCHEMA_INVALID"),
        ("module app { fn main()->I32 { break; } }", "E_SCHEMA_INVALID"),
        ("module app { type Maybe=Option<I32>; fn main()->I32 {let x:Maybe=Some(1);match x {Some(n)=>{return n;}}} }", "E_NON_EXHAUSTIVE_MATCH"),
        ("module app { type A=Result<I32,I32>;type B=Result<I32,I64>;fn f()->A{return Ok(1);}fn main()->B{let x:I32=f()?;return Ok(x);} }", "E_TYPE_MISMATCH"),
        (r#"module app { fn main()->I32 effects [alloc] { let x:I32=1; f"value ${x}"; return 0; } }"#, "E_TYPE_MISMATCH"),
    ] {
        let errors = parse(source, 0).unwrap_err(); assert!(errors.iter().any(|error| error.code == code), "{errors:#?}");
    }
}

#[test]
fn generated_runtime_types_do_not_collide_across_functions() {
    let graph = parse("module app { fn a()->Unit effects [process] {runtime.print_i64(1);return;} fn b()->Unit effects [alloc] {runtime.string_concat(\"a\",\"b\");return;} fn main()->I32{return 0;} }", 0).unwrap();
    let ids: std::collections::BTreeSet<_> = graph.types.iter().map(|ty| &ty.entity_id).collect();
    assert_eq!(ids.len(), graph.types.len());
    assert!(il_checker::check(&graph).is_empty(), "{:?}", il_checker::check(&graph));
}

#[test]
fn record_fields_evaluate_in_source_order_and_store_in_declaration_order() {
    let execution = run("module app { type Pair=record {first:I64,second:I64}; fn mark(n:I64)->I64 effects [process] {runtime.print_i64(n);return n;} fn main()->Pair effects [process] {return Pair {second:mark(2),first:mark(1)};} }", Limits::default());
    assert_eq!(execution.stdout, b"21");
    let ValueData::Record(fields) = execution.value.unwrap().data else {panic!("expected record");};
    assert_eq!(fields[0].data, ValueData::Integer("1".into()));
    assert_eq!(fields[1].data, ValueData::Integer("2".into()));
}

#[test]
fn runtime_materialization_reuses_public_core_module_identity() {
    let graph = parse("@id(\"stdlib.core\") module core visibility public {} module app {fn main()->Unit effects [process] {runtime.print_i64(42);return;} }", 0).unwrap();
    assert_eq!(graph.modules.iter().filter(|module| module.path == "core").count(), 1);
    assert!(graph.modules.iter().find(|module| module.entity_id == "stdlib.core").unwrap().declarations.contains(&"core.IoError".into()));
    assert!(il_checker::check(&graph).is_empty(), "{:?}", il_checker::check(&graph));
    let errors = parse("module core {} module app {fn main()->Unit effects [process] {runtime.print_i64(42);return;} }", 0).unwrap_err();
    assert!(errors.iter().any(|error| error.code == "E_NAME_NOT_FOUND"));
}

#[test]
fn flat_expression_and_else_if_chains_have_bounded_ast_depth() {
    let sources = [
        format!("module app {{fn main()->I32 {{return {};}}}}", vec!["1"; 10_000].join("+")),
        format!("module app {{fn main()->I32 {{return value{};}}}}", "?".repeat(10_000)),
        format!("module app {{fn main()->I32 {{{}{{return 0;}} return 1;}}}}", "if false {} else ".repeat(10_000)),
    ];
    for source in sources {
        let errors = parse(&source, 0).unwrap_err();
        assert!(errors.iter().any(|error| error.code == "E_RESOURCE_LIMIT"), "{errors:#?}");
    }
}

#[test]
fn structured_examples_execute_and_roundtrip() {
    integer(include_str!("../../examples/core/control_flow.il"), 18);
    let execution = run(include_str!("../../examples/core/template.il"), Limits::default());
    let ValueData::Variant(value) = execution.value.unwrap().data else {panic!("expected result");};
    assert_eq!(value.tag, "Ok");
    assert_eq!(value.fields[0].data, ValueData::Integer("11".into()));
    assert_eq!(execution.live_allocations, 0);
}
