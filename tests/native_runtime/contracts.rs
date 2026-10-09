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
        if mode=="stack_enter_budget"{il_rt_enter(ctx,b"outer".as_ptr(),5);il_rt_location(ctx,b"call_site".as_ptr(),9);let long="a".repeat(600_000);il_rt_enter(ctx,long.as_ptr(),long.len()as u64);panic!("oversized frame accepted");}
        if mode=="stack_location_budget"{il_rt_enter(ctx,b"outer".as_ptr(),5);il_rt_location(ctx,b"safe".as_ptr(),4);let long="a".repeat(1_048_576);il_rt_location(ctx,long.as_ptr(),long.len()as u64);panic!("oversized location accepted");}
        if mode=="stack_initial_budget"{il_rt_location(ctx,b"safe".as_ptr(),4);let long="a".repeat(1_048_576);il_rt_location(ctx,long.as_ptr(),long.len()as u64);panic!("oversized initial location accepted");}
        if mode=="stack_explicit_budget"{il_rt_enter(ctx,b"outer".as_ptr(),5);il_rt_location(ctx,b"safe".as_ptr(),4);let long="a".repeat(1_048_576);il_rt_trap(ctx,b"E_PANIC".as_ptr(),7,b"panic".as_ptr(),5,long.as_ptr(),long.len()as u64);}
        if mode=="concat_attempts"{authorize_policy(ctx,"concat-attempts",serde_json::json!({"schema_version":"1.0.0","grants":[],"test_faults":{"allocation_fail_after":2,"io_max_chunk":null,"io_fail_after":null,"accept_fail_after":null}}),serde_json::json!([]));let mut a=buffer;il_rt_buffer_new(ctx,&mut a,[0u8;100].as_ptr(),100,b"full".as_ptr(),4);let mut unused=buffer;assert_eq!(il_rt_concat(ctx,&mut unused,&a,&a,b"quota".as_ptr(),5),1);il_rt_buffer_free(ctx,&mut a);il_rt_buffer_new(ctx,&mut unused,ptr::null(),0,b"after".as_ptr(),5);panic!("quota allocation attempt did not consume the fault counter");}
        if mode=="stack"{il_rt_enter(ctx,b"outer".as_ptr(),5);il_rt_location(ctx,b"call_inner".as_ptr(),10);il_rt_enter(ctx,b"inner".as_ptr(),5);il_rt_location(ctx,b"panic_site".as_ptr(),10);il_rt_trap(ctx,b"E_PANIC".as_ptr(),7,b"panic".as_ptr(),5,b"panic_site".as_ptr(),10);}
        if mode=="quota"{let deadline=DeadlineValue{tag:0,padding:0,instant:0};il_rt_stdin_read(ctx,&mut buffer,101,&deadline);}
        if mode=="missing_grant"{let requirements=br#"[{"entity_id":"read","kind":"FileRead","scope":"/tmp"}]"#;il_rt_authorize(ctx,requirements.as_ptr(),requirements.len()as u64,i32::MAX);}
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

#[cfg(target_os="linux")]
#[test]
fn failed_stdout_child(){
    let Ok(mode)=std::env::var("IL_RUNTIME_STDOUT_FAILURE")else{return};
    use std::os::fd::IntoRawFd;
    let captured=mode!="application_pipe";
    let fd=if captured{std::fs::OpenOptions::new().write(true).create_new(true).open(std::env::var("IL_RUNTIME_STDOUT_REPORT").unwrap()).unwrap().into_raw_fd()}else{-1};
    unsafe{
        // Rust's test harness ordinarily ignores SIGPIPE before running tests;
        // restore a native C/LLVM startup signal disposition before the call.
        let mut action:libc::sigaction=std::mem::zeroed();action.sa_sigaction=libc::SIG_DFL;
        assert_eq!(libc::sigemptyset(&mut action.sa_mask),0);assert_eq!(libc::sigaction(libc::SIGPIPE,&action,ptr::null_mut()),0);
        let failed_fd=if mode=="captured_full"{std::fs::OpenOptions::new().write(true).open("/dev/full").unwrap().into_raw_fd()}else{
            let mut pipe=[-1;2];assert_eq!(libc::pipe(pipe.as_mut_ptr()),0);assert_eq!(libc::close(pipe[0]),0);pipe[1]
        };
        assert_eq!(libc::dup2(failed_fd,libc::STDOUT_FILENO),libc::STDOUT_FILENO);assert_eq!(libc::close(failed_fd),0);
        let ctx=il_rt_context_new(captured as u32,100,8,100,10000,0,fd);
        let status=il_rt_print_i64(ctx,42);assert_eq!(status,2,"write must return IoError::Write + 1");
        if captured{
            let start=br#"{"type":"U32","data":{"kind":"integer","value":"#;
            il_rt_json_raw(ctx,start.as_ptr(),start.len() as u64);il_rt_json_u64(ctx,status as u64);il_rt_json_raw(ctx,b"}}".as_ptr(),2);
        }
        il_rt_finish(ctx);
    }
    std::process::exit(0);
}

#[cfg(target_os="linux")]
#[test]
fn startup_turns_broken_pipe_into_typed_error_without_false_capture(){
    for mode in ["application_pipe","captured_pipe","captured_full"]{
        let path=std::env::temp_dir().join(format!("il-native-runtime-{}-{mode}.json",std::process::id()));
        let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","failed_stdout_child","--nocapture"]).env("IL_RUNTIME_STDOUT_FAILURE",mode).env("IL_RUNTIME_STDOUT_REPORT",&path).output().unwrap();
        assert_eq!(output.status.code(),Some(0),"{mode}: {:?}",output);assert!(output.stderr.is_empty());
        if mode!="application_pipe"{
            let report:il_execution_model::Execution=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();std::fs::remove_file(path).unwrap();
            assert_eq!(report.status,il_execution_model::ExecutionStatus::Returned);assert_eq!(report.value,Some(il_execution_model::Value::integer("U32",2)));assert!(report.stdout.is_empty(),"OS rejected every byte; captured stdout must be empty");assert!(report.stderr.is_empty());
        }
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

#[cfg(unix)]
unsafe fn authorize_policy(ctx:*mut Context,name:&str,policy:serde_json::Value,requirements:serde_json::Value){
    use std::os::fd::AsRawFd;use std::io::Write;
    let path=std::env::temp_dir().join(format!("il-native-policy-{}-{name}.json",std::process::id()));let mut file=std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap();file.write_all(&serde_json::to_vec(&policy).unwrap()).unwrap();
    let requirements=serde_json::to_vec(&requirements).unwrap();unsafe{il_rt_authorize(ctx,requirements.as_ptr(),requirements.len()as u64,file.as_raw_fd());}drop(file);std::fs::remove_file(path).unwrap();
}

#[cfg(unix)]
#[test]
fn captured_host_file_abi_and_returned_owner_cleanup(){unsafe{
    use il_runtime_handles::HandleId;
    let scope=std::env::temp_dir().join(format!("il-native-scope-{}",std::process::id()));std::fs::create_dir(&scope).unwrap();std::fs::write(scope.join("input"),b"abcdef").unwrap();
    let(path,fd)=report_file("host");let ctx=il_rt_context_new(1,100,8,1000,10000,0,fd);
    let grants=serde_json::json!([{"entity_id":"read","kind":"FileRead","scope":scope.to_str().unwrap()}]);authorize_policy(ctx,"host",serde_json::json!({"schema_version":"1.0.0","grants":grants,"test_faults":null}),grants);
    il_rt_enter(ctx,b"main".as_ptr(),4);il_rt_location(ctx,b"open".as_ptr(),4);let mut name=Buffer{pointer:ptr::null_mut(),length:0,capacity:0};il_rt_buffer_new(ctx,&mut name,b"input".as_ptr(),5,b"name".as_ptr(),4);
    let mut file=HandleId{slot:99,generation:99};assert_eq!(il_rt_file_open_read(ctx,&mut file,0,&name),0);il_rt_trace_file(ctx,&file);
    let deadline=DeadlineValue{tag:0,padding:0,instant:0};let mut value=Buffer{pointer:ptr::null_mut(),length:0,capacity:0};assert_eq!(il_rt_file_read_some(ctx,&mut value,&file,2,&deadline),0);assert_eq!(std::slice::from_raw_parts(value.pointer,value.length as usize),b"ab");il_rt_buffer_free(ctx,&mut value);
    assert_eq!(il_rt_file_close(ctx,&file),0);assert_eq!(il_rt_file_close(ctx,&file),3);let stale=file;assert_eq!(il_rt_file_open_read(ctx,&mut file,0,&name),0);assert_eq!(file.slot,stale.slot);assert_ne!(file.generation,stale.generation);il_rt_buffer_free(ctx,&mut name);
    let start=br#"{"type":"core.File","data":{"kind":"resource","value":"#;il_rt_json_raw(ctx,start.as_ptr(),start.len()as u64);il_rt_json_file(ctx,&file);il_rt_json_raw(ctx,b"}}".as_ptr(),2);il_rt_leave(ctx);il_rt_finish(ctx);
    let report:il_execution_model::Execution=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();assert_eq!(report.live_handles,1);assert_eq!(report.live_allocations,0);assert_eq!(report.handle_events.len(),4);assert_eq!(report.handle_events[2].error.as_deref(),Some("Closed"));assert!(report.stack_trace.is_empty());
    assert!(matches!(report.value.unwrap().data,il_execution_model::ValueData::Resource(value) if value.slot==file.slot&&value.generation==file.generation));
    let input=scope.join("input");assert!(!std::fs::read_dir("/proc/self/fd").unwrap().filter_map(|e|e.ok()).any(|e|std::fs::read_link(e.path()).ok().as_ref()==Some(&input)));std::fs::remove_file(path).unwrap();std::fs::remove_dir_all(scope).unwrap();
}}

#[cfg(unix)]
#[test]
fn language_stack_quota_trap_and_startup_grant_rejection(){
    for(mode,code)in [("stack","E_PANIC"),("quota","E_RESOURCE_LIMIT"),("missing_grant","E_CAPABILITY_MISSING"),("concat_attempts","E_ALLOCATION_FAILED")]{
        let path=std::env::temp_dir().join(format!("il-native-runtime-{}-{mode}.json",std::process::id()));let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","captured_trap_child","--nocapture"]).env("IL_RUNTIME_TRAP_REPORT",&path).env("IL_RUNTIME_TRAP_MODE",mode).output().unwrap();assert_eq!(output.status.code(),Some(101),"{output:?}");let report:il_execution_model::Execution=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();std::fs::remove_file(path).unwrap();assert_eq!(report.diagnostics[0].code,code);

        if mode=="stack"{assert_eq!(report.stack_trace.len(),2);assert_eq!(report.stack_trace[0].function_id,"inner");assert_eq!(report.stack_trace[0].call_site.as_deref(),Some("call_inner"));assert_eq!(report.stack_trace[0].entity_id,"panic_site");assert_eq!(report.stack_trace[1].function_id,"outer");assert_eq!(report.stack_trace[1].call_site,None);assert_eq!(report.stack_trace[1].entity_id,"call_inner");}
        if mode=="missing_grant"{assert_eq!(report.steps,0);assert_eq!(report.live_allocations,0);assert_eq!(report.live_handles,0);}
    }
}

#[test]
fn native_allocation_fault_is_typed_without_consuming_existing_owner(){unsafe{
    let(path,fd)=report_file("allocation-fault");let ctx=il_rt_context_new(1,100,8,100,10000,0,fd);
    authorize_policy(ctx,"allocation-fault",serde_json::json!({"schema_version":"1.0.0","grants":[],"test_faults":{"allocation_fail_after":1,"io_max_chunk":null,"io_fail_after":null,"accept_fail_after":null}}),serde_json::json!([]));
    let mut buffer=Buffer{pointer:ptr::null_mut(),length:0,capacity:0};let mut output=buffer;il_rt_buffer_new(ctx,&mut buffer,b"abc".as_ptr(),3,b"first".as_ptr(),5);assert_eq!(il_rt_concat(ctx,&mut output,&buffer,&buffer,b"concat".as_ptr(),6),1);assert!(output.pointer.is_null());assert_eq!(std::slice::from_raw_parts(buffer.pointer,3),b"abc");il_rt_buffer_free(ctx,&mut buffer);
    let json=br#"{"type":"Unit","data":{"kind":"unit"}}"#;il_rt_json_raw(ctx,json.as_ptr(),json.len()as u64);il_rt_finish(ctx);let report:il_execution_model::Execution=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();assert_eq!(report.live_allocations,0);assert_eq!(report.peak_heap_bytes,3);assert_eq!(report.lifecycle.len(),1);std::fs::remove_file(path).unwrap();
}}

#[test]
fn stack_capture_budget_traps_before_replacing_bounded_evidence(){
    for mode in ["stack_enter_budget","stack_location_budget","stack_initial_budget","stack_explicit_budget"]{let path=std::env::temp_dir().join(format!("il-native-runtime-{}-{mode}.json",std::process::id()));let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","captured_trap_child","--nocapture"]).env("IL_RUNTIME_TRAP_REPORT",&path).env("IL_RUNTIME_TRAP_MODE",mode).output().unwrap();assert_eq!(output.status.code(),Some(101),"{output:?}");let bytes=std::fs::read(&path).unwrap();assert!(bytes.len()<2000);let report:il_execution_model::Execution=serde_json::from_slice(&bytes).unwrap();assert_eq!(report.diagnostics[0].code,"E_RESOURCE_LIMIT");assert_eq!(report.diagnostics[0].cause,"stack trace budget exhausted");let current=if mode=="stack_enter_budget"{"call_site"}else{"safe"};assert_eq!(report.diagnostics[0].entity_id.as_deref(),Some(current));if mode=="stack_initial_budget"{assert!(report.stack_trace.is_empty())}else{assert_eq!(report.stack_trace.len(),1);assert_eq!(report.stack_trace[0].function_id,"outer");assert_eq!(report.stack_trace[0].entity_id,current);}std::fs::remove_file(path).unwrap();}
}

#[test]
fn byte_primitives_validate_ranges_utf8_and_explicit_copies(){unsafe{
    let ctx=il_rt_context_new(0,0,0,0,0,0,-1);let empty=Buffer{pointer:ptr::null_mut(),length:0,capacity:0};let mut input=empty;let mut output=empty;let mut one=empty;let mut joined=empty;il_rt_buffer_new(ctx,&mut input,b"abc".as_ptr(),3,b"input".as_ptr(),5);
    assert_eq!(il_rt_bytes_get(ctx,&input,1),b'b');assert_eq!(il_rt_bytes_slice(ctx,&mut output,&input,1,2),0);assert_eq!(std::slice::from_raw_parts(output.pointer,2),b"bc");assert_eq!(il_rt_bytes_slice(ctx,&mut one,&input,u64::MAX,2),6);assert!(one.pointer.is_null());assert_eq!(il_rt_bytes_from_u8(ctx,&mut one,0xff),0);let mut invalid=empty;assert_eq!(il_rt_string_from_utf8(ctx,&mut invalid,&one),6);assert!(invalid.pointer.is_null());
    assert_eq!(il_rt_bytes_concat(ctx,&mut joined,&output,&one),0);assert_eq!(std::slice::from_raw_parts(joined.pointer,3),&[b'b',b'c',0xff]);assert!(!il_rt_bytes_equal(ctx,&input,&output));let mut copy=empty;assert_eq!(il_rt_string_to_bytes(ctx,&mut copy,&input),0);assert_ne!(input.pointer,copy.pointer);assert!(il_rt_string_equal(ctx,&input,&copy));let mut utf8=empty;assert_eq!(il_rt_string_from_utf8(ctx,&mut utf8,&copy),0);assert!(il_rt_bytes_equal(ctx,&input,&utf8));
    for buffer in [&mut utf8,&mut copy,&mut joined,&mut one,&mut output,&mut input]{il_rt_buffer_free(ctx,buffer);}il_rt_finish(ctx);
}}
