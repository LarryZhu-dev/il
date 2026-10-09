//! Direct LLVM 14 text emission from independently verified Native IR.
use il_native_ir::*;
use il_graph::Diagnostic;
use serde::{Serialize,Deserialize};
use std::collections::{BTreeMap,BTreeSet};
use std::fmt::Write;

#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct SourceLocation{pub function:String,pub entity_id:String,pub llvm_line:usize}
pub struct LlvmArtifact{pub ir:String,pub entity_map:Vec<SourceLocation>,pub stage:il_hir::StageRecord}
fn ty(t:&Type)->String{match t{Type::Void=>"void".into(),Type::Ptr=>"i8*".into(),Type::Int{bits}=>format!("i{bits}")}}
pub fn emit(program:&Program)->Result<LlvmArtifact,Vec<Diagnostic>>{
    let errors=verify(program);if !errors.is_empty(){return Err(errors);}
    let mut e=Emitter{ir:String::new(),map:vec![],types:BTreeMap::new(),globals:program.globals.iter().map(|g|(g.name.clone(),g.bytes.len().max(1))).collect(),signatures:program.externs.iter().map(|f|(f.symbol.clone(),f.signature.clone())).chain(program.functions.iter().map(|f|(f.symbol.clone(),f.signature.clone()))).collect(),next:0,metadata:vec![],debug:0,function:String::new(),trap_data:BTreeMap::new(),profile:program.runtime_profile,exports:match &program.mode{BuildMode::Exports{entries}=>entries.iter().map(|e|symbol(e)).collect(),_=>BTreeSet::new()}};
    e.ir.push_str("; Intelligent language direct LLVM backend\nsource_filename = \"program.ll\"\ntarget datalayout = \"e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128\"\ntarget triple = \"x86_64-unknown-linux-gnu\"\n");
    for global in &program.globals{writeln!(e.ir,"@{} = private unnamed_addr constant [{} x i8] c\"{}\", align 1",global.name,global.bytes.len().max(1),escaped(if global.bytes.is_empty(){&[0]}else{&global.bytes})).unwrap();}
    // Checked arithmetic traps use immutable strings, never host source generation.
    for(code,message)in [("E_INTEGER_OVERFLOW","checked integer operation trapped"),("E_DIVIDE_BY_ZERO","checked integer operation trapped"),("E_INVALID_SHIFT","checked integer operation trapped"),("E_INTEGER_OVERFLOW","integer cast out of range")]{e.trap_global(code);e.trap_global(message);}
    for function in &program.functions{for block in &function.blocks{for instruction in &block.instructions{if matches!(instruction.operation,InstructionKind::Checked{..}|InstructionKind::CheckedCast{..}){e.trap_global(&instruction.entity_id);}}}}
    for external in &program.externs{writeln!(e.ir,"declare {} @{}({}){}",ty(&external.signature.result),external.symbol,external.signature.parameters.iter().map(ty).collect::<Vec<_>>().join(", "),if external.noreturn{" noreturn"}else{""}).unwrap();}
    if program.runtime_profile!=RuntimeProfile::None{e.ir.push_str("declare void @llvm.memmove.p0i8.p0i8.i64(i8*, i8*, i64, i1)\ndeclare void @llvm.memset.p0i8.i64(i8*, i8, i64, i1)\n");}
    if program.runtime_profile==RuntimeProfile::None{e.ir.push_str("declare void @llvm.trap() cold noreturn nounwind\n");}
    for bits in [8,16,32,64]{for prefix in ["s","u"]{for op in ["add","sub","mul"]{writeln!(e.ir,"declare {{ i{bits}, i1 }} @llvm.{prefix}{op}.with.overflow.i{bits}(i{bits}, i{bits})").unwrap();}}}
    e.metadata.extend(["!0 = distinct !DICompileUnit(language: DW_LANG_C, file: !1, producer: \"Intelligent language\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)".into(),"!1 = !DIFile(filename: \"program.ll\", directory: \".\")".into(),"!2 = !{i32 2, !\"Dwarf Version\", i32 4}".into(),"!3 = !{i32 2, !\"Debug Info Version\", i32 3}".into(),"!4 = !DISubroutineType(types: !5)".into(),"!5 = !{}".into()]);
    for function in &program.functions{e.function(function);}
    e.ir.push_str("!llvm.dbg.cu = !{!0}\n!llvm.module.flags = !{!2, !3}\n");for line in &e.metadata{writeln!(e.ir,"{line}").unwrap();}
    let input_hash=program.hash().map_err(|err|vec![Diagnostic::error("E_STATE_INCONSISTENT",None,&err.to_string(),program.source_revision)])?;
    let stage=il_hir::StageRecord{stage:"codegen_x86_64".into(),input_hash,output_hash:il_graph::hash_bytes(e.ir.as_bytes()),compiler_version:program.compiler_version.clone(),diagnostics:vec![]};Ok(LlvmArtifact{ir:e.ir,entity_map:e.map,stage})
}
fn escaped(bytes:&[u8])->String{bytes.iter().map(|b|format!("\\{b:02X}")).collect()}
struct Emitter{ir:String,map:Vec<SourceLocation>,types:BTreeMap<String,Type>,globals:BTreeMap<String,usize>,signatures:BTreeMap<String,Signature>,next:usize,metadata:Vec<String>,debug:usize,function:String,trap_data:BTreeMap<String,(String,usize)>,profile:RuntimeProfile,exports:BTreeSet<String>}
impl Emitter{
    fn memory_loop(&mut self,destination:&Operand,source:Option<&Operand>,bytes:u64){
        if bytes==0{return;}
        let forward=self.label();let backward=self.label();let done=self.label();
        if let Some(source)=source{let condition=self.fresh();self.line(format!("{condition} = icmp ule {}, {}",self.typed(destination),self.operand(source)));self.line(format!("br i1 {condition}, label %{forward}, label %{backward}"));}else{self.line(format!("br label %{forward}"));}
        for(reverse,start)in [(false,forward),(true,backward)]{if reverse&&source.is_none(){continue;}writeln!(self.ir,"{start}:").unwrap();let body=self.label();self.line(format!("br label %{body}"));writeln!(self.ir,"{body}:").unwrap();let index=self.fresh();let next=self.fresh();self.line(format!("{index} = phi i64 [ {}, %{start} ], [ {next}, %{body} ]",if reverse{bytes-1}else{0}));let dst=self.fresh();self.line(format!("{dst} = getelementptr i8, {}, i64 {index}",self.typed(destination)));let value=if let Some(source)=source{let src=self.fresh();let value=self.fresh();self.line(format!("{src} = getelementptr i8, {}, i64 {index}",self.typed(source)));self.line(format!("{value} = load volatile i8, i8* {src}, align 1"));value}else{"0".into()};self.line(format!("store volatile i8 {value}, i8* {dst}, align 1"));self.line(format!("{next} = {} i64 {index}, 1",if reverse{"sub"}else{"add"}));let end=self.fresh();self.line(format!("{end} = icmp eq i64 {}, {}",if reverse{&index}else{&next},if reverse{0}else{bytes}));self.line(format!("br i1 {end}, label %{done}, label %{body}"));}
        writeln!(self.ir,"{done}:").unwrap();
    }
    fn fresh(&mut self)->String{let name=format!("%cg{}",self.next);self.next+=1;name}
    fn label(&mut self)->String{let name=format!("cg_block_{}",self.next);self.next+=1;name}
    fn line(&mut self,text:impl AsRef<str>){writeln!(self.ir,"  {}, !dbg !{}",text.as_ref(),self.debug).unwrap();}
    fn trap_global(&mut self,text:&str){if self.trap_data.contains_key(text){return;}let name=format!("il_trap_{}",self.trap_data.len());let size=text.len().max(1);writeln!(self.ir,"@{name} = private unnamed_addr constant [{size} x i8] c\"{}\", align 1",escaped(text.as_bytes())).unwrap();self.trap_data.insert(text.into(),(name,size));}
    fn text(&self,text:&str)->String{let(name,size)=&self.trap_data[text];format!("i8* getelementptr ([{size} x i8], [{size} x i8]* @{name}, i64 0, i64 0), i64 {}",text.len())}
    fn operand_type(&self,o:&Operand)->Type{match o{Operand::Register{name}=>self.types[name].clone(),Operand::Integer{bits,..}=>Type::int(*bits),_=>Type::Ptr}}
    fn operand(&self,o:&Operand)->String{match o{Operand::Register{name}=>format!("%{name}"),Operand::Integer{value,..}=>value.clone(),Operand::Null=>"null".into(),Operand::Global{name}=>{let size=self.globals[name];format!("getelementptr ([{size} x i8], [{size} x i8]* @{name}, i64 0, i64 0)")}}}
    fn typed(&self,o:&Operand)->String{format!("{} {}",ty(&self.operand_type(o)),self.operand(o))}
    fn function(&mut self,f:&Function){
        self.next=0;self.types.clear();self.function=f.symbol.clone();for(p,t)in f.parameters.iter().zip(&f.signature.parameters){self.types.insert(p.clone(),t.clone());}
        let scope=self.metadata.len();self.metadata.push(format!("!{scope} = distinct !DISubprogram(name: \"{}\", linkageName: \"{}\", scope: !1, file: !1, line: 1, type: !4, scopeLine: 1, spFlags: DISPFlagDefinition, unit: !0)",escaped(f.entity_id.as_bytes()),f.symbol));
        writeln!(self.ir,"\ndefine {}{} @{}({}) !dbg !{scope} {{",if self.profile==RuntimeProfile::None&&!self.exports.contains(&f.symbol){"internal "}else{""},ty(&f.signature.result),f.symbol,f.parameters.iter().zip(&f.signature.parameters).map(|(p,t)|format!("{} %{p}",ty(t))).collect::<Vec<_>>().join(", ")).unwrap();
        for block in &f.blocks{writeln!(self.ir,"{}:",block.name).unwrap();for instruction in &block.instructions{let line=self.ir.lines().count()+1;self.debug=self.metadata.len();self.metadata.push(format!("!{} = !DILocation(line: {}, column: 1, scope: !{scope})",self.debug,line));self.map.push(SourceLocation{function:f.symbol.clone(),entity_id:instruction.entity_id.clone(),llvm_line:line});self.instruction(instruction);}
            if block.instructions.is_empty(){self.debug=self.metadata.len();self.metadata.push(format!("!{} = !DILocation(line: 1, column: 1, scope: !{scope})",self.debug));}
            match &block.terminator{Terminator::Return{value}=>self.line(value.as_ref().map(|v|format!("ret {}",self.typed(v))).unwrap_or_else(||"ret void".into())),Terminator::Branch{target}=>self.line(format!("br label %{target}")),Terminator::CondBranch{condition,yes,no}=>self.line(format!("br {}, label %{yes}, label %{no}",self.typed(condition))),Terminator::Switch{value,cases,default}=>self.line(format!("switch {}, label %{default} [ {} ]",self.typed(value),cases.iter().map(|(n,l)|format!("{} {n}, label %{l}",ty(&self.operand_type(value)))).collect::<Vec<_>>().join(" "))),Terminator::Unreachable=>self.line("unreachable")};
        }self.ir.push_str("}\n");
    }
    fn instruction(&mut self,i:&Instruction){
        let result=i.result.as_ref().map(|n|format!("%{n}")).unwrap_or_default();let out=match &i.operation{
            InstructionKind::Alloca{bytes,align}=>{self.line(format!("{result} = alloca i8, i64 {bytes}, align {align}"));Type::Ptr},
            InstructionKind::Offset{pointer,bytes}=>{self.line(format!("{result} = getelementptr i8, {}, i64 {bytes}",self.typed(pointer)));Type::Ptr},
            InstructionKind::Load{pointer,ty:t,align}=>{let cast=self.fresh();self.line(format!("{cast} = bitcast {} to {}*",self.typed(pointer),ty(t)));self.line(format!("{result} = load {}, {}* {cast}, align {align}",ty(t),ty(t)));t.clone()},
            InstructionKind::Store{pointer,value,align}=>{let cast=self.fresh();let t=ty(&self.operand_type(value));self.line(format!("{cast} = bitcast {} to {t}*",self.typed(pointer)));self.line(format!("store {}, {t}* {cast}, align {align}",self.typed(value)));Type::Void},
            InstructionKind::Copy{destination,source,bytes}=>{if self.profile==RuntimeProfile::None{self.memory_loop(destination,Some(source),*bytes);}else{self.line(format!("call void @llvm.memmove.p0i8.p0i8.i64({}, {}, i64 {bytes}, i1 false)",self.typed(destination),self.typed(source)));}Type::Void},
            InstructionKind::Zero{pointer,bytes}=>{if self.profile==RuntimeProfile::None{self.memory_loop(pointer,None,*bytes);}else{self.line(format!("call void @llvm.memset.p0i8.i64({}, i8 0, i64 {bytes}, i1 false)",self.typed(pointer)));}Type::Void},
            InstructionKind::Binary{operation,left,right}=>{let op=match operation{Binary::Add=>"add",Binary::Sub=>"sub",Binary::Mul=>"mul",Binary::And=>"and",Binary::Or=>"or",Binary::Xor=>"xor"};self.line(format!("{result} = {op} {}, {}",self.typed(left),self.operand(right)));self.operand_type(left)},
            InstructionKind::Compare{predicate,left,right}=>{let pred=format!("{predicate:?}").to_lowercase();self.line(format!("{result} = icmp {pred} {}, {}",self.typed(left),self.operand(right)));Type::int(1)},
            InstructionKind::Convert{operation,value,bits}=>{let op=match operation{Conversion::Truncate=>"trunc",Conversion::SignExtend=>"sext",Conversion::ZeroExtend=>"zext"};self.line(format!("{result} = {op} {} to i{bits}",self.typed(value)));Type::int(*bits)},
            InstructionKind::Call{symbol,arguments}=>{let t=self.signatures[symbol].result.clone();self.line(format!("{}call {} @{symbol}({})",if t==Type::Void{String::new()}else{format!("{result} = ")},ty(&t),arguments.iter().map(|a|self.typed(a)).collect::<Vec<_>>().join(", ")));t},
            InstructionKind::Checked{operation,signed,left,right,trap}=>{self.checked(&result,*operation,*signed,left,right,trap,&i.entity_id);self.operand_type(left)},
            InstructionKind::CheckedCast{signed_source,signed_target,bits,value,trap}=>{self.checked_cast(&result,*signed_source,*signed_target,*bits,value,trap,&i.entity_id);Type::int(*bits)},
            InstructionKind::Trap=>{self.line("call void @llvm.trap()");Type::Void},
        };if let Some(name)=&i.result{self.types.insert(name.clone(),out);}
    }
    fn guard(&mut self,invalid:&str,strategy:&TrapStrategy,code:&str,message:&str,entity:&str){let fail=self.label();let good=self.label();self.line(format!("br i1 {invalid}, label %{fail}, label %{good}"));writeln!(self.ir,"{fail}:").unwrap();match strategy{TrapStrategy::Runtime{context}=>self.line(format!("call void @il_rt_trap({}, {}, {}, {})",self.typed(context),self.text(code),self.text(message),self.text(entity))),TrapStrategy::Minimal=>self.line(format!("call void @il_min_trap({}, {}, {})",self.text(code),self.text(message),self.text(entity))),TrapStrategy::Intrinsic=>self.line("call void @llvm.trap()")};self.line("unreachable");writeln!(self.ir,"{good}:").unwrap();}
    fn checked(&mut self,result:&str,op:Checked,signed:bool,left:&Operand,right:&Operand,ctx:&TrapStrategy,entity:&str){
        let Type::Int{bits}=self.operand_type(left)else{unreachable!()};let a=self.operand(left);let b=self.operand(right);let t=format!("i{bits}");
        match op{
            Checked::Add|Checked::Sub|Checked::Mul=>{let name=match op{Checked::Add=>"add",Checked::Sub=>"sub",_=>"mul"};let pair=self.fresh();let overflow=self.fresh();self.line(format!("{pair} = call {{ {t}, i1 }} @llvm.{}{name}.with.overflow.{t}({t} {a}, {t} {b})",if signed{"s"}else{"u"}));self.line(format!("{overflow} = extractvalue {{ {t}, i1 }} {pair}, 1"));self.guard(&overflow,ctx,"E_INTEGER_OVERFLOW","checked integer operation trapped",entity);self.line(format!("{result} = extractvalue {{ {t}, i1 }} {pair}, 0"));},
            Checked::Div|Checked::Rem=>{let zero=self.fresh();self.line(format!("{zero} = icmp eq {t} {b}, 0"));self.guard(&zero,ctx,"E_DIVIDE_BY_ZERO","checked integer operation trapped",entity);if signed{let min=self.fresh();let neg=self.fresh();let bad=self.fresh();self.line(format!("{min} = icmp eq {t} {a}, {}",-(1i128<<(bits-1))));self.line(format!("{neg} = icmp eq {t} {b}, -1"));self.line(format!("{bad} = and i1 {min}, {neg}"));self.guard(&bad,ctx,"E_INTEGER_OVERFLOW","checked integer operation trapped",entity);}let name=if matches!(op,Checked::Div){"div"}else{"rem"};self.line(format!("{result} = {}{name} {t} {a}, {b}",if signed{"s"}else{"u"}));},
            Checked::Shl|Checked::Shr=>{let invalid=self.fresh();self.line(format!("{invalid} = icmp uge {t} {b}, {bits}"));self.guard(&invalid,ctx,"E_INVALID_SHIFT","checked integer operation trapped",entity);if matches!(op,Checked::Shr){self.line(format!("{result} = {} {t} {a}, {b}",if signed{"ashr"}else{"lshr"}));}else{let wide=bits*2;let wa=self.fresh();let wb=self.fresh();let shifted=self.fresh();let back=self.fresh();let bad=self.fresh();self.line(format!("{wa} = {} {t} {a} to i{wide}",if signed{"sext"}else{"zext"}));self.line(format!("{wb} = zext {t} {b} to i{wide}"));self.line(format!("{shifted} = shl i{wide} {wa}, {wb}"));self.line(format!("{result} = trunc i{wide} {shifted} to {t}"));self.line(format!("{back} = {} {t} {result} to i{wide}",if signed{"sext"}else{"zext"}));self.line(format!("{bad} = icmp ne i{wide} {back}, {shifted}"));self.guard(&bad,ctx,"E_INTEGER_OVERFLOW","checked integer operation trapped",entity);}}
        }
    }
    fn checked_cast(&mut self,result:&str,signed_source:bool,signed_target:bool,bits:u16,value:&Operand,ctx:&TrapStrategy,entity:&str){
        let Type::Int{bits:source}=self.operand_type(value)else{unreachable!()};let wide=self.fresh();self.line(format!("{wide} = {} {} to i128",if signed_source{"sext"}else{"zext"},self.typed(value)));let(min,max)=if signed_target{(-(1i128<<(bits-1)),(1i128<<(bits-1))-1)}else{(0,(1i128<<bits)-1)};let low=self.fresh();let high=self.fresh();let bad=self.fresh();self.line(format!("{low} = icmp slt i128 {wide}, {min}"));self.line(format!("{high} = icmp sgt i128 {wide}, {max}"));self.line(format!("{bad} = or i1 {low}, {high}"));self.guard(&bad,ctx,"E_INTEGER_OVERFLOW","integer cast out of range",entity);if bits==source{self.line(format!("{result} = add {} , 0",self.typed(value)));}else{let op=if bits<source{"trunc"}else if signed_source{"sext"}else{"zext"};self.line(format!("{result} = {op} {} to i{bits}",self.typed(value)));}
    }
}
