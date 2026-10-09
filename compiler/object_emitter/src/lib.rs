//! LLVM verification, optimization and target object emission with fixed tool identities.
pub mod process;
use process::bounded_command;
use serde::{Deserialize,Serialize};
use sha2::{Digest,Sha256};
use std::{fs,io::Write,path::{Path,PathBuf},process::Command,time::Duration};
#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub enum Profile { Debug, Release }
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct Artifact { pub path:PathBuf, pub sha256:String }
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct CommandRecord { pub stage:String,pub program:PathBuf,pub arguments:Vec<String>,pub exit_code:Option<i32>,pub stdout:String,pub stderr:String }
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct ToolIdentity { pub path:PathBuf,pub sha256:String,pub version:String }
pub fn hash(bytes:&[u8])->String {format!("sha256:{:x}",Sha256::digest(bytes))}
pub fn artifact(path:&Path)->Result<Artifact,String>{fs::read(path).map(|b|Artifact{path:path.to_path_buf(),sha256:hash(&b)}).map_err(|e|e.to_string())}

pub fn durable_write(path:&Path,bytes:&[u8])->Result<(),String>{let mut file=fs::File::create(path).map_err(|e|e.to_string())?;file.write_all(bytes).and_then(|_|file.sync_all()).map_err(|e|e.to_string())}
pub fn sync_directory(path:&Path)->Result<(),String>{#[cfg(unix)]{fs::File::open(path).and_then(|file|file.sync_all()).map_err(|e|e.to_string())}#[cfg(not(unix))]{let _=path;Ok(())}}
pub fn save_json(path:&Path,value:&impl Serialize)->Result<(),String>{let bytes=serde_json::to_vec_pretty(value).map_err(|e|e.to_string())?;durable_write(path,&bytes)}

pub fn invoke(commands:&mut Vec<CommandRecord>,directory:&Path,stage:&str,program:&str,args:Vec<String>)->Result<(),String>{
    if !["/usr/bin/opt-14","/usr/bin/llc-14","/usr/bin/clang-14"].contains(&program){return Err("tool is outside the fixed LLVM allowlist".into())}
    let mut command=Command::new(program);command.args(&args).current_dir(directory).env_clear().env("PATH","/usr/bin:/bin");
    let output=bounded_command(command,Duration::from_secs(60));
    let (exit_code,stdout,stderr)=match output{Ok(v)=>v,Err(v)=>{
        commands.push(CommandRecord{stage:stage.into(),program:program.into(),arguments:args,exit_code:v.exit_code,stdout:String::from_utf8_lossy(&v.stdout).into_owned(),stderr:String::from_utf8_lossy(&v.stderr).into_owned()});let _=save_json(&directory.join("commands.json"),commands);return Err(v.message);
    }};
    commands.push(CommandRecord{stage:stage.into(),program:program.into(),arguments:args,exit_code,stdout:String::from_utf8_lossy(&stdout).into_owned(),stderr:String::from_utf8_lossy(&stderr).into_owned()});
    save_json(&directory.join("commands.json"),commands)?;
    if exit_code!=Some(0){return Err(format!("{stage} failed with {exit_code:?}"))}Ok(())
}


#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct ObjectArtifacts {pub llvm_ir:Artifact,pub optimized_ir:Artifact,pub object:Artifact}

pub fn identify_tool(tool:&str,directory:&Path,commands:&mut Vec<CommandRecord>,tools:&mut Vec<ToolIdentity>)->Result<(),String>{
    if !["/usr/bin/opt-14","/usr/bin/llc-14","/usr/bin/clang-14"].contains(&tool){return Err("tool is outside the fixed LLVM allowlist".into())}
    invoke(commands,directory,"identify_tool",tool,vec!["--version".into()])?;
    let version=commands.last().unwrap().stdout.clone();
    if !version.lines().any(|line|line.split_whitespace().any(|word|word=="14.0.6")){return Err(format!("{tool} is not the locked LLVM 14.0.6 tool"))}
    tools.push(ToolIdentity{path:tool.into(),sha256:artifact(Path::new(tool))?.sha256,version});Ok(())
}

/// Verify LLVM, run the selected fixed optimization level, emit and validate an ELF relocatable object.
pub fn emit(llvm_ir:&str,directory:&Path,profile:Profile,commands:&mut Vec<CommandRecord>,tools:&mut Vec<ToolIdentity>)->Result<ObjectArtifacts,String>{
    if std::env::consts::OS!="linux"||std::env::consts::ARCH!="x86_64"{return Err("object emission requires Linux x86-64".into())}
    if !directory.is_dir(){return Err("object output directory must already exist".into())}
    for name in ["program.ll","optimized.ll","program.o"]{if directory.join(name).exists(){return Err("object emission requires fresh artifact paths".into())}}
    for tool in ["/usr/bin/opt-14","/usr/bin/llc-14"]{identify_tool(tool,directory,commands,tools)?;}
    let ll=directory.join("program.ll");let optimized=directory.join("optimized.ll");let object=directory.join("program.o");durable_write(&ll,llvm_ir.as_bytes())?;
    let opt=match profile{Profile::Debug=>"-O0",Profile::Release=>"-O2"};
    invoke(commands,directory,"verify_llvm","/usr/bin/opt-14",vec!["-verify".into(),"-disable-output".into(),"program.ll".into()])?;
    invoke(commands,directory,"optimize_llvm","/usr/bin/opt-14",vec![opt.into(),"-S".into(),"program.ll".into(),"-o".into(),"optimized.ll".into()])?;
    invoke(commands,directory,"emit_object","/usr/bin/llc-14",vec![opt.into(),"-filetype=obj".into(),"-mtriple=x86_64-unknown-linux-gnu".into(),"-relocation-model=pic".into(),"optimized.ll".into(),"-o".into(),"program.o".into()])?;
    let bytes=fs::read(&object).map_err(|e|e.to_string())?;
    if bytes.len()<64||&bytes[..4]!=b"\x7fELF"||bytes[4]!=2||bytes[5]!=1||u16::from_le_bytes([bytes[16],bytes[17]])!=1||u16::from_le_bytes([bytes[18],bytes[19]])!=62{return Err("object is not a Linux x86-64 ELF relocatable file".into())}
    for path in [&ll,&optimized,&object]{fs::File::open(path).and_then(|file|file.sync_all()).map_err(|e|e.to_string())?;}
    sync_directory(directory)?;Ok(ObjectArtifacts{llvm_ir:artifact(&ll)?,optimized_ir:artifact(&optimized)?,object:artifact(&object)?})
}

#[cfg(all(test,target_os="linux"))]
mod tests {
    use super::*;
    use super::process::{bounded_command,OUTPUT_LIMIT};
    use std::time::Instant;
    #[test]
    fn bounded_output_preserves_bytes(){
        let mut command=Command::new("/usr/bin/python3");command.args(["-c","import os; os.write(1,b'out\\x00'); os.write(2,b'err')"]);
        let(status,out,err)=bounded_command(command,Duration::from_secs(5)).map_err(|e|e.message).unwrap();assert_eq!(status,Some(0));assert_eq!(out,b"out\0");assert_eq!(err,b"err");
    }
    #[test]
    fn output_limit_kills_and_reaps(){
        let mut command=Command::new("/usr/bin/python3");command.args(["-c","import os; data=b'x'*65536\nwhile True: os.write(1,data)"]);
        let failure=bounded_command(command,Duration::from_secs(5)).err().expect("unlimited output accepted");assert_eq!(failure.message,"native output limit exceeded");assert_eq!(failure.stdout.len(),OUTPUT_LIMIT as usize);
    }
    #[test]
    fn timeout_kills_and_reaps(){
        let mut command=Command::new("/usr/bin/python3");command.args(["-c","import time;time.sleep(60)"]);
        let begin=Instant::now();let failure=bounded_command(command,Duration::from_millis(50)).err().expect("timeout accepted");assert_eq!(failure.message,"native process timeout");assert!(begin.elapsed()<Duration::from_secs(5));
    }
    #[test]
    fn exited_process_cannot_leave_inherited_pipe_holders(){
        let mut command=Command::new("/usr/bin/python3");command.args(["-c","import os,time\nif os.fork()==0: time.sleep(60)\nelse: os._exit(0)"]);
        let begin=Instant::now();let(status,_,_)=bounded_command(command,Duration::from_secs(2)).map_err(|e|e.message).unwrap();assert_eq!(status,Some(0));assert!(begin.elapsed()<Duration::from_secs(2));
    }
    #[test]
    fn success_record_write_failure_propagates(){
        let path=std::env::temp_dir().join(format!("il-link-driver-{}-directory",std::process::id()));fs::create_dir(&path).unwrap();let result=save_json(&path,&vec!["evidence"]);fs::remove_dir(&path).unwrap();assert!(result.is_err());
    }
    #[test]
    fn object_stage_emits_relocatable_without_linking(){
        let root=std::env::temp_dir().join(format!("il-object-emitter-{}-object",std::process::id()));fs::create_dir(&root).unwrap();let mut commands=vec![];let mut tools=vec![];
        let result=emit("target triple = \"x86_64-unknown-linux-gnu\"\ndefine i64 @answer() { ret i64 42 }\n",&root,Profile::Debug,&mut commands,&mut tools).unwrap();
        let bytes=fs::read(&result.object.path).unwrap();assert_eq!(u16::from_le_bytes([bytes[16],bytes[17]]),1);assert_eq!(tools.len(),2);assert_eq!(commands.len(),5);assert!(!root.join("program.elf").exists());
        assert!(emit("",&root,Profile::Debug,&mut commands,&mut tools).is_err());fs::remove_dir_all(root).unwrap();
    }
}
