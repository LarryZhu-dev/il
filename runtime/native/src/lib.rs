//! Native ABI substrate. This crate contains no language opcode dispatcher or IR reader.
use il_execution_model::{Execution,ExecutionStatus,LifecycleEvent,LifecycleKind,Limits,Value,StackFrame,HandleEvent,HandleEventKind,STACK_TRACE_MAX_BYTES,json_string_bytes,stack_frame_json_bytes,stack_trace_push_bytes,stack_trace_replace_bytes};
use il_graph::{Diagnostic,Capability};
use il_runtime_allocator::{Allocator,AllocFailure,OwnedBuffer};
use il_runtime_startup::{HostPolicy,ValidatedPolicy,InheritedStdio,GrantId};
use il_runtime_handles::{HostResources,HandleId,Deadline,IoFailure,IoError};
use serde::Deserialize;
use std::{collections::BTreeMap,fs::File,io::Write,ptr,slice};

#[repr(C)]
#[derive(Clone,Copy,Debug)]
pub struct Buffer { pub pointer:*mut u8,pub length:u64,pub capacity:u64 }
#[repr(C)]
#[derive(Clone,Copy,Debug)]
pub struct DeadlineValue {pub tag:u32,pub padding:u32,pub instant:u64}
struct Allocation {id:u64,buffer:OwnedBuffer}
pub struct Context {
    captured:bool,limits:Limits,revision:u64,report_fd:i32,steps:u64,depth:u32,next_id:u64,
    allocations:BTreeMap<usize,Allocation>,stdout:Vec<u8>,stderr:Vec<u8>,events:Vec<LifecycleEvent>,capture_bytes:u64,
    current:String,json:Vec<u8>,pending:Option<LifecycleEvent>,shape_nodes:u64,shape_depth:u32,shape_kind:u32,
    allocator:Allocator,host:HostResources,authorized:bool,frames:Vec<StackFrame>,handle_events:Vec<HandleEvent>,stack_bytes:u64,
}
impl Context {
    fn fail(&mut self,code:&str,message:&str,entity:Option<&str>)->! {
        let selected=entity.unwrap_or(&self.current);let oversized=json_string_bytes(selected)>STACK_TRACE_MAX_BYTES;
        let code=if oversized{"E_RESOURCE_LIMIT"}else{code};let message=if oversized{"stack trace budget exhausted"}else{message};
        let entity=if oversized{self.current.clone()}else{selected.to_owned()};
        let mut diagnostic=Diagnostic::error(code,Some(&entity),message,self.revision);diagnostic.stage="native_runtime".into();diagnostic.retryable=false;
        if self.captured {self.report(ExecutionStatus::Trapped,None,vec![diagnostic]);}
        else {let _=writeln!(std::io::stderr().lock(),"{code} at {entity}: {message}");for frame in self.frames.iter().rev(){let _=writeln!(std::io::stderr().lock(),"  {} at {}",frame.function_id,frame.entity_id);}}
        std::process::exit(101)
    }
    fn set_location(&mut self,entity:&str){
        let next=if let Some(frame)=self.frames.last(){stack_trace_replace_bytes(self.stack_bytes,stack_frame_json_bytes(&frame.function_id,frame.call_site.as_deref(),&frame.entity_id),stack_frame_json_bytes(&frame.function_id,frame.call_site.as_deref(),entity))}else{if json_string_bytes(entity)>STACK_TRACE_MAX_BYTES{None}else{Some(self.stack_bytes)}};
        let Some(next)=next else{self.fail("E_RESOURCE_LIMIT","stack trace budget exhausted",None)};self.stack_bytes=next;self.current=entity.to_owned();if let Some(frame)=self.frames.last_mut(){frame.entity_id=entity.to_owned();}
    }
    fn check_charge(&mut self,bytes:u64){if self.captured&&bytes>self.limits.max_output_bytes.saturating_sub(self.capture_bytes){self.fail("E_RESOURCE_LIMIT","capture budget exhausted",None)}}
    fn charge(&mut self,bytes:u64){if !self.captured{return}self.check_charge(bytes);self.capture_bytes+=bytes;}
    fn event(&mut self,event:LifecycleEvent){
        if !self.captured{return}
        let encoded=serde_json::to_vec(&event).unwrap_or_else(|_|self.fail("E_STATE_INCONSISTENT","lifecycle encoding failed",None));
        self.charge(encoded.len() as u64+u64::from(!self.events.is_empty()));self.events.push(event);
    }
    fn emit(&mut self,bytes:&[u8]){if self.captured{self.charge(bytes.len() as u64);self.json.extend_from_slice(bytes);}}
    fn drain_handles(&mut self){
        for event in self.host.take_events(){if !self.captured{continue}
            let kind=match event.kind{il_runtime_handles::HandleEventKind::Opened=>HandleEventKind::Open,il_runtime_handles::HandleEventKind::Closed=>HandleEventKind::Close,il_runtime_handles::HandleEventKind::Dropped=>HandleEventKind::Drop};
            let event=HandleEvent{kind,entity_id:self.current.clone(),slot:event.handle.slot,generation:event.handle.generation,error:event.error.map(|error|format!("{error:?}"))};
            let encoded=serde_json::to_vec(&event).unwrap_or_else(|_|self.fail("E_STATE_INCONSISTENT","handle event encoding failed",None));self.charge(encoded.len()as u64+u64::from(!self.handle_events.is_empty()));self.handle_events.push(event);
        }
    }
    fn report(&mut self,status:ExecutionStatus,value:Option<Value>,diagnostics:Vec<Diagnostic>){
        let stack_trace=if status==ExecutionStatus::Trapped{self.frames.iter().rev().cloned().collect()}else{vec![]};
        let execution=Execution{status,value,stdout:std::mem::take(&mut self.stdout),stderr:std::mem::take(&mut self.stderr),diagnostics,steps:self.steps,peak_heap_bytes:self.allocator.peak_bytes(),live_allocations:self.allocations.len()as u64,lifecycle:std::mem::take(&mut self.events),live_handles:self.host.live_handles(),handle_events:std::mem::take(&mut self.handle_events),stack_trace};
        let encoded=serde_json::to_vec(&execution).unwrap_or_else(|_|std::process::exit(102));
        use std::os::fd::FromRawFd;
        if self.report_fd<0{std::process::exit(102)}
        let mut file=unsafe{File::from_raw_fd(self.report_fd)};
        if file.write_all(&encoded).and_then(|_|file.flush()).is_err(){std::process::exit(102)}self.report_fd=-1;
    }
    fn allocation_event(&self,entity:&str)->LifecycleEvent{LifecycleEvent{kind:LifecycleKind::Allocate,entity_id:entity.into(),allocation_ids:vec![self.next_id]}}
    fn check_allocation_event(&mut self,entity:&str){if self.captured{let event=self.allocation_event(entity);let length=serde_json::to_vec(&event).unwrap().len()as u64+u64::from(!self.events.is_empty());self.check_charge(length);}}
    fn adopt(&mut self,mut owned:OwnedBuffer,entity:&str)->Buffer{
        let result=Buffer{pointer:owned.as_mut_ptr(),length:owned.len()as u64,capacity:owned.capacity()as u64};let id=self.next_id;self.event(self.allocation_event(entity));self.next_id+=1;self.allocations.insert(result.pointer as usize,Allocation{id,buffer:owned});result
    }
    fn try_buffer(&mut self,data:&[u8],entity:&str)->Result<Buffer,AllocFailure>{self.check_allocation_event(entity);let owned=self.allocator.copy(data)?;Ok(self.adopt(owned,entity))}
    fn checked_buffer(&mut self,buffer:Buffer)->&[u8]{
        let Some(allocation)=self.allocations.get(&(buffer.pointer as usize))else{self.fail("E_STATE_INCONSISTENT","unknown native buffer allocation",None)};
        if allocation.buffer.len()as u64!=buffer.length||allocation.buffer.capacity()as u64!=buffer.capacity{self.fail("E_STATE_INCONSISTENT","native buffer layout mismatch",None)}
        unsafe{slice::from_raw_parts(buffer.pointer,buffer.length as usize)}
    }
    fn io_status(&mut self,result:Result<(),IoFailure>)->u32{match result{Ok(())=>0,Err(IoFailure::Error(error))=>error as u32+1,Err(IoFailure::ResourceLimit)=>self.fail("E_RESOURCE_LIMIT","heap budget exhausted",None),Err(IoFailure::Unsupported)=>self.fail("E_UNSUPPORTED_FEATURE","kernel cannot enforce resource containment",None)}}
    fn capture_output(&mut self,data:&[u8],stderr:bool){if self.captured{self.charge(data.len()as u64);if stderr{self.stderr.extend_from_slice(data)}else{self.stdout.extend_from_slice(data)}}}
    fn print(&mut self,data:&[u8])->u32{
        self.check_charge(data.len()as u64);let mut written=0;
        while written<data.len(){match self.host.stdout_write(data,written,Deadline::Infinite){Ok(0)=>return IoError::Write as u32+1,Ok(count)=>{self.capture_output(&data[written..written+count],false);written+=count},Err(error)=>return self.io_status(Err(error))}}
        0
    }
}

unsafe fn context<'a>(pointer:*mut Context)->&'a mut Context{if pointer.is_null(){std::process::exit(102)}unsafe{&mut *pointer}}
unsafe fn bytes<'a>(pointer:*const u8,length:u64)->&'a [u8]{if length==0{return &[]}if pointer.is_null()||length>isize::MAX as u64{std::process::exit(102)}unsafe{slice::from_raw_parts(pointer,length as usize)}}
unsafe fn text<'a>(pointer:*const u8,length:u64)->&'a str{std::str::from_utf8(unsafe{bytes(pointer,length)}).unwrap_or_else(|_|std::process::exit(102))}

#[no_mangle]
pub extern "C" fn il_rt_context_new(mode:u32,max_steps:u64,max_depth:u32,max_heap:u64,max_output:u64,revision:u64,report_fd:i32)->*mut Context{
    let limits=Limits{max_steps,max_call_depth:max_depth,max_heap_bytes:max_heap,max_output_bytes:max_output};
    if mode>1||(mode==1&&(!limits.valid()||report_fd<0)){std::process::exit(102)}
    if il_runtime_startup::initialize_signals().is_err(){std::process::exit(102)}
    let allocator=Allocator::new(None,if mode==1{Some(max_heap)}else{None});
    let policy=ValidatedPolicy::new(HostPolicy::empty(),&[],mode==1).unwrap();
    let stdio=InheritedStdio::capture().unwrap_or_else(|_|std::process::exit(102));
    let host=HostResources::new(policy,stdio,allocator.clone());
    Box::into_raw(Box::new(Context{captured:mode==1,limits,revision,report_fd,steps:0,depth:0,next_id:1,allocations:BTreeMap::new(),stdout:vec![],stderr:vec![],events:vec![],capture_bytes:0,current:"entry".into(),json:vec![],pending:None,shape_nodes:0,shape_depth:0,shape_kind:0,allocator,host,authorized:false,frames:vec![],handle_events:vec![],stack_bytes:2}))
}

#[no_mangle]
pub unsafe extern "C" fn il_rt_finish(pointer:*mut Context){
    let ctx=unsafe{context(pointer)};
    if ctx.captured{
        let mut decoder=serde_json::Deserializer::from_slice(&ctx.json);decoder.disable_recursion_limit();
        let value=Value::deserialize(&mut decoder).and_then(|value|decoder.end().map(|_|value)).unwrap_or_else(|_|ctx.fail("E_STATE_INCONSISTENT","invalid generated result JSON",None));
        ctx.report(ExecutionStatus::Returned,Some(value),vec![]);
    }
    unsafe{drop(Box::from_raw(pointer));}
}

#[no_mangle]
pub unsafe extern "C" fn il_rt_enter(pointer:*mut Context,entity:*const u8,length:u64){
    let ctx=unsafe{context(pointer)};let function_id=unsafe{text(entity,length)};let call_site=if ctx.frames.is_empty(){None}else{Some(ctx.current.as_str())};
    if ctx.captured&&ctx.depth>=ctx.limits.max_call_depth{ctx.fail("E_RESOURCE_LIMIT","call depth budget exhausted",None)}
    let size=stack_frame_json_bytes(function_id,call_site,function_id);let Some(next)=stack_trace_push_bytes(ctx.stack_bytes,ctx.frames.len(),size)else{ctx.fail("E_RESOURCE_LIMIT","stack trace budget exhausted",None)};
    let frame=StackFrame{function_id:function_id.to_owned(),call_site:call_site.map(str::to_owned),entity_id:function_id.to_owned()};ctx.stack_bytes=next;ctx.current=function_id.to_owned();ctx.frames.push(frame);ctx.depth+=1;
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_leave(pointer:*mut Context){let ctx=unsafe{context(pointer)};if ctx.depth==0{ctx.fail("E_STATE_INCONSISTENT","unbalanced native call depth",None)}ctx.depth-=1;let frame=ctx.frames.pop().unwrap();ctx.stack_bytes-=stack_frame_json_bytes(&frame.function_id,frame.call_site.as_deref(),&frame.entity_id)+u64::from(!ctx.frames.is_empty());if let Some(frame)=ctx.frames.last(){ctx.current=frame.entity_id.clone();}}
#[no_mangle]
pub unsafe extern "C" fn il_rt_tick(pointer:*mut Context,entity:*const u8,length:u64){let ctx=unsafe{context(pointer)};ctx.set_location(unsafe{text(entity,length)});if ctx.captured&&ctx.steps>=ctx.limits.max_steps{ctx.fail("E_RESOURCE_LIMIT","instruction budget exhausted",None)}ctx.steps=ctx.steps.saturating_add(1);}
#[no_mangle]
pub unsafe extern "C" fn il_rt_location(pointer:*mut Context,entity:*const u8,length:u64){unsafe{context(pointer)}.set_location(unsafe{text(entity,length)});}
#[no_mangle]
pub unsafe extern "C" fn il_rt_trap(pointer:*mut Context,code:*const u8,code_len:u64,message:*const u8,message_len:u64,entity:*const u8,entity_len:u64)->!{unsafe{context(pointer)}.fail(unsafe{text(code,code_len)},unsafe{text(message,message_len)},Some(unsafe{text(entity,entity_len)}))}

#[no_mangle]
pub unsafe extern "C" fn il_rt_buffer_new(pointer:*mut Context,out:*mut Buffer,source:*const u8,length:u64,entity:*const u8,entity_len:u64){
    let ctx=unsafe{context(pointer)};let entity=unsafe{text(entity,entity_len)};let source=unsafe{bytes(source,length)};
    let buffer=ctx.try_buffer(source,entity).unwrap_or_else(|error|ctx.fail(if error==AllocFailure::ResourceLimit{"E_RESOURCE_LIMIT"}else{"E_ALLOCATION_FAILED"},"buffer allocation failed",Some(entity)));
    unsafe{out.write(buffer);}
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_buffer_clone(pointer:*mut Context,out:*mut Buffer,source:*const Buffer,entity:*const u8,entity_len:u64){
    let ctx=unsafe{context(pointer)};let source=unsafe{*source};ctx.checked_buffer(source);
    let entity=unsafe{text(entity,entity_len)};
    let buffer=ctx.try_buffer(unsafe{bytes(source.pointer,source.length)},entity).unwrap_or_else(|error|ctx.fail(if error==AllocFailure::ResourceLimit{"E_RESOURCE_LIMIT"}else{"E_ALLOCATION_FAILED"},"buffer allocation failed",Some(entity)));
    unsafe{out.write(buffer);}
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_buffer_free(pointer:*mut Context,buffer:*mut Buffer){
    let ctx=unsafe{context(pointer)};let value=unsafe{*buffer};ctx.checked_buffer(value);
    let allocation=ctx.allocations.remove(&(value.pointer as usize)).unwrap_or_else(||ctx.fail("E_DOUBLE_DROP","native buffer already released",None));
    drop(allocation);unsafe{buffer.write(Buffer{pointer:ptr::null_mut(),length:0,capacity:0});}
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_concat(pointer:*mut Context,out:*mut Buffer,a:*const Buffer,b:*const Buffer,entity:*const u8,entity_len:u64)->u32{
    let ctx=unsafe{context(pointer)};let a=unsafe{*a};let b=unsafe{*b};ctx.checked_buffer(a);ctx.checked_buffer(b);
    let Some(length)=a.length.checked_add(b.length).filter(|length|*length<=isize::MAX as u64)else{return 2};
    let entity=unsafe{text(entity,entity_len)};ctx.check_allocation_event(entity);
    match ctx.allocator.allocate(length as usize){
        Ok(mut owned)=>{owned.as_mut_slice()[..a.length as usize].copy_from_slice(unsafe{bytes(a.pointer,a.length)});owned.as_mut_slice()[a.length as usize..].copy_from_slice(unsafe{bytes(b.pointer,b.length)});let buffer=ctx.adopt(owned,entity);unsafe{out.write(buffer);}0},
        Err(AllocFailure::CapacityOverflow)=>2,Err(AllocFailure::ResourceLimit|AllocFailure::OutOfMemory)=>1,
    }
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_print_i64(pointer:*mut Context,value:i64)->u32{unsafe{context(pointer)}.print(value.to_string().as_bytes())}
#[no_mangle]
pub unsafe extern "C" fn il_rt_print_buffer(pointer:*mut Context,buffer:*const Buffer)->u32{
    let ctx=unsafe{context(pointer)};let buffer=unsafe{*buffer};ctx.checked_buffer(buffer);ctx.print(unsafe{bytes(buffer.pointer,buffer.length)})
}

#[no_mangle]
pub unsafe extern "C" fn il_rt_trace_begin(pointer:*mut Context,kind:u32,entity:*const u8,length:u64){
    let ctx=unsafe{context(pointer)};if !ctx.captured{return}if ctx.pending.is_some(){ctx.fail("E_STATE_INCONSISTENT","nested lifecycle event",None)}
    let kind=match kind{0=>LifecycleKind::Allocate,1=>LifecycleKind::Move,2=>LifecycleKind::Borrow,3=>LifecycleKind::EndBorrow,4=>LifecycleKind::Drop,5=>LifecycleKind::Return,_=>ctx.fail("E_STATE_INCONSISTENT","unknown lifecycle kind",None)};
    ctx.pending=Some(LifecycleEvent{kind,entity_id:unsafe{text(entity,length)}.into(),allocation_ids:vec![]});
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_trace_buffer(pointer:*mut Context,buffer:*const Buffer){
    let ctx=unsafe{context(pointer)};if !ctx.captured{return}let buffer=unsafe{*buffer};ctx.checked_buffer(buffer);
    let id=ctx.allocations[&(buffer.pointer as usize)].id;
    let Some(event)=ctx.pending.as_mut()else{ctx.fail("E_STATE_INCONSISTENT","lifecycle leaf without event",None)};event.allocation_ids.push(id);
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_trace_end(pointer:*mut Context){let ctx=unsafe{context(pointer)};if !ctx.captured{return}let event=ctx.pending.take().unwrap_or_else(||ctx.fail("E_STATE_INCONSISTENT","lifecycle end without event",None));ctx.event(event);}

#[no_mangle]
pub unsafe extern "C" fn il_rt_json_raw(pointer:*mut Context,value:*const u8,length:u64){unsafe{context(pointer)}.emit(unsafe{bytes(value,length)});}
#[no_mangle]
pub unsafe extern "C" fn il_rt_json_quoted(pointer:*mut Context,value:*const u8,length:u64){
    let ctx=unsafe{context(pointer)};if !ctx.captured{return}let value=unsafe{text(value,length)};
    let length=value.bytes().map(|b|match b{b'"'|b'\\'|b'\n'|b'\r'|b'\t'|8|12=>2u64,0..=31=>6,_=>1}).sum::<u64>()+2;
    ctx.charge(length);let encoded=serde_json::to_vec(value).unwrap_or_else(|_|ctx.fail("E_STATE_INCONSISTENT","string encoding failed",None));ctx.json.extend(encoded);
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_json_i64(pointer:*mut Context,value:i64){let string=format!("\"{value}\"");unsafe{context(pointer)}.emit(string.as_bytes());}
#[no_mangle]
pub unsafe extern "C" fn il_rt_json_u64(pointer:*mut Context,value:u64){let string=format!("\"{value}\"");unsafe{context(pointer)}.emit(string.as_bytes());}
#[no_mangle]
pub unsafe extern "C" fn il_rt_json_bytes(pointer:*mut Context,value:*const u8,length:u64){
    let ctx=unsafe{context(pointer)};if !ctx.captured{return}let value=unsafe{bytes(value,length)};
    let length=2+value.iter().map(|b|if *b>=100{3}else if *b>=10{2}else{1}).sum::<u64>()+value.len().saturating_sub(1) as u64;ctx.charge(length);
    let encoded=serde_json::to_vec(value).unwrap_or_else(|_|ctx.fail("E_STATE_INCONSISTENT","byte encoding failed",None));ctx.json.extend(encoded);
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_json_buffer(pointer:*mut Context,source:*const Buffer,utf8:u32){
    let ctx=unsafe{context(pointer)};let source=unsafe{*source};ctx.checked_buffer(source);
    match utf8{
        0=>unsafe{il_rt_json_bytes(pointer,source.pointer,source.length)},
        1=>unsafe{il_rt_json_quoted(pointer,source.pointer,source.length)},
        _=>ctx.fail("E_STATE_INCONSISTENT","invalid buffer encoding mode",None),
    }
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_shape_begin(pointer:*mut Context,kind:u32){let ctx=unsafe{context(pointer)};ctx.shape_nodes=0;ctx.shape_depth=0;ctx.shape_kind=kind;}
#[no_mangle]
pub unsafe extern "C" fn il_rt_shape_enter(pointer:*mut Context){let ctx=unsafe{context(pointer)};if !ctx.captured{return}ctx.shape_nodes+=1;ctx.shape_depth+=1;if ctx.shape_depth>65||ctx.shape_nodes>100_000{ctx.fail("E_RESOURCE_LIMIT",if ctx.shape_kind==0{"returned value shape budget exceeded"}else{"clone shape budget exceeded"},None)}}
#[no_mangle]
pub unsafe extern "C" fn il_rt_shape_leave(pointer:*mut Context){let ctx=unsafe{context(pointer)};if !ctx.captured{return}if ctx.shape_depth==0{ctx.fail("E_STATE_INCONSISTENT","unbalanced shape traversal",None)}ctx.shape_depth-=1;}

#[no_mangle]
pub unsafe extern "C" fn il_rt_authorize(pointer:*mut Context,requirements:*const u8,length:u64,policy_fd:i32){
    let ctx=unsafe{context(pointer)};
    if ctx.authorized||ctx.steps!=0||ctx.depth!=0||ctx.allocator.attempts()!=0||ctx.host.live_handles()!=0{ctx.fail("E_STATE_INCONSISTENT","authority must be initialized once before user execution",None)}
    let requirements:Vec<Capability>=serde_json::from_slice(unsafe{bytes(requirements,length)}).unwrap_or_else(|_|ctx.fail("E_HOST_POLICY_INVALID","malformed compiled requirement table",None));
    if requirements.windows(2).any(|pair|pair[0].entity_id>=pair[1].entity_id){ctx.fail("E_HOST_POLICY_INVALID","compiled requirements are not uniquely ordered",None)}
    let policy=il_runtime_startup::load_policy_fd(policy_fd).unwrap_or_else(|error|ctx.fail(error.code,&error.message,None));
    let policy=ValidatedPolicy::new(policy,&requirements,ctx.captured).unwrap_or_else(|error|ctx.fail(error.code,&error.message,None));
    let allocator=Allocator::new(policy.faults(),if ctx.captured{Some(ctx.limits.max_heap_bytes)}else{None});
    let stdio=InheritedStdio::capture().unwrap_or_else(|error|ctx.fail(error.code,&error.message,None));
    ctx.host=HostResources::new(policy,stdio,allocator.clone());ctx.allocator=allocator;ctx.authorized=true;
}
unsafe fn deadline(ctx:&mut Context,value:*const DeadlineValue)->Deadline{
    if value.is_null(){ctx.fail("E_STATE_INCONSISTENT","null deadline",None)}let value=unsafe{*value};match value.tag{0=>Deadline::Infinite,1=>Deadline::At(value.instant),_=>ctx.fail("E_STATE_INCONSISTENT","invalid deadline tag",None)}
}
unsafe fn file(ctx:&mut Context,value:*const HandleId)->HandleId{if value.is_null(){ctx.fail("E_STATE_INCONSISTENT","null file",None)}unsafe{*value}}
unsafe fn path<'a>(ctx:&mut Context,value:*const Buffer)->&'a str{let value=unsafe{*value};ctx.checked_buffer(value);std::str::from_utf8(unsafe{bytes(value.pointer,value.length)}).unwrap_or_else(|_|ctx.fail("E_STATE_INCONSISTENT","file path is not UTF-8",None))}
fn return_buffer(ctx:&mut Context,out:*mut Buffer,result:Result<OwnedBuffer,IoFailure>)->u32{ctx.drain_handles();match result{Ok(buffer)=>{let entity=ctx.current.clone();let value=ctx.adopt(buffer,&entity);unsafe{out.write(value);}0},Err(error)=>ctx.io_status(Err(error))}}
fn return_count(ctx:&mut Context,out:*mut u64,result:Result<usize,IoFailure>)->u32{ctx.drain_handles();match result{Ok(count)=>{unsafe{out.write(count as u64);}0},Err(error)=>ctx.io_status(Err(error))}}
#[no_mangle]
pub unsafe extern "C" fn il_rt_file_open_read(pointer:*mut Context,out:*mut HandleId,grant:u64,value:*const Buffer)->u32{let ctx=unsafe{context(pointer)};let path=unsafe{path(ctx,value)};let result=ctx.host.open_read(GrantId(grant),path);ctx.drain_handles();match result{Ok(handle)=>{unsafe{out.write(handle);}0},Err(error)=>ctx.io_status(Err(error))}}
#[no_mangle]
pub unsafe extern "C" fn il_rt_file_open_write(pointer:*mut Context,out:*mut HandleId,grant:u64,value:*const Buffer)->u32{let ctx=unsafe{context(pointer)};let path=unsafe{path(ctx,value)};let result=ctx.host.open_write(GrantId(grant),path);ctx.drain_handles();match result{Ok(handle)=>{unsafe{out.write(handle);}0},Err(error)=>ctx.io_status(Err(error))}}
#[no_mangle]
pub unsafe extern "C" fn il_rt_file_read_some(pointer:*mut Context,out:*mut Buffer,handle:*const HandleId,maximum:u64,when:*const DeadlineValue)->u32{
    let ctx=unsafe{context(pointer)};let handle=unsafe{file(ctx,handle)};let when=unsafe{deadline(ctx,when)};let entity=ctx.current.clone();ctx.check_allocation_event(&entity);let result=ctx.host.read_some(handle,maximum as usize,when);return_buffer(ctx,out,result)
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_file_write_some(pointer:*mut Context,out:*mut u64,handle:*const HandleId,value:*const Buffer,offset:u64,when:*const DeadlineValue)->u32{
    let ctx=unsafe{context(pointer)};let handle=unsafe{file(ctx,handle)};let when=unsafe{deadline(ctx,when)};let value=unsafe{*value};ctx.checked_buffer(value);let result=ctx.host.write_some(handle,unsafe{bytes(value.pointer,value.length)},offset as usize,when);return_count(ctx,out,result)
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_file_close(pointer:*mut Context,handle:*const HandleId)->u32{let ctx=unsafe{context(pointer)};let handle=unsafe{file(ctx,handle)};let result=ctx.host.close(handle);ctx.drain_handles();ctx.io_status(result)}
#[no_mangle]
pub unsafe extern "C" fn il_rt_file_drop(pointer:*mut Context,handle:*const HandleId){let ctx=unsafe{context(pointer)};let handle=unsafe{file(ctx,handle)};let result=ctx.host.drop_handle(handle);ctx.drain_handles();let _=ctx.io_status(result);}
#[no_mangle]
pub unsafe extern "C" fn il_rt_trace_file(pointer:*mut Context,handle:*const HandleId){let ctx=unsafe{context(pointer)};let handle=unsafe{file(ctx,handle)};if ctx.host.validate_handle(handle).is_err(){ctx.fail("E_STATE_INCONSISTENT","unknown or closed native file",None)}}
#[no_mangle]
pub unsafe extern "C" fn il_rt_json_file(pointer:*mut Context,handle:*const HandleId){unsafe{il_rt_trace_file(pointer,handle);}let ctx=unsafe{context(pointer)};let handle=unsafe{*handle};let json=format!("{{\"kind\":\"file\",\"slot\":{},\"generation\":{}}}",handle.slot,handle.generation);ctx.emit(json.as_bytes());}
#[no_mangle]
pub unsafe extern "C" fn il_rt_file_read(pointer:*mut Context,out:*mut Buffer,grant:u64,value:*const Buffer)->u32{let ctx=unsafe{context(pointer)};let path=unsafe{path(ctx,value)};let entity=ctx.current.clone();ctx.check_allocation_event(&entity);let result=ctx.host.read_all(GrantId(grant),path);return_buffer(ctx,out,result)}
#[no_mangle]
pub unsafe extern "C" fn il_rt_file_write(pointer:*mut Context,out:*mut u64,grant:u64,name:*const Buffer,value:*const Buffer)->u32{let ctx=unsafe{context(pointer)};let path=unsafe{path(ctx,name)};let value=unsafe{*value};ctx.checked_buffer(value);let result=ctx.host.write_all(GrantId(grant),path,unsafe{bytes(value.pointer,value.length)});return_count(ctx,out,result)}
#[no_mangle]
pub unsafe extern "C" fn il_rt_stdin_read(pointer:*mut Context,out:*mut Buffer,maximum:u64,when:*const DeadlineValue)->u32{let ctx=unsafe{context(pointer)};let when=unsafe{deadline(ctx,when)};let entity=ctx.current.clone();ctx.check_allocation_event(&entity);let result=ctx.host.stdin_read(maximum as usize,when);return_buffer(ctx,out,result)}
unsafe fn stream_write(pointer:*mut Context,out:*mut u64,value:*const Buffer,offset:u64,when:*const DeadlineValue,stderr:bool)->u32{
    let ctx=unsafe{context(pointer)};let when=unsafe{deadline(ctx,when)};let value=unsafe{*value};ctx.checked_buffer(value);let data=unsafe{bytes(value.pointer,value.length)};
    if offset<=value.length{ctx.check_charge(value.length-offset);}
    let result=if stderr{ctx.host.stderr_write(data,offset as usize,when)}else{ctx.host.stdout_write(data,offset as usize,when)};
    if let Ok(count)=result{ctx.capture_output(&data[offset as usize..offset as usize+count],stderr);}return_count(ctx,out,result)
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_stdout_write(pointer:*mut Context,out:*mut u64,value:*const Buffer,offset:u64,when:*const DeadlineValue)->u32{unsafe{stream_write(pointer,out,value,offset,when,false)}}
#[no_mangle]
pub unsafe extern "C" fn il_rt_stderr_write(pointer:*mut Context,out:*mut u64,value:*const Buffer,offset:u64,when:*const DeadlineValue)->u32{unsafe{stream_write(pointer,out,value,offset,when,true)}}
#[no_mangle]
pub unsafe extern "C" fn il_rt_clock_now(pointer:*mut Context,grant:u64)->u64{let ctx=unsafe{context(pointer)};ctx.host.clock_now(GrantId(grant)).unwrap_or_else(|_|ctx.fail("E_CAPABILITY_MISSING","clock grant is not authorized",None))}
