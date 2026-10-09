//! Native ABI substrate. This crate contains no language opcode dispatcher or IR reader.
use il_execution_model::{Execution,ExecutionStatus,LifecycleEvent,LifecycleKind,Limits,Value};
use il_graph::Diagnostic;
use serde::Deserialize;
use std::{alloc::{alloc,dealloc,Layout},collections::BTreeMap,fs::File,io::Write,ptr,slice};

#[repr(C)]
#[derive(Clone,Copy,Debug)]
pub struct Buffer { pub pointer:*mut u8,pub length:u64,pub capacity:u64 }

struct Allocation { id:u64,charged:u64,capacity:usize }
pub struct Context {
    captured:bool,limits:Limits,revision:u64,report_fd:i32,steps:u64,depth:u32,heap:u64,peak:u64,next_id:u64,
    allocations:BTreeMap<usize,Allocation>,stdout:Vec<u8>,events:Vec<LifecycleEvent>,capture_bytes:u64,
    current:String,json:Vec<u8>,pending:Option<LifecycleEvent>,shape_nodes:u64,shape_depth:u32,shape_kind:u32,
}

impl Drop for Context {
    fn drop(&mut self){for(pointer,allocation)in &self.allocations{unsafe{dealloc(*pointer as *mut u8,Layout::from_size_align_unchecked(allocation.capacity,1));}}}
}

impl Context {
    fn fail(&mut self,code:&str,message:&str,entity:Option<&str>)->! {
        let entity=entity.unwrap_or(&self.current).to_owned();
        let mut diagnostic=Diagnostic::error(code,Some(&entity),message,self.revision);diagnostic.stage="native_runtime".into();diagnostic.retryable=false;
        if self.captured {self.report(ExecutionStatus::Trapped,None,vec![diagnostic]);}
        else {let _=writeln!(std::io::stderr().lock(),"{code} at {entity}: {message}");}
        std::process::exit(101)
    }
    fn charge(&mut self,bytes:u64){if !self.captured{return}if bytes>self.limits.max_output_bytes.saturating_sub(self.capture_bytes){self.fail("E_RESOURCE_LIMIT","capture budget exhausted",None)}self.capture_bytes+=bytes;}
    fn event(&mut self,event:LifecycleEvent){
        if !self.captured{return}
        let encoded=serde_json::to_vec(&event).unwrap_or_else(|_|self.fail("E_STATE_INCONSISTENT","lifecycle encoding failed",None));
        self.charge(encoded.len() as u64+u64::from(!self.events.is_empty()));self.events.push(event);
    }
    fn emit(&mut self,bytes:&[u8]){if self.captured{self.charge(bytes.len() as u64);self.json.extend_from_slice(bytes);}}
    fn report(&mut self,status:ExecutionStatus,value:Option<Value>,diagnostics:Vec<Diagnostic>){
        let execution=Execution{status,value,stdout:std::mem::take(&mut self.stdout),stderr:vec![],diagnostics,steps:self.steps,peak_heap_bytes:self.peak,live_allocations:self.allocations.len() as u64,lifecycle:std::mem::take(&mut self.events)};
        let encoded=serde_json::to_vec(&execution).unwrap_or_else(|_|std::process::exit(102));
        #[cfg(unix)]{
            use std::os::fd::FromRawFd;
            if self.report_fd<0{std::process::exit(102)}
            let mut file=unsafe{File::from_raw_fd(self.report_fd)};
            if file.write_all(&encoded).and_then(|_|file.flush()).is_err(){std::process::exit(102)}
            self.report_fd=-1;
        }
        #[cfg(not(unix))]{let _=encoded;std::process::exit(102)}
    }
    fn try_buffer(&mut self,data:&[u8],entity:&str)->Option<Buffer>{
        let charged=data.len() as u64;
        if self.captured&&charged>self.limits.max_heap_bytes.saturating_sub(self.heap){return None}
        let capacity=data.len().max(1);let layout=Layout::from_size_align(capacity,1).ok()?;
        let pointer=unsafe{alloc(layout)};if pointer.is_null(){return None}
        unsafe{ptr::copy_nonoverlapping(data.as_ptr(),pointer,data.len());}
        let id=self.next_id;
        self.event(LifecycleEvent{kind:LifecycleKind::Allocate,entity_id:entity.into(),allocation_ids:vec![id]});
        self.next_id+=1;self.allocations.insert(pointer as usize,Allocation{id,charged,capacity});self.heap+=charged;self.peak=self.peak.max(self.heap);
        Some(Buffer{pointer,length:charged,capacity:capacity as u64})
    }
    fn checked_buffer(&mut self,buffer:Buffer)->&[u8]{
        let Some(allocation)=self.allocations.get(&(buffer.pointer as usize))else{self.fail("E_STATE_INCONSISTENT","unknown native buffer allocation",None)};
        if allocation.charged!=buffer.length||allocation.capacity as u64!=buffer.capacity{self.fail("E_STATE_INCONSISTENT","native buffer layout mismatch",None)}
        unsafe{slice::from_raw_parts(buffer.pointer,buffer.length as usize)}
    }
    fn print(&mut self,data:&[u8])->u32{
        if self.captured&&data.len() as u64>self.limits.max_output_bytes.saturating_sub(self.capture_bytes){self.fail("E_RESOURCE_LIMIT","capture budget exhausted",None)}
        let mut written=0;
        while written<data.len(){
            // Only the OS write result establishes progress. A buffered writer
            // could accept bytes and subsequently lose them on a failed flush.
            let count=unsafe{libc::write(libc::STDOUT_FILENO,data[written..].as_ptr().cast(),data.len()-written)};
            if count<0{if std::io::Error::last_os_error().kind()==std::io::ErrorKind::Interrupted{continue}break}
            if count==0{break}
            let count=count as usize;
            if self.captured{self.stdout.extend_from_slice(&data[written..written+count]);self.capture_bytes+=count as u64;}
            written+=count;
        }
        if written==data.len(){0}else{2}
    }
}

unsafe fn context<'a>(pointer:*mut Context)->&'a mut Context{if pointer.is_null(){std::process::exit(102)}unsafe{&mut *pointer}}
unsafe fn bytes<'a>(pointer:*const u8,length:u64)->&'a [u8]{if length==0{return &[]}if pointer.is_null()||length>isize::MAX as u64{std::process::exit(102)}unsafe{slice::from_raw_parts(pointer,length as usize)}}
unsafe fn text<'a>(pointer:*const u8,length:u64)->&'a str{std::str::from_utf8(unsafe{bytes(pointer,length)}).unwrap_or_else(|_|std::process::exit(102))}

#[no_mangle]
pub extern "C" fn il_rt_context_new(mode:u32,max_steps:u64,max_depth:u32,max_heap:u64,max_output:u64,revision:u64,report_fd:i32)->*mut Context{
    let limits=Limits{max_steps,max_call_depth:max_depth,max_heap_bytes:max_heap,max_output_bytes:max_output};
    if mode>1||(mode==1&&(!limits.valid()||report_fd<0)){std::process::exit(102)}
    // LLVM supplies main, so Rust's usual std startup does not install its
    // SIGPIPE policy. Closed output pipes must return EPIPE to the typed API.
    #[cfg(unix)]unsafe{
        let mut action:libc::sigaction=std::mem::zeroed();action.sa_sigaction=libc::SIG_IGN;
        if libc::sigemptyset(&mut action.sa_mask)!=0||libc::sigaction(libc::SIGPIPE,&action,ptr::null_mut())!=0{std::process::exit(102)}
    }
    Box::into_raw(Box::new(Context{captured:mode==1,limits,revision,report_fd,steps:0,depth:0,heap:0,peak:0,next_id:1,allocations:BTreeMap::new(),stdout:vec![],events:vec![],capture_bytes:0,current:"entry".into(),json:vec![],pending:None,shape_nodes:0,shape_depth:0,shape_kind:0}))
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
pub unsafe extern "C" fn il_rt_enter(pointer:*mut Context,entity:*const u8,length:u64){let ctx=unsafe{context(pointer)};if ctx.depth==0{ctx.current=unsafe{text(entity,length)}.into();}if ctx.captured&&ctx.depth>=ctx.limits.max_call_depth{ctx.fail("E_RESOURCE_LIMIT","call depth budget exhausted",None)}ctx.depth+=1;}
#[no_mangle]
pub unsafe extern "C" fn il_rt_leave(pointer:*mut Context){let ctx=unsafe{context(pointer)};if ctx.depth==0{ctx.fail("E_STATE_INCONSISTENT","unbalanced native call depth",None)}ctx.depth-=1;}
#[no_mangle]
pub unsafe extern "C" fn il_rt_tick(pointer:*mut Context,entity:*const u8,length:u64){let ctx=unsafe{context(pointer)};ctx.current=unsafe{text(entity,length)}.into();if ctx.captured&&ctx.steps>=ctx.limits.max_steps{ctx.fail("E_RESOURCE_LIMIT","instruction budget exhausted",None)}ctx.steps=ctx.steps.saturating_add(1);}
#[no_mangle]
pub unsafe extern "C" fn il_rt_location(pointer:*mut Context,entity:*const u8,length:u64){unsafe{context(pointer)}.current=unsafe{text(entity,length)}.into();}
#[no_mangle]
pub unsafe extern "C" fn il_rt_trap(pointer:*mut Context,code:*const u8,code_len:u64,message:*const u8,message_len:u64,entity:*const u8,entity_len:u64)->!{unsafe{context(pointer)}.fail(unsafe{text(code,code_len)},unsafe{text(message,message_len)},Some(unsafe{text(entity,entity_len)}))}

#[no_mangle]
pub unsafe extern "C" fn il_rt_buffer_new(pointer:*mut Context,out:*mut Buffer,source:*const u8,length:u64,entity:*const u8,entity_len:u64){
    let ctx=unsafe{context(pointer)};let entity=unsafe{text(entity,entity_len)};let source=unsafe{bytes(source,length)};
    let buffer=ctx.try_buffer(source,entity).unwrap_or_else(||ctx.fail("E_RESOURCE_LIMIT","heap budget exhausted",Some(entity)));
    unsafe{out.write(buffer);}
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_buffer_clone(pointer:*mut Context,out:*mut Buffer,source:*const Buffer,entity:*const u8,entity_len:u64){
    let ctx=unsafe{context(pointer)};let source=unsafe{*source};ctx.checked_buffer(source);
    let entity=unsafe{text(entity,entity_len)};
    let buffer=ctx.try_buffer(unsafe{bytes(source.pointer,source.length)},entity).unwrap_or_else(||ctx.fail("E_RESOURCE_LIMIT","heap budget exhausted",Some(entity)));
    unsafe{out.write(buffer);}
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_buffer_free(pointer:*mut Context,buffer:*mut Buffer){
    let ctx=unsafe{context(pointer)};let value=unsafe{*buffer};ctx.checked_buffer(value);
    let allocation=ctx.allocations.remove(&(value.pointer as usize)).unwrap_or_else(||ctx.fail("E_DOUBLE_DROP","native buffer already released",None));
    ctx.heap-=allocation.charged;unsafe{dealloc(value.pointer,Layout::from_size_align_unchecked(allocation.capacity,1));buffer.write(Buffer{pointer:ptr::null_mut(),length:0,capacity:0});}
}
#[no_mangle]
pub unsafe extern "C" fn il_rt_concat(pointer:*mut Context,out:*mut Buffer,a:*const Buffer,b:*const Buffer,entity:*const u8,entity_len:u64)->u32{
    let ctx=unsafe{context(pointer)};let a=unsafe{*a};let b=unsafe{*b};ctx.checked_buffer(a);ctx.checked_buffer(b);
    let Some(length)=a.length.checked_add(b.length).filter(|length|*length<=isize::MAX as u64)else{return 2};
    if ctx.captured&&length>ctx.limits.max_heap_bytes.saturating_sub(ctx.heap){return 1}
    let mut data=Vec::new();if data.try_reserve_exact(length as usize).is_err(){return 1}
    data.extend_from_slice(unsafe{bytes(a.pointer,a.length)});data.extend_from_slice(unsafe{bytes(b.pointer,b.length)});
    if let Some(buffer)=ctx.try_buffer(&data,unsafe{text(entity,entity_len)}){unsafe{out.write(buffer);}0}else{1}
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
