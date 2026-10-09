use il_native_ir::*;
use il_execution_model::{Value,Limits};
fn program(source:&str)->Program{
    let graph=il_frontend::parse(source,0).unwrap();let entry=graph.functions[0].entity_id.clone();let hir=il_hir::lower(&graph).unwrap();let mir=il_mir::lower(&hir).unwrap();lower(&mir,&BuildMode::Captured{entry,arguments:vec![],limits:Limits::default()}).unwrap()
}
#[test]fn arithmetic_is_memory_explicit_and_hash_bound(){let p=program("module a { fn main()->I32 { return 2 + 3 * 4; } }");assert!(verify(&p).is_empty());assert!(p.functions[0].blocks.iter().flat_map(|b|&b.instructions).any(|i|matches!(i.operation,InstructionKind::Checked{..})));assert_eq!(p.hash().unwrap(),p.hash().unwrap());let text=serde_json::to_string(&p).unwrap();assert!(!text.contains("graph"));assert!(!text.contains("opcode"));}
#[test]fn reject_untrusted_external_and_undefined_call(){let mut p=program("module a { fn main()->I32 { return 0; } }");p.externs[0].symbol="system".into();assert!(!verify(&p).is_empty());let mut p=program("module a { fn main()->I32 { return 0; } }");p.functions[0].blocks[0].instructions.push(Instruction{result:None,entity_id:"attack".into(),operation:InstructionKind::Call{symbol:"system".into(),arguments:vec![]}});assert!(!verify(&p).is_empty());}
#[test]fn reject_result_type_dominance_and_names(){let p=program("module a { fn main()->I32 { return 0; } }");for name in ["bad name","x); system()","0start"]{let mut bad=p.clone();bad.functions[0].symbol=name.into();assert!(!verify(&bad).is_empty());}let mut bad=p;bad.functions[0].blocks[0].instructions.insert(0,Instruction{result:Some("early".into()),entity_id:"early".into(),operation:InstructionKind::Load{pointer:Operand::reg("undefined"),ty:Type::int(32),align:4}});assert!(!verify(&bad).is_empty());}
#[test]fn reject_tampered_layout_and_arity(){let mut p=program("module a { fn main()->I32 { return 0; } }");p.layouts.get_mut("I32").unwrap().size=8;assert!(!verify(&p).is_empty());let graph=il_frontend::parse("module a { fn f(x:I64)->I64 { return x; } }",0).unwrap();let mir=il_mir::lower(&il_hir::lower(&graph).unwrap()).unwrap();assert_eq!(lower(&mir,&BuildMode::Captured{entry:graph.functions[0].entity_id.clone(),arguments:vec![Value::integer("I8",1)],limits:Limits::default()}).unwrap_err()[0].code,"E_TYPE_MISMATCH");}
#[test]fn ownership_and_nominal_layout_lower(){let p=program("module a { fn main()->String effects [alloc] { let x:String=\"hello\"; let y:String=clone(x); drop(x); return y; } }");assert!(verify(&p).is_empty());assert_eq!(p.layouts["String"].size,24);assert!(p.functions.iter().any(|f|f.symbol.starts_with("il_drop_")));}
#[test]fn reject_poison_width_out_of_bounds_and_unknown_fields(){
    let original=program("module a { fn main()->I32 { return 1 + 2; } }");
    let mut p=original.clone();let instruction=p.functions[0].blocks.iter_mut().flat_map(|b|&mut b.instructions).find(|i|matches!(i.operation,InstructionKind::Checked{..})).unwrap();if let InstructionKind::Checked{left,right,..}=&mut instruction.operation{*left=Operand::int(128,1);*right=Operand::int(128,2);}assert!(!verify(&p).is_empty());
    let mut p=original.clone();p.functions[0].blocks[0].instructions.insert(1,Instruction{result:Some("overflow_pointer".into()),entity_id:"bad".into(),operation:InstructionKind::Offset{pointer:Operand::reg("n0"),bytes:1000}});assert!(!verify(&p).is_empty());
    let mut p=original.clone();p.layouts.get_mut("String").unwrap().size=u64::MAX;assert!(std::panic::catch_unwind(||verify(&p)).is_ok());assert!(!verify(&p).is_empty());
    let mut p=original.clone();let instruction=p.functions[0].blocks.iter_mut().flat_map(|b|&mut b.instructions).find(|i|matches!(i.operation,InstructionKind::Checked{..})).unwrap();if let InstructionKind::Checked{context,..}=&mut instruction.operation{*context=Operand::Null;}assert!(!verify(&p).is_empty());
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
    let p=lower(&mir,&BuildMode::Captured{entry:"app.identity".into(),arguments:vec![Value::string("owned")],limits:Limits{max_output_bytes:1,..Limits::default()}}).unwrap();
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
