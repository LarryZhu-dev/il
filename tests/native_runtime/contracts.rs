use il_native_runtime::*;
use std::ptr;

#[test]
fn application_has_no_implicit_budget_and_abi_buffers_are_real(){unsafe{
    let context=il_rt_context_new(0,1,1,1,1,0,-1);let entity=b"allocation";
    for _ in 0..4{il_rt_enter(context,entity.as_ptr(),entity.len() as u64);il_rt_tick(context,entity.as_ptr(),entity.len() as u64);}
    let mut a=Buffer{pointer:ptr::null_mut(),length:0,capacity:0};let mut b=a;let mut c=a;
    il_rt_buffer_new(context,&mut a,b"hello".as_ptr(),5,entity.as_ptr(),entity.len() as u64);
    il_rt_buffer_new(context,&mut b,b" world".as_ptr(),6,entity.as_ptr(),entity.len() as u64);
    assert_eq!(std::slice::from_raw_parts(a.pointer,5),b"hello");
    assert_eq!(il_rt_concat(context,&mut c,&a,&b,entity.as_ptr(),entity.len() as u64),0);assert_eq!(std::slice::from_raw_parts(c.pointer,c.length as usize),b"hello world");
    il_rt_buffer_free(context,&mut c);il_rt_buffer_free(context,&mut b);il_rt_buffer_free(context,&mut a);
    for _ in 0..4{il_rt_leave(context);}il_rt_finish(context);
}}

#[test]
fn empty_buffers_have_distinct_physical_identities(){unsafe{
    let context=il_rt_context_new(0,0,0,0,0,0,-1);let mut a=Buffer{pointer:ptr::null_mut(),length:0,capacity:0};let mut b=a;
    il_rt_buffer_new(context,&mut a,ptr::null(),0,b"a".as_ptr(),1);il_rt_buffer_new(context,&mut b,ptr::null(),0,b"b".as_ptr(),1);
    assert_ne!(a.pointer,b.pointer);assert_eq!(a.length,0);assert_eq!(a.capacity,1);il_rt_buffer_free(context,&mut a);il_rt_buffer_free(context,&mut b);il_rt_finish(context);
}}

#[test]
fn buffer_layout_matches_frozen_abi(){assert_eq!(std::mem::size_of::<Buffer>(),24);assert_eq!(std::mem::align_of::<Buffer>(),8);}

#[cfg(unix)]
fn report_file(name:&str)->(std::path::PathBuf,i32){
    use std::os::fd::IntoRawFd;
    let path=std::env::temp_dir().join(format!("il-native-runtime-{}-{name}.json",std::process::id()));
    let file=std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();(path,file.into_raw_fd())
}

#[cfg(unix)]
#[test]
fn captured_values_trace_and_typed_allocation_failure(){unsafe{
    let(path,fd)=report_file("capture");let ctx=il_rt_context_new(1,100,8,3,10000,7,fd);
    let mut buffer=Buffer{pointer:ptr::null_mut(),length:0,capacity:0};let mut unused=buffer;
    il_rt_enter(ctx,b"main".as_ptr(),4);il_rt_tick(ctx,b"allocate".as_ptr(),8);il_rt_buffer_new(ctx,&mut buffer,b"abc".as_ptr(),3,b"allocate".as_ptr(),8);
    assert_eq!(il_rt_concat(ctx,&mut unused,&buffer,&buffer,b"concat".as_ptr(),6),1);
    il_rt_trace_begin(ctx,4,b"drop".as_ptr(),4);il_rt_trace_buffer(ctx,&buffer);il_rt_trace_end(ctx);il_rt_buffer_free(ctx,&mut buffer);il_rt_leave(ctx);
    let start=br#"{"type":"U64","data":{"kind":"integer","value":"#;il_rt_json_raw(ctx,start.as_ptr(),start.len() as u64);il_rt_json_u64(ctx,u64::MAX);il_rt_json_raw(ctx,b"}}".as_ptr(),2);il_rt_finish(ctx);
    let report:il_execution_model::Execution=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();std::fs::remove_file(path).unwrap();
    assert_eq!(report.status,il_execution_model::ExecutionStatus::Returned);assert_eq!(report.value,Some(il_execution_model::Value::integer("U64",u64::MAX as i128)));assert_eq!(report.steps,1);assert_eq!(report.live_allocations,0);assert_eq!(report.peak_heap_bytes,3);assert_eq!(report.lifecycle.len(),2);assert_eq!(report.lifecycle[0].allocation_ids,vec![1]);assert_eq!(report.lifecycle[1].kind,il_execution_model::LifecycleKind::Drop);
}}

#[cfg(unix)]
#[test]
fn captured_json_utf8_control_and_bytes(){unsafe{
    let(path,fd)=report_file("json");let ctx=il_rt_context_new(1,100,8,100,10000,0,fd);
    let start=br#"{"type":"String","data":{"kind":"string","value":"#;il_rt_json_raw(ctx,start.as_ptr(),start.len() as u64);let text="中\0\n\"\\";il_rt_json_quoted(ctx,text.as_ptr(),text.len() as u64);il_rt_json_raw(ctx,b"}}".as_ptr(),2);il_rt_finish(ctx);
    let report:il_execution_model::Execution=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();std::fs::remove_file(path).unwrap();assert_eq!(report.value,Some(il_execution_model::Value::string(text)));
}}

#[cfg(unix)]
#[test]
fn captured_trap_child(){
    let Ok(path)=std::env::var("IL_RUNTIME_TRAP_REPORT")else{return};
    use std::os::fd::IntoRawFd;let fd=std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap().into_raw_fd();
    let mode=std::env::var("IL_RUNTIME_TRAP_MODE").unwrap_or_default();
    unsafe{let ctx=il_rt_context_new(1,1,8,100,if mode=="capture"{1}else{10000},0,fd);let mut buffer=Buffer{pointer:ptr::null_mut(),length:0,capacity:0};
        if mode=="shape"{il_rt_shape_begin(ctx,1);for _ in 0..66{il_rt_shape_enter(ctx);}}
        if mode=="forged_clone"||mode=="forged_json"{let forged=Buffer{pointer:1usize as *mut u8,length:16,capacity:16};if mode=="forged_clone"{il_rt_buffer_clone(ctx,&mut buffer,&forged,b"clone".as_ptr(),5)}else{il_rt_json_buffer(ctx,&forged,1)}}
        il_rt_buffer_new(ctx,&mut buffer,b"live".as_ptr(),4,b"owner".as_ptr(),5);il_rt_tick(ctx,b"first".as_ptr(),5);il_rt_tick(ctx,b"exhausted".as_ptr(),9);}
    panic!("resource trap returned");
}

#[cfg(unix)]
#[test]
fn capture_allocation_failure_preserves_previous_owner_counts(){
    let path=std::env::temp_dir().join(format!("il-native-runtime-{}-capture-trap.json",std::process::id()));
    let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","captured_trap_child","--nocapture"]).env("IL_RUNTIME_TRAP_REPORT",&path).env("IL_RUNTIME_TRAP_MODE","capture").output().unwrap();
    assert_eq!(output.status.code(),Some(101));let report:il_execution_model::Execution=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();std::fs::remove_file(path).unwrap();assert_eq!(report.live_allocations,0);assert_eq!(report.peak_heap_bytes,0);assert!(report.lifecycle.is_empty());
}

#[cfg(unix)]
#[test]
fn depth64_result_and_clone_shape_boundary(){unsafe{
    use serde::Deserialize;
    let(path,fd)=report_file("deep");let ctx=il_rt_context_new(1,100,8,100,100000,0,fd);
    il_rt_shape_begin(ctx,0);for _ in 0..65{il_rt_shape_enter(ctx);}for _ in 0..65{il_rt_shape_leave(ctx);}
    let mut json=r#"{"type":"Unit","data":{"kind":"unit"}}"#.to_owned();for _ in 0..64{json=format!(r#"{{"type":"Nested","data":{{"kind":"tuple","value":[{json}]}}}}"#);}
    il_rt_json_raw(ctx,json.as_ptr(),json.len() as u64);il_rt_finish(ctx);
    let bytes=std::fs::read(&path).unwrap();let mut decoder=serde_json::Deserializer::from_slice(&bytes);decoder.disable_recursion_limit();let report=il_execution_model::Execution::deserialize(&mut decoder).unwrap();assert_eq!(report.status,il_execution_model::ExecutionStatus::Returned);std::fs::remove_file(path).unwrap();
    let path=std::env::temp_dir().join(format!("il-native-runtime-{}-shape-trap.json",std::process::id()));let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","captured_trap_child","--nocapture"]).env("IL_RUNTIME_TRAP_REPORT",&path).env("IL_RUNTIME_TRAP_MODE","shape").output().unwrap();assert_eq!(output.status.code(),Some(101));let report:il_execution_model::Execution=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();std::fs::remove_file(path).unwrap();assert_eq!(report.diagnostics[0].code,"E_RESOURCE_LIMIT");assert_eq!(report.diagnostics[0].cause,"clone shape budget exceeded");
}}

#[cfg(unix)]
#[test]
fn registered_buffer_clone_and_result_encoding(){unsafe{
    let(path,fd)=report_file("clone");let ctx=il_rt_context_new(1,100,8,100,10000,0,fd);let text="中\0";
    let mut original=Buffer{pointer:ptr::null_mut(),length:0,capacity:0};let mut copy=original;
    il_rt_buffer_new(ctx,&mut original,text.as_ptr(),text.len() as u64,b"original".as_ptr(),8);
    il_rt_buffer_clone(ctx,&mut copy,&original,b"clone".as_ptr(),5);assert_ne!(original.pointer,copy.pointer);il_rt_buffer_free(ctx,&mut original);
    let start=br#"{"type":"String","data":{"kind":"string","value":"#;il_rt_json_raw(ctx,start.as_ptr(),start.len() as u64);il_rt_json_buffer(ctx,&copy,1);il_rt_json_raw(ctx,b"}}".as_ptr(),2);il_rt_finish(ctx);
    let report:il_execution_model::Execution=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();std::fs::remove_file(path).unwrap();assert_eq!(report.value,Some(il_execution_model::Value::string(text)));assert_eq!(report.live_allocations,1);assert_eq!(report.peak_heap_bytes,8);
}}

#[cfg(unix)]
#[test]
fn forged_payload_pointer_is_rejected_before_reading(){
    for mode in ["forged_clone","forged_json"]{
        let path=std::env::temp_dir().join(format!("il-native-runtime-{}-{mode}.json",std::process::id()));let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","captured_trap_child","--nocapture"]).env("IL_RUNTIME_TRAP_REPORT",&path).env("IL_RUNTIME_TRAP_MODE",mode).output().unwrap();assert_eq!(output.status.code(),Some(101));let report:il_execution_model::Execution=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();std::fs::remove_file(path).unwrap();assert_eq!(report.diagnostics[0].code,"E_STATE_INCONSISTENT");assert_eq!(report.live_allocations,0);
    }
}

#[cfg(unix)]
#[test]
fn resource_trap_exit_and_no_unwind(){
    let path=std::env::temp_dir().join(format!("il-native-runtime-{}-trap.json",std::process::id()));
    let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","captured_trap_child","--nocapture"]).env("IL_RUNTIME_TRAP_REPORT",&path).output().unwrap();
    assert_eq!(output.status.code(),Some(101));assert!(output.stderr.is_empty());let report:il_execution_model::Execution=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();std::fs::remove_file(path).unwrap();
    assert_eq!(report.status,il_execution_model::ExecutionStatus::Trapped);assert_eq!(report.diagnostics[0].code,"E_RESOURCE_LIMIT");assert_eq!(report.diagnostics[0].entity_id.as_deref(),Some("exhausted"));assert_eq!(report.live_allocations,1);assert_eq!(report.steps,1);assert!(!report.lifecycle.iter().any(|v|v.kind==il_execution_model::LifecycleKind::Drop));
}
