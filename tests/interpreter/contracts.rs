use il_graph::{Attributes, Opcode, Operation, ValueDef, Parameter, TypeDef, TypeKind, Layout, Effect};
use il_interpreter::execute;
use il_execution_model::{ExecutionStatus, Limits, Value, ValueData, LifecycleKind};
use il_mir::{Program,Function,Block,Terminator,DropAction,Edge};

fn value(id:&str,ty:&str)->ValueDef { ValueDef { entity_id:id.into(),type_ref:ty.into() } }
fn op(id:&str,code:Opcode,inputs:&[&str],outputs:&[(&str,&str)],attributes:Attributes)->Operation {
    Operation { entity_id:id.into(),opcode:code,inputs:inputs.iter().map(|s|s.to_string()).collect(),outputs:outputs.iter().map(|(id,ty)|value(id,ty)).collect(),attributes,effects:vec![],consumes:vec![],produces:vec![] }
}
fn program(ty:&str,params:&[(&str,&str)],operations:Vec<Operation>,result:Option<&str>)->Program {
    Program { schema_version:"1.0.0".into(),compiler_version:"0.1.0".into(),input_hash:format!("sha256:{}","0".repeat(64)),source_revision:0,target:il_graph::TARGET.into(),types:vec![],capabilities:vec![],functions:vec![Function {
        entity_id:"main".into(),name:"main".into(),parameters:params.iter().map(|(id,ty)|Parameter{entity_id:id.to_string(),name:id.replace('.',"_"),type_ref:ty.to_string()}).collect(),result:ty.into(),effects:vec![],capabilities:vec![],blocks:vec![Block {
            entity_id:"main.entry".into(),arguments:vec![],operations,terminator:Terminator::Return{entity_id:"main.return".into(),value:result.map(str::to_owned),cleanup:vec![]}
        }]
    }] }
}
fn binary(ty:&str,opcode:Opcode,a:i128,b:i128)->il_execution_model::Execution {
    let p=program(ty,&[("a",ty),("b",ty)],vec![op("calculate",opcode,&["a","b"],&[("answer",ty)],Attributes::Empty{})],Some("answer"));
    execute(&p,"main",&[Value::integer(ty,a),Value::integer(ty,b)],Limits::default())
}
fn success(e:&il_execution_model::Execution,expected:Value) { assert_eq!(e.status,ExecutionStatus::Returned,"{:?}",e.diagnostics);assert_eq!(e.value,Some(expected));assert!(e.diagnostics.is_empty()); }
fn trapped(e:&il_execution_model::Execution,code:&str) { assert_eq!(e.status,ExecutionStatus::Trapped,"{:?}",e.diagnostics);assert_eq!(e.diagnostics[0].code,code);assert!(e.value.is_none()); }

#[test]
fn checked_integer_boundaries_every_width() {
    for (ty,signed,bits) in [("I8",true,8),("I16",true,16),("I32",true,32),("I64",true,64),("U8",false,8),("U16",false,16),("U32",false,32),("U64",false,64),("Usize",false,64)] {
        let max=if signed {(1i128<<(bits-1))-1}else{(1i128<<bits)-1}; let min=if signed {-(1i128<<(bits-1))}else{0};
        success(&binary(ty,Opcode::Add,max,0),Value::integer(ty,max));
        for (code,a,b,diagnostic) in [(Opcode::Add,max,1,"E_INTEGER_OVERFLOW"),(Opcode::Sub,min,1,"E_INTEGER_OVERFLOW"),(Opcode::Mul,max,2,"E_INTEGER_OVERFLOW"),(Opcode::Div,1,0,"E_DIVIDE_BY_ZERO"),(Opcode::Rem,1,0,"E_DIVIDE_BY_ZERO"),(Opcode::Shl,1,bits,"E_INVALID_SHIFT"),(Opcode::Shr,1,bits,"E_INVALID_SHIFT"),(Opcode::Shl,max,1,"E_INTEGER_OVERFLOW")] {
            let e=binary(ty,code,a,b);trapped(&e,diagnostic);assert_eq!(e.diagnostics[0].entity_id.as_deref(),Some("calculate"));assert!(e.stdout.is_empty());assert!(e.stderr.is_empty());
        }
        if signed {
            trapped(&binary(ty,Opcode::Div,min,-1),"E_INTEGER_OVERFLOW");trapped(&binary(ty,Opcode::Rem,min,-1),"E_INTEGER_OVERFLOW");
            trapped(&binary(ty,Opcode::Shl,1,-1),"E_INVALID_SHIFT");trapped(&binary(ty,Opcode::Shl,1,bits-1),"E_INTEGER_OVERFLOW");
            success(&binary(ty,Opcode::Div,-7,3),Value::integer(ty,-2));success(&binary(ty,Opcode::Rem,-7,3),Value::integer(ty,-1));
            success(&binary(ty,Opcode::Shl,-1,bits-1),Value::integer(ty,min));success(&binary(ty,Opcode::Shr,min,bits-1),Value::integer(ty,-1));
        } else { success(&binary(ty,Opcode::Shl,1,bits-1),Value::integer(ty,1i128<<(bits-1)));success(&binary(ty,Opcode::Shr,max,bits-1),Value::integer(ty,1)); }
        for (code,a,b,answer) in [(Opcode::BitAnd,6,3,2),(Opcode::BitOr,6,3,7),(Opcode::BitXor,6,3,5)] {success(&binary(ty,code,a,b),Value::integer(ty,answer));}
    }
}

#[test]
fn explicit_casts_and_finite_complement() {
    for (from,to,n,expected) in [("U64","I64",u64::MAX as i128,None),("I64","U64",-1,None),("U8","I16",255,Some(255))] {
        let p=program(to,&[("arg",from)],vec![op("cast",Opcode::Cast,&["arg"],&[("answer",to)],Attributes::Cast{target_type:to.into()})],Some("answer"));
        let e=execute(&p,"main",&[Value::integer(from,n)],Limits::default());if let Some(n)=expected{success(&e,Value::integer(to,n))}else{trapped(&e,"E_INTEGER_OVERFLOW")}
    }
    for (ty,n,expected) in [("U8",0,255),("I8",0,-1),("U64",0,u64::MAX as i128)] {
        let p=program(ty,&[("arg",ty)],vec![op("not",Opcode::Not,&["arg"],&[("answer",ty)],Attributes::Empty{})],Some("answer"));
        success(&execute(&p,"main",&[Value::integer(ty,n)],Limits::default()),Value::integer(ty,expected));
    }
}

#[test]
fn malformed_arguments_rejected_without_effects() {
    let p=program("I64",&[("arg","I64")],vec![],Some("arg"));
    for arg in [Value::string("42"),Value{type_ref:"I64".into(),data:ValueData::Integer("01".into())},Value{type_ref:"I64".into(),data:ValueData::Integer("-0".into())},Value::integer("I64",i128::MAX)] {
        let e=execute(&p,"main",&[arg],Limits::default());assert_eq!(e.status,ExecutionStatus::Rejected);assert_eq!(e.diagnostics[0].code,"E_TYPE_MISMATCH");assert_eq!(e.steps,0);assert!(e.lifecycle.is_empty());
    }
}

#[test]
fn instruction_limit_and_determinism() {
    let p=program("I64",&[("arg","I64")],vec![op("move",Opcode::Move,&["arg"],&[("answer","I64")],Attributes::Empty{})],Some("answer"));
    let a=[Value::integer("I64",42)];let e=execute(&p,"main",&a,Limits::default());success(&e,a[0].clone());assert_eq!(e,execute(&p,"main",&a,Limits::default()));
    let e=execute(&p,"main",&a,Limits{max_steps:1,..Limits::default()});trapped(&e,"E_RESOURCE_LIMIT");assert_eq!(e.steps,1);assert_eq!(e.diagnostics[0].entity_id.as_deref(),Some("main.return"));
}

#[test]
fn owned_return_and_explicit_cleanup_order() {
    let mut p=program("String",&[("first","String"),("second","String"),("returned","String")],vec![],Some("returned"));
    if let Terminator::Return{cleanup,..}=&mut p.functions[0].blocks[0].terminator{*cleanup=vec![DropAction{value_id:"second".into(),type_ref:"String".into()},DropAction{value_id:"first".into(),type_ref:"String".into()}];}
    let e=execute(&p,"main",&[Value::string("one"),Value::string("two"),Value::string("三")],Limits::default());success(&e,Value::string("三"));assert_eq!(e.live_allocations,1);assert_eq!(e.peak_heap_bytes,9);
    assert_eq!(e.lifecycle.iter().filter(|v|v.kind==LifecycleKind::Drop).map(|v|v.entity_id.as_str()).collect::<Vec<_>>(),vec!["second","first"]);
    if let Terminator::Return{cleanup,..}=&mut p.functions[0].blocks[0].terminator{cleanup.clear();}
    let e=execute(&p,"main",&[Value::string("one"),Value::string("two"),Value::string("三")],Limits::default());assert_eq!(e.status,ExecutionStatus::Rejected);assert_eq!(e.steps,0);assert!(e.lifecycle.is_empty());
}

#[test]
fn heap_and_capture_limits_and_trap_no_unwind() {
    let mut p=program("String",&[("arg","String")],vec![],Some("arg"));
    let e=execute(&p,"main",&[Value::string("hello")],Limits{max_heap_bytes:4,..Limits::default()});assert_eq!(e.status,ExecutionStatus::Rejected);assert!(e.lifecycle.is_empty());
    let e=execute(&p,"main",&[Value::string("hello")],Limits{max_output_bytes:1,..Limits::default()});trapped(&e,"E_RESOURCE_LIMIT");assert!(e.lifecycle.is_empty());
    p.functions[0].blocks[0].terminator=Terminator::Trap{entity_id:"panic".into(),code:"TEST_PANIC".into()};
    let e=execute(&p,"main",&[Value::string("hello")],Limits::default());trapped(&e,"E_EXPLICIT_TRAP");assert_eq!(e.live_allocations,1);assert!(!e.lifecycle.iter().any(|v|v.kind==LifecycleKind::Drop));
}

#[test]
fn selected_edge_owned_transfer() {
    let mut p=program("String",&[("condition","Bool"),("left","String"),("right","String")],vec![],None);
    let edge=|target:&str,arg:&str,drop:&str|Edge{target:target.into(),arguments:vec![arg.into()],cleanup:vec![DropAction{value_id:drop.into(),type_ref:"String".into()}]};
    p.functions[0].blocks[0].terminator=Terminator::CondBranch{entity_id:"choose".into(),condition:"condition".into(),then_edge:edge("yes","left","right"),else_edge:edge("no","right","left")};
    // Parameters remain live across edges; discard on return according to MIR's parameter lifetime rule.
    if let Terminator::CondBranch{then_edge,else_edge,..}=&mut p.functions[0].blocks[0].terminator{then_edge.cleanup.clear();else_edge.cleanup.clear();}
    for (block,arg,remaining) in [("yes","yes.arg","right"),("no","no.arg","left")] {
        p.functions[0].blocks.push(Block{entity_id:block.into(),arguments:vec![value(arg,"String")],operations:vec![],terminator:Terminator::Return{entity_id:format!("{block}.return"),value:Some(arg.into()),cleanup:vec![DropAction{value_id:remaining.into(),type_ref:"String".into()}]}});
    }
    for (condition,expected) in [(true,"left"),(false,"right")] {
        let e=execute(&p,"main",&[Value::boolean(condition),Value::string("left"),Value::string("right")],Limits::default());success(&e,Value::string(expected));assert_eq!(e.live_allocations,1);
    }
}

#[test]
fn recursion_depth_budget() {
    let mut p=program("Unit",&[],vec![op("call",Opcode::Call,&[],&[],Attributes::Call{callee:"main".into()})],None);
    let e=execute(&p,"main",&[],Limits{max_call_depth:8,..Limits::default()});trapped(&e,"E_RESOURCE_LIMIT");assert_eq!(e.steps,8);
    let e=execute(&p,"main",&[],Limits::default());trapped(&e,"E_RESOURCE_LIMIT");assert_eq!(e.steps,128);
    p.functions[0].blocks[0].operations.clear();p.functions[0].blocks[0].terminator=Terminator::Return{entity_id:"return".into(),value:None,cleanup:vec![]};success(&execute(&p,"main",&[],Limits::default()),Value::unit());
}

fn typedef(id:&str,kind:TypeKind,parameters:Vec<String>)->TypeDef{TypeDef{entity_id:id.into(),kind,parameters,layout:Layout::Inferred,integer:None,fields:vec![],variants:vec![]}}

#[test]
fn captured_print_utf8_nul_and_lengths() {
    let mut print=op("print",Opcode::RuntimeCall,&["arg"],&[("printed","PrintResult")],Attributes::RuntimeCall{symbol:"print_string".into()});print.effects=vec![Effect::Process];
    let mut p=program("Usize",&[("arg","String")],vec![print,op("len",Opcode::RuntimeCall,&["arg"],&[("length","Usize")],Attributes::RuntimeCall{symbol:"string_len".into()})],Some("length"));
    p.functions[0].effects=vec![Effect::Process];
    let mut io=typedef("core.IoError",TypeKind::Sum,vec![]);io.variants=il_checker::runtime_error_variants("core.IoError").unwrap().iter().map(|name|il_graph::Variant{name:name.to_string(),fields:vec![]}).collect();
    p.types=vec![io,typedef("PrintResult",TypeKind::Result,vec!["Unit".into(),"core.IoError".into()])];
    if let Terminator::Return{cleanup,..}=&mut p.functions[0].blocks[0].terminator{cleanup.push(DropAction{value_id:"arg".into(),type_ref:"String".into()});}
    let e=execute(&p,"main",&[Value::string("中\0a")],Limits::default());success(&e,Value::integer("Usize",5));assert_eq!(e.stdout,"中\0a".as_bytes());assert_eq!(e.live_allocations,0);
    let e=execute(&p,"main",&[Value::string("中\0a")],Limits{max_output_bytes:44,..Limits::default()});trapped(&e,"E_RESOURCE_LIMIT");assert!(e.stdout.is_empty());
    p.functions[0].blocks[0].operations[0].outputs[0].type_ref="I64".into();let e=execute(&p,"main",&[Value::string("中\0a")],Limits::default());assert_eq!(e.status,ExecutionStatus::Rejected);assert!(e.stdout.is_empty());assert_eq!(e.steps,0);
}

#[test]
fn copy_drop_is_non_consuming_and_unit_calls_have_no_output() {
    let mut p=program("I64",&[("arg","I64")],vec![op("drop",Opcode::Drop,&["arg"],&[],Attributes::Empty{}),op("call",Opcode::Call,&[],&[],Attributes::Call{callee:"helper".into()})],Some("arg"));
    let mut helper=program("Unit",&[],vec![],None).functions.remove(0);helper.entity_id="helper".into();helper.name="helper".into();helper.blocks[0].entity_id="helper.entry".into();helper.blocks[0].terminator=Terminator::Return{entity_id:"helper.return".into(),value:None,cleanup:vec![]};p.functions.push(helper);
    success(&execute(&p,"main",&[Value::integer("I64",42)],Limits::default()),Value::integer("I64",42));
}

#[test]
fn opaque_external_values_rejected() {
    let mut p=program("Unit",&[("arg","Handle")],vec![],None);let mut handle=typedef("Handle",TypeKind::Record,vec![]);handle.layout=Layout::Opaque;p.types.push(handle);
    if let Terminator::Return{cleanup,..}=&mut p.functions[0].blocks[0].terminator{cleanup.push(DropAction{value_id:"arg".into(),type_ref:"Handle".into()});}
    let e=execute(&p,"main",&[Value{type_ref:"Handle".into(),data:ValueData::Record(vec![])}],Limits::default());assert_eq!(e.status,ExecutionStatus::Rejected);assert_eq!(e.diagnostics[0].code,"E_UNSUPPORTED_FEATURE");assert_eq!(e.steps,0);assert!(e.lifecycle.is_empty());
}

#[test]
fn strict_wire_data_and_required_nullable_execution_value() {
    for text in [r#"{"kind":"unit","value":null}"#,r#"{"kind":"unit","extra":1}"#,r#"{"kind":"integer","value":1}"#,r#"{"kind":"bool","value":true,"extra":1}"#] {assert!(serde_json::from_str::<ValueData>(text).is_err(),"accepted {text}");}
    assert_eq!(serde_json::from_str::<ValueData>(r#"{"kind":"unit"}"#).unwrap(),ValueData::Unit);
    let p=program("Unit",&[],vec![],None);let e=execute(&p,"main",&[],Limits::default());let mut json=serde_json::to_value(e).unwrap();json.as_object_mut().unwrap().remove("value");assert!(serde_json::from_value::<il_execution_model::Execution>(json).is_err());
}

#[test]
fn borrowed_and_cloned_owned_values_have_distinct_allocations() {
    let borrow=op("borrow",Opcode::Borrow,&["arg"],&[("view","String")],Attributes::Empty{});
    let mut clone=op("clone",Opcode::Clone,&["view"],&[("copy","String")],Attributes::Empty{});clone.effects=vec![Effect::Alloc];clone.produces=clone.outputs.clone();
    let mut p=program("String",&[("arg","String")],vec![borrow,clone,op("end",Opcode::EndBorrow,&["view"],&[],Attributes::Empty{})],Some("copy"));p.functions[0].effects=vec![Effect::Alloc];
    if let Terminator::Return{cleanup,..}=&mut p.functions[0].blocks[0].terminator{cleanup.push(DropAction{value_id:"arg".into(),type_ref:"String".into()});}
    let e=execute(&p,"main",&[Value::string("abc")],Limits::default());success(&e,Value::string("abc"));assert_eq!(e.peak_heap_bytes,6);assert_eq!(e.live_allocations,1);
    assert_eq!(e.lifecycle.iter().filter(|v|v.kind==LifecycleKind::Allocate).map(|v|v.allocation_ids.clone()).collect::<Vec<_>>(),vec![vec![1],vec![2]]);
    assert!(e.lifecycle.iter().any(|v|v.kind==LifecycleKind::Borrow&&v.allocation_ids==vec![1]));assert!(e.lifecycle.iter().any(|v|v.kind==LifecycleKind::Drop&&v.allocation_ids==vec![1]));
    trapped(&execute(&p,"main",&[Value::string("abc")],Limits{max_heap_bytes:5,..Limits::default()}),"E_RESOURCE_LIMIT");
}

#[test]
fn concat_result_and_typed_budget_failure() {
    let mut concat=op("concat",Opcode::RuntimeCall,&["a","b"],&[("joined","ConcatResult")],Attributes::RuntimeCall{symbol:"string_concat".into()});concat.effects=vec![Effect::Alloc];concat.produces=concat.outputs.clone();
    let mut p=program("ConcatResult",&[("a","String"),("b","String")],vec![concat],Some("joined"));p.functions[0].effects=vec![Effect::Alloc];
    let mut error=typedef("core.AllocError",TypeKind::Sum,vec![]);error.variants=il_checker::runtime_error_variants("core.AllocError").unwrap().iter().map(|name|il_graph::Variant{name:name.to_string(),fields:vec![]}).collect();p.types=vec![error,typedef("ConcatResult",TypeKind::Result,vec!["String".into(),"core.AllocError".into()])];
    if let Terminator::Return{cleanup,..}=&mut p.functions[0].blocks[0].terminator{*cleanup=vec![DropAction{value_id:"b".into(),type_ref:"String".into()},DropAction{value_id:"a".into(),type_ref:"String".into()}];}
    let e=execute(&p,"main",&[Value::string("你好"),Value::string("!")],Limits::default());
    success(&e,Value{type_ref:"ConcatResult".into(),data:ValueData::Variant(il_execution_model::VariantValue{tag:"Ok".into(),fields:vec![Value::string("你好!")]})});assert_eq!(e.peak_heap_bytes,14);assert_eq!(e.live_allocations,1);
    let e=execute(&p,"main",&[Value::string("你好"),Value::string("!")],Limits{max_heap_bytes:7,..Limits::default()});
    success(&e,Value{type_ref:"ConcatResult".into(),data:ValueData::Variant(il_execution_model::VariantValue{tag:"Err".into(),fields:vec![Value{type_ref:"core.AllocError".into(),data:ValueData::Variant(il_execution_model::VariantValue{tag:"OutOfMemory".into(),fields:vec![]})}]})});assert_eq!(e.live_allocations,0);
}

#[test]
fn loop_block_arguments_and_parameter_lifetimes() {
    let mut p=program("I64",&[("limit","I64"),("step","I64")],vec![op("zero",Opcode::Const,&[],&[("initial","I64")],Attributes::Constant{value:il_graph::Literal::Integer(0)})],None);
    let edge=|target:&str,args:&[&str]|Edge{target:target.into(),arguments:args.iter().map(|v|v.to_string()).collect(),cleanup:vec![]};
    p.functions[0].blocks[0].terminator=Terminator::Branch{entity_id:"start".into(),edge:edge("loop",&["initial"])};
    p.functions[0].blocks.extend([
        Block{entity_id:"loop".into(),arguments:vec![value("i","I64")],operations:vec![op("compare",Opcode::Lt,&["i","limit"],&[("again","Bool")],Attributes::Empty{})],terminator:Terminator::CondBranch{entity_id:"choose".into(),condition:"again".into(),then_edge:edge("body",&["i"]),else_edge:edge("done",&["i"])}},
        Block{entity_id:"body".into(),arguments:vec![value("current","I64")],operations:vec![op("add",Opcode::Add,&["current","step"],&[("next","I64")],Attributes::Empty{})],terminator:Terminator::Branch{entity_id:"repeat".into(),edge:edge("loop",&["next"])}},
        Block{entity_id:"done".into(),arguments:vec![value("answer","I64")],operations:vec![],terminator:Terminator::Return{entity_id:"done.return".into(),value:Some("answer".into()),cleanup:vec![]}}
    ]);
    let e=execute(&p,"main",&[Value::integer("I64",10),Value::integer("I64",1)],Limits::default());success(&e,Value::integer("I64",10));assert_eq!(e.steps,45);
    trapped(&execute(&p,"main",&[Value::integer("I64",10),Value::integer("I64",0)],Limits{max_steps:100,..Limits::default()}),"E_RESOURCE_LIMIT");
}

#[test]
fn locked_comparison_fixture() {
    let contract:serde_json::Value=serde_json::from_str(include_str!("../fixtures/execution/numeric-contracts.json")).unwrap();
    for case in contract["cases"].as_array().unwrap() {
        let ty=case["type"].as_str().unwrap();let opcode:Opcode=serde_json::from_value(case["opcode"].clone()).unwrap();
        let p=program("Bool",&[("a",ty),("b",ty)],vec![op("compare",opcode,&["a","b"],&[("answer","Bool")],Attributes::Empty{})],Some("answer"));
        let args=[Value::integer(ty,case["a"].as_str().unwrap().parse().unwrap()),Value::integer(ty,case["b"].as_str().unwrap().parse().unwrap())];
        success(&execute(&p,"main",&args,Limits::default()),Value::boolean(case["answer"].as_bool().unwrap()));
    }
}

#[test]
fn tuple_construction_projection_and_bool_not() {
    let mut p=program("Bool",&[("a","Bool"),("n","I64")],vec![op("tuple",Opcode::Tuple,&["a","n"],&[("pair","Pair")],Attributes::Empty{}),op("get",Opcode::TupleGet,&["pair"],&[("flag","Bool")],Attributes::TupleGet{index:0}),op("not",Opcode::Not,&["flag"],&[("answer","Bool")],Attributes::Empty{})],Some("answer"));p.types.push(typedef("Pair",TypeKind::Tuple,vec!["Bool".into(),"I64".into()]));
    success(&execute(&p,"main",&[Value::boolean(true),Value::integer("I64",42)],Limits::default()),Value::boolean(false));
}

#[test]
fn owned_variant_payload_and_tag() {
    let mut p=program("String",&[("choice","Maybe")],vec![],None);p.types.push(typedef("Maybe",TypeKind::Option,vec!["String".into()]));p.functions[0].effects=vec![Effect::Alloc];
    let edge=|target:&str,args:&[&str]|Edge{target:target.into(),arguments:args.iter().map(|v|v.to_string()).collect(),cleanup:vec![]};
    p.functions[0].blocks[0].terminator=Terminator::Switch{entity_id:"choose".into(),value:"choice".into(),cases:vec![il_mir::SwitchEdge{tag:"Some".into(),edge:edge("some",&["choice"])},il_mir::SwitchEdge{tag:"None".into(),edge:edge("none",&["choice"])}],default:edge("invalid",&[])};
    let mut payload=op("payload",Opcode::Payload,&["some.arg"],&[("inner","String")],Attributes::Empty{});payload.consumes=vec!["some.arg".into()];payload.produces=payload.outputs.clone();
    let mut drop=op("drop",Opcode::Drop,&["none.arg"],&[],Attributes::Empty{});drop.consumes=vec!["none.arg".into()];
    let mut constant=op("constant",Opcode::Const,&[],&[("text","String")],Attributes::Constant{value:il_graph::Literal::String("<none>".into())});constant.effects=vec![Effect::Alloc];constant.produces=constant.outputs.clone();
    p.functions[0].blocks.extend([
        Block{entity_id:"some".into(),arguments:vec![value("some.arg","Maybe")],operations:vec![payload],terminator:Terminator::Return{entity_id:"some.return".into(),value:Some("inner".into()),cleanup:vec![]}},
        Block{entity_id:"none".into(),arguments:vec![value("none.arg","Maybe")],operations:vec![drop,constant],terminator:Terminator::Return{entity_id:"none.return".into(),value:Some("text".into()),cleanup:vec![]}},
        Block{entity_id:"invalid".into(),arguments:vec![],operations:vec![],terminator:Terminator::Trap{entity_id:"trap".into(),code:"INVALID_TAG".into()}}
    ]);
    for (tag,fields,expected) in [("Some",vec![Value::string("payload")],"payload"),("None",vec![],"<none>")] {
        let arg=Value{type_ref:"Maybe".into(),data:ValueData::Variant(il_execution_model::VariantValue{tag:tag.into(),fields})};let e=execute(&p,"main",&[arg],Limits::default());success(&e,Value::string(expected));assert_eq!(e.live_allocations,1);
    }
    let mut p=program("U32",&[("arg","Maybe")],vec![op("tag",Opcode::Tag,&["arg"],&[("index","U32")],Attributes::Empty{})],Some("index"));p.types.push(typedef("Maybe",TypeKind::Option,vec!["I64".into()]));
    let arg=Value{type_ref:"Maybe".into(),data:ValueData::Variant(il_execution_model::VariantValue{tag:"Some".into(),fields:vec![Value::integer("I64",42)]})};success(&execute(&p,"main",&[arg],Limits::default()),Value::integer("U32",1));
}

#[test]
fn bytes_mutable_borrow_and_exact_capture_encoding_budget() {
    let mut p=program("Usize",&[("arg","Bytes")],vec![op("borrow",Opcode::BorrowMut,&["arg"],&[("view","Bytes")],Attributes::Empty{}),op("length",Opcode::RuntimeCall,&["view"],&[("len","Usize")],Attributes::RuntimeCall{symbol:"bytes_len".into()}),op("end",Opcode::EndBorrow,&["view"],&[],Attributes::Empty{})],Some("len"));
    if let Terminator::Return{cleanup,..}=&mut p.functions[0].blocks[0].terminator{cleanup.push(DropAction{value_id:"arg".into(),type_ref:"Bytes".into()});}
    let args=[Value{type_ref:"Bytes".into(),data:ValueData::Bytes(vec![0,255,1])}];let e=execute(&p,"main",&args,Limits::default());success(&e,Value::integer("Usize",3));assert_eq!(e.live_allocations,0);
    let cost=serde_json::to_vec(&e.lifecycle).unwrap().len()-2+serde_json::to_vec(e.value.as_ref().unwrap()).unwrap().len();
    success(&execute(&p,"main",&args,Limits{max_output_bytes:cost as u64,..Limits::default()}),Value::integer("Usize",3));
    trapped(&execute(&p,"main",&args,Limits{max_output_bytes:cost as u64-1,..Limits::default()}),"E_RESOURCE_LIMIT");
}
