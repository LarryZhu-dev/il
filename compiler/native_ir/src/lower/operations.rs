use super::*;
impl Lower{
    pub(super) fn operation(&mut self,b:&mut Builder,ctx:Operand,op:&il_graph::Operation,slots:&BTreeMap<String,Operand>,types:&BTreeMap<String,String>,mir:&il_mir::Program)->Result<()>{
        let output=op.outputs.first().map(|v|slots[&v.entity_id].clone());let output_type=op.outputs.first().map(|v|v.type_ref.as_str()).unwrap_or("Unit");
        let layout=self.program.layouts[output_type].clone();let input=op.inputs.iter().map(|id|slots[id].clone()).collect::<Vec<_>>();
        let load=|b:&mut Builder,index:usize,table:&LayoutTable|{let l=&table[&types[&op.inputs[index]]];b.load(input[index].clone(),l.scalar().unwrap(),l.align)};
        let scalar=match op.opcode{
            Opcode::Const=>{let Attributes::Constant{value}=&op.attributes else{unreachable!()};let data=match value{Literal::Unit(())=>ValueData::Unit,Literal::Bool(v)=>ValueData::Bool(*v),Literal::Integer(v)=>ValueData::Integer(v.to_string()),Literal::Unsigned(v)=>ValueData::Integer(v.to_string()),Literal::String(v)=>ValueData::String(v.clone()),Literal::Bytes(v)=>ValueData::Bytes(v.clone())};self.initialize(b,ctx.clone(),output.clone().unwrap(),&Value{type_ref:output_type.into(),data},&op.entity_id)?;None},
            Opcode::Add|Opcode::Sub|Opcode::Mul|Opcode::Div|Opcode::Rem|Opcode::Shl|Opcode::Shr=>{
                let operation=match op.opcode{Opcode::Add=>Checked::Add,Opcode::Sub=>Checked::Sub,Opcode::Mul=>Checked::Mul,Opcode::Div=>Checked::Div,Opcode::Rem=>Checked::Rem,Opcode::Shl=>Checked::Shl,Opcode::Shr=>Checked::Shr,_=>unreachable!()};
                let left=load(b,0,&self.program.layouts);let right=load(b,1,&self.program.layouts);Some(b.emit(InstructionKind::Checked{operation,signed:layout.integer().unwrap().0,left,right,trap:self.trap_strategy(ctx.clone())},true))
            }
            Opcode::BitAnd|Opcode::BitOr|Opcode::BitXor=>{let operation=match op.opcode{Opcode::BitAnd=>Binary::And,Opcode::BitOr=>Binary::Or,_=>Binary::Xor};let left=load(b,0,&self.program.layouts);let right=load(b,1,&self.program.layouts);Some(b.emit(InstructionKind::Binary{operation,left,right},true))},
            Opcode::Eq|Opcode::Ne|Opcode::Lt|Opcode::Le|Opcode::Gt|Opcode::Ge=>{
                let signed=self.program.layouts[&types[&op.inputs[0]]].integer().is_some_and(|v|v.0);let predicate=match(op.opcode,signed){(Opcode::Eq,_)=>Predicate::Eq,(Opcode::Ne,_)=>Predicate::Ne,(Opcode::Lt,true)=>Predicate::Slt,(Opcode::Le,true)=>Predicate::Sle,(Opcode::Gt,true)=>Predicate::Sgt,(Opcode::Ge,true)=>Predicate::Sge,(Opcode::Lt,false)=>Predicate::Ult,(Opcode::Le,false)=>Predicate::Ule,(Opcode::Gt,false)=>Predicate::Ugt,_=>Predicate::Uge};let left=load(b,0,&self.program.layouts);let right=load(b,1,&self.program.layouts);let test=b.cmp(predicate,left,right);Some(b.cast(test,1,8,false))
            }
            Opcode::Not=>{let value=load(b,0,&self.program.layouts);let bits=layout.scalar().unwrap();let Type::Int{bits}=bits else{unreachable!()};Some(b.emit(InstructionKind::Binary{operation:Binary::Xor,left:value,right:Operand::int(bits,if matches!(layout.shape,Shape::Bool){1}else{-1})},true))},
            Opcode::Cast=>{let source=&self.program.layouts[&types[&op.inputs[0]]];let(signed_source,_)=source.integer().unwrap();let(signed_target,bits)=layout.integer().unwrap();let value=load(b,0,&self.program.layouts);Some(b.emit(InstructionKind::CheckedCast{signed_source,signed_target,bits,value,trap:self.trap_strategy(ctx.clone())},true))},
            Opcode::Move|Opcode::Borrow|Opcode::BorrowMut=>{self.trace(b,ctx.clone(),if op.opcode==Opcode::Move{1}else{2},output_type,input[0].clone(),&op.entity_id);b.copy(output.clone().unwrap(),input[0].clone(),layout.size);None},
            Opcode::EndBorrow=>{self.trace(b,ctx.clone(),3,&types[&op.inputs[0]],input[0].clone(),&op.entity_id);None},
            Opcode::Clone=>{if !self.full(){b.copy(output.clone().unwrap(),input[0].clone(),layout.size);return Ok(());}b.rt("shape_begin",vec![ctx.clone(),Operand::int(32,1)],false);let(p,n)=self.text(&op.entity_id);b.call(helper("clone",output_type),vec![ctx.clone(),output.clone().unwrap(),input[0].clone(),p,n],false);None},
            Opcode::Drop=>{self.drop(b,ctx.clone(),&types[&op.inputs[0]],input[0].clone(),&op.entity_id);None},
            Opcode::Record|Opcode::Tuple|Opcode::Variant=>{
                let target=output.clone().unwrap();b.zero(target.clone(),layout.size);
                let(fields,offsets)=match &layout.shape{
                    Shape::Record{fields,offsets,..}|Shape::Tuple{fields,offsets}=>(fields.clone(),offsets.clone()),
                    Shape::Sum{payload,variants}=>{let Attributes::Variant{variant,..}=&op.attributes else{unreachable!()};let(index,v)=variants.iter().enumerate().find(|(_,v)|&v.name==variant).unwrap();b.store(target.clone(),Operand::int(32,index),4);(v.fields.clone(),v.offsets.iter().map(|n|n+payload).collect())},_=>unreachable!()};
                for(index,(ty,offset))in fields.iter().zip(offsets).enumerate(){self.trace(b,ctx.clone(),1,ty,input[index].clone(),&op.entity_id);let field=b.offset(target.clone(),offset);b.copy(field,input[index].clone(),self.program.layouts[ty].size);}None
            }
            Opcode::Field|Opcode::TupleGet=>{
                let source=&self.program.layouts[&types[&op.inputs[0]]];let offset=match(&source.shape,&op.attributes){(Shape::Record{names,offsets,..},Attributes::Field{field})=>offsets[names.iter().position(|name|name==field).unwrap()],(Shape::Tuple{offsets,..},Attributes::TupleGet{index})=>offsets[*index as usize],_=>unreachable!()};let field=b.offset(input[0].clone(),offset);b.copy(output.clone().unwrap(),field,layout.size);None
            }
            Opcode::Tag=>Some(b.load(input[0].clone(),Type::int(32),4)),
            Opcode::Payload=>{
                let source_type=&types[&op.inputs[0]];self.trace(b,ctx.clone(),1,source_type,input[0].clone(),&op.entity_id);let Shape::Sum{payload,variants}=&self.program.layouts[source_type].shape else{unreachable!()};let fields:Vec<_>=op.outputs.iter().map(|v|v.type_ref.clone()).collect();let variant=variants.iter().find(|v|v.fields==fields).unwrap();for(value,offset)in op.outputs.iter().zip(&variant.offsets){let source=b.offset(input[0].clone(),payload+offset);b.copy(slots[&value.entity_id].clone(),source,self.program.layouts[&value.type_ref].size);}None
            }
            Opcode::Call=>{
                let Attributes::Call{callee}=&op.attributes else{unreachable!()};let function=mir.functions.iter().find(|f|&f.entity_id==callee).unwrap();let mut arguments=if self.full(){vec![ctx.clone()]}else{vec![]};if layout.scalar().is_none()&&layout.size>0{arguments.push(output.clone().unwrap());}
                for(index,param)in function.parameters.iter().enumerate(){let source=self.program.layouts[&param.type_ref].clone();self.trace(b,ctx.clone(),1,&param.type_ref,input[index].clone(),&op.entity_id);if let Some(ty)=source.scalar(){arguments.push(b.load(input[index].clone(),ty,source.align));}else if source.size>0{arguments.push(input[index].clone());}}
                let result=b.call(symbol(callee),arguments,layout.scalar().is_some());self.entity_call(b,"location",ctx.clone(),&op.entity_id);if layout.scalar().is_some(){Some(result)}else{None}
            }
            Opcode::RuntimeCall=>{
                let Attributes::RuntimeCall{symbol,capability}=&op.attributes else{unreachable!()};match symbol.as_str(){
                    "string_len"|"bytes_len"=>{let pointer=b.offset(input[0].clone(),8);Some(b.load(pointer,Type::int(64),8))},
                    "print_i64"|"print_string"=>{let value=if symbol=="print_i64"{load(b,0,&self.program.layouts)}else{input[0].clone()};let status=if self.full(){b.rt(if symbol=="print_i64"{"print_i64"}else{"print_buffer"},vec![ctx.clone(),value],true)}else{b.call("il_min_print_i64",vec![value],true)};self.runtime_result(b,output.clone().unwrap(),&layout,status,None);None},
                    "string_concat"=>{let buffer=b.alloca(&self.program.layouts["String"]);let(p,n)=self.text(&op.entity_id);let status=b.rt("concat",vec![ctx.clone(),buffer.clone(),input[0].clone(),input[1].clone(),p,n],true);self.runtime_result(b,output.clone().unwrap(),&layout,status,Some(buffer));None},
                    "clock_now"=>{let index=self.program.requirements.iter().position(|r|Some(&r.entity_id)==capability.as_ref()).ok_or_else(||self.error(&op.entity_id,"E_CAPABILITY_MISSING","runtime requirement selector missing"))?;Some(b.rt("clock_now",vec![ctx.clone(),Operand::int(64,index)],true))},
                    "file_open_read"|"file_open_write"|"file_read"|"file_write"|"file_read_some"|"file_write_some"|"file_close"|"stdin_read"|"stdout_write"|"stderr_write"=>{
                        let Shape::Sum{variants,..}=&layout.shape else{unreachable!()};let success_ty=variants[0].fields[0].clone();let success=if self.program.layouts[&success_ty].size>0{Some(b.alloca(&self.program.layouts[&success_ty]))}else{None};
                        let mut arguments=vec![ctx.clone()];if let Some(pointer)=&success{arguments.push(pointer.clone());}
                        if matches!(symbol.as_str(),"file_open_read"|"file_open_write"|"file_read"|"file_write"){let index=self.program.requirements.iter().position(|r|Some(&r.entity_id)==capability.as_ref()).ok_or_else(||self.error(&op.entity_id,"E_CAPABILITY_MISSING","runtime requirement selector missing"))?;arguments.push(Operand::int(64,index));}
                        if symbol=="file_close"{self.trace(b,ctx.clone(),1,&types[&op.inputs[0]],input[0].clone(),&op.entity_id);}
                        for(index,pointer)in input.iter().enumerate(){let physical=&self.program.layouts[&types[&op.inputs[index]]];arguments.push(if let Some(ty)=physical.scalar(){b.load(pointer.clone(),ty,physical.align)}else{pointer.clone()});}
                        let status=b.rt(symbol,arguments,true);self.runtime_result(b,output.clone().unwrap(),&layout,status,success);None
                    },
                    _=>return Err(self.error(&op.entity_id,"E_UNSUPPORTED_FEATURE","runtime symbol has no native lowering")),
                }
            }
            _=>return Err(self.error(&op.entity_id,"E_SCHEMA_INVALID","terminator cannot be a native operation")),
        };
        if let Some(value)=scalar{b.store(output.clone().unwrap(),value,layout.align);}Ok(())
    }
    fn runtime_result(&mut self,b:&mut Builder,target:Operand,layout:&TypeLayout,status:Operand,success:Option<Operand>){
        let Shape::Sum{payload,variants}=&layout.shape else{unreachable!()};b.zero(target.clone(),layout.size);let yes=b.label("runtime_ok");let no=b.label("runtime_err");let done=b.label("runtime_done");let condition=b.cmp(Predicate::Eq,status.clone(),Operand::int(32,0));b.function.blocks[b.at].terminator=Terminator::CondBranch{condition,yes:yes.clone(),no:no.clone()};
        b.at=b.block(yes);if let Some(success)=success{let dest=b.offset(target.clone(),payload+variants[0].offsets[0]);b.copy(dest,success,self.program.layouts[&variants[0].fields[0]].size);}b.goto(&done);
        b.at=b.block(no);b.store(target.clone(),Operand::int(32,1),4);let error=b.offset(target,payload+variants[1].offsets[0]);let tag=b.emit(InstructionKind::Binary{operation:Binary::Sub,left:status,right:Operand::int(32,1)},true);b.store(error,tag,4);b.goto(&done);b.at=b.block(done);
    }
    pub(super) fn initialize(&mut self,b:&mut Builder,ctx:Operand,destination:Operand,value:&Value,entity:&str)->Result<()>{
        let layout=self.program.layouts[&value.type_ref].clone();b.zero(destination.clone(),layout.size);
        let fields=match(&layout.shape,&value.data){
            (Shape::Unit,ValueData::Unit)=>return Ok(()),
            (Shape::Bool,ValueData::Bool(value))=>{b.store(destination,Operand::int(8,*value as u8),1);return Ok(());},
            (Shape::Integer{bits,..},ValueData::Integer(value))=>{b.store(destination,Operand::int(*bits,value),layout.align);return Ok(());},
            (Shape::Buffer{..},ValueData::String(value))=>{let data=self.global(value);let(p,n)=self.text(entity);b.rt("buffer_new",vec![ctx,destination,data,Operand::int(64,value.len()),p,n],false);return Ok(());},
            (Shape::Buffer{..},ValueData::Bytes(value))=>{let data=self.global(value);let(p,n)=self.text(entity);b.rt("buffer_new",vec![ctx,destination,data,Operand::int(64,value.len()),p,n],false);return Ok(());},
            (Shape::Record{offsets,..},ValueData::Record(values))|(Shape::Tuple{offsets,..},ValueData::Tuple(values))=>(offsets.clone(),values),
            (Shape::Sum{payload,variants},ValueData::Variant(value))=>{let(index,variant)=variants.iter().enumerate().find(|(_,v)|v.name==value.tag).unwrap();b.store(destination.clone(),Operand::int(32,index),4);(variant.offsets.iter().map(|o|o+payload).collect(),&value.fields)},
            _=>return Err(self.error(entity,"E_TYPE_MISMATCH","native constant type mismatch")),
        };for(offset,value)in fields.0.iter().zip(fields.1){let field=b.offset(destination.clone(),*offset);self.initialize(b,ctx.clone(),field,value,entity)?;}Ok(())
    }
    pub(super) fn harness(&mut self,function:&il_mir::Function,mode:&BuildMode)->Result<Function>{
        if self.program.runtime_profile==RuntimeProfile::Minimal{let mut b=Builder::new("il_entry_main".into(),"native.startup".into(),Signature{result:Type::int(32),parameters:vec![]},vec![]);let result=b.call(symbol(&function.entity_id),vec![],true);b.ret(Some(result));return Ok(b.function);}
        let mut b=Builder::new("main".into(),"native.harness".into(),Signature{result:Type::int(32),parameters:vec![]},vec![]);
        let(captured,arguments,limits)=match mode{BuildMode::Application{..}=>(false,vec![],Limits::default()),BuildMode::Captured{arguments,limits,..}=>(true,arguments.clone(),*limits),BuildMode::Exports{..}=>unreachable!()};
        let ctx=b.rt("context_new",vec![Operand::int(32,captured as u32),Operand::int(64,limits.max_steps),Operand::int(32,limits.max_call_depth),Operand::int(64,limits.max_heap_bytes),Operand::int(64,limits.max_output_bytes),Operand::int(64,self.program.source_revision),Operand::int(32,if captured{3}else{-1})],true);
        self.entity_call(&mut b,"location",ctx.clone(),&function.entity_id);
        let requirements=il_graph::canonical_bytes(&self.program.requirements).map_err(|error|self.error(&function.entity_id,"E_STATE_INCONSISTENT",&error.to_string()))?;let length=requirements.len();let pointer=self.global(requirements);b.rt("authorize",vec![ctx.clone(),pointer,Operand::int(64,length),Operand::int(32,4)],false);
        let result_layout=self.program.layouts[&function.result].clone();let output=b.alloca(&result_layout);let mut args=vec![ctx.clone()];if result_layout.scalar().is_none()&&result_layout.size>0{args.push(output.clone());}
        for(arg,param)in arguments.iter().zip(&function.parameters){let layout=self.program.layouts[&param.type_ref].clone();let slot=b.alloca(&layout);self.initialize(&mut b,ctx.clone(),slot.clone(),arg,&param.entity_id)?;if let Some(ty)=layout.scalar(){args.push(b.load(slot,ty,layout.align));}else if layout.size>0{args.push(slot);}}
        let result=b.call(symbol(&function.entity_id),args,result_layout.scalar().is_some());if result_layout.scalar().is_some(){b.store(output.clone(),result.clone(),result_layout.align);}
        if captured{b.rt("shape_begin",vec![ctx.clone(),Operand::int(32,0)],false);b.call(helper("json",&function.result),vec![ctx.clone(),output],false);}
        b.rt("finish",vec![ctx],false);b.ret(Some(if captured{Operand::int(32,0)}else{result}));Ok(b.function)
    }
}
