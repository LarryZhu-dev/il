use il_execution_model::{Value,ValueData,VariantValue,Limits,ExecutionStatus};
use std::{path::{Path,PathBuf},fs};
fn repository()->PathBuf{Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")}
fn source()->String{["core","alloc","io","time","net","json","http","test","tracing"].iter().map(|name|fs::read_to_string(repository().join(format!("packages/{name}/lib.il"))).unwrap()).collect::<Vec<_>>().join("\n")}
fn program()->il_mir::Program{let graph=il_frontend::parse(&source(),0).unwrap_or_else(|errors|panic!("frontend: {errors:#?}"));il_mir::lower(&il_hir::lower(&graph).unwrap()).unwrap()}
struct HttpCandidate(Option<tempfile::TempDir>);
impl HttpCandidate { fn path(&self)->&Path { self.0.as_ref().unwrap().path() } }
impl Drop for HttpCandidate { fn drop(&mut self) { if std::thread::panicking() { let path=self.0.take().unwrap().keep();eprintln!("retained HTTP package failure: {}",path.display()); } } }
fn bytes(value:&[u8])->Value{Value{type_ref:"Bytes".into(),data:ValueData::Bytes(value.to_vec())}}
fn run(program:&il_mir::Program,entry:&str,args:&[Value])->Value{let result=il_interpreter::execute(program,entry,args,Limits{max_steps:1_000_000,max_output_bytes:1_048_576,..Limits::default()});assert_eq!(result.status,ExecutionStatus::Returned,"{entry}: {:?}; steps={}, events={}",result.diagnostics,result.steps,result.lifecycle.len());result.value.unwrap()}
fn variant(value:Value,expected:&str)->Vec<Value>{let ValueData::Variant(VariantValue{tag,fields})=value.data else{panic!("expected variant {value:?}")};assert_eq!(tag,expected);fields}
#[test]
fn package_sources_are_checked_and_json_is_utf8_correct(){
 let p=program();
 for text in ["", "Larry", "quote\" slash\\ newline\n nul\0", "\u{4e2d}\u{1f600}"]{let value=run(&p,"json.quote",&[Value::string(text)]);let result=variant(value,"Ok").remove(0);let ValueData::Bytes(encoded)=result.data else{panic!()};assert_eq!(serde_json::from_slice::<String>(&encoded).unwrap(),text);}
 let value=run(&p,"json.message",&[Value::string("Hello, Larry")]);let result=variant(value,"Ok").remove(0);let ValueData::Bytes(encoded)=result.data else{panic!()};assert_eq!(serde_json::from_slice::<serde_json::Value>(&encoded).unwrap(),serde_json::json!({"message":"Hello, Larry"}));
 assert_eq!(run(&p,"time.elapsed_ns",&[Value::integer("U64",10),Value::integer("U64",42)]),Value::integer("U64",32));
}
#[test]
fn il_request_parser_covers_protocol_and_path_encoding(){
 let p=program();
 for(request,status)in [(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n".as_slice(),200),(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n",505),(b"GET /health HTTP/1.1\r\n\r\n",400),(b"GET /hello/%FF HTTP/1.1\r\nHost: x\r\n\r\n",400),(b"GET /hello/%2f HTTP/1.1\r\nHost: x\r\n\r\n",400),(b"GET /health HTTP/1.1\r\nHost: x\r\nContent-Length: 1\r\n\r\n",413),(b"GET /health HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\ncontent-length: 0\r\n\r\n",400),(b"GET /health HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n",400),(b"GET /health HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n",400),(b"CONNECT target:443 HTTP/1.1\r\nHost: x\r\n\r\n",405)]{
  let result=run(&p,"http.parse",&[bytes(request)]);if status==200{let fields=variant(variant(result,"Ok").remove(0),"Parsed");assert_eq!(fields,[bytes(b"GET"),bytes(b"/health")]);}else{let fields=variant(variant(result,"Err").remove(0),"Protocol");assert_eq!(fields,[Value::integer("U16",status)]);}
 }
 let parsed=variant(variant(run(&p,"http.parse",&[bytes(b"GET /hello/%2541?unused=%FF HTTP/1.1\r\nHost: x\r\n\r\n")]),"Ok").remove(0),"Parsed");assert_eq!(parsed[1],bytes(b"/hello/%41"));
}
#[test]
fn route_matching_and_response_lengths_are_actual_il(){
 let p=program();
 for(path,pattern,matched)in [("/health","/health",Some("")),("/health/","/health",None),("/hello/Larry","/hello/{name}",Some("Larry")),("/hello/","/hello/{name}",None),("/hello/a/b","/hello/{name}",None)]{let result=variant(run(&p,"http.match_path",&[bytes(path.as_bytes()),bytes(pattern.as_bytes())]),"Ok").remove(0);if let Some(name)=matched{assert_eq!(variant(result,"Match"),[Value::string(name)]);}else{assert!(variant(result,"Miss").is_empty());}}
 let response=Value{type_ref:"http.Response".into(),data:ValueData::Variant(VariantValue{tag:"Response".into(),fields:vec![Value::integer("U16",200),Value::string("application/json"),bytes("\u{4e2d}".as_bytes())]})};let result=variant(run(&p,"http.encode",&[response]),"Ok").remove(0);let ValueData::Bytes(encoded)=result.data else{panic!()};let text=String::from_utf8(encoded).unwrap();assert!(text.contains("Content-Length: 3\r\n"));assert!(text.contains("Connection: close\r\n"));assert!(text.ends_with("\r\n\r\n\u{4e2d}"));
}

#[test]
fn package_manifests_export_checked_entities_and_test_contracts(){
 let graph=il_frontend::parse(&source(),0).unwrap();
 for name in ["time","net","json","http","test","tracing"]{
  let manifest:serde_json::Value=serde_json::from_slice(&fs::read(repository().join(format!("packages/{name}/package_manifest.json"))).unwrap()).unwrap();
  for function in manifest["public_functions"].as_array().unwrap(){assert!(graph.functions.iter().any(|f|f.entity_id==function.as_str().unwrap()),"{function}");}
  for ty in manifest["public_types"].as_array().unwrap(){assert!(graph.types.iter().any(|t|t.entity_id==ty.as_str().unwrap()),"{ty}");}
  for key in ["unit_tests","blackbox_tests","schema_index"]{for path in manifest[key].as_array().unwrap(){assert!(repository().join(path.as_str().unwrap()).is_file());}}
 }
 let p=il_mir::lower(&il_hir::lower(&graph).unwrap()).unwrap();
 assert!(variant(run(&p,"test.assert_true",&[Value::boolean(true)]),"Ok").len()==1);
 assert!(variant(variant(run(&p,"test.assert_i64",&[Value::integer("I64",1),Value::integer("I64",2)]),"Err").remove(0),"Assertion").is_empty());
 assert!(variant(run(&p,"test.assert_bytes",&[bytes(b"abc"),bytes(b"abc")]),"Ok").len()==1);
 let deadline=Value{type_ref:"core.Deadline".into(),data:ValueData::Variant(VariantValue{tag:"At".into(),fields:vec![Value::integer("U64",123)]})};
 assert_eq!(run(&p,"time.expired",&[deadline,Value::integer("U64",123)]),Value::boolean(true));
 let trace=il_interpreter::execute(&p,"tracing.event",&[Value::string("quoted\"\n")],Limits::default());assert_eq!(trace.status,ExecutionStatus::Returned,"{:?}",trace.diagnostics);assert_eq!(serde_json::from_slice::<serde_json::Value>(&trace.stderr).unwrap(),serde_json::json!({"code":"quoted\"\n"}));assert_eq!(trace.live_allocations,0);
}

#[test]
#[cfg(target_os="linux")]
fn parser_and_package_contracts_execute_native_debug_and_release(){
 use std::process::Command;
 let text=source()+"\n"+&fs::read_to_string(repository().join("tests/http_blackbox/packages.il")).unwrap();
 let graph=il_frontend::parse(&text,0).unwrap_or_else(|d|panic!("{d:#?}"));let mir=il_mir::lower(&il_hir::lower(&graph).unwrap()).unwrap();
 let root=repository().join("target/http-native-packages");fs::create_dir_all(&root).unwrap();let temporary=HttpCandidate(Some(tempfile::tempdir_in(root).unwrap()));let directory=temporary.path().to_path_buf();
 let target=directory.join("runtime-target");let output=Command::new("cargo").current_dir(repository()).args(["build","--locked","--offline","-p","il-native-runtime","--lib","--message-format=json","--target-dir"]).arg(&target).output().unwrap();assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
 let archive=String::from_utf8(output.stdout).unwrap().lines().filter_map(|line|serde_json::from_str::<serde_json::Value>(line).ok()).filter(|v|v["reason"]=="compiler-artifact"&&v["target"]["name"]=="il_native_runtime").flat_map(|v|v["filenames"].as_array().unwrap().clone()).filter_map(|v|v.as_str().filter(|n|n.ends_with(".a")).map(PathBuf::from)).next().unwrap();
 let options=il_native_ir::BuildOptions{mode:il_native_ir::BuildMode::Application{entry:"main".into()},runtime_profile:il_native_ir::RuntimeProfile::Full};let native=il_native_ir::lower(&mir,&options,&[]).unwrap();let emitted=il_codegen_x86_64::emit(&native).unwrap();
 for(name,profile)in [("debug",il_object_emitter::Profile::Debug),("release",il_object_emitter::Profile::Release)]{
  let build=directory.join(name);fs::create_dir(&build).unwrap();let artifacts=il_link_driver::compile(&emitted.ir,&build,&il_link_driver::LinkPlan::Full{archive:archive.clone()},profile).unwrap();let output=Command::new(&artifacts.executable.unwrap().path).env_clear().output().unwrap();
  if output.status.code()!=Some(0){panic!("{name} package acceptance returned {:?}: {}",output.status.code(),String::from_utf8_lossy(&output.stderr));}
  assert!(output.stdout.is_empty());assert!(output.stderr.is_empty());
 }
}
