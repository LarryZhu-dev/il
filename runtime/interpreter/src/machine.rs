use crate::numbers;
use il_execution_model::*;
use il_graph::{Attributes, Diagnostic, Graph, Literal, Opcode, Operation, TypeDef, TypeKind, ResourceKind};
use il_mir::{DropAction, Program, Terminator};
use il_runtime_allocator::{AllocFailure,Allocator,OwnedBuffer};
use il_runtime_handles::{Deadline,HandleId,HostResources,IoError,IoFailure};
use il_runtime_startup::{HostPolicy,InheritedStdio,ValidatedPolicy};
use std::{collections::{BTreeMap, BTreeSet}, rc::Rc};

type Fault = (&'static str, String);
type Run<T> = Result<T, Fault>;
type Frame = BTreeMap<String, RValue>;

#[derive(Clone)]
struct RValue { ty: String, data: Rc<Data>, allocations: Vec<u64>, handles: Vec<(ResourceKind,HandleId)>, borrowed: bool }
enum Data {
    Unit, Bool(bool), Integer(i128), String(String), Bytes(Vec<u8>), Resource { kind:ResourceKind, handle:HandleId },
    Record(Vec<RValue>), Tuple(Vec<RValue>), Variant(String, Vec<RValue>),
}
impl RValue {
    fn scalar(ty: &str, data: Data) -> Self { Self { ty: ty.into(), data: Rc::new(data), allocations: vec![], handles: vec![], borrowed: false } }
    fn aggregate(ty: &str, data: Data, fields: &[RValue]) -> Self {
        Self { ty: ty.into(), data: Rc::new(data), allocations: fields.iter().flat_map(|v| v.allocations.iter().copied()).collect(), handles:fields.iter().flat_map(|v|v.handles.iter().copied()).collect(), borrowed: false }
    }
    fn integer(&self) -> Run<i128> { if let Data::Integer(v) = &*self.data { Ok(*v) } else { Err(("E_TYPE_MISMATCH", "integer value required".into())) } }
    fn boolean(&self) -> Run<bool> { if let Data::Bool(v) = &*self.data { Ok(*v) } else { Err(("E_TYPE_MISMATCH", "boolean value required".into())) } }
}

struct Types<'a> { definitions: BTreeMap<&'a str, &'a TypeDef>, owned: BTreeSet<String> }
impl<'a> Types<'a> {
    fn new(program: &'a Program) -> Self {
        let mut view = Graph::empty(); view.types = program.types.clone();
        let owned = program.types.iter().map(|ty| ty.entity_id.clone()).chain(["String".into(), "Bytes".into()])
            .filter(|name| il_checker::is_owned_type(&view, name)).collect();
        Self { definitions: program.types.iter().map(|t| (t.entity_id.as_str(), t)).collect(), owned }
    }
    fn integer(&self, ty: &str) -> Option<(bool, u8)> {
        match ty {
            "I8" => Some((true,8)), "I16" => Some((true,16)), "I32" => Some((true,32)), "I64" => Some((true,64)),
            "U8" => Some((false,8)), "U16" => Some((false,16)), "U32" => Some((false,32)), "U64" | "Usize" => Some((false,64)),
            _ => self.definitions.get(ty).and_then(|t| t.integer.as_ref()).map(|v| (v.signed,v.bits)),
        }
    }
    fn variants(&self, ty: &str) -> Option<Vec<(String,Vec<String>)>> {
        let t = self.definitions.get(ty)?;
        match t.kind {
            TypeKind::Sum => Some(t.variants.iter().map(|v| (v.name.clone(),v.fields.clone())).collect()),
            TypeKind::Option => Some(vec![("None".into(),vec![]),("Some".into(),t.parameters.clone())]),
            TypeKind::Result => Some(vec![("Ok".into(),vec![t.parameters[0].clone()]),("Err".into(),vec![t.parameters[1].clone()])]),
            _ => None,
        }
    }
    fn validate(&self, value: &Value, expected: &str, depth: usize, nodes: &mut usize, heap: &mut u64) -> Run<()> {
        *nodes += 1;
        if depth > 64 || *nodes > 100_000 { return Err(("E_RESOURCE_LIMIT", "argument shape budget exceeded".into())); }
        if matches!(&value.data,ValueData::Resource(_)) {return Err(("E_UNSUPPORTED_FEATURE","captured arguments cannot forge resource handles".into()));}
        if value.type_ref != expected { return Err(("E_TYPE_MISMATCH", format!("argument type must be {expected}"))); }
        if self.definitions.get(expected).is_some_and(|ty| ty.layout == il_graph::Layout::Opaque) {
            return Err(("E_UNSUPPORTED_FEATURE", "captured arguments cannot forge opaque resource handles".into()));
        }
        let fields: Option<(&[Value],Vec<String>)> = match &value.data {
            ValueData::Integer(text) => {
                let (signed,bits) = self.integer(expected).ok_or(("E_TYPE_MISMATCH", "integer type required".into()))?;
                let n = text.parse::<i128>().map_err(|_| ("E_TYPE_MISMATCH", "invalid integer argument".into()))?;
                if n.to_string() != *text || numbers::checked(n,signed,bits).is_err() { return Err(("E_TYPE_MISMATCH", "integer argument must be canonical and in range".into())); }
                None
            }
            ValueData::Unit if expected == "Unit" => None,
            ValueData::Bool(_) if expected == "Bool" => None,
            ValueData::String(s) if expected == "String" => { *heap = heap.saturating_add(s.len() as u64); None }
            ValueData::Bytes(b) if expected == "Bytes" => { *heap = heap.saturating_add(b.len() as u64); None }
            ValueData::Record(values) => {
                let ty = self.definitions.get(expected).filter(|t| t.kind == TypeKind::Record).ok_or(("E_TYPE_MISMATCH", "record type required".into()))?;
                Some((values,ty.fields.iter().map(|f| f.type_ref.clone()).collect()))
            }
            ValueData::Tuple(values) => {
                let ty = self.definitions.get(expected).filter(|t| t.kind == TypeKind::Tuple).ok_or(("E_TYPE_MISMATCH", "tuple type required".into()))?;
                Some((values,ty.parameters.clone()))
            }
            ValueData::Variant(v) => {
                let fs = self.variants(expected).and_then(|vs| vs.into_iter().find(|(tag,_)| *tag == v.tag)).ok_or(("E_TYPE_MISMATCH", "unknown variant argument".into()))?.1;
                Some((&v.fields,fs))
            }
            _ => return Err(("E_TYPE_MISMATCH", "argument representation does not match type".into())),
        };
        if let Some((values,expected)) = fields {
            if values.len() != expected.len() { return Err(("E_TYPE_MISMATCH", "argument field count mismatch".into())); }
            for (v,ty) in values.iter().zip(expected) { self.validate(v,&ty,depth+1,nodes,heap)?; }
        }
        Ok(())
    }
}

fn diagnostic(program: &Program, code: &str, entity: &str, cause: impl Into<String>) -> Diagnostic {
    let mut d = Diagnostic::error(code, Some(entity), cause, program.source_revision);
    d.stage = "interpreter".into(); d.retryable = false; d
}

/// Execute independently verified MIR with no privileged host grants.
pub fn execute(program: &Program, entry: &str, arguments: &[Value], limits: Limits) -> Execution {
    execute_with_policy(program,entry,arguments,limits,&HostPolicy::empty())
}

/// Execute verified MIR under an explicit trusted host policy.
pub fn execute_with_policy(program: &Program, entry: &str, arguments: &[Value], limits: Limits, policy:&HostPolicy) -> Execution {
    // The guest call-depth limit must not depend on the embedding thread's stack size.
    std::thread::scope(|scope|{
        match std::thread::Builder::new().name("il-interpreter".into()).stack_size(16 * 1024 * 1024)
            .spawn_scoped(scope,||execute_inner(program,entry,arguments,limits,policy)){
            Ok(worker)=>worker.join().unwrap_or_else(|_|Execution::rejected(vec![diagnostic(program,"E_STATE_INCONSISTENT",entry,"interpreter worker failed")])),
            Err(error)=>Execution::rejected(vec![diagnostic(program,"E_RESOURCE_LIMIT",entry,format!("interpreter worker unavailable: {error}"))]),
        }
    })
}
fn execute_inner(program: &Program, entry: &str, arguments: &[Value], limits: Limits, policy:&HostPolicy) -> Execution {
    let diagnostics = il_mir::verify_with_capabilities(program,&policy.grants);
    if !diagnostics.is_empty() { return Execution::rejected(diagnostics); }
    let reject = |code, cause: String| Execution::rejected(vec![diagnostic(program,code,entry,cause)]);
    if !limits.valid() { return reject("E_RESOURCE_LIMIT", "invalid interpreter limits".into()); }
    let Some(index) = program.functions.iter().position(|f| f.entity_id == entry) else { return reject("E_NAME_NOT_FOUND", "entry function not found".into()); };
    let function = &program.functions[index];
    if function.parameters.len() != arguments.len() { return reject("E_TYPE_MISMATCH", "entry argument count mismatch".into()); }
    let types = Types::new(program);
    let (mut nodes,mut heap) = (0,0);
    for (arg,param) in arguments.iter().zip(&function.parameters) {
        if contains_resource(arg){return reject("E_UNSUPPORTED_FEATURE","captured arguments cannot forge resource handles".into());}
        if let Err((code,cause)) = types.validate(arg,&param.type_ref,0,&mut nodes,&mut heap) { return reject(code,cause); }
    }
    if heap > limits.max_heap_bytes { return reject("E_RESOURCE_LIMIT", "entry arguments exceed heap budget".into()); }
    let checked=match ValidatedPolicy::new(policy.clone(),&program.capabilities,true){Ok(v)=>v,Err(e)=>return reject(e.code,e.message)};
    let allocator=Allocator::new(checked.faults(),Some(limits.max_heap_bytes));
    let stdio=match InheritedStdio::captured_output(){Ok(v)=>v,Err(e)=>return reject(e.code,e.message)};
    let host=HostResources::new(checked,stdio,allocator.clone());
    let mut machine = Machine { program,types,limits,steps:0,allocator,host,next_allocation:1,allocations:BTreeMap::new(),
        stdout:vec![],stderr:vec![],lifecycle:vec![],handle_events:vec![],stack:vec![],stack_bytes:2,captured:0,current:"entry".into() };
    let result = (|| {
        machine.location(entry)?;
        let args = arguments.iter().zip(&function.parameters).map(|(v,p)| machine.import(v,&p.entity_id)).collect::<Run<Vec<_>>>()?;
        let result = machine.call(index,args,1)?;
        let mut nodes = 0;
        machine.export(&result,0,&mut nodes)
    })();
    let (status,value,diagnostics) = match result {
        Ok(value) => (ExecutionStatus::Returned,Some(value),vec![]),
        Err((code,cause)) => (ExecutionStatus::Trapped,None,vec![diagnostic(program,code,&machine.current,cause)]),
    };
    Execution { status,value,stdout:machine.stdout,stderr:machine.stderr,diagnostics,steps:machine.steps,
        peak_heap_bytes:machine.allocator.peak_bytes(),live_allocations:machine.allocator.live_allocations(),lifecycle:machine.lifecycle,
        live_handles:machine.host.live_handles(),handle_events:machine.handle_events,stack_trace:machine.stack.into_iter().rev().collect() }
}

struct Machine<'a> {
    program: &'a Program, types: Types<'a>, limits: Limits, steps:u64, allocator:Allocator, host:HostResources,
    next_allocation:u64, allocations:BTreeMap<u64,OwnedBuffer>, stdout:Vec<u8>, stderr:Vec<u8>, lifecycle:Vec<LifecycleEvent>,
    handle_events:Vec<HandleEvent>,stack:Vec<StackFrame>,stack_bytes:u64,captured:u64, current:String,
}
impl Machine<'_> {
    fn capture(&mut self, bytes:u64) -> Run<()> {
        if bytes > self.limits.max_output_bytes.saturating_sub(self.captured) { return Err(("E_RESOURCE_LIMIT","capture budget exhausted".into())); }
        self.captured += bytes; Ok(())
    }
    fn event(&mut self, kind:LifecycleKind, entity:&str, ids:&[u64]) -> Run<()> {
        let event=LifecycleEvent { kind, entity_id:entity.into(), allocation_ids:ids.to_vec() };
        let bytes=serde_json::to_vec(&event).map_err(|_|("E_STATE_INCONSISTENT","lifecycle serialization failed".into()))?.len() as u64;
        self.capture(bytes + u64::from(!self.lifecycle.is_empty()))?;
        self.lifecycle.push(event); Ok(())
    }
    fn location(&mut self,entity:&str)->Run<()> {
        let fault=||("E_RESOURCE_LIMIT","stack trace budget exhausted".into());
        if let Some(frame)=self.stack.last_mut(){
            let old=stack_frame_json_bytes(&frame.function_id,frame.call_site.as_deref(),&frame.entity_id);
            let new=stack_frame_json_bytes(&frame.function_id,frame.call_site.as_deref(),entity);
            self.stack_bytes=stack_trace_replace_bytes(self.stack_bytes,old,new).ok_or_else(fault)?;
            frame.entity_id=entity.into();
        }else if json_string_bytes(entity)>STACK_TRACE_MAX_BYTES{return Err(fault())}
        self.current=entity.into();Ok(())
    }
    fn enter(&mut self,function:&str,depth:u32)->Run<()> {
        let call_site=if depth==1{None}else{Some(self.current.clone())};
        let bytes=stack_frame_json_bytes(function,call_site.as_deref(),function);
        self.stack_bytes=stack_trace_push_bytes(self.stack_bytes,self.stack.len(),bytes).ok_or(("E_RESOURCE_LIMIT","stack trace budget exhausted".into()))?;
        self.stack.push(StackFrame{function_id:function.into(),call_site,entity_id:function.into()});self.current=function.into();Ok(())
    }
    fn leave(&mut self){
        let old_depth=self.stack.len();
        if let Some(frame)=self.stack.pop(){self.stack_bytes-=stack_frame_json_bytes(&frame.function_id,frame.call_site.as_deref(),&frame.entity_id)+u64::from(old_depth>1);}
    }
    fn tick(&mut self, entity:&str) -> Run<()> {
        self.location(entity)?;
        if self.steps >= self.limits.max_steps { return Err(("E_RESOURCE_LIMIT","instruction budget exhausted".into())); }
        self.steps += 1; Ok(())
    }
    fn check_allocation_event(&self,entity:&str)->Run<()> {
        let event=LifecycleEvent{kind:LifecycleKind::Allocate,entity_id:entity.into(),allocation_ids:vec![self.next_allocation]};
        let bytes=serde_json::to_vec(&event).map_err(|_|("E_STATE_INCONSISTENT","lifecycle serialization failed".into()))?.len() as u64 + u64::from(!self.lifecycle.is_empty());
        if bytes>self.limits.max_output_bytes.saturating_sub(self.captured){Err(("E_RESOURCE_LIMIT","capture budget exhausted".into()))}else{Ok(())}
    }
    fn allocate(&mut self, ty:&str, data:Data, bytes:u64, entity:&str) -> Run<RValue> {
        self.check_allocation_event(entity)?;
        let buffer=self.allocator.allocate(bytes as usize).map_err(allocation_fault)?;
        self.adopt(ty,data,buffer,entity)
    }
    fn adopt(&mut self,ty:&str,data:Data,buffer:OwnedBuffer,entity:&str)->Run<RValue>{
        let id = self.next_allocation;
        self.event(LifecycleKind::Allocate,entity,&[id])?;
        self.next_allocation += 1; self.allocations.insert(id,buffer);
        Ok(RValue { ty:ty.into(),data:Rc::new(data),allocations:vec![id],handles:vec![],borrowed:false })
    }
    fn host_events(&mut self)->Run<()> {
        for e in self.host.take_events(){
            let event=HandleEvent{kind:match e.kind{il_runtime_handles::HandleEventKind::Opened=>HandleEventKind::Open,il_runtime_handles::HandleEventKind::Closed=>HandleEventKind::Close,il_runtime_handles::HandleEventKind::Dropped=>HandleEventKind::Drop},entity_id:self.current.clone(),slot:e.handle.slot,generation:e.handle.generation,error:e.error.map(|v|format!("{v:?}"))};
            self.capture(serde_json::to_vec(&event).map_err(|_|("E_STATE_INCONSISTENT","handle event serialization failed".into()))?.len() as u64 + u64::from(!self.handle_events.is_empty()))?;
            self.handle_events.push(event);
        } Ok(())
    }
    fn import(&mut self, value:&Value, entity:&str) -> Run<RValue> {
        let data = match &value.data {
            ValueData::Unit => Data::Unit, ValueData::Bool(v) => Data::Bool(*v),
            ValueData::Integer(v) => Data::Integer(v.parse().map_err(|_| ("E_TYPE_MISMATCH","invalid integer".into()))?),
            ValueData::String(v) => return self.allocate(&value.type_ref,Data::String(v.clone()),v.len() as u64,entity),
            ValueData::Bytes(v) => return self.allocate(&value.type_ref,Data::Bytes(v.clone()),v.len() as u64,entity),
            ValueData::Record(v) | ValueData::Tuple(v) => {
                let fields = v.iter().map(|v| self.import(v,entity)).collect::<Run<Vec<_>>>()?;
                let data = if matches!(&value.data,ValueData::Record(_)) { Data::Record(fields.clone()) } else { Data::Tuple(fields.clone()) };
                return Ok(RValue::aggregate(&value.type_ref,data,&fields));
            }
            ValueData::Resource(_) => return Err(("E_UNSUPPORTED_FEATURE","captured arguments cannot forge resources".into())),
            ValueData::Variant(v) => {
                let fields = v.fields.iter().map(|v| self.import(v,entity)).collect::<Run<Vec<_>>>()?;
                return Ok(RValue::aggregate(&value.type_ref,Data::Variant(v.tag.clone(),fields.clone()),&fields));
            }
        }; Ok(RValue::scalar(&value.type_ref,data))
    }
    fn export(&mut self, value:&RValue, depth:usize, nodes:&mut usize) -> Run<Value> {
        *nodes += 1;
        if depth > 64 || *nodes > 100_000 { return Err(("E_RESOURCE_LIMIT","returned value shape budget exceeded".into())); }
        let skeleton = match &*value.data {
            Data::Unit=>ValueData::Unit,Data::Bool(v)=>ValueData::Bool(*v),Data::Integer(v)=>ValueData::Integer(v.to_string()),
            Data::String(_)=>ValueData::String(String::new()),Data::Bytes(_)=>ValueData::Bytes(vec![]),
            Data::Record(_)=>ValueData::Record(vec![]),Data::Tuple(_)=>ValueData::Tuple(vec![]),
            Data::Resource{kind,handle:h}=>ValueData::Resource(ResourceValue{kind:*kind,slot:h.slot,generation:h.generation}),
            Data::Variant(tag,_)=>ValueData::Variant(VariantValue{tag:tag.clone(),fields:vec![]}),
        };
        let overhead=serde_json::to_vec(&Value{type_ref:value.ty.clone(),data:skeleton}).map_err(|_|("E_STATE_INCONSISTENT","value serialization failed".into()))?.len() as u64;
        let content=match &*value.data {
            Data::String(s)=>s.bytes().map(|b|match b{b'"'|b'\\'|b'\n'|b'\r'|b'\t'|8|12=>2,0..=31=>6,_=>1}).sum(),
            Data::Bytes(bs)=>bs.iter().map(|b|if *b>=100{3}else if *b>=10{2}else{1}).sum::<u64>()+bs.len().saturating_sub(1) as u64,
            Data::Record(fs)|Data::Tuple(fs)|Data::Variant(_,fs)=>fs.len().saturating_sub(1) as u64,
            _=>0,
        };
        self.capture(overhead+content)?;
        let data = match &*value.data {
            Data::Unit => ValueData::Unit, Data::Bool(v) => ValueData::Bool(*v),
            Data::Integer(v) => ValueData::Integer(v.to_string()),
            Data::Resource{kind,handle:h} => {self.host.validate_handle(*h,*kind).map_err(io_fault)?;ValueData::Resource(ResourceValue{kind:*kind,slot:h.slot,generation:h.generation})},
            Data::String(v) => ValueData::String(v.clone()),
            Data::Bytes(v) => ValueData::Bytes(v.clone()),
            Data::Record(v) => ValueData::Record(v.iter().map(|v| self.export(v,depth+1,nodes)).collect::<Run<_>>()?),
            Data::Tuple(v) => ValueData::Tuple(v.iter().map(|v| self.export(v,depth+1,nodes)).collect::<Run<_>>()?),
            Data::Variant(tag,v) => ValueData::Variant(VariantValue { tag:tag.clone(),fields:v.iter().map(|v| self.export(v,depth+1,nodes)).collect::<Run<_>>()? }),
        }; Ok(Value { type_ref:value.ty.clone(),data })
    }
    fn duplicate(&mut self,value:&RValue,entity:&str,depth:usize,nodes:&mut usize) -> Run<RValue> {
        *nodes += 1;
        if depth > 64 || *nodes > 100_000 { return Err(("E_RESOURCE_LIMIT","clone shape budget exceeded".into())); }
        if !self.types.owned.contains(&value.ty) { let mut v=value.clone();v.borrowed=false;return Ok(v); }
        let fields = match &*value.data {
            Data::Resource{..}=>return Err(("E_UNSUPPORTED_FEATURE","resource cannot be cloned".into())),
            Data::String(v) => return self.allocate(&value.ty,Data::String(v.clone()),v.len() as u64,entity),
            Data::Bytes(v) => return self.allocate(&value.ty,Data::Bytes(v.clone()),v.len() as u64,entity),
            Data::Record(v) | Data::Tuple(v) | Data::Variant(_,v) => v.iter().map(|v| self.duplicate(v,entity,depth+1,nodes)).collect::<Run<Vec<_>>>()?,
            _ => { let mut v = value.clone(); v.borrowed = false; return Ok(v); }
        };
        let data = match &*value.data { Data::Record(_) => Data::Record(fields.clone()), Data::Tuple(_) => Data::Tuple(fields.clone()), Data::Variant(tag,_) => Data::Variant(tag.clone(),fields.clone()), _ => unreachable!() };
        Ok(RValue::aggregate(&value.ty,data,&fields))
    }
    fn get(frame:&Frame,id:&str) -> Run<RValue> { frame.get(id).cloned().ok_or(("E_STATE_INCONSISTENT",format!("missing MIR value {id}"))) }
    fn transfer(&mut self,frame:&mut Frame,id:&str,entity:&str) -> Run<RValue> {
        let v = Self::get(frame,id)?;
        if self.types.owned.contains(&v.ty) {
            if v.borrowed { return Err(("E_BORROW_ESCAPE","borrow cannot be transferred".into())); }
            self.event(LifecycleKind::Move,entity,&v.allocations)?; frame.remove(id);
        }
        Ok(v)
    }
    fn drop_value(&mut self,frame:&mut Frame,id:&str,entity:&str) -> Run<()> {
        let v = Self::get(frame,id)?;
        if v.borrowed { return Err(("E_BORROW_CONFLICT","cannot drop borrowed view".into())); }
        if !self.types.owned.contains(&v.ty) { return Ok(()); }
        self.event(LifecycleKind::Drop,entity,&v.allocations)?;
        for id in &v.allocations {
            self.allocations.remove(id).ok_or(("E_DOUBLE_DROP","allocation already freed".into()))?;
        }
        for (kind,handle) in &v.handles { let _ = self.host.drop_handle(*handle,*kind); self.host_events()?; }
        frame.remove(id); Ok(())
    }
    fn cleanup(&mut self,frame:&mut Frame,actions:&[DropAction]) -> Run<()> {
        for action in actions { self.tick(&action.value_id)?; self.drop_value(frame,&action.value_id,&action.value_id)?; } Ok(())
    }
    fn call(&mut self,index:usize,args:Vec<RValue>,depth:u32) -> Run<RValue> {
        if depth > self.limits.max_call_depth { return Err(("E_RESOURCE_LIMIT","call depth budget exhausted".into())); }
        let function = &self.program.functions[index];
        self.enter(&function.entity_id,depth)?;
        let mut frame:Frame = function.parameters.iter().map(|p| p.entity_id.clone()).zip(args).collect();
        let mut block_index = 0;
        loop {
            let block = &function.blocks[block_index];
            for operation in &block.operations { self.tick(&operation.entity_id)?; self.operation(&mut frame,operation,depth)?; }
            let term = &block.terminator; let entity = term.entity_id(); self.tick(entity)?;
            let edge = match term {
                Terminator::Return { value,cleanup,.. } => {
                    let value = if let Some(id) = value { self.transfer(&mut frame,id,entity)? } else { RValue::scalar("Unit",Data::Unit) };
                    self.cleanup(&mut frame,cleanup)?;
                    self.location(entity)?;
                    if self.types.owned.contains(&value.ty) { self.event(LifecycleKind::Return,entity,&value.allocations)?; }
                    self.leave();
                    return Ok(value);
                }
                Terminator::Trap { code,.. } => return Err(("E_EXPLICIT_TRAP",format!("explicit trap: {code}"))),
                Terminator::Branch { edge,.. } => edge,
                Terminator::CondBranch { condition,then_edge,else_edge,.. } => if Self::get(&frame,condition)?.boolean()? { then_edge } else { else_edge },
                Terminator::Switch { value,cases,default,.. } => {
                    let value = Self::get(&frame,value)?;
                    let Data::Variant(tag,_) = &*value.data else { return Err(("E_TYPE_MISMATCH","switch needs variant".into())); };
                    cases.iter().find(|case| case.tag == *tag).map(|case| &case.edge).unwrap_or(default)
                }
            };
            let values = edge.arguments.iter().map(|id| self.transfer(&mut frame,id,entity)).collect::<Run<Vec<_>>>()?;
            self.cleanup(&mut frame,&edge.cleanup)?;
            frame.retain(|id,_| function.parameters.iter().any(|p| p.entity_id == *id));
            block_index = function.blocks.iter().position(|b| b.entity_id == edge.target).ok_or(("E_NAME_NOT_FOUND","unknown MIR target".into()))?;
            for (arg,value) in function.blocks[block_index].arguments.iter().zip(values) { frame.insert(arg.entity_id.clone(),value); }
        }
    }

    fn operation(&mut self,frame:&mut Frame,op:&Operation,depth:u32) -> Run<()> {
        let values = op.inputs.iter().map(|id| Self::get(frame,id)).collect::<Run<Vec<_>>>()?;
        let ty = op.outputs.first().map(|v| v.type_ref.as_str()).unwrap_or("Unit");
        use Opcode::*;
        let outputs = match op.opcode {
            Const => {
                let Attributes::Constant { value } = &op.attributes else { return Err(("E_SCHEMA_INVALID","constant attributes missing".into())); };
                let data = match value {
                    Literal::Unit(()) => ValueData::Unit, Literal::Bool(v) => ValueData::Bool(*v), Literal::Integer(v) => ValueData::Integer(v.to_string()),
                    Literal::Unsigned(v) => ValueData::Integer(v.to_string()), Literal::String(v) => ValueData::String(v.clone()), Literal::Bytes(v) => ValueData::Bytes(v.clone()),
                }; vec![self.import(&Value { type_ref:ty.into(),data },&op.entity_id)?]
            }
            Add | Sub | Mul | Div | Rem | Shl | Shr | BitAnd | BitOr | BitXor => {
                let (signed,bits) = self.types.integer(ty).ok_or(("E_TYPE_MISMATCH","integer type required".into()))?;
                let n = numbers::calculate(op.opcode,values[0].integer()?,values[1].integer()?,signed,bits).map_err(|code| (code,"checked integer operation trapped".into()))?;
                vec![RValue::scalar(ty,Data::Integer(n))]
            }
            Eq | Ne | Lt | Le | Gt | Ge => {
                let order = match (&*values[0].data,&*values[1].data) { (Data::Bool(a),Data::Bool(b)) => a.cmp(b), (Data::Integer(a),Data::Integer(b)) => a.cmp(b), _ => return Err(("E_TYPE_MISMATCH","comparison operand types invalid".into())) };
                let yes = match op.opcode { Eq => order.is_eq(), Ne => !order.is_eq(), Lt => order.is_lt(), Le => !order.is_gt(), Gt => order.is_gt(), Ge => !order.is_lt(), _ => unreachable!() };
                vec![RValue::scalar("Bool",Data::Bool(yes))]
            }
            Not => {
                let data = if ty == "Bool" { Data::Bool(!values[0].boolean()?) } else { let (signed,bits) = self.types.integer(ty).ok_or(("E_TYPE_MISMATCH","integer type required".into()))?; Data::Integer(numbers::complement(values[0].integer()?,signed,bits)) };
                vec![RValue::scalar(ty,data)]
            }
            Cast => {
                let (signed,bits) = self.types.integer(ty).ok_or(("E_TYPE_MISMATCH","integer cast target required".into()))?;
                let n = numbers::checked(values[0].integer()?,signed,bits).map_err(|code| (code,"integer cast out of range".into()))?;
                vec![RValue::scalar(ty,Data::Integer(n))]
            }
            Move => vec![self.transfer(frame,&op.inputs[0],&op.entity_id)?],
            Clone => vec![self.duplicate(&values[0],&op.entity_id,0,&mut 0)?],
            Borrow | BorrowMut => { let mut v=values[0].clone();v.borrowed=true;self.event(LifecycleKind::Borrow,&op.entity_id,&v.allocations)?;vec![v] }
            EndBorrow => { self.event(LifecycleKind::EndBorrow,&op.entity_id,&values[0].allocations)?;frame.remove(&op.inputs[0]);vec![] }
            Drop => { self.drop_value(frame,&op.inputs[0],&op.entity_id)?;vec![] }
            Record | Tuple | Variant => {
                let fields = op.inputs.iter().map(|id| self.transfer(frame,id,&op.entity_id)).collect::<Run<Vec<_>>>()?;
                let data = match (&op.opcode,&op.attributes) {
                    (Record,_) => Data::Record(fields.clone()), (Tuple,_) => Data::Tuple(fields.clone()),
                    (Variant,Attributes::Variant { variant,.. }) => Data::Variant(variant.clone(),fields.clone()),
                    _ => return Err(("E_SCHEMA_INVALID","variant attributes missing".into())),
                }; vec![RValue::aggregate(ty,data,&fields)]
            }
            Field | TupleGet => {
                let index = match &op.attributes {
                    Attributes::TupleGet { index } => *index as usize,
                    Attributes::Field { field } => self.types.definitions.get(values[0].ty.as_str()).and_then(|t| t.fields.iter().position(|f| f.name == *field)).ok_or(("E_NAME_NOT_FOUND","unknown field".into()))?,
                    _ => return Err(("E_SCHEMA_INVALID","projection attributes missing".into())),
                };
                let fs = match &*values[0].data { Data::Record(fs) | Data::Tuple(fs) => fs, _ => return Err(("E_TYPE_MISMATCH","aggregate required".into())) };
                vec![fs.get(index).cloned().ok_or(("E_TYPE_MISMATCH","projection out of range".into()))?]
            }
            Tag => {
                let Data::Variant(tag,_) = &*values[0].data else { return Err(("E_TYPE_MISMATCH","variant required".into())); };
                let index = self.types.variants(&values[0].ty).and_then(|v| v.iter().position(|(t,_)| t == tag)).ok_or(("E_TYPE_MISMATCH","unknown variant tag".into()))?;
                vec![RValue::scalar("U32",Data::Integer(index as i128))]
            }
            Payload => {
                let source = self.transfer(frame,&op.inputs[0],&op.entity_id)?;
                let Data::Variant(_,fs) = &*source.data else { return Err(("E_TYPE_MISMATCH","variant required".into())); }; fs.clone()
            }
            Call => {
                let Attributes::Call { callee } = &op.attributes else { return Err(("E_SCHEMA_INVALID","callee missing".into())); };
                let index = self.program.functions.iter().position(|f| f.entity_id == *callee).ok_or(("E_NAME_NOT_FOUND","callee not found".into()))?;
                let args = op.inputs.iter().map(|id| self.transfer(frame,id,&op.entity_id)).collect::<Run<Vec<_>>>()?;
                let value = self.call(index,args,depth+1)?;
                self.location(&op.entity_id)?; if op.outputs.is_empty() { vec![] } else { vec![value] }
            }
            RuntimeCall => {
                if let Attributes::RuntimeCall{symbol,..}=&op.attributes{let signature=il_checker::runtime_signature(symbol).ok_or(("E_UNSUPPORTED_FEATURE","unknown runtime signature".into()))?;for (id,parameter) in op.inputs.iter().zip(signature.parameters){if parameter.passing==il_checker::Passing::Owned{self.transfer(frame,id,&op.entity_id)?;}}}
                vec![self.runtime(op,&values,ty)?]
            },
            _ => return Err(("E_SCHEMA_INVALID","terminator cannot execute as operation".into())),
        };
        if outputs.len() != op.outputs.len() { return Err(("E_STATE_INCONSISTENT","MIR output arity mismatch".into())); }
        for (def,value) in op.outputs.iter().zip(outputs) { frame.insert(def.entity_id.clone(),value); }
        Ok(())
    }
    fn variant(&self,ty:&str,tag:&str,fields:Vec<RValue>) -> RValue { RValue::aggregate(ty,Data::Variant(tag.into(),fields.clone()),&fields) }
    fn io_result(&self,ty:&str,result:Result<RValue,IoFailure>)->Run<RValue>{
        match result {Ok(v)=>Ok(self.variant(ty,"Ok",vec![v])),Err(IoFailure::Error(error))=>Ok(self.variant(ty,"Err",vec![self.variant("core.IoError",&format!("{error:?}"),vec![])])),Err(error)=>Err(io_fault(error))}
    }
    fn buffer_result(&mut self,ty:&str,result:Result<OwnedBuffer,IoFailure>,entity:&str)->Run<RValue>{
        let value=match result{Ok(buffer)=>{let data=Data::Bytes(buffer.as_slice().to_vec());Ok(self.adopt("Bytes",data,buffer,entity)?)},Err(e)=>Err(e)};
        self.io_result(ty,value)
    }
    fn copy_bytes_result(&mut self,ty:&str,source:&[u8],utf8:bool,entity:&str)->Run<RValue>{
        self.check_allocation_event(entity)?;let result=self.allocator.copy(source).map_err(IoFailure::from);
        if !utf8{return self.buffer_result(ty,result,entity)}
        let value=match result{Ok(buffer)=>{let text=std::str::from_utf8(buffer.as_slice()).map_err(|_|("E_STATE_INCONSISTENT","validated UTF-8 changed".into()))?.to_owned();Ok(self.adopt("String",Data::String(text),buffer,entity)?)},Err(error)=>Err(error)};self.io_result(ty,value)
    }
    fn write_stream(&mut self,stderr:bool,bytes:&[u8],offset:usize,deadline:Deadline)->Run<Result<usize,IoFailure>>{
        let remaining=bytes.len().saturating_sub(offset);
        if remaining as u64>self.limits.max_output_bytes.saturating_sub(self.captured){return Err(("E_RESOURCE_LIMIT","capture budget exhausted".into()))}
        let result=if stderr{self.host.stderr_write(bytes,offset,deadline)}else{self.host.stdout_write(bytes,offset,deadline)};
        if let Ok(count)=result {self.capture(count as u64)?;if stderr{self.stderr.extend_from_slice(&bytes[offset..offset+count]);}else{self.stdout.extend_from_slice(&bytes[offset..offset+count]);}}
        Ok(result)
    }
    fn runtime(&mut self,op:&Operation,values:&[RValue],ty:&str) -> Run<RValue> {
        let Attributes::RuntimeCall { symbol, capability } = &op.attributes else { return Err(("E_SCHEMA_INVALID","runtime symbol missing".into())); };
        if matches!(symbol.as_str(),"file_read"|"file_read_some"|"stdin_read"|"string_concat"|"net_read"){self.check_allocation_event(&op.entity_id)?;}
        let grant=||capability.as_deref().and_then(|id|self.host.policy().grant_id(id)).ok_or(("E_CAPABILITY_MISSING","runtime grant unavailable".into()));
        match symbol.as_str() {
            "print_i64" | "print_string" => {
                let bytes = if symbol == "print_i64" { values[0].integer()?.to_string().into_bytes() } else { string(&values[0])?.as_bytes().to_vec() };
                let mut offset=0;
                while offset<bytes.len(){match self.write_stream(false,&bytes,offset,Deadline::Infinite)?{Ok(0)=>return self.io_result(ty,Err(IoFailure::Error(IoError::Write))),Ok(count)=>offset+=count,Err(error)=>return self.io_result(ty,Err(error))}}
                self.io_result(ty,Ok(RValue::scalar("Unit",Data::Unit)))
            }
            "string_len" | "bytes_len" => {
                let len = match &*values[0].data { Data::String(v) => v.len(), Data::Bytes(v) => v.len(), _ => return Err(("E_TYPE_MISMATCH","string or bytes required".into())) };
                Ok(RValue::scalar("Usize",Data::Integer(len as i128)))
            }
            "string_concat" => {
                let (a,b)=(string(&values[0])?,string(&values[1])?);
                let Some(len)=a.len().checked_add(b.len()) else { return Ok(self.variant(ty,"Err",vec![self.variant("core.AllocError","CapacityOverflow",vec![])])); };
                let mut buffer=match self.allocator.allocate(len){Ok(v)=>v,Err(error)=>{let tag=if error==AllocFailure::CapacityOverflow{"CapacityOverflow"}else{"OutOfMemory"};return Ok(self.variant(ty,"Err",vec![self.variant("core.AllocError",tag,vec![])]))}};
                buffer.as_mut_slice()[..a.len()].copy_from_slice(a.as_bytes());buffer.as_mut_slice()[a.len()..].copy_from_slice(b.as_bytes());
                let text=String::from_utf8(buffer.as_slice().to_vec()).map_err(|_|("E_STATE_INCONSISTENT","invalid UTF-8 concatenation".into()))?;
                let value=self.adopt("String",Data::String(text),buffer,&op.entity_id)?;
                Ok(self.variant(ty,"Ok",vec![value]))
            }
            "bytes_get"=>{let source=bytes(&values[0])?;let index=usize_value(&values[1])?;let byte=source.get(index).ok_or(("E_INDEX_OUT_OF_BOUNDS","byte index out of bounds".into()))?;Ok(RValue::scalar("U8",Data::Integer(*byte as i128)))},
            "bytes_equal"|"string_equal"=>{let equal=if symbol=="bytes_equal"{bytes(&values[0])?==bytes(&values[1])?}else{string(&values[0])?==string(&values[1])?};Ok(RValue::scalar("Bool",Data::Bool(equal)))},
            "bytes_slice"=>{let source=bytes(&values[0])?;let start=usize_value(&values[1])?;let length=usize_value(&values[2])?;let slice=start.checked_add(length).and_then(|end|source.get(start..end));let Some(slice)=slice else{return self.io_result(ty,Err(IoError::InvalidData.into()))};self.copy_bytes_result(ty,slice,false,&op.entity_id)},
            "bytes_concat"=>{let(a,b)=(bytes(&values[0])?,bytes(&values[1])?);let length=a.len().checked_add(b.len()).filter(|length|*length<=isize::MAX as usize).ok_or(("E_RESOURCE_LIMIT","byte concatenation capacity exceeded".into()))?;self.check_allocation_event(&op.entity_id)?;let result=self.allocator.allocate(length).map(|mut buffer|{buffer.as_mut_slice()[..a.len()].copy_from_slice(a);buffer.as_mut_slice()[a.len()..].copy_from_slice(b);buffer}).map_err(IoFailure::from);self.buffer_result(ty,result,&op.entity_id)},
            "bytes_from_u8"=>{let byte=u8::try_from(values[0].integer()?).map_err(|_|("E_TYPE_MISMATCH","U8 required".into()))?;self.copy_bytes_result(ty,&[byte],false,&op.entity_id)},
            "string_to_bytes"=>self.copy_bytes_result(ty,string(&values[0])?.as_bytes(),false,&op.entity_id),
            "string_from_utf8"=>{let source=bytes(&values[0])?;if std::str::from_utf8(source).is_err(){return self.io_result(ty,Err(IoError::InvalidData.into()))}self.copy_bytes_result(ty,source,true,&op.entity_id)},
            "net_opened_at"=>{let now=self.host.net_opened_at(resource(&values[0],ResourceKind::Stream)?).map_err(io_fault)?;Ok(RValue::scalar("U64",Data::Integer(now as i128)))},
            "net_listen"=>{let result=self.host.net_listen(grant()?);self.host_events()?;self.io_result(ty,result.map(|handle|resource_value(ResourceKind::Listener,handle)))},
            "net_connect"=>{let result=self.host.net_connect(grant()?,deadline(&values[0])?);self.host_events()?;self.io_result(ty,result.map(|handle|resource_value(ResourceKind::Stream,handle)))},
            "net_accept"=>{let result=self.host.net_accept(resource(&values[0],ResourceKind::Listener)?,deadline(&values[1])?);self.host_events()?;self.io_result(ty,result.map(|handle|resource_value(ResourceKind::Stream,handle)))},
            "net_read"=>{let result=self.host.net_read(resource(&values[0],ResourceKind::Stream)?,usize_value(&values[1])?,deadline(&values[2])?);self.buffer_result(ty,result,&op.entity_id)},
            "net_write"=>{let result=self.host.net_write(resource(&values[0],ResourceKind::Stream)?,bytes(&values[1])?,usize_value(&values[2])?,deadline(&values[3])?);self.io_result(ty,result.map(|v|RValue::scalar("Usize",Data::Integer(v as i128))))},
            "net_close_listener"|"net_close_stream"=>{let kind=if symbol=="net_close_listener"{ResourceKind::Listener}else{ResourceKind::Stream};let result=self.host.close(resource(&values[0],kind)?,kind);self.host_events()?;self.io_result(ty,result.map(|_|RValue::scalar("Unit",Data::Unit)))},
            "file_open_read" | "file_open_write" => {
                let grant=grant()?;let path=string(&values[0])?;
                let result=if symbol=="file_open_read"{self.host.open_read(grant,path)}else{self.host.open_write(grant,path)};
                self.host_events()?;
                self.io_result(ty,result.map(|handle|resource_value(ResourceKind::File,handle)))
            }
            "file_close" => {let result=self.host.close(resource(&values[0],ResourceKind::File)?,ResourceKind::File);self.host_events()?;self.io_result(ty,result.map(|_|RValue::scalar("Unit",Data::Unit)))}
            "file_read_some" => {let result=self.host.read_some(resource(&values[0],ResourceKind::File)?,usize_value(&values[1])?,deadline(&values[2])?);self.buffer_result(ty,result,&op.entity_id)}
            "file_write_some" => {let result=self.host.write_some(resource(&values[0],ResourceKind::File)?,bytes(&values[1])?,usize_value(&values[2])?,deadline(&values[3])?);self.io_result(ty,result.map(|v|RValue::scalar("Usize",Data::Integer(v as i128))))}
            "stdin_read" => {let result=self.host.stdin_read(usize_value(&values[0])?,deadline(&values[1])?);self.buffer_result(ty,result,&op.entity_id)}
            "stdout_write" | "stderr_write" => {let result=self.write_stream(symbol=="stderr_write",bytes(&values[0])?,usize_value(&values[1])?,deadline(&values[2])?)?;self.io_result(ty,result.map(|v|RValue::scalar("Usize",Data::Integer(v as i128))))}
            "clock_now" => {let result=self.host.clock_now(grant()?).map_err(io_fault)?;Ok(RValue::scalar("U64",Data::Integer(result as i128)))}
            "file_read" => {let result=self.host.read_all(grant()?,string(&values[0])?);self.host_events()?;self.buffer_result(ty,result,&op.entity_id)}
            "file_write" => {let result=self.host.write_all(grant()?,string(&values[0])?,bytes(&values[1])?);self.host_events()?;self.io_result(ty,result.map(|v|RValue::scalar("Usize",Data::Integer(v as i128))))}
            _ => Err(("E_UNSUPPORTED_FEATURE",format!("runtime symbol {symbol} is unsupported"))),
        }
    }
}
fn allocation_fault(error:AllocFailure)->Fault{match error{AllocFailure::ResourceLimit=>("E_RESOURCE_LIMIT","heap budget exhausted".into()),_=>("E_ALLOCATION_FAILED","guest allocation failed".into())}}
fn io_fault(error:IoFailure)->Fault{match error{IoFailure::ResourceLimit=>("E_RESOURCE_LIMIT","heap budget exhausted".into()),IoFailure::Unsupported=>("E_UNSUPPORTED_FEATURE","host resource operation unavailable".into()),IoFailure::Error(e)=>("E_STATE_INCONSISTENT",format!("unexpected host error {e:?}"))}}
fn string(value:&RValue)->Run<&str>{if let Data::String(v)=&*value.data{Ok(v)}else{Err(("E_TYPE_MISMATCH","string required".into()))}}
fn bytes(value:&RValue)->Run<&[u8]>{if let Data::Bytes(v)=&*value.data{Ok(v)}else{Err(("E_TYPE_MISMATCH","bytes required".into()))}}
fn resource(value:&RValue,expected:ResourceKind)->Run<HandleId>{if let Data::Resource{kind,handle}=&*value.data{if *kind==expected{return Ok(*handle)}}Err(("E_TYPE_MISMATCH","resource kind mismatch".into()))}
fn resource_value(kind:ResourceKind,handle:HandleId)->RValue{RValue{ty:kind.nominal_type().into(),data:Rc::new(Data::Resource{kind,handle}),allocations:vec![],handles:vec![(kind,handle)],borrowed:false}}
fn usize_value(value:&RValue)->Run<usize>{usize::try_from(value.integer()?).map_err(|_|("E_TYPE_MISMATCH","Usize required".into()))}
fn deadline(value:&RValue)->Run<Deadline>{match &*value.data{Data::Variant(tag,fields)if tag=="Infinite"&&fields.is_empty()=>Ok(Deadline::Infinite),Data::Variant(tag,fields)if tag=="At"&&fields.len()==1=>Ok(Deadline::At(u64::try_from(fields[0].integer()?).map_err(|_|("E_TYPE_MISMATCH","deadline instant must be U64".into()))?)),_=>Err(("E_TYPE_MISMATCH","Deadline required".into()))}}

fn contains_resource(value:&Value)->bool{match &value.data{ValueData::Resource(_)=>true,ValueData::Record(fields)|ValueData::Tuple(fields)=>fields.iter().any(contains_resource),ValueData::Variant(v)=>v.fields.iter().any(contains_resource),_=>false}}
