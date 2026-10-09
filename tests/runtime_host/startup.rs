use il_graph::{Capability,CapabilityKind};
use il_runtime_startup::*;
use std::{fs,os::fd::{AsRawFd,FromRawFd,OwnedFd}};

fn grant(id:&str,kind:CapabilityKind,scope:Option<String>)->Capability{Capability{entity_id:id.into(),kind,scope}}

#[test]
fn closed_policy_schema_and_faults(){
    let good=br#"{"schema_version":"1.0.0","grants":[],"test_faults":null}"#;assert_eq!(HostPolicy::parse(good).unwrap(),HostPolicy::empty());
    for json in [r#"{"schema_version":"1.0.0","grants":[]}"#,r#"{"schema_version":"1.0.0","grants":[],"test_faults":null,"extra":1}"#,r#"{"schema_version":"1.0.0","schema_version":"1.0.0","grants":[],"test_faults":null}"#,r#"{"schema_version":"1.0.0","grants":[],"test_faults":{"allocation_fail_after":null,"io_max_chunk":0,"io_fail_after":null}}"#,r#"{"schema_version":"1.0.0","grants":[],"test_faults":{"allocation_fail_after":null,"io_max_chunk":1}}"#]{assert!(HostPolicy::parse(json.as_bytes()).is_err(),"accepted {json}");}
    assert!(HostPolicy::parse(&vec![b' ';POLICY_MAX_BYTES+1]).is_err());
    let mut policy=HostPolicy::empty();let clock=grant("clock",CapabilityKind::ClockRead,None);policy.grants=vec![clock.clone(),clock];assert!(policy.validate().is_err());
    policy.grants=vec![grant("file",CapabilityKind::FileRead,Some("relative".into()))];assert!(policy.validate().is_err());
    policy.grants=vec![grant("clock",CapabilityKind::ClockRead,Some("scope".into()))];assert!(policy.validate().is_err());
}

#[test]
fn exact_grants_and_stable_indices_are_checked_before_start(){
    let dir=tempfile::tempdir().unwrap();let read=grant("z_read",CapabilityKind::FileRead,Some(dir.path().to_str().unwrap().into()));let clock=grant("a_clock",CapabilityKind::ClockRead,None);
    let mut policy=HostPolicy::empty();policy.grants=vec![read.clone(),clock.clone()];
    let checked=ValidatedPolicy::new(policy.clone(),&[read.clone(),clock.clone()],false).unwrap();assert_eq!(checked.grant_id("a_clock"),Some(GrantId(0)));assert_eq!(checked.grant_id("z_read"),Some(GrantId(1)));assert!(checked.directory(GrantId(1),CapabilityKind::FileRead).is_ok());assert!(checked.directory(GrantId(0),CapabilityKind::FileRead).is_err());
    let mut wrong=read.clone();wrong.scope=Some("/".into());assert!(ValidatedPolicy::new(policy.clone(),&[wrong],true).is_err());assert!(ValidatedPolicy::new(policy.clone(),&[read.clone(),read],true).is_err());
    policy.test_faults=Some(Faults{allocation_fail_after:Some(0),io_max_chunk:None,io_fail_after:None});assert!(ValidatedPolicy::new(policy.clone(),&[],false).is_err());assert!(ValidatedPolicy::new(policy,&[],true).is_ok());
}

#[test]
fn policy_fd_is_bounded_regular_input_and_stdio_flags_are_preserved(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("policy.json");fs::write(&path,br#"{"schema_version":"1.0.0","grants":[],"test_faults":null}"#).unwrap();let file=fs::File::open(path).unwrap();assert_eq!(load_policy_fd(file.as_raw_fd()).unwrap(),HostPolicy::empty());assert_eq!(load_policy_fd(i32::MAX).unwrap(),HostPolicy::empty());
    let mut pipe=[0;2];assert_eq!(unsafe{libc::pipe2(pipe.as_mut_ptr(),libc::O_CLOEXEC|libc::O_NONBLOCK)},0);let read=unsafe{OwnedFd::from_raw_fd(pipe[0])};let _write=unsafe{OwnedFd::from_raw_fd(pipe[1])};assert!(load_policy_fd(read.as_raw_fd()).is_err());
    let before=unsafe{libc::fcntl(0,libc::F_GETFL)};let stdio=InheritedStdio::capture().unwrap();let after=unsafe{libc::fcntl(0,libc::F_GETFL)};assert_eq!(before,after);if let Some(stdin)=stdio.stdin.as_ref(){assert!(stdin.as_raw_fd()>=5);assert_ne!(unsafe{libc::fcntl(stdin.as_raw_fd(),libc::F_GETFD)}&libc::FD_CLOEXEC,0);}
}

#[test]
fn captured_output_is_real_private_cloexec_io(){
    let stdio=InheritedStdio::captured_output().unwrap();let stdout=stdio.stdout.as_ref().unwrap();let stderr=stdio.stderr.as_ref().unwrap();assert!(stdout.as_raw_fd()>=5&&stderr.as_raw_fd()>=5);assert_ne!(stdout.as_raw_fd(),stderr.as_raw_fd());
    unsafe{assert_eq!(libc::fcntl(stdout.as_raw_fd(),libc::F_GETFL)&libc::O_NONBLOCK,0);assert_ne!(libc::fcntl(stdout.as_raw_fd(),libc::F_GETFD)&libc::FD_CLOEXEC,0);let mut stat=std::mem::MaybeUninit::<libc::stat>::uninit();assert_eq!(libc::fstat(stdout.as_raw_fd(),stat.as_mut_ptr()),0);assert_eq!(stat.assume_init().st_mode&libc::S_IFMT,libc::S_IFIFO);let bytes=[b'x';8192];for _ in 0..128{assert_eq!(libc::write(stdout.as_raw_fd(),bytes.as_ptr().cast(),bytes.len()),bytes.len()as isize);}}
    drop(stdio);
}

#[test]
fn closed_standard_stream_child(){
    if std::env::var_os("IL_TEST_CLOSED_STDIN").is_none(){return}
    let dir=tempfile::tempdir().unwrap();unsafe{libc::close(0);libc::close(4);}
    let grant=grant("read",CapabilityKind::FileRead,Some(dir.path().to_str().unwrap().into()));let policy=HostPolicy{schema_version:"1.0.0".into(),grants:vec![grant.clone()],test_faults:None};let checked=ValidatedPolicy::new(policy,&[grant],true).unwrap();assert!(checked.directory(GrantId(0),CapabilityKind::FileRead).unwrap().as_raw_fd()>=5);assert!(InheritedStdio::capture().unwrap().stdin.is_none());let captured=InheritedStdio::captured_output().unwrap();assert!(captured.stdin.is_none());assert_eq!(unsafe{libc::fcntl(0,libc::F_GETFD)},-1);assert_eq!(unsafe{libc::fcntl(4,libc::F_GETFD)},-1);
}
#[test]
fn authority_and_capture_never_repurpose_closed_reserved_descriptors(){let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","closed_standard_stream_child"]).env("IL_TEST_CLOSED_STDIN","1").output().unwrap();assert!(result.status.success(),"{result:?}");}
