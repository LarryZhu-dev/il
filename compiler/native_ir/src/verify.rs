use crate::*;
use std::collections::{BTreeMap,BTreeSet};
use il_graph::Diagnostic;
fn name(value:&str)->bool{!value.is_empty()&&value.len()<=1024&&value.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_')&&!value.as_bytes()[0].is_ascii_digit()}
fn legal(ty:&Type)->bool{matches!(ty,Type::Void|Type::Ptr)||matches!(ty,Type::Int{bits:1|8|16|32|64|128})}
pub fn verify(program:&Program)->Vec<Diagnostic>{
    let mut errors=vec![];let mut error=|entity:&str,message:&str|{if errors.len()<100{let mut d=Diagnostic::error("E_NATIVE_IR_INVALID",Some(entity),message,program.source_revision);d.stage="verify_native_ir".into();errors.push(d);}};
    if program.schema_version!="1.0.0"||program.target!="x86_64-unknown-linux-gnu"||program.input_hash.len()!=71||!program.input_hash.starts_with("sha256:")||!program.input_hash[7..].bytes().all(|b|b.is_ascii_hexdigit()){error("native","invalid Native IR envelope");}
    if program.functions.len()>4096||program.globals.len()>100_000||program.layouts.len()>300||program.functions.iter().map(|f|f.blocks.iter().map(|b|b.instructions.len()).sum::<usize>()).sum::<usize>()>500_000{return vec![Diagnostic::error("E_RESOURCE_LIMIT",None,"Native IR budget exceeded",program.source_revision)];}
    verify_authority(program,&mut error);
    let expected=runtime_externals(program.runtime_profile);if program.externs!=expected{error("native","external ABI is a closed exact runtime function table");}
    for(ty,layout)in &program.layouts{if layout.size>67_108_864||layout.align>8||!layout.align.is_power_of_two(){error(ty,"invalid bounded native layout");}}
    if !errors.is_empty(){return errors;}
    let mut error=|entity:&str,message:&str|{if errors.len()<100{let mut d=Diagnostic::error("E_NATIVE_IR_INVALID",Some(entity),message,program.source_revision);d.stage="verify_native_ir".into();errors.push(d);}};
    for(ty,layout)in &program.layouts{if il_graph::ResourceKind::from_nominal(ty)!=match layout.shape{Shape::Resource{resource}=>Some(resource),_=>None}{error(ty,"resource must retain its exact opaque nominal layout");}if layout::compute(layout.shape.clone(),&program.layouts).as_ref()!=Some(layout){error(ty,"invalid computed type layout");}}
    let mut globals=BTreeSet::new();for global in &program.globals{if !name(&global.name)||!globals.insert(global.name.clone())||global.bytes.len()>1_048_576{error(&global.name,"invalid or duplicate native global");}}
    let mut signatures:BTreeMap<String,Signature>=expected.into_iter().map(|e|(e.symbol,e.signature)).collect();for function in &program.functions{if !name(&function.symbol)||globals.contains(&function.symbol)||signatures.insert(function.symbol.clone(),function.signature.clone()).is_some(){error(&function.entity_id,"invalid or duplicate function symbol");}}
    match (&program.mode,program.runtime_profile) {
        (BuildMode::Application{..}|BuildMode::Captured{..},RuntimeProfile::Full)=>if signatures.get("main")!=Some(&Signature{result:Type::int(32),parameters:vec![]}){error("native","missing native main ABI");},
        (BuildMode::Application{..},RuntimeProfile::Minimal)=>if signatures.get("il_entry_main")!=Some(&Signature{result:Type::int(32),parameters:vec![]}){error("native","missing minimal entry ABI");},
        (BuildMode::Exports{entries},RuntimeProfile::None)=>{let mut seen=BTreeSet::new();if entries.is_empty(){error("native","exports cannot be empty");}for entry in entries{if !seen.insert(entry)||!program.functions.iter().any(|f|f.entity_id==*entry&&f.public)||signatures.get(&symbol(entry)).is_none_or(|s|s.result==Type::Ptr||s.parameters.contains(&Type::Ptr)){error(entry,"export requires unique scalar ABI");}}},
        _=>error("native","runtime and entry mode disagree")
    }
    for function in &program.functions{
        let entity=&function.entity_id;if function.blocks.is_empty()||function.blocks.len()>1024||function.parameters.len()!=function.signature.parameters.len(){error(entity,"invalid function parameter or block count");continue;}
        if !legal(&function.signature.result)||function.signature.parameters.iter().any(|t|!legal(t)||*t==Type::Void){error(entity,"invalid physical function signature");}
        let stack_bytes:u64=function.blocks.iter().flat_map(|b|&b.instructions).filter_map(|i|if let InstructionKind::Alloca{bytes,..}=i.operation{Some(bytes)}else{None}).fold(0u64,u64::saturating_add);
        if stack_bytes>32768{error(entity,"native stack frame exceeds 32768 byte compilation limit");}
        let mut definitions:BTreeMap<String,(Type,usize,usize)>=BTreeMap::new();for(param,ty)in function.parameters.iter().zip(&function.signature.parameters){if !name(param)||definitions.insert(param.clone(),(ty.clone(),usize::MAX,0)).is_some(){error(entity,"invalid or duplicate parameter register");}}
        let labels:BTreeMap<_,_>=function.blocks.iter().enumerate().map(|(i,b)|(b.name.clone(),i)).collect();if labels.len()!=function.blocks.len()||labels.keys().any(|n|!name(n)){error(entity,"invalid or duplicate block label");}
        let mut preds=vec![vec![];function.blocks.len()];
        for(index,block)in function.blocks.iter().enumerate(){for target in targets(&block.terminator){if let Some(target)=labels.get(target){preds[*target].push(index);}else{error(entity,"branch target is undefined");}}}
        let all:BTreeSet<_>=(0..function.blocks.len()).collect();let mut dom=vec![all.clone();function.blocks.len()];dom[0]=BTreeSet::from([0]);let mut changed=true;while changed{changed=false;for index in 1..dom.len(){let mut next=if preds[index].is_empty(){BTreeSet::new()}else{all.clone()};for p in &preds[index]{next=next.intersection(&dom[*p]).copied().collect();}next.insert(index);if next!=dom[index]{dom[index]=next;changed=true;}}}
        // All native operations are acyclic in their SSA definition order; slots carry loop state.
        for(index,block)in function.blocks.iter().enumerate(){for(position,instruction)in block.instructions.iter().enumerate(){
            if instruction.entity_id.is_empty(){error(entity,"instruction lacks debug entity");}
            let result=infer(&instruction.operation,&definitions,&globals,&signatures);
            match result{Ok(ty)=>{if let Some(register)=&instruction.result{if ty==Type::Void||!name(register)||definitions.insert(register.clone(),(ty,index,position)).is_some(){error(entity,"invalid or duplicate instruction result");}}else if ty!=Type::Void{error(entity,"value instruction requires a result");}},Err(message)=>error(entity,&message)}
        }}
        memory_safety(function,program,&definitions,&mut |m|error(entity,m));
        for(index,block)in function.blocks.iter().enumerate(){for(position,instruction)in block.instructions.iter().enumerate(){for operand in operands(&instruction.operation){check_use(operand,index,position,&definitions,&dom,&mut |m|error(entity,m));}}
            let mut check=|operand:&Operand|{check_use(operand,index,usize::MAX,&definitions,&dom,&mut |m|error(entity,m));operand_type(operand,&definitions,&globals)};
            match &block.terminator{
                Terminator::Return{value}=>{let result=value.as_ref().map(&mut check).transpose();if result.ok().flatten().unwrap_or(Type::Void)!=function.signature.result{error(entity,"return type differs from ABI");}},
                Terminator::CondBranch{condition,..}=>if check(condition)!=Ok(Type::int(1)){error(entity,"conditional branch requires i1");},
                Terminator::Switch{value,cases,..}=>{if !matches!(check(value),Ok(Type::Int{..})){error(entity,"switch requires integer");}if cases.iter().map(|c|c.0).collect::<BTreeSet<_>>().len()!=cases.len(){error(entity,"duplicate switch case");}},_=>{}
            }
        }
    }
    let mut calls=call_safety(program);errors.append(&mut calls);errors.truncate(100);errors
}

fn verify_authority(program:&Program,error:&mut impl FnMut(&str,&str)){
    if program.requirements.len()>256||program.requirements.windows(2).any(|pair|pair[0].entity_id>=pair[1].entity_id)||program.requirements.iter().any(|r|!il_graph::valid_id(&r.entity_id)||r.scope.as_ref().is_some_and(|s|s.len()>4096)){error("native","invalid canonical capability requirements");}
    if program.runtime_profile!=RuntimeProfile::Full{if !program.requirements.is_empty(){error("native","reduced profiles cannot require host authority");}return;}
    let bytes=il_graph::canonical_bytes(&program.requirements).unwrap_or_default();
    let mut authorizations=0;let mut contexts=0;
    for function in &program.functions{let calls:Vec<_>=function.blocks.iter().flat_map(|b|&b.instructions).filter_map(|i|if let InstructionKind::Call{symbol,arguments}=&i.operation{Some((symbol.as_str(),arguments))}else{None}).collect();
        if function.symbol=="main"{let entry_calls:Vec<_>=function.blocks.first().into_iter().flat_map(|b|&b.instructions).filter_map(|i|if let InstructionKind::Call{symbol,..}=&i.operation{Some(symbol.as_str())}else{None}).collect();if entry_calls.len()<3||entry_calls[0]!="il_rt_context_new"||entry_calls[1]!="il_rt_location"||entry_calls[2]!="il_rt_authorize"{error(&function.entity_id,"entry must authorize immediately after context and initial location before any branch");}}
        for(symbol,args)in calls{
            if symbol=="il_rt_context_new"{contexts+=1;}
            if symbol=="il_rt_authorize"{authorizations+=1;
                let valid=args.len()==4&&function.symbol=="main"&&matches!(&args[1],Operand::Global{name} if program.globals.iter().any(|g|g.name==*name&&g.bytes==bytes))&&args[2]==Operand::int(64,bytes.len())&&args[3]==Operand::int(32,4);
                if !valid{error(&function.entity_id,"authorization requires exact embedded requirements and inherited policy descriptor 4");}
            }
            let requirement=symbol.strip_prefix("il_rt_").and_then(il_checker::runtime_signature).and_then(|signature|signature.capability.map(|kind|(if matches!(signature.result,il_checker::RuntimeResult::Result{..}){2}else{1},kind)));
            if let Some((index,kind))=requirement{let selected=args.get(index).and_then(|arg|if let Operand::Integer{bits:64,value}=arg{value.parse::<usize>().ok()}else{None}).and_then(|index|program.requirements.get(index));if selected.is_none_or(|r|r.kind!=kind){error(&function.entity_id,"privileged runtime operation requires a constant matching requirement index");}}
        }
    }
    if authorizations!=1||contexts!=1{error("native","full runtime requires exactly one context and authorization");}
}

#[derive(Clone,Default,PartialEq,Eq)]
struct Requirement{bytes:u64,align:u32,write:bool}
#[derive(Clone)]
enum Address{Parameter(usize,u64),Allocation(u64,u32,bool),Context}
fn call_safety(program:&Program)->Vec<Diagnostic>{
    let mut errors=vec![];let mut report=|entity:&str,message:&str|{if errors.len()<100{let mut d=Diagnostic::error("E_NATIVE_IR_INVALID",Some(entity),message,program.source_revision);d.stage="verify_native_ir".into();errors.push(d);}};
    let mut requirements:BTreeMap<String,Vec<Requirement>>=program.functions.iter().map(|f|(f.symbol.clone(),vec![Requirement::default();f.parameters.len()])).collect();
    let address=|o:&Operand,map:&BTreeMap<String,Address>|match o{Operand::Register{name}=>map.get(name).cloned(),Operand::Global{name}=>program.globals.iter().find(|g|g.name==*name).map(|g|Address::Allocation(g.bytes.len().max(1)as u64,1,false)),_=>None};
    let constant=|o:&Operand|if let Operand::Integer{value,..}=o{value.parse::<u64>().ok()}else{None};
    // Minimum indirect-parameter extents flow backward through calls until stable.
    for pass in 0..=program.functions.len().min(512){
        let mut changed=false;
        for f in &program.functions{
            let mut own=requirements[&f.symbol].clone();let mut map=BTreeMap::new();let mut regtypes:BTreeMap<String,Type>=f.parameters.iter().cloned().zip(f.signature.parameters.iter().cloned()).collect();
            for(index,(name,ty))in f.parameters.iter().zip(&f.signature.parameters).enumerate(){if *ty==Type::Ptr{map.insert(name.clone(),if index==0&&program.runtime_profile==RuntimeProfile::Full{Address::Context}else{Address::Parameter(index,0)});}}
            for block in &f.blocks{for i in &block.instructions{
                if let InstructionKind::Checked{trap,..}|InstructionKind::CheckedCast{trap,..}=&i.operation{match(trap,program.runtime_profile){(TrapStrategy::Runtime{context},RuntimeProfile::Full)=>if !matches!(address(context,&map),Some(Address::Context)){report(&i.entity_id,"checked operation requires the live runtime context");},(TrapStrategy::Minimal,RuntimeProfile::Minimal)|(TrapStrategy::Intrinsic,RuntimeProfile::None)=>{},_=>report(&i.entity_id,"trap strategy disagrees with runtime profile")}}
                if matches!(i.operation,InstructionKind::Trap)&&program.runtime_profile!=RuntimeProfile::None{report(&i.entity_id,"intrinsic trap requires no-runtime profile");}
                let mut needs=vec![];let mut result_address=None;let mut result_type=None;
                match &i.operation{
                    InstructionKind::Trap=>{},
                    InstructionKind::Alloca{bytes,align}=>{result_address=Some(Address::Allocation(*bytes,*align,true));result_type=Some(Type::Ptr);},
                    InstructionKind::Offset{pointer,bytes}=>{result_address=match address(pointer,&map){Some(Address::Parameter(index,offset))=>Some(Address::Parameter(index,offset.saturating_add(*bytes))),Some(Address::Allocation(size,a,w))=>Some(Address::Allocation(size.saturating_sub(*bytes),if *bytes==0{a}else{a.min(1<<bytes.trailing_zeros().min(31))},w)),_=>{report(&i.entity_id,"offset cannot use a runtime context or unknown pointer");None}};result_type=Some(Type::Ptr);},
                    InstructionKind::Load{pointer,ty,align}=>{needs.push((pointer.clone(),size(ty),*align,false));result_type=Some(ty.clone());},
                    InstructionKind::Store{pointer,value,align}=>{let t=match value{Operand::Register{name}=>regtypes.get(name).cloned(),Operand::Integer{bits,..}=>Some(Type::int(*bits)),_=>Some(Type::Ptr)};needs.push((pointer.clone(),t.as_ref().map(size).unwrap_or(0),*align,true));},
                    InstructionKind::Copy{destination,source,bytes}=>{needs.push((destination.clone(),*bytes,1,true));needs.push((source.clone(),*bytes,1,false));},
                    InstructionKind::Zero{pointer,bytes}=>needs.push((pointer.clone(),*bytes,1,true)),
                    InstructionKind::Call{symbol,arguments}=>{
                        let Some(signature)=program.externs.iter().find(|e|e.symbol==*symbol).map(|e|&e.signature).or_else(||program.functions.iter().find(|f|f.symbol==*symbol).map(|f|&f.signature))else{continue};result_type=Some(signature.result.clone());
                        if symbol=="il_rt_context_new"{if f.symbol!="main"{report(&i.entity_id,"only the native entry may create a runtime context");}result_address=Some(Address::Context);}
                        else if program.runtime_profile==RuntimeProfile::Full&&arguments.first().is_none_or(|ctx|!matches!(address(ctx,&map),Some(Address::Context))){report(&i.entity_id,"call requires the live function runtime context");}
                        if let Some(callee)=requirements.get(symbol){for(argument,need)in arguments.iter().zip(callee){if need.bytes>0{needs.push((argument.clone(),need.bytes,need.align,need.write));}}}
                        else{
                            let mut add=|index:usize,bytes:u64,align:u32,write:bool|{if let Some(value)=arguments.get(index){needs.push((value.clone(),bytes,align,write));}};
                            match symbol.as_str(){
                                "il_rt_authorize"=>{if let Some(bytes)=arguments.get(2).and_then(constant){add(1,bytes,1,false);}else{report(&i.entity_id,"authorization bytes require static extent");}},
                                "il_rt_file_open_read"|"il_rt_file_open_write"=>{add(1,16,8,true);add(3,24,8,false);},
                                "il_rt_file_read"=>{add(1,24,8,true);add(3,24,8,false);},
                                "il_rt_file_write"=>{add(1,8,8,true);add(3,24,8,false);add(4,24,8,false);},
                                "il_rt_file_read_some"=>{add(1,24,8,true);add(2,16,8,false);add(4,16,8,false);},
                                "il_rt_file_write_some"=>{add(1,8,8,true);add(2,16,8,false);add(3,24,8,false);add(5,16,8,false);},
                                "il_rt_file_close"|"il_rt_file_drop"|"il_rt_trace_file"|"il_rt_json_file"|"il_rt_net_close_listener"|"il_rt_listener_drop"|"il_rt_trace_listener"|"il_rt_json_listener"|"il_rt_net_close_stream"|"il_rt_stream_drop"|"il_rt_trace_stream"|"il_rt_json_stream"=>add(1,16,8,false),
                                "il_rt_net_opened_at"=>add(1,16,8,false),
                                "il_rt_net_listen"=>add(1,16,8,true),
                                "il_rt_net_connect"=>{add(1,16,8,true);add(3,16,8,false);},
                                "il_rt_net_accept"=>{add(1,16,8,true);add(2,16,8,false);add(3,16,8,false);},
                                "il_rt_net_read"=>{add(1,24,8,true);add(2,16,8,false);add(4,16,8,false);},
                                "il_rt_net_write"=>{add(1,8,8,true);add(2,16,8,false);add(3,24,8,false);add(5,16,8,false);},
                                "il_rt_bytes_get"=>add(1,24,8,false),
                                "il_rt_bytes_equal"|"il_rt_string_equal"=>{add(1,24,8,false);add(2,24,8,false);},
                                "il_rt_bytes_slice"|"il_rt_string_to_bytes"|"il_rt_string_from_utf8"=>{add(1,24,8,true);add(2,24,8,false);},
                                "il_rt_bytes_concat"=>{add(1,24,8,true);add(2,24,8,false);add(3,24,8,false);},
                                "il_rt_bytes_from_u8"=>add(1,24,8,true),
                                "il_rt_stdin_read"=>{add(1,24,8,true);add(3,16,8,false);},
                                "il_rt_stdout_write"|"il_rt_stderr_write"=>{add(1,8,8,true);add(2,24,8,false);add(4,16,8,false);},
                                "il_rt_buffer_new"=>{add(1,24,8,true);let bytes=arguments.get(3).and_then(constant);if let Some(bytes)=bytes{add(2,bytes,1,false);}else{report(&i.entity_id,"buffer construction requires a constant source extent");}},
                                "il_rt_buffer_free"|"il_rt_trace_buffer"|"il_rt_print_buffer"=>add(1,24,8,symbol=="il_rt_buffer_free"),
                                "il_rt_buffer_clone"=>{add(1,24,8,true);add(2,24,8,false);},
                                "il_rt_json_buffer"=>add(1,24,8,false),
                                "il_rt_concat"=>{add(1,24,8,true);add(2,24,8,false);add(3,24,8,false);},
                                "il_rt_json_raw"|"il_rt_json_quoted"|"il_rt_json_bytes"=>{if let Some(bytes)=arguments.get(2).and_then(constant){add(1,bytes,1,false);}else{report(&i.entity_id,"raw JSON input requires a static proven extent");}},
                                _=>{}
                            }
                        }
                    }
                    InstructionKind::Binary{left,..}|InstructionKind::Checked{left,..}=>result_type=match left{Operand::Register{name}=>regtypes.get(name).cloned(),Operand::Integer{bits,..}=>Some(Type::int(*bits)),_=>None},
                    InstructionKind::Compare{..}=>result_type=Some(Type::int(1)),InstructionKind::Convert{bits,..}|InstructionKind::CheckedCast{bits,..}=>result_type=Some(Type::int(*bits)),
                }
                for(pointer,bytes,align,write)in needs{if bytes==0{continue;}match address(&pointer,&map){
                    Some(Address::Parameter(index,offset))=>{let need=&mut own[index];need.bytes=need.bytes.max(offset.saturating_add(bytes));need.align=need.align.max(align);need.write|=write;if need.bytes>67_108_864{report(&i.entity_id,"indirect parameter extent exceeds native limit");}},
                    Some(Address::Allocation(available,guarantee,writable))=>{if bytes>available||align>guarantee||write&&!writable{report(&i.entity_id,"call or memory access violates pointer extent, alignment or mutability");}},
                    _=>report(&i.entity_id,"memory use lacks a proven non-null allocation"),
                }}
                if let Some(name)=&i.result{if let Some(t)=result_type{regtypes.insert(name.clone(),t);}if let Some(a)=result_address{map.insert(name.clone(),a);}}
            }}
            if own!=requirements[&f.symbol]{requirements.insert(f.symbol.clone(),own);changed=true;}
        }
        if !changed{break;}if pass==program.functions.len().min(512){report("native","indirect memory requirements did not converge");}
    }errors
}
fn size(t:&Type)->u64{match t{Type::Void=>0,Type::Ptr=>8,Type::Int{bits}=>(*bits as u64).div_ceil(8)}}
type Definitions=BTreeMap<String,(Type,usize,usize)>;
pub(crate) fn operand_type(value:&Operand,defs:&Definitions,globals:&BTreeSet<String>)->Result<Type,String>{match value{
    Operand::Register{name}=>defs.get(name).map(|v|v.0.clone()).ok_or_else(||format!("undefined register {name}")),
    Operand::Global{name}=>if globals.contains(name){Ok(Type::Ptr)}else{Err("undefined global".into())},Operand::Null=>Ok(Type::Ptr),
    Operand::Integer{bits,value}=>{if !matches!(bits,1|8|16|32|64|128){return Err("invalid integer width".into());}let n=value.parse::<i128>().map_err(|_|"invalid integer literal")?;if n.to_string()!=*value{return Err("noncanonical integer literal".into());}if *bits<128&&(n< -(1i128<<(*bits-1))||n>((1i128<<*bits)-1)){return Err("integer constant does not fit width".into());}Ok(Type::int(*bits))}
}}
fn infer(op:&InstructionKind,defs:&Definitions,globals:&BTreeSet<String>,signatures:&BTreeMap<String,Signature>)->Result<Type,String>{
    let ty=|v:&Operand|operand_type(v,defs,globals);let pointer=|v:&Operand|->Result<(),String>{if ty(v)?==Type::Ptr{Ok(())}else{Err("memory address must be pointer".into())}};let aligned=|a:u32|if a.is_power_of_two()&&a<=16{Ok(())}else{Err("invalid memory alignment".to_string())};
    match op{
        InstructionKind::Trap=>Ok(Type::Void),
        InstructionKind::Alloca{bytes,align}=>{aligned(*align)?;if *bytes==0||*bytes>67_108_864{return Err("alloca size out of bounds".into());}Ok(Type::Ptr)},
        InstructionKind::Offset{pointer:p,bytes}=>{pointer(p)?;if *bytes>67_108_864{return Err("pointer offset out of bounds".into());}Ok(Type::Ptr)},
        InstructionKind::Load{pointer:p,ty:t,align}=>{pointer(p)?;aligned(*align)?;if !legal(t)||*t==Type::Void{return Err("invalid loaded type".into());}Ok(t.clone())},
        InstructionKind::Store{pointer:p,value,align}=>{pointer(p)?;ty(value)?;aligned(*align)?;Ok(Type::Void)},
        InstructionKind::Copy{destination,source,bytes}=>{pointer(destination)?;pointer(source)?;if *bytes>67_108_864{return Err("copy size out of bounds".into());}Ok(Type::Void)},
        InstructionKind::Zero{pointer:p,bytes}=>{pointer(p)?;if *bytes>67_108_864{return Err("zero size out of bounds".into());}Ok(Type::Void)},
        InstructionKind::Binary{left,right,..}|InstructionKind::Checked{left,right,..}=>{let t=ty(left)?;if !matches!(t,Type::Int{..})||ty(right)?!=t{return Err("binary integer types differ".into());}if let InstructionKind::Checked{trap,..}=op{if let TrapStrategy::Runtime{context}=trap{pointer(context)?;}if !matches!(t,Type::Int{bits:8|16|32|64}){return Err("checked integer width unsupported".into());}}Ok(t)},
        InstructionKind::Compare{left,right,..}=>{let t=ty(left)?;if !matches!(t,Type::Int{..})||ty(right)?!=t{return Err("comparison types differ".into());}Ok(Type::int(1))},
        InstructionKind::Convert{operation,value,bits}=>{let Type::Int{bits:source}=ty(value)?else{return Err("integer conversion needs integer".into())};if !legal(&Type::int(*bits))||match operation{Conversion::Truncate=>source<=*bits,_=>source>=*bits}{return Err("invalid conversion widths".into());}Ok(Type::int(*bits))},
        InstructionKind::CheckedCast{value,bits,trap,..}=>{if let TrapStrategy::Runtime{context}=trap{pointer(context)?;}if !matches!(ty(value)?,Type::Int{bits:8|16|32|64})||!matches!(bits,8|16|32|64){return Err("checked cast width invalid".into());}Ok(Type::int(*bits))},
        InstructionKind::Call{symbol,arguments}=>{let signature=signatures.get(symbol).ok_or("undefined callee")?;if arguments.len()!=signature.parameters.len(){return Err(format!("callee {symbol} argument count mismatch"));}for(value,expected)in arguments.iter().zip(&signature.parameters){if ty(value)?!=*expected{return Err(format!("callee {symbol} argument type mismatch"));}}Ok(signature.result.clone())},
    }
}
fn targets(term:&Terminator)->Vec<&str>{match term{Terminator::Branch{target}=>vec![target],Terminator::CondBranch{yes,no,..}=>vec![yes,no],Terminator::Switch{cases,default,..}=>cases.iter().map(|(_,target)|target.as_str()).chain(std::iter::once(default.as_str())).collect(),_=>vec![]}}
fn operands(op:&InstructionKind)->Vec<&Operand>{match op{InstructionKind::Alloca{..}|InstructionKind::Trap=>vec![],InstructionKind::Offset{pointer,..}|InstructionKind::Load{pointer,..}|InstructionKind::Zero{pointer,..}=>vec![pointer],InstructionKind::Store{pointer,value,..}=>vec![pointer,value],InstructionKind::Copy{destination,source,..}=>vec![destination,source],InstructionKind::Binary{left,right,..}|InstructionKind::Compare{left,right,..}=>vec![left,right],InstructionKind::Convert{value,..}=>vec![value],InstructionKind::Checked{left,right,trap,..}=>{let mut v=vec![left,right];if let TrapStrategy::Runtime{context}=trap{v.push(context);}v},InstructionKind::CheckedCast{value,trap,..}=>{let mut v=vec![value];if let TrapStrategy::Runtime{context}=trap{v.push(context);}v},InstructionKind::Call{arguments,..}=>arguments.iter().collect()}}
fn check_use(value:&Operand,block:usize,position:usize,defs:&Definitions,dom:&[BTreeSet<usize>],error:&mut impl FnMut(&str)){if let Operand::Register{name}=value{if let Some((_,owner,at))=defs.get(name){if *owner!=usize::MAX&&((*owner==block&&*at>=position)||(*owner!=block&&!dom[block].contains(owner))){error("register definition does not dominate its use");}}else{error("undefined SSA register");}}}

#[derive(Clone)]
struct Region { bytes:Option<u64>,align:u32,writable:bool }
fn memory_safety(function:&Function,program:&Program,definitions:&Definitions,error:&mut impl FnMut(&str)){
    let mut regions=BTreeMap::new();for(name,ty)in function.parameters.iter().zip(&function.signature.parameters){if *ty==Type::Ptr{regions.insert(name.clone(),Region{bytes:None,align:8,writable:true});}}
    let region=|operand:&Operand,regions:&BTreeMap<String,Region>|match operand{
        Operand::Register{name}=>regions.get(name).cloned(),Operand::Global{name}=>program.globals.iter().find(|g|&g.name==name).map(|g|Region{bytes:Some(g.bytes.len().max(1)as u64),align:1,writable:false}),_=>None
    };
    fn access(region:Option<Region>,bytes:u64,align:u32,write:bool,error:&mut impl FnMut(&str)){
        if let Some(r)=region{if r.bytes.is_some_and(|available|bytes>available){error("memory access exceeds allocation extent");}if align>r.align{error("memory alignment exceeds pointer guarantee");}if write&&!r.writable{error("cannot write immutable global data");}}else{error("memory access requires a known allocation or ABI parameter");}
    }
    for(block_index,block)in function.blocks.iter().enumerate(){for instruction in &block.instructions{
        let result=match &instruction.operation{
            InstructionKind::Alloca{bytes,align}=>{if block_index!=0{error("alloca must appear in function entry block");}Some(Region{bytes:Some(*bytes),align:*align,writable:true})},
            InstructionKind::Offset{pointer,bytes}=>region(pointer,&regions).map(|r|{if r.bytes.is_some_and(|size|*bytes>size){error("pointer offset exceeds allocation extent");}Region{bytes:r.bytes.map(|size|size.saturating_sub(*bytes)),align:if *bytes==0{r.align}else{r.align.min(1u32<<bytes.trailing_zeros().min(31))},writable:r.writable}}),
            InstructionKind::Load{pointer,ty,align}=>{let bytes=match ty{Type::Ptr=>8,Type::Int{bits}=>(*bits as u64).div_ceil(8),Type::Void=>0};access(region(pointer,&regions),bytes,*align,false,error);None},
            InstructionKind::Store{pointer,value,align}=>{let globals=program.globals.iter().map(|g|g.name.clone()).collect();let bytes=match operand_type(value,definitions,&globals){Ok(Type::Int{bits})=>(bits as u64).div_ceil(8),Ok(Type::Ptr)=>8,_=>0};access(region(pointer,&regions),bytes,*align,true,error);None},
            InstructionKind::Copy{destination,source,bytes}=>{access(region(destination,&regions),*bytes,1,true,error);access(region(source,&regions),*bytes,1,false,error);None},
            InstructionKind::Zero{pointer,bytes}=>{access(region(pointer,&regions),*bytes,1,true,error);None},
            _=>None,
        };if let(Some(name),Some(region))=(&instruction.result,result){regions.insert(name.clone(),region);}
    }}
}
