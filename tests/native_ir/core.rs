use il_native_ir::*;
use il_execution_model::{Value,Limits};
fn full(mir:&il_mir::Program,mode:&BuildMode)->Result<Program,Vec<il_graph::Diagnostic>>{lower(mir,&BuildOptions{mode:mode.clone(),runtime_profile:RuntimeProfile::Full},&[])}
fn program(source:&str)->Program{
    let graph=il_frontend::parse(source,0).unwrap();let entry=graph.functions[0].entity_id.clone();let hir=il_hir::lower(&graph).unwrap();let mir=il_mir::lower(&hir).unwrap();full(&mir,&BuildMode::Captured{entry,arguments:vec![],limits:Limits::default()}).unwrap()
}
#[test]fn arithmetic_is_memory_explicit_and_hash_bound(){let p=program("module a { fn main()->I32 { return 2 + 3 * 4; } }");assert!(verify(&p).is_empty());assert!(p.functions[0].blocks.iter().flat_map(|b|&b.instructions).any(|i|matches!(i.operation,InstructionKind::Checked{..})));assert_eq!(p.hash().unwrap(),p.hash().unwrap());let text=serde_json::to_string(&p).unwrap();assert!(!text.contains("graph"));assert!(!text.contains("opcode"));}
#[test]fn reject_untrusted_external_and_undefined_call(){let mut p=program("module a { fn main()->I32 { return 0; } }");p.externs[0].symbol="system".into();assert!(!verify(&p).is_empty());let mut p=program("module a { fn main()->I32 { return 0; } }");p.functions[0].blocks[0].instructions.push(Instruction{result:None,entity_id:"attack".into(),operation:InstructionKind::Call{symbol:"system".into(),arguments:vec![]}});assert!(!verify(&p).is_empty());}
#[test]fn reject_result_type_dominance_and_names(){let p=program("module a { fn main()->I32 { return 0; } }");for name in ["bad name","x); system()","0start"]{let mut bad=p.clone();bad.functions[0].symbol=name.into();assert!(!verify(&bad).is_empty());}let mut bad=p;bad.functions[0].blocks[0].instructions.insert(0,Instruction{result:Some("early".into()),entity_id:"early".into(),operation:InstructionKind::Load{pointer:Operand::reg("undefined"),ty:Type::int(32),align:4}});assert!(!verify(&bad).is_empty());}
#[test]fn reject_tampered_layout_and_arity(){let mut p=program("module a { fn main()->I32 { return 0; } }");p.layouts.get_mut("I32").unwrap().size=8;assert!(!verify(&p).is_empty());let graph=il_frontend::parse("module a { fn f(x:I64)->I64 { return x; } }",0).unwrap();let mir=il_mir::lower(&il_hir::lower(&graph).unwrap()).unwrap();assert_eq!(full(&mir,&BuildMode::Captured{entry:graph.functions[0].entity_id.clone(),arguments:vec![Value::integer("I8",1)],limits:Limits::default()}).unwrap_err()[0].code,"E_TYPE_MISMATCH");}
#[test]fn ownership_and_nominal_layout_lower(){let p=program("module a { fn main()->String effects [alloc] { let x:String=\"hello\"; let y:String=clone(x); drop(x); return y; } }");assert!(verify(&p).is_empty());assert_eq!(p.layouts["String"].size,24);assert!(p.functions.iter().any(|f|f.symbol.starts_with("il_drop_")));}
#[test]fn reject_poison_width_out_of_bounds_and_unknown_fields(){
    let original=program("module a { fn main()->I32 { return 1 + 2; } }");
    let mut p=original.clone();let instruction=p.functions[0].blocks.iter_mut().flat_map(|b|&mut b.instructions).find(|i|matches!(i.operation,InstructionKind::Checked{..})).unwrap();if let InstructionKind::Checked{left,right,..}=&mut instruction.operation{*left=Operand::int(128,1);*right=Operand::int(128,2);}assert!(!verify(&p).is_empty());
    let mut p=original.clone();p.functions[0].blocks[0].instructions.insert(1,Instruction{result:Some("overflow_pointer".into()),entity_id:"bad".into(),operation:InstructionKind::Offset{pointer:Operand::reg("n0"),bytes:1000}});assert!(!verify(&p).is_empty());
    let mut p=original.clone();p.layouts.get_mut("String").unwrap().size=u64::MAX;assert!(std::panic::catch_unwind(||verify(&p)).is_ok());assert!(!verify(&p).is_empty());
    let mut p=original.clone();let instruction=p.functions[0].blocks.iter_mut().flat_map(|b|&mut b.instructions).find(|i|matches!(i.operation,InstructionKind::Checked{..})).unwrap();if let InstructionKind::Checked{trap,..}=&mut instruction.operation{*trap=TrapStrategy::Runtime{context:Operand::Null};}assert!(!verify(&p).is_empty());
    let mut json=serde_json::to_value(&original).unwrap();json["functions"][0]["blocks"][0]["instructions"][0].as_object_mut().unwrap().remove("result");assert!(serde_json::from_value::<Program>(json).is_err());
    let mut json=serde_json::to_value(&original).unwrap();json["unexpected"]=true.into();assert!(serde_json::from_value::<Program>(json).is_err());
}
#[test]fn indirect_calls_require_context_and_proven_buffer_extents(){
    let original=program("module a { fn main()->String effects [alloc] { let x:String=\"hello\"; return clone(x); } }");
    let mut bad=original.clone();let call=bad.functions[0].blocks.iter_mut().flat_map(|b|&mut b.instructions).find(|i|matches!(&i.operation,InstructionKind::Call{symbol,..} if symbol.starts_with("il_clone_"))).unwrap();if let InstructionKind::Call{arguments,..}=&mut call.operation{arguments[0]=Operand::Null;}assert!(!verify(&bad).is_empty());
    let mut bad=original.clone();let call=bad.functions[0].blocks.iter_mut().flat_map(|b|&mut b.instructions).find(|i|matches!(&i.operation,InstructionKind::Call{symbol,..} if symbol.starts_with("il_clone_"))).unwrap();if let InstructionKind::Call{arguments,..}=&mut call.operation{arguments[2]=Operand::Global{name:original.globals.iter().find(|g|g.bytes.len()==5).unwrap().name.clone()};}assert!(!verify(&bad).is_empty());
    let mut bad=original.clone();let call=bad.functions.iter_mut().flat_map(|f|&mut f.blocks).flat_map(|b|&mut b.instructions).find(|i|matches!(&i.operation,InstructionKind::Call{symbol,..} if symbol=="il_rt_buffer_new")).unwrap();if let InstructionKind::Call{arguments,..}=&mut call.operation{arguments[3]=Operand::int(64,1_000_000);}assert!(!verify(&bad).is_empty());
}
#[test]fn malformed_native_ir_mutations_never_panic(){
    let original=program("module a { fn main()->I32 { return 1; } }");
    for index in 0..128{let mut p=original.clone();match index%6{0=>p.layouts.get_mut("I32").unwrap().size=index as u64*100000000,1=>p.layouts.get_mut("I32").unwrap().align=index,2=>p.functions[0].signature.parameters.push(Type::Void),3=>p.functions[0].blocks[0].name=format!("x{index} bad"),4=>p.externs[0].signature.parameters.clear(),_=>p.input_hash="é".repeat(index as usize)}let result=std::panic::catch_unwind(||verify(&p));assert!(result.is_ok(),"mutation {index}");assert!(!result.unwrap().is_empty());}
}
#[test]fn raw_twenty_four_byte_data_keeps_byte_alignment(){
    let p=program("module a { fn main()->String effects [alloc] { return \"123456789012345678901234\"; } }");
    assert!(verify(&p).is_empty());
    let global=p.globals.iter().find(|g|g.bytes==b"123456789012345678901234").unwrap().name.clone();
    let mut p=p;let json=p.functions.iter_mut().flat_map(|f|&mut f.blocks).flat_map(|b|&mut b.instructions).find(|i|matches!(&i.operation,InstructionKind::Call{symbol,..} if symbol=="il_rt_json_quoted")).unwrap();
    if let InstructionKind::Call{arguments,..}=&mut json.operation{arguments[1]=Operand::Global{name:global};arguments[2]=Operand::int(64,24);}
    assert!(verify(&p).is_empty());
}
#[test]fn captured_argument_import_uses_entry_diagnostic_location(){
    let graph=il_frontend::parse("@id(\"app\") module app { @id(\"app.identity\") fn identity(@id(\"app.input\") value:String)->String { return value; } }",0).unwrap();
    let mir=il_mir::lower(&il_hir::lower(&graph).unwrap()).unwrap();
    let p=full(&mir,&BuildMode::Captured{entry:"app.identity".into(),arguments:vec![Value::string("owned")],limits:Limits{max_output_bytes:1,..Limits::default()}}).unwrap();
    let harness=p.functions.iter().find(|f|f.symbol=="main").unwrap();
    let calls:Vec<_>=harness.blocks.iter().flat_map(|b|&b.instructions).filter_map(|i|if let InstructionKind::Call{symbol,arguments}=&i.operation{Some((symbol.as_str(),arguments))}else{None}).collect();
    let create=calls.iter().position(|(s,_)|*s=="il_rt_context_new").unwrap();
    let location=calls.iter().position(|(s,_)|*s=="il_rt_location").unwrap();
    let import=calls.iter().position(|(s,_)|*s=="il_rt_buffer_new").unwrap();
    let entry=calls.iter().position(|(s,_)|*s==symbol("app.identity")).unwrap();
    assert!(create<location&&location<import&&import<entry);
    let Operand::Global{name}=&calls[location].1[1] else{panic!("entry location requires stable global")};
    assert_eq!(p.globals.iter().find(|g|g.name==*name).unwrap().bytes,b"app.identity");
    let Operand::Global{name}=&calls[import].1[4] else{panic!("allocation requires its parameter entity")};
    assert_eq!(p.globals.iter().find(|g|g.name==*name).unwrap().bytes,b"app.input");
    assert!(!calls[create+1..entry].iter().any(|(s,_)|matches!(*s,"il_rt_tick"|"il_rt_enter")));
}

fn host_graph(symbol:&str)->il_graph::Graph{
    use il_graph::*;
    let mut graph=il_frontend::parse("@id(\"app\") module app visibility public { @id(\"main\") fn main()->I32 { return 0; } }",0).unwrap();
    let signature=il_checker::runtime_signature(symbol).unwrap();
    graph.types.extend([
        TypeDef{entity_id:"core.File".into(),kind:TypeKind::Record,layout:Layout::Opaque,parameters:vec![],integer:None,fields:vec![Field{name:"slot".into(),type_ref:"U64".into()},Field{name:"generation".into(),type_ref:"U64".into()}],variants:vec![]},
        TypeDef{entity_id:"core.Deadline".into(),kind:TypeKind::Sum,layout:Layout::Inferred,parameters:vec![],integer:None,fields:vec![],variants:vec![Variant{name:"Infinite".into(),fields:vec![]},Variant{name:"At".into(),fields:vec!["U64".into()]}]},
        TypeDef{entity_id:"core.IoError".into(),kind:TypeKind::Sum,layout:Layout::Inferred,parameters:vec![],integer:None,fields:vec![],variants:il_checker::runtime_error_variants("core.IoError").unwrap().iter().map(|name|Variant{name:(*name).into(),fields:vec![]}).collect()},
    ]);
    graph.capabilities=vec![Capability{entity_id:"host.write".into(),kind:CapabilityKind::FileWrite,scope:Some("/tmp".into())},Capability{entity_id:"host.read".into(),kind:CapabilityKind::FileRead,scope:Some("/tmp".into())},Capability{entity_id:"host.clock".into(),kind:CapabilityKind::ClockRead,scope:None}];
    for kind in [ResourceKind::Listener,ResourceKind::Stream]{graph.types.push(TypeDef{entity_id:kind.nominal_type().into(),kind:TypeKind::Record,layout:Layout::Opaque,parameters:vec![],integer:None,fields:vec![Field{name:"slot".into(),type_ref:"U64".into()},Field{name:"generation".into(),type_ref:"U64".into()}],variants:vec![]});}
    if let Some(kind)=signature.capability{if matches!(kind,CapabilityKind::Listen|CapabilityKind::Connect){graph.capabilities.push(Capability{entity_id:"host.socket".into(),kind,scope:Some("127.0.0.1:8080".into())});}}
    let selected=signature.capability.and_then(|kind|graph.capabilities.iter().find(|c|c.kind==kind).map(|c|c.entity_id.clone()));
    let result=match signature.result{il_checker::RuntimeResult::Exact(ty)=>ty.to_owned(),il_checker::RuntimeResult::Result{success,error}=>{graph.types.push(TypeDef{entity_id:"app.Result".into(),kind:TypeKind::Result,layout:Layout::Inferred,parameters:vec![success.into(),error.into()],integer:None,fields:vec![],variants:vec![]});"app.Result".into()}};
    let parameters:Vec<_>=signature.parameters.iter().enumerate().map(|(index,p)|Parameter{entity_id:format!("host.input{index}"),name:format!("input{index}"),type_ref:p.type_ref.into()}).collect();
    let inputs=parameters.iter().map(|p|p.entity_id.clone()).collect();let owned=il_checker::is_owned_type(&graph,&result);let output=ValueDef{entity_id:"host.output".into(),type_ref:result.clone()};
    let consumes=parameters.iter().zip(signature.parameters).filter(|(_,p)|p.passing==il_checker::Passing::Owned).map(|(p,_)|p.entity_id.clone()).collect();
    let operation=Operation{entity_id:"host.call".into(),opcode:Opcode::RuntimeCall,inputs,outputs:vec![output.clone()],attributes:Attributes::RuntimeCall{symbol:symbol.into(),capability:selected.clone()},effects:signature.effects.to_vec(),consumes,produces:if owned{vec![output]}else{vec![]}};
    let mut effects=signature.effects.to_vec();if parameters.iter().any(|p|p.type_ref=="core.File")&&!effects.contains(&Effect::Fs){effects.push(Effect::Fs);}
    graph.functions.push(il_graph::Function{entity_id:"host".into(),name:"host".into(),parameters,result,effects,capabilities:selected.into_iter().collect(),contracts:vec![],blocks:vec![il_graph::Block{entity_id:"host.entry".into(),arguments:vec![],operations:vec![operation],terminator:Operation{entity_id:"host.return".into(),opcode:Opcode::Return,inputs:vec!["host.output".into()],outputs:vec![],attributes:Attributes::Empty{},effects:vec![],consumes:if owned{vec!["host.output".into()]}else{vec![]},produces:vec![]}}]});
    graph.modules[0].declarations.extend(graph.types.iter().map(|t|t.entity_id.clone()).chain(std::iter::once("host".into())));graph
}
fn lower_host(symbol:&str)->Program{let graph=host_graph(symbol);let hir=il_hir::lower_with_capabilities(&graph,&graph.capabilities).unwrap();let mir=il_mir::lower_with_capabilities(&hir,&graph.capabilities).unwrap();lower(&mir,&BuildOptions{mode:BuildMode::Application{entry:"main".into()},runtime_profile:RuntimeProfile::Full},&graph.capabilities).unwrap()}
#[test]
fn all_host_operations_have_closed_native_abi_and_resource_helpers(){
    for symbol_name in ["file_open_read","file_open_write","file_read_some","file_write_some","file_close","file_read","file_write","stdin_read","stdout_write","stderr_write","clock_now"]{
        let p=lower_host(symbol_name);assert!(verify(&p).is_empty());assert_eq!(p.requirements.iter().map(|r|r.entity_id.as_str()).collect::<Vec<_>>(),["host.clock","host.read","host.write"]);assert_eq!(p.layouts["core.File"].size,16);assert_eq!(p.layouts["core.Deadline"].size,16);
        assert!(p.functions.iter().flat_map(|f|&f.blocks).flat_map(|b|&b.instructions).any(|i|matches!(&i.operation,InstructionKind::Call{symbol,..}if symbol==&format!("il_rt_{symbol_name}"))));
        assert!(!p.functions.iter().any(|f|f.symbol==format!("il_clone_{}",symbol("core.File"))));
        if symbol_name.starts_with("file_open"){assert!(!p.functions.iter().any(|f|f.symbol==format!("il_clone_{}",symbol("app.Result"))));}
    }
}
#[test]
fn native_authorization_rejects_forged_tables_selectors_and_policy_inputs(){
    let original=lower_host("clock_now");
    for bad_index in [Operand::int(64,3),Operand::int(64,1),Operand::reg("p0")]{let mut bad=original.clone();let call=bad.functions.iter_mut().flat_map(|f|&mut f.blocks).flat_map(|b|&mut b.instructions).find(|i|matches!(&i.operation,InstructionKind::Call{symbol,..}if symbol=="il_rt_clock_now")).unwrap();if let InstructionKind::Call{arguments,..}=&mut call.operation{arguments[1]=bad_index;}assert!(!verify(&bad).is_empty());}
    for mutation in 0..5{let mut bad=original.clone();if mutation==0{bad.requirements.swap(0,1);}else{let main=bad.functions.iter_mut().find(|f|f.symbol=="main").unwrap();let call=main.blocks[0].instructions.iter_mut().find(|i|matches!(&i.operation,InstructionKind::Call{symbol,..}if symbol=="il_rt_authorize")).unwrap();if let InstructionKind::Call{symbol,arguments}=&mut call.operation{match mutation{1=>arguments[3]=Operand::int(32,0),2=>arguments[2]=Operand::int(64,0),3=>arguments[1]=Operand::Null,_=>*symbol="il_rt_location".into()}}}assert!(!verify(&bad).is_empty(),"mutation {mutation}");}
    let mut bad=original;let main=bad.functions.iter_mut().find(|f|f.symbol=="main").unwrap();let entry=&mut main.blocks[0].instructions;let auth=entry.iter().position(|i|matches!(&i.operation,InstructionKind::Call{symbol,..}if symbol=="il_rt_authorize")).unwrap();let call=entry.remove(auth);entry.push(call);assert!(!verify(&bad).is_empty());
}
#[test]
fn captured_resource_arguments_are_output_only_even_for_record_forgery(){
    use il_execution_model::{ValueData,ResourceValue};
    for (kind,close) in [(il_graph::ResourceKind::File,"file_close"),(il_graph::ResourceKind::Listener,"net_close_listener"),(il_graph::ResourceKind::Stream,"net_close_stream")]{
        let mut graph=host_graph(close);let function=graph.functions.iter_mut().find(|f|f.entity_id=="host").unwrap();function.blocks[0].operations.clear();function.result=kind.nominal_type().into();function.blocks[0].terminator.inputs=vec![function.parameters[0].entity_id.clone()];function.blocks[0].terminator.consumes=function.blocks[0].terminator.inputs.clone();
        let mir=il_mir::lower_with_capabilities(&il_hir::lower_with_capabilities(&graph,&graph.capabilities).unwrap(),&graph.capabilities).unwrap();
        for data in [ValueData::Resource(ResourceValue{kind,slot:0,generation:1}),ValueData::Record(vec![Value::integer("U64",0),Value::integer("U64",1)])]{let result=lower(&mir,&BuildOptions{runtime_profile:RuntimeProfile::Full,mode:BuildMode::Captured{entry:"host".into(),arguments:vec![Value{type_ref:kind.nominal_type().into(),data}],limits:Limits::default()}},&graph.capabilities);assert_eq!(result.unwrap_err()[0].code,"E_UNSUPPORTED_FEATURE");}
    }
}

#[test]
fn network_and_byte_primitives_preserve_nominal_abi_and_constant_authority(){
    for symbol_name in ["net_opened_at","net_listen","net_connect","net_accept","net_read","net_write","net_close_listener","net_close_stream","bytes_get","bytes_slice","bytes_concat","bytes_from_u8","string_to_bytes","string_from_utf8","bytes_equal","string_equal"]{
        let p=lower_host(symbol_name);assert!(verify(&p).is_empty());
        for kind in [il_graph::ResourceKind::File,il_graph::ResourceKind::Listener,il_graph::ResourceKind::Stream]{let layout=&p.layouts[kind.nominal_type()];assert_eq!(layout.size,16);assert_eq!(layout.align,8);assert_eq!(layout.shape,Shape::Resource{resource:kind});assert!(!p.functions.iter().any(|f|f.symbol==format!("il_clone_{}",symbol(kind.nominal_type()))));}
        if matches!(symbol_name,"bytes_equal"|"string_equal"){let signature=&p.externs.iter().find(|e|e.symbol==format!("il_rt_{symbol_name}")).unwrap().signature;assert_eq!(signature.result,Type::int(1));assert!(p.functions[1].blocks.iter().flat_map(|b|&b.instructions).any(|i|matches!(i.operation,InstructionKind::Convert{bits:8,..})));}
        if matches!(symbol_name,"net_listen"|"net_connect"){let mut bad=p.clone();let call=bad.functions.iter_mut().flat_map(|f|&mut f.blocks).flat_map(|b|&mut b.instructions).find(|i|matches!(&i.operation,InstructionKind::Call{symbol,..}if symbol==&format!("il_rt_{symbol_name}"))).unwrap();if let InstructionKind::Call{arguments,..}=&mut call.operation{arguments[2]=Operand::int(64,0);}assert!(!verify(&bad).is_empty());}
        let mut bad=p;bad.layouts.get_mut("net.Listener").unwrap().shape=Shape::Resource{resource:il_graph::ResourceKind::Stream};assert!(!verify(&bad).is_empty());
    }
}
