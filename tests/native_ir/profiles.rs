use il_native_ir::{BuildMode,BuildOptions,RuntimeProfile,Program,symbol};
use il_link_driver::{LinkPlan,compile};
use std::{path::{Path,PathBuf},fs,process::Command};

fn source(text:&str,profile:RuntimeProfile,mode:BuildMode)->Result<Program,Vec<il_graph::Diagnostic>>{
    let graph=il_frontend::parse(text,0).unwrap();let mir=il_mir::lower(&il_hir::lower(&graph).unwrap()).unwrap();il_native_ir::lower(&mir,&BuildOptions{mode,runtime_profile:profile},&[])
}
fn root(name:&str)->PathBuf{let p=std::env::temp_dir().join(format!("il-profile-{}-{name}",std::process::id()));fs::create_dir(&p).unwrap();p}
fn command(program:&str,arguments:&[&str])->std::process::Output{let result=Command::new(program).args(arguments).output().unwrap();assert!(result.status.success(),"{} {:?}: {}",program,arguments,String::from_utf8_lossy(&result.stderr));result}

#[test]
fn profile_restrictions_cover_unselected_functions_and_exports(){
    let pure="@id(\"app\") module app visibility public { @id(\"answer\") fn answer()->I64 { return 42; } }";
    let none=source(pure,RuntimeProfile::None,BuildMode::Exports{entries:vec!["answer".into()]}).unwrap();assert!(none.externs.is_empty());assert_eq!(none.functions[0].signature.parameters.len(),0);
    let mut tampered=none.clone();tampered.functions[0].public=false;assert!(!il_native_ir::verify(&tampered).is_empty());
    assert!(source(pure,RuntimeProfile::None,BuildMode::Application{entry:"answer".into()}).is_err());
    let hidden="@id(\"app\") module app visibility public { @id(\"answer\") fn answer()->I64 { return 42; } fn unused()->String effects [alloc] { return \"forbidden\"; } }";
    assert!(source(hidden,RuntimeProfile::None,BuildMode::Exports{entries:vec!["answer".into()]}).is_err());
    let private="@id(\"app\") module app { @id(\"answer\") fn answer()->I64 { return 42; } }";
    assert!(source(private,RuntimeProfile::None,BuildMode::Exports{entries:vec!["answer".into()]}).is_err());
}

#[test]
#[cfg(target_os="linux")]
fn none_emits_freestanding_reproducible_object_and_real_machine_traps(){
    use std::os::unix::process::ExitStatusExt;
    let root=root("none");let text="@id(\"app\") module app visibility public { type Maybe=Option<I64>; fn identity(value:Maybe)->Maybe { return value; } @id(\"answer\") fn answer(x:I64)->I64 { let some:Maybe=Some(x); let value:Maybe=identity(some); match value { Some(number)=>{return number+1;} None=>{return 0;} } } }";
    let p=source(text,RuntimeProfile::None,BuildMode::Exports{entries:vec!["answer".into()]}).unwrap();let llvm=il_codegen_x86_64::emit(&p).unwrap();assert!(!llvm.ir.contains("@il_rt_"));assert!(!llvm.ir.contains("@llvm.mem"));
    let mut hashes=vec![];for name in ["first","second"]{let dir=root.join(name);fs::create_dir(&dir).unwrap();let built=compile(&llvm.ir,&dir,&LinkPlan::None,il_object_emitter_profile(false)).unwrap();assert!(built.runtime.is_none()&&built.executable.is_none());hashes.push(built.object.sha256.clone());let nm=command("/usr/bin/nm",&["-u",built.object.path.to_str().unwrap()]);assert!(nm.stdout.is_empty());let headers=command("/usr/bin/readelf",&["-h",built.object.path.to_str().unwrap()]);assert!(String::from_utf8_lossy(&headers.stdout).contains("REL (Relocatable file)"));
        for (input,trap) in [(41i64,false),(i64::MAX,true)]{let harness=dir.join(if trap{"trap.ll"}else{"host.ll"});let executable=dir.join(if trap{"trap"}else{"host"});fs::write(&harness,format!("declare i64 @{}(i64)\ndefine i32 @main() {{ %r=call i64 @{}(i64 {input})\n %ok=icmp eq i64 %r,42\n %bad=xor i1 %ok,true\n %code=zext i1 %bad to i32\n ret i32 %code }}",symbol("answer"),symbol("answer"))).unwrap();command("/usr/bin/clang-14",&[harness.to_str().unwrap(),built.object.path.to_str().unwrap(),"-o",executable.to_str().unwrap()]);let status=Command::new(executable).status().unwrap();if trap{assert_eq!(status.signal(),Some(4));}else{assert_eq!(status.code(),Some(0));}}
    }assert_eq!(hashes[0],hashes[1]);fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(target_os="linux")]
fn minimal_is_separate_static_no_std_and_preserves_stdout_and_trap_contracts(){
    let root=root("minimal");let archive=root.join("libil_minimal_runtime.a");let repository=Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");command("/usr/bin/python3",&[repository.join("runtime/minimal/build.py").to_str().unwrap(),"--output",archive.to_str().unwrap()]);
    let text="@id(\"app\") module app { @id(\"app.main\") fn main()->I32 effects [process] { runtime.print_i64(-9223372036854775808); return 37; } }";
    let p=source(text,RuntimeProfile::Minimal,BuildMode::Application{entry:"app.main".into()}).unwrap();let llvm=il_codegen_x86_64::emit(&p).unwrap();assert!(!llvm.ir.contains("@il_rt_"));let mut hashes=vec![];
    for name in ["first","second"]{let dir=root.join(name);fs::create_dir(&dir).unwrap();let built=compile(&llvm.ir,&dir,&LinkPlan::Minimal{archive:archive.clone()},il_object_emitter_profile(true)).unwrap();let exe=built.executable.as_ref().unwrap();hashes.push(exe.sha256.clone());let output=Command::new(&exe.path).output().unwrap();assert_eq!(output.status.code(),Some(37));assert_eq!(output.stdout,b"-9223372036854775808");assert!(output.stderr.is_empty());let symbols=command("/usr/bin/nm",&["-u",exe.path.to_str().unwrap()]);assert!(symbols.stdout.is_empty());let dynamic=command("/usr/bin/readelf",&["-d",exe.path.to_str().unwrap()]);assert!(String::from_utf8_lossy(&dynamic.stdout).contains("There is no dynamic section"));}
    assert_eq!(hashes[0],hashes[1]);
    {
        use std::os::fd::FromRawFd;
        unsafe extern "C"{fn pipe(fds:*mut i32)->i32;fn close(fd:i32)->i32;}
        let mut fds=[-1;2];assert_eq!(unsafe{pipe(fds.as_mut_ptr())},0);assert_eq!(unsafe{close(fds[0])},0);
        let writer=unsafe{fs::File::from_raw_fd(fds[1])};let status=Command::new(root.join("first/program.elf")).stdout(writer).status().unwrap();assert_eq!(status.code(),Some(37));
    }
    let p=source("@id(\"app\") module app { @id(\"app.main\") fn main()->I32 { return 2147483647+1; } }",RuntimeProfile::Minimal,BuildMode::Application{entry:"app.main".into()}).unwrap();let dir=root.join("trap");fs::create_dir(&dir).unwrap();let built=compile(&il_codegen_x86_64::emit(&p).unwrap().ir,&dir,&LinkPlan::Minimal{archive},il_object_emitter_profile(false)).unwrap();let output=Command::new(&built.executable.unwrap().path).output().unwrap();assert_eq!(output.status.code(),Some(101));assert!(String::from_utf8_lossy(&output.stderr).contains("E_INTEGER_OVERFLOW"));fs::remove_dir_all(root).unwrap();
}
fn il_object_emitter_profile(release:bool)->il_object_emitter::Profile{if release{il_object_emitter::Profile::Release}else{il_object_emitter::Profile::Debug}}
