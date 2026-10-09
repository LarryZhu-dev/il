use crate::*;
use il_graph::{Diagnostic,Opcode,Attributes,Literal};
use il_execution_model::{Value,ValueData};
use std::collections::BTreeMap;
mod helpers;
mod operations;

type Result<T>=std::result::Result<T,Vec<Diagnostic>>;
pub fn lower(mir:&il_mir::Program,mode:&BuildMode)->Result<Program>{
    let errors=il_mir::verify(mir);if !errors.is_empty(){return Err(errors);}
    let fail=|code:&str,message:&str|vec![Diagnostic::error(code,None,message,mir.source_revision)];
    if mir.target!="x86_64-unknown-linux-gnu"{return Err(fail("E_UNSUPPORTED_FEATURE","native target requires Linux x86-64"));}
    let layouts=layout::build(&mir.types).map_err(|m|fail("E_UNSUPPORTED_FEATURE",&m))?;
    let entry=match mode{BuildMode::Application{entry}|BuildMode::Captured{entry,..}=>entry};
    let function=mir.functions.iter().find(|f|&f.entity_id==entry).ok_or_else(||fail("E_NAME_NOT_FOUND","native entry function is undeclared"))?;
    if matches!(mode,BuildMode::Application{..}) && (function.result!="I32"||!function.parameters.is_empty()){return Err(fail("E_TYPE_MISMATCH","application entry must have signature () -> I32"));}
    if let BuildMode::Captured{arguments,limits,..}=mode {
        if arguments.len()!=function.parameters.len(){return Err(fail("E_TYPE_MISMATCH","native entry argument count mismatch"));}
        if !(1..=1_000_000).contains(&limits.max_steps)||!(1..=128).contains(&limits.max_call_depth)||!(1..=67_108_864).contains(&limits.max_heap_bytes)||!(1..=1_048_576).contains(&limits.max_output_bytes){return Err(fail("E_RESOURCE_LIMIT","invalid captured execution limits"));}
        let(mut nodes,mut heap)=(0,0);for(arg,param)in arguments.iter().zip(&function.parameters){validate_value(arg,&param.type_ref,&layouts,0,&mut nodes,&mut heap).map_err(|m|fail(if m=="argument shape budget exceeded"{"E_RESOURCE_LIMIT"}else{"E_TYPE_MISMATCH"},&m))?;}
        if heap>limits.max_heap_bytes{return Err(fail("E_RESOURCE_LIMIT","entry arguments exceed heap budget"));}
    }
    let input=il_graph::canonical_bytes(&(mir.hash().map_err(|e|fail("E_STATE_INCONSISTENT",&e.to_string()))?,mode)).map_err(|e|fail("E_STATE_INCONSISTENT",&e.to_string()))?;
    let mut lower=Lower{program:Program{schema_version:"1.0.0".into(),compiler_version:mir.compiler_version.clone(),input_hash:il_graph::hash_bytes(&input),source_revision:mir.source_revision,target:mir.target.clone(),mode:mode.clone(),layouts,globals:vec![],externs:runtime_externals(),functions:vec![]},global_index:BTreeMap::new()};
    for function in &mir.functions { let compiled=lower.function(function,mir)?;lower.program.functions.push(compiled); }
    lower.helpers()?;
    let main=lower.harness(function,mode)?;lower.program.functions.push(main);
    let errors=verify(&lower.program);if errors.is_empty(){Ok(lower.program)}else{Err(errors)}
}
pub(crate) struct Lower { program:Program,global_index:BTreeMap<Vec<u8>,String> }
pub(crate) struct Builder { function:Function,at:usize,counter:usize,entity:String }
impl Builder{
    fn new(symbol:String,entity:String,signature:Signature,parameters:Vec<String>)->Self{Self{function:Function{symbol,entity_id:entity.clone(),signature,parameters,blocks:vec![Block{name:"entry".into(),instructions:vec![],terminator:Terminator::Unreachable}]},at:0,counter:0,entity}}
    fn fresh(&mut self)->String{let result=format!("n{}",self.counter);self.counter+=1;result}
    fn emit(&mut self,operation:InstructionKind,has_result:bool)->Operand{let result=if has_result{Some(self.fresh())}else{None};self.function.blocks[self.at].instructions.push(Instruction{result:result.clone(),entity_id:self.entity.clone(),operation});result.map(Operand::reg).unwrap_or(Operand::Null)}
    fn block(&mut self,name:impl Into<String>)->usize{let at=self.function.blocks.len();self.function.blocks.push(Block{name:name.into(),instructions:vec![],terminator:Terminator::Unreachable});at}
    fn label(&mut self,prefix:&str)->String{format!("{prefix}_{}",self.fresh())}
    fn goto(&mut self,name:&str){self.function.blocks[self.at].terminator=Terminator::Branch{target:name.into()};}
    fn ret(&mut self,value:Option<Operand>){self.function.blocks[self.at].terminator=Terminator::Return{value};}
    fn alloca(&mut self,layout:&TypeLayout)->Operand{let name=self.fresh();self.function.blocks[0].instructions.push(Instruction{result:Some(name.clone()),entity_id:self.entity.clone(),operation:InstructionKind::Alloca{bytes:layout.size.max(1),align:layout.align}});Operand::reg(name)}
    fn offset(&mut self,pointer:Operand,bytes:u64)->Operand{if bytes==0{pointer}else{self.emit(InstructionKind::Offset{pointer,bytes},true)}}
    fn load(&mut self,pointer:Operand,ty:Type,align:u32)->Operand{self.emit(InstructionKind::Load{pointer,ty,align},true)}
    fn store(&mut self,pointer:Operand,value:Operand,align:u32){self.emit(InstructionKind::Store{pointer,value,align},false);}
    fn copy(&mut self,destination:Operand,source:Operand,bytes:u64){if bytes>0{self.emit(InstructionKind::Copy{destination,source,bytes},false);}}
    fn zero(&mut self,pointer:Operand,bytes:u64){if bytes>0{self.emit(InstructionKind::Zero{pointer,bytes},false);}}
    fn call(&mut self,symbol:impl Into<String>,arguments:Vec<Operand>,result:bool)->Operand{self.emit(InstructionKind::Call{symbol:symbol.into(),arguments},result)}
    fn rt(&mut self,name:&str,args:Vec<Operand>,result:bool)->Operand{self.call(format!("il_rt_{name}"),args,result)}
    fn cmp(&mut self,predicate:Predicate,left:Operand,right:Operand)->Operand{self.emit(InstructionKind::Compare{predicate,left,right},true)}
    fn cast(&mut self,value:Operand,source:u16,bits:u16,signed:bool)->Operand{if source==bits{value}else{self.emit(InstructionKind::Convert{operation:if bits<source{Conversion::Truncate}else if signed{Conversion::SignExtend}else{Conversion::ZeroExtend},value,bits},true)}}
}
impl Lower{
    fn error(&self,entity:&str,code:&str,message:&str)->Vec<Diagnostic>{vec![Diagnostic::error(code,Some(entity),message,self.program.source_revision)]}
    fn global(&mut self,bytes:impl AsRef<[u8]>)->Operand{let bytes=bytes.as_ref();let name=if let Some(name)=self.global_index.get(bytes){name.clone()}else{let name=format!("il_data_{}",self.program.globals.len());self.program.globals.push(Global{name:name.clone(),bytes:bytes.to_vec()});self.global_index.insert(bytes.to_vec(),name.clone());name};Operand::Global{name}}
    fn text(&mut self,text:&str)->(Operand,Operand){(self.global(text.as_bytes()),Operand::int(64,text.len()))}
    fn entity_call(&mut self,b:&mut Builder,name:&str,ctx:Operand,entity:&str){let(p,n)=self.text(entity);b.rt(name,vec![ctx,p,n],false);}
    fn trace(&mut self,b:&mut Builder,ctx:Operand,kind:u32,ty:&str,pointer:Operand,entity:&str){if !self.program.layouts[ty].owned && kind!=2 && kind!=3{return;}let(p,n)=self.text(entity);b.rt("trace_begin",vec![ctx.clone(),Operand::int(32,kind),p,n],false);b.call(helper("trace",ty),vec![ctx.clone(),pointer],false);b.rt("trace_end",vec![ctx],false);}
    fn drop(&mut self,b:&mut Builder,ctx:Operand,ty:&str,pointer:Operand,entity:&str){self.trace(b,ctx.clone(),4,ty,pointer.clone(),entity);if self.program.layouts[ty].owned{b.call(helper("drop",ty),vec![ctx,pointer],false);}}
    fn signature(&self,function:&il_mir::Function)->Signature{let result=&self.program.layouts[&function.result];let mut parameters=vec![Type::Ptr];if result.scalar().is_none()&&result.size>0{parameters.push(Type::Ptr);}for p in &function.parameters{let layout=&self.program.layouts[&p.type_ref];if layout.size>0{parameters.push(layout.scalar().unwrap_or(Type::Ptr));}}Signature{result:result.scalar().unwrap_or(Type::Void),parameters}}
    fn function(&mut self,function:&il_mir::Function,mir:&il_mir::Program)->Result<Function>{
        let signature=self.signature(function);let parameters=(0..signature.parameters.len()).map(|i|format!("p{i}")).collect();let mut b=Builder::new(symbol(&function.entity_id),function.entity_id.clone(),signature,parameters);let ctx=Operand::reg("p0");
        let result_layout=self.program.layouts[&function.result].clone();let mut parameter_index=if result_layout.scalar().is_none()&&result_layout.size>0{2}else{1};
        let mut slots=BTreeMap::new();let mut types=BTreeMap::new();
        for p in &function.parameters{let layout=self.program.layouts[&p.type_ref].clone();let slot=b.alloca(&layout);if layout.size>0{let input=Operand::reg(format!("p{parameter_index}"));parameter_index+=1;if layout.scalar().is_some(){b.store(slot.clone(),input,layout.align);}else{b.copy(slot.clone(),input,layout.size);}}slots.insert(p.entity_id.clone(),slot);types.insert(p.entity_id.clone(),p.type_ref.clone());}
        for block in &function.blocks{for value in block.arguments.iter().chain(block.operations.iter().flat_map(|op|op.outputs.iter())){let slot=b.alloca(&self.program.layouts[&value.type_ref]);slots.insert(value.entity_id.clone(),slot);types.insert(value.entity_id.clone(),value.type_ref.clone());}}
        self.entity_call(&mut b,"enter",ctx.clone(),&function.entity_id);
        let blocks:BTreeMap<_,_>=function.blocks.iter().enumerate().map(|(i,block)|(block.entity_id.clone(),format!("bb{i}"))).collect();b.goto(&blocks[&function.blocks[0].entity_id]);
        for block in &function.blocks{
            b.at=b.block(blocks[&block.entity_id].clone());
            for op in &block.operations{b.entity=op.entity_id.clone();self.entity_call(&mut b,"tick",ctx.clone(),&op.entity_id);self.operation(&mut b,ctx.clone(),op,&slots,&types,mir)?;}
            b.entity=block.terminator.entity_id().into();self.entity_call(&mut b,"tick",ctx.clone(),block.terminator.entity_id());
            match &block.terminator{
                il_mir::Terminator::Return{value,cleanup,entity_id}=>{
                    let returned=value.as_ref().map(|id|{self.trace(&mut b,ctx.clone(),1,&types[id],slots[id].clone(),entity_id);if let Some(ty)=result_layout.scalar(){Some(b.load(slots[id].clone(),ty,result_layout.align))}else{if result_layout.size>0{b.copy(Operand::reg("p1"),slots[id].clone(),result_layout.size);}None}}).flatten();
                    self.cleanup(&mut b,ctx.clone(),cleanup,&slots);
                    self.entity_call(&mut b,"location",ctx.clone(),entity_id);
                    if let Some(id)=value{self.trace(&mut b,ctx.clone(),5,&types[id],slots[id].clone(),entity_id);}
                    b.rt("leave",vec![ctx.clone()],false);b.ret(returned);
                }
                il_mir::Terminator::Trap{entity_id,code}=>self.trap(&mut b,ctx.clone(),"E_EXPLICIT_TRAP",&format!("explicit trap: {code}"),entity_id),
                il_mir::Terminator::Branch{entity_id,edge}=>self.edge(&mut b,ctx.clone(),entity_id,edge,&slots,&types,&blocks,function),
                il_mir::Terminator::CondBranch{entity_id,condition,then_edge,else_edge}=>{
                    let condition=b.load(slots[condition].clone(),Type::int(8),1);let condition=b.cmp(Predicate::Ne,condition,Operand::int(8,0));let yes=b.label("edge_yes");let no=b.label("edge_no");b.function.blocks[b.at].terminator=Terminator::CondBranch{condition,yes:yes.clone(),no:no.clone()};
                    b.at=b.block(yes);self.edge(&mut b,ctx.clone(),entity_id,then_edge,&slots,&types,&blocks,function);b.at=b.block(no);self.edge(&mut b,ctx.clone(),entity_id,else_edge,&slots,&types,&blocks,function);
                }
                il_mir::Terminator::Switch{entity_id,value,cases,default}=>{
                    let Shape::Sum{variants,..}=&self.program.layouts[&types[value]].shape else{unreachable!()};let variants=variants.clone();let tag=b.load(slots[value].clone(),Type::int(32),4);let mut native_cases=vec![];let mut work=vec![];
                    for case in cases{let label=b.label("edge_case");let index=variants.iter().position(|v|v.name==case.tag).unwrap();native_cases.push((index as u64,label.clone()));work.push((label,&case.edge));}let default_label=b.label("edge_default");b.function.blocks[b.at].terminator=Terminator::Switch{value:tag,cases:native_cases,default:default_label.clone()};
                    for(label,edge)in work{b.at=b.block(label);self.edge(&mut b,ctx.clone(),entity_id,edge,&slots,&types,&blocks,function);}b.at=b.block(default_label);self.edge(&mut b,ctx.clone(),entity_id,default,&slots,&types,&blocks,function);
                }
            }
        }Ok(b.function)
    }
    fn cleanup(&mut self,b:&mut Builder,ctx:Operand,cleanup:&[il_mir::DropAction],slots:&BTreeMap<String,Operand>){for action in cleanup{b.entity=action.value_id.clone();self.entity_call(b,"tick",ctx.clone(),&action.value_id);self.drop(b,ctx.clone(),&action.type_ref,slots[&action.value_id].clone(),&action.value_id);}}
    fn edge(&mut self,b:&mut Builder,ctx:Operand,entity:&str,edge:&il_mir::Edge,slots:&BTreeMap<String,Operand>,types:&BTreeMap<String,String>,blocks:&BTreeMap<String,String>,function:&il_mir::Function){
        let mut copies=vec![];for id in &edge.arguments{let layout=self.program.layouts[&types[id]].clone();self.trace(b,ctx.clone(),1,&types[id],slots[id].clone(),entity);let copy=if let Some(ty)=layout.scalar(){b.load(slots[id].clone(),ty,layout.align)}else{let temporary=b.alloca(&layout);b.copy(temporary.clone(),slots[id].clone(),layout.size);temporary};copies.push((copy,layout));}
        self.cleanup(b,ctx, &edge.cleanup,slots);let target=function.blocks.iter().find(|block|block.entity_id==edge.target).unwrap();for(argument,(value,layout))in target.arguments.iter().zip(copies){if layout.scalar().is_some(){b.store(slots[&argument.entity_id].clone(),value,layout.align);}else{b.copy(slots[&argument.entity_id].clone(),value,layout.size);}}b.goto(&blocks[&edge.target]);
    }
    fn trap(&mut self,b:&mut Builder,ctx:Operand,code:&str,message:&str,entity:&str){let(c,cn)=self.text(code);let(m,mn)=self.text(message);let(e,en)=self.text(entity);b.rt("trap",vec![ctx,c,cn,m,mn,e,en],false);b.function.blocks[b.at].terminator=Terminator::Unreachable;}
}
fn helper(operation:&str,ty:&str)->String{format!("il_{operation}_{}",symbol(ty))}

fn validate_value(value:&Value,ty:&str,table:&LayoutTable,depth:usize,nodes:&mut usize,heap:&mut u64)->std::result::Result<(),String>{
    *nodes+=1;if depth>64||*nodes>100_000{return Err("argument shape budget exceeded".into());}if value.type_ref!=ty{return Err("argument type mismatch".into());}
    let fields=match(&table[ty].shape,&value.data){
        (Shape::Unit,ValueData::Unit)|(Shape::Bool,ValueData::Bool(_))=>return Ok(()),
        (Shape::Integer{signed,bits},ValueData::Integer(value))=>{let n=value.parse::<i128>().map_err(|_|"invalid integer")?;let(min,max)=if *signed{(-(1i128<<(*bits-1)),(1i128<<(*bits-1))-1)}else{(0,(1i128<<*bits)-1)};if value!=&n.to_string()||n<min||n>max{return Err("integer argument out of range".into());}return Ok(());},
        (Shape::Buffer{utf8:true},ValueData::String(value))=>{*heap=heap.saturating_add(value.len() as u64);return Ok(());},
        (Shape::Buffer{utf8:false},ValueData::Bytes(value))=>{*heap=heap.saturating_add(value.len() as u64);return Ok(());},
        (Shape::Record{fields,..},ValueData::Record(values))|(Shape::Tuple{fields,..},ValueData::Tuple(values))=>(fields,values),
        (Shape::Sum{variants,..},ValueData::Variant(value))=>{let variant=variants.iter().find(|v|v.name==value.tag).ok_or("unknown variant")?;(&variant.fields,&value.fields)},
        _=>return Err("argument representation mismatch".into()),
    };if fields.0.len()!=fields.1.len(){return Err("argument arity mismatch".into());}for(ty,value)in fields.0.iter().zip(fields.1){validate_value(value,ty,table,depth+1,nodes,heap)?;}Ok(())
}
