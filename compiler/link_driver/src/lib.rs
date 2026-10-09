//! Fixed LLVM object/link toolchain and isolated native execution.
use il_execution_model::Execution;
use il_native_ir::RuntimeProfile;
use serde::{Deserialize, Serialize};
use il_object_emitter::{Artifact,CommandRecord,ToolIdentity,Profile,artifact,save_json,sync_directory,invoke,identify_tool};
use il_object_emitter::process::{bounded_command,OUTPUT_LIMIT};
use std::{fs,path::{Path,PathBuf},process::Command,time::Duration};

#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildArtifacts {
    pub profile:Profile,pub runtime_profile:RuntimeProfile,pub llvm_ir:Artifact,pub optimized_ir:Artifact,pub object:Artifact,
    #[serde(deserialize_with="required_nullable")]
    pub executable:Option<Artifact>,
    #[serde(deserialize_with="required_nullable")]
    pub runtime:Option<Artifact>,pub tools:Vec<ToolIdentity>,pub commands:Vec<CommandRecord>,pub record_path:PathBuf,
}
fn required_nullable<'de,D:serde::Deserializer<'de>,T:Deserialize<'de>>(d:D)->Result<Option<T>,D::Error>{Option::<T>::deserialize(d)}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum LinkPlan { Full { archive:PathBuf }, Minimal { archive:PathBuf }, None }
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct BuildFailure { pub code:String,pub message:String,pub directory:PathBuf,pub commands:Vec<CommandRecord>,pub tools:Vec<ToolIdentity> }
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct NativeRun { pub exit_code:i32,pub stdout:Vec<u8>,pub stderr:Vec<u8>,pub execution:Execution,pub report_artifact:Artifact }
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct RunFailure { pub code:String,pub message:String,pub exit_code:Option<i32>,pub stdout:Vec<u8>,pub stderr:Vec<u8>,pub report_path:PathBuf }

/// Compile only LLVM emitted by the verified frontend into a private candidate directory.
pub fn compile(llvm_ir:&str,output_dir:&Path,plan:&LinkPlan,profile:Profile)->Result<BuildArtifacts,BuildFailure>{
    let mut failure=BuildFailure{code:"E_NATIVE_BUILD_FAILED".into(),message:String::new(),directory:output_dir.into(),commands:vec![],tools:vec![]};
    let result=compile_inner(llvm_ir,output_dir,plan,profile,&mut failure).and_then(|result|{save_json(&output_dir.join("build-report.json"),&result)?;sync_directory(output_dir)?;Ok(result)});
    match result{
        Ok(result)=>Ok(result),
        Err(message)=>{failure.message=message;if let Err(error)=save_json(&output_dir.join("build-failure.json"),&failure){failure.message.push_str(&format!("; failed to preserve failure report: {error}"));}Err(failure)}
    }
}

fn compile_inner(llvm_ir:&str,output_dir:&Path,plan:&LinkPlan,profile:Profile,failure:&mut BuildFailure)->Result<BuildArtifacts,String>{
    if std::env::consts::OS!="linux"||std::env::consts::ARCH!="x86_64"{return Err("P05 native compilation requires Linux x86-64".into())}
    if !output_dir.is_dir(){return Err("candidate directory must already exist".into())}
    let output_dir=output_dir.canonicalize().map_err(|e|e.to_string())?;
    if ["program.ll","optimized.ll","program.o","program.elf","build-report.json"].iter().any(|p|output_dir.join(p).exists()){return Err("candidate directory contains existing build artifacts".into())}
    let objects=il_object_emitter::emit(llvm_ir,&output_dir,profile,&mut failure.commands,&mut failure.tools)?;
    let runtime_archive=match plan{LinkPlan::Full{archive}|LinkPlan::Minimal{archive}=>Some(archive.canonicalize().map_err(|e|e.to_string())?),LinkPlan::None=>None};
    let runtime=runtime_archive.as_ref().map(|path|artifact(path)).transpose()?;
    let runtime_profile=match plan{LinkPlan::Full{..}=>RuntimeProfile::Full,LinkPlan::Minimal{..}=>RuntimeProfile::Minimal,LinkPlan::None=>RuntimeProfile::None};
    if runtime_profile==RuntimeProfile::None{return Ok(BuildArtifacts{profile,runtime_profile,llvm_ir:objects.llvm_ir,optimized_ir:objects.optimized_ir,object:objects.object,executable:None,runtime:None,tools:failure.tools.clone(),commands:failure.commands.clone(),record_path:output_dir.join("build-report.json")});}
    identify_tool("/usr/bin/clang-14",&output_dir,&mut failure.commands,&mut failure.tools)?;
    let executable=output_dir.join("program.elf");
    let mut arguments=vec!["--target=x86_64-unknown-linux-gnu".into(),"program.o".into(),runtime_archive.unwrap().to_string_lossy().into_owned()];
    if runtime_profile==RuntimeProfile::Minimal{arguments.extend(["-nostdlib","-static","-Wl,--gc-sections","-Wl,-e,_start","-Wl,-u,_start"].map(str::to_owned));}else{arguments.extend(["-ldl","-lpthread","-lm"].map(str::to_owned));}
    arguments.extend(["-Wl,--build-id=none","-o","program.elf"].map(str::to_owned));
    invoke(&mut failure.commands,&output_dir,"link_runtime","/usr/bin/clang-14",arguments)?;
    let bytes=fs::read(&executable).map_err(|e|e.to_string())?;
    if bytes.len()<64||&bytes[..4]!=b"\x7fELF"||bytes[4]!=2||bytes[5]!=1||u16::from_le_bytes([bytes[18],bytes[19]])!=62||![2,3].contains(&u16::from_le_bytes([bytes[16],bytes[17]])){return Err("linked artifact is not a Linux x86-64 ELF executable".into())}
    for path in [&executable]{fs::File::open(path).and_then(|file|file.sync_all()).map_err(|e|e.to_string())?;}
    Ok(BuildArtifacts{profile,runtime_profile,llvm_ir:objects.llvm_ir,optimized_ir:objects.optimized_ir,object:objects.object,executable:Some(artifact(&executable)?),runtime,tools:failure.tools.clone(),commands:failure.commands.clone(),record_path:output_dir.join("build-report.json")})
}

/// Run a captured LLVM harness with its report on inherited fd 3 and no ambient environment.
pub fn run(executable:&Path,report_path:&Path,policy_path:&Path,timeout:Duration)->Result<NativeRun,RunFailure>{
    let mut failure=RunFailure{code:"E_NATIVE_EXECUTION_FAILED".into(),message:String::new(),exit_code:None,stdout:vec![],stderr:vec![],report_path:report_path.into()};
    let result=(||->Result<NativeRun,String>{
        if timeout.is_zero()||timeout>Duration::from_secs(60){return Err("native execution timeout must be in (0,60] seconds".into())}
        let executable=executable.canonicalize().map_err(|e|e.to_string())?;
        let report=fs::OpenOptions::new().read(true).write(true).create_new(true).open(report_path).map_err(|e|e.to_string())?;
        let policy=fs::File::open(policy_path).map_err(|e|e.to_string())?;
        if policy.metadata().map_err(|e|e.to_string())?.len()>65_536{return Err("host policy exceeds 64 KiB".into());}
        let mut command=Command::new(executable);command.env_clear().current_dir(report_path.parent().ok_or("report directory is required")?);
        #[cfg(target_os="linux")]{
            use std::os::{fd::AsRawFd,unix::process::CommandExt};
            let source=report.as_raw_fd();let policy_source=policy.as_raw_fd();
            unsafe{command.pre_exec(move||{unsafe extern "C"{fn dup2(old:i32,new:i32)->i32;fn fcntl(fd:i32,command:i32,...)->i32;fn close(fd:i32)->i32;fn setrlimit(resource:i32,limit:*const [u64;2])->i32;}
                let saved_report=fcntl(source,1030,5);if saved_report<0{return Err(std::io::Error::last_os_error());}
                let saved_policy=fcntl(policy_source,1030,5);if saved_policy<0{close(saved_report);return Err(std::io::Error::last_os_error());}
                if dup2(saved_report,3)<0||dup2(saved_policy,4)<0||fcntl(3,2,0)<0||fcntl(4,2,0)<0{let error=std::io::Error::last_os_error();close(saved_report);close(saved_policy);return Err(error);}
                close(saved_report);close(saved_policy);
                let limit=[OUTPUT_LIMIT,OUTPUT_LIMIT];if setrlimit(1, &limit)<0{return Err(std::io::Error::last_os_error())}Ok(())});}
        }
        #[cfg(not(target_os="linux"))]{return Err("native execution requires Linux".into());}
        let (code,out,err)=bounded_command(command,timeout).map_err(|e|{failure.exit_code=e.exit_code;failure.stdout=e.stdout;failure.stderr=e.stderr;e.message})?;
        failure.exit_code=code;failure.stdout=out.clone();failure.stderr=err.clone();
        if report.metadata().map_err(|e|e.to_string())?.len()>OUTPUT_LIMIT{return Err("native report limit exceeded".into())}
        let bytes=fs::read(report_path).map_err(|e|e.to_string())?;
        let mut depth=0u32;let mut quoted=false;let mut escaped=false;
        for byte in &bytes{if quoted{if escaped{escaped=false}else if *byte==b'\\'{escaped=true}else if *byte==b'"'{quoted=false}}else{match *byte{b'"'=>quoted=true,b'{'|b'['=>{depth+=1;if depth>512{return Err("native report nesting limit exceeded".into())}},b'}'|b']'=>depth=depth.saturating_sub(1),_=>{}}}}
        let mut decoder=serde_json::Deserializer::from_slice(&bytes);decoder.disable_recursion_limit();
        let execution=Execution::deserialize(&mut decoder).and_then(|execution|decoder.end().map(|_|execution)).map_err(|e|format!("invalid native report: {e}"))?;
        if execution.stdout!=out||execution.stderr!=err{return Err("native report output differs from actual process output".into())}
        let expected=match execution.status{il_execution_model::ExecutionStatus::Returned=>0,il_execution_model::ExecutionStatus::Trapped=>101,il_execution_model::ExecutionStatus::Rejected=>return Err("compiled harness cannot report rejected execution".into())};
        if code!=Some(expected){return Err("native exit status differs from execution report".into())}
        Ok(NativeRun{exit_code:expected,stdout:out,stderr:err,execution,report_artifact:artifact(report_path)?})
    })();
    result.map_err(|message|{failure.message=message;let _=save_json(&report_path.with_extension("failure.json"),&failure);failure})
}

#[cfg(all(test,target_os="linux"))]
mod tests{
    use super::*;
    #[test]
    fn fixed_toolchain_builds_real_elf_and_preserves_invalid_ir_evidence(){
        let root=std::env::temp_dir().join(format!("il-link-driver-{}-pipeline",std::process::id()));fs::create_dir(&root).unwrap();
        let archive=Path::new("/usr/lib/x86_64-linux-gnu/libm.a");
        let source="target triple = \"x86_64-unknown-linux-gnu\"\ndefine i32 @main() {\nentry:\n ret i32 37\n}\n";
        let mut hashes=vec![];
        for name in ["first","second"]{
            let dir=root.join(name);fs::create_dir(&dir).unwrap();let build=compile(source,&dir,&LinkPlan::Full{archive:archive.into()},Profile::Release).unwrap();
            assert_eq!(build.tools.len(),3);assert!(build.commands.iter().any(|r|r.stage=="verify_llvm"&&r.exit_code==Some(0)));assert!(build.record_path.is_file());
            let status=Command::new(&build.executable.as_ref().unwrap().path).env_clear().status().unwrap();assert_eq!(status.code(),Some(37));hashes.push(build.executable.as_ref().unwrap().sha256.clone());
        }
        assert_eq!(hashes[0],hashes[1]);
        let dir=root.join("invalid");fs::create_dir(&dir).unwrap();let failure=compile("not LLVM IR",&dir,&LinkPlan::Full{archive:archive.into()},Profile::Debug).unwrap_err();
        assert!(failure.commands.iter().any(|r|r.stage=="verify_llvm"&&r.exit_code!=Some(0)&&!r.stderr.is_empty()));assert_eq!(fs::read_to_string(dir.join("program.ll")).unwrap(),"not LLVM IR");assert!(dir.join("build-failure.json").is_file());assert!(!dir.join("program.elf").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn captured_run_uses_fd3_and_checks_real_output(){
        let root=std::env::temp_dir().join(format!("il-link-driver-{}-fd3",std::process::id()));fs::create_dir(&root).unwrap();
        let report=il_execution_model::Execution{status:il_execution_model::ExecutionStatus::Returned,value:Some(il_execution_model::Value::unit()),stdout:b"ok".to_vec(),stderr:vec![],diagnostics:vec![],steps:0,peak_heap_bytes:0,live_allocations:0,lifecycle:vec![],live_handles:0,handle_events:vec![],stack_trace:vec![]};
        let json=serde_json::to_vec(&report).unwrap();let encoded=json.iter().map(|v|format!("\\{v:02X}")).collect::<String>();let n=json.len();
        let source=format!("target triple = \"x86_64-unknown-linux-gnu\"\n@report = private constant [{n} x i8] c\"{encoded}\"\n@out = private constant [2 x i8] c\"ok\"\ndeclare i64 @write(i32, i8*, i64)\ndeclare i64 @read(i32, i8*, i64)\ndefine i32 @main() {{\nentry:\n %policy=alloca i8\n %count=call i64 @read(i32 4,i8* %policy,i64 1)\n %first=load i8,i8* %policy\n %valid=icmp eq i8 %first,123\n %code=select i1 %valid,i32 0,i32 42\n %r=call i64 @write(i32 3,i8* getelementptr ([{n} x i8], [{n} x i8]* @report,i64 0,i64 0),i64 {n})\n %o=call i64 @write(i32 1,i8* getelementptr ([2 x i8],[2 x i8]* @out,i64 0,i64 0),i64 2)\n ret i32 %code\n}}\n");
        let build=compile(&source,&root,&LinkPlan::Full{archive:Path::new("/usr/lib/x86_64-linux-gnu/libm.a").into()},Profile::Debug).unwrap();let policy=root.join("policy.json");fs::write(&policy,b"{\"schema_version\":\"1.0.0\",\"grants\":[],\"test_faults\":null}").unwrap();let run=run(&build.executable.as_ref().unwrap().path,&root.join("execution.json"),&policy,Duration::from_secs(5)).unwrap();assert_eq!(run.stdout,b"ok");assert_eq!(run.execution,report);assert_eq!(run.exit_code,0);fs::remove_dir_all(root).unwrap();
    }
}
