use il_execution_model::{Value,ValueData,VariantValue,Limits,ExecutionStatus};
use std::{path::{Path,PathBuf},fs,process::Command};
fn repository()->PathBuf{Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")}
fn source()->String{["packages/core/lib.il","packages/alloc/lib.il","packages/io/lib.il","examples/hello/main.il"].iter().map(|path|fs::read_to_string(repository().join(path)).unwrap()).collect::<Vec<_>>().join("\n")}
struct Candidate(Option<tempfile::TempDir>);
impl Candidate{fn new()->Self{let root=repository().join("target/package-tests");fs::create_dir_all(&root).unwrap();Self(Some(tempfile::tempdir_in(root).unwrap()))}fn path(&self)->&Path{self.0.as_ref().unwrap().path()}}
impl Drop for Candidate{fn drop(&mut self){if std::thread::panicking(){let path=self.0.take().unwrap().keep();eprintln!("retained package test failure: {}",path.display());}}}
fn stream_arguments()->Vec<Value>{vec![Value{type_ref:"Bytes".into(),data:ValueData::Bytes(b"abcde".to_vec())},Value{type_ref:"core.Deadline".into(),data:ValueData::Variant(VariantValue{tag:"Infinite".into(),fields:vec![]})}]}
fn chunk_policy(fail:Option<u64>)->il_runtime_startup::HostPolicy{let mut policy=il_runtime_startup::HostPolicy::empty();policy.test_faults=Some(il_runtime_startup::Faults{allocation_fail_after:None,io_max_chunk:Some(2),io_fail_after:fail});policy}
fn program()->il_mir::Program{let graph=il_frontend::parse(&source(),0).unwrap();il_mir::lower(&il_hir::lower(&graph).unwrap()).unwrap()}
#[test]
fn package_sources_parse_check_and_execute(){
    let mir=program();let cases=[("core.max_i64",vec![Value::integer("I64",7),Value::integer("I64",42)],Value::integer("I64",42)),("alloc.string_size",vec![Value::string("\u{4e2d}")],Value::integer("Usize",3))];
    for(entry,args,expected)in cases{let output=il_interpreter::execute(&mir,entry,&args,Limits::default());assert_eq!(output.status,ExecutionStatus::Returned,"{output:?}");assert_eq!(output.value,Some(expected));assert_eq!(output.live_allocations,0);assert_eq!(output.live_handles,0);}
    let output=il_interpreter::execute(&mir,"hello.main",&[],Limits::default());assert_eq!(output.status,ExecutionStatus::Returned,"{output:?}");assert_eq!(output.value,Some(Value::integer("I32",0)));assert_eq!(output.stdout,b"Hello from Intelligent language!\n");assert_eq!(output.live_allocations,0);assert_eq!(output.live_handles,0);
    for fail in [None,Some(2)]{let output=il_interpreter::execute_with_policy(&mir,"io.write_stdout",&stream_arguments(),Limits::default(),&chunk_policy(fail));assert_eq!(output.status,ExecutionStatus::Returned,"{output:?}");assert_eq!(output.stdout,if fail.is_some(){b"abcd".to_vec()}else{b"abcde".to_vec()});assert_eq!(output.live_allocations,0);let ValueData::Variant(result)=output.value.unwrap().data else{panic!("expected Result")};assert_eq!(result.tag,if fail.is_some(){"Err"}else{"Ok"});}
    let output=il_interpreter::execute(&mir,"alloc.clone_bytes",&[Value{type_ref:"Bytes".into(),data:ValueData::Bytes(vec![0,255,3])}],Limits::default());assert_eq!(output.status,ExecutionStatus::Returned,"{output:?}");assert_eq!(output.live_allocations,1);assert_eq!(output.value.unwrap().data,ValueData::Bytes(vec![0,255,3]));
}
#[test]
fn package_manifest_exports_are_actual_checked_entities(){
    let graph=il_frontend::parse(&source(),0).unwrap();
    for name in ["core","alloc","io"]{let manifest:serde_json::Value=serde_json::from_slice(&fs::read(repository().join(format!("packages/{name}/package_manifest.json"))).unwrap()).unwrap();for function in manifest["public_functions"].as_array().unwrap(){assert!(graph.functions.iter().any(|f|f.entity_id==function.as_str().unwrap()));}for ty in manifest["public_types"].as_array().unwrap(){assert!(graph.types.iter().any(|t|t.entity_id==ty.as_str().unwrap()));}for field in ["unit_tests","blackbox_tests","schema_index"]{assert!(!manifest[field].as_array().unwrap().is_empty());}}
}
#[test]
#[cfg(target_os="linux")]
fn hello_package_sources_execute_as_debug_and_release_native(){
    let mir=program();let directory=Candidate::new();let target=directory.path().join("runtime-target");
    let output=Command::new("cargo").current_dir(repository()).args(["build","--locked","--offline","-p","il-native-runtime","--lib","--message-format=json","--target-dir"]).arg(&target).output().unwrap();assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    let archive=String::from_utf8(output.stdout).unwrap().lines().filter_map(|line|serde_json::from_str::<serde_json::Value>(line).ok()).filter(|value|value["reason"]=="compiler-artifact"&&value["target"]["name"]=="il_native_runtime").flat_map(|value|value["filenames"].as_array().unwrap().clone()).filter_map(|value|value.as_str().filter(|name|name.ends_with(".a")).map(PathBuf::from)).next().expect("cargo must report the exact produced static archive");
    let options=il_native_ir::BuildOptions{mode:il_native_ir::BuildMode::Application{entry:"hello.main".into()},runtime_profile:il_native_ir::RuntimeProfile::Full};let native=il_native_ir::lower(&mir,&options,&[]).unwrap();let emitted=il_codegen_x86_64::emit(&native).unwrap();
    for(name,profile)in [("debug",il_object_emitter::Profile::Debug),("release",il_object_emitter::Profile::Release)]{let build=directory.path().join(name);fs::create_dir(&build).unwrap();let artifacts=il_link_driver::compile(&emitted.ir,&build,&il_link_driver::LinkPlan::Full{archive:archive.clone()},profile).unwrap();let output=Command::new(&artifacts.executable.unwrap().path).env_clear().output().unwrap();assert_eq!(output.status.code(),Some(0),"{}",String::from_utf8_lossy(&output.stderr));assert_eq!(output.stdout,b"Hello from Intelligent language!\n");assert!(output.stderr.is_empty());}
    for fail in [None,Some(2)]{let policy=chunk_policy(fail);let arguments=stream_arguments();let expected=il_interpreter::execute_with_policy(&mir,"io.write_stdout",&arguments,Limits::default(),&policy);let options=il_native_ir::BuildOptions{mode:il_native_ir::BuildMode::Captured{entry:"io.write_stdout".into(),arguments,limits:Limits::default()},runtime_profile:il_native_ir::RuntimeProfile::Full};let native=il_native_ir::lower(&mir,&options,&[]).unwrap();let llvm=il_codegen_x86_64::emit(&native).unwrap();
        for(name,profile)in [("debug",il_object_emitter::Profile::Debug),("release",il_object_emitter::Profile::Release)]{let build=directory.path().join(format!("stdout-{}-{name}",if fail.is_some(){"failure"}else{"success"}));fs::create_dir(&build).unwrap();let artifacts=il_link_driver::compile(&llvm.ir,&build,&il_link_driver::LinkPlan::Full{archive:archive.clone()},profile).unwrap();let policy_path=build.join("host-policy.json");fs::write(&policy_path,serde_json::to_vec(&policy).unwrap()).unwrap();let result=il_link_driver::run(&artifacts.executable.unwrap().path,&build.join("report.json"),&policy_path,std::time::Duration::from_secs(5)).unwrap();assert_eq!(result.execution.status,expected.status);assert_eq!(result.execution.value,expected.value);assert_eq!(result.execution.stdout,expected.stdout);assert_eq!(result.execution.live_allocations,0);assert_eq!(result.execution.live_handles,0);assert_eq!(result.stdout,expected.stdout);}
    }

}
