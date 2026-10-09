use il_graph::{Capability,CapabilityKind};
use il_runtime_allocator::Allocator;
use il_runtime_handles::*;
use il_runtime_startup::*;
use std::{fs,os::{fd::{AsRawFd,FromRawFd,OwnedFd},unix::fs::symlink},sync::atomic::{AtomicUsize,Ordering},time::{Duration,Instant}};

fn pipe(nonblocking:bool)->(OwnedFd,OwnedFd){let mut fds=[0;2];assert_eq!(unsafe{libc::pipe2(fds.as_mut_ptr(),libc::O_CLOEXEC|if nonblocking{libc::O_NONBLOCK}else{0})},0);unsafe{(OwnedFd::from_raw_fd(fds[0]),OwnedFd::from_raw_fd(fds[1]))}}
fn host(path:&std::path::Path,faults:Option<Faults>,stdio:InheritedStdio,limit:Option<u64>)->HostResources{
    let grants=vec![Capability{entity_id:"read".into(),kind:CapabilityKind::FileRead,scope:Some(path.to_str().unwrap().into())},Capability{entity_id:"write".into(),kind:CapabilityKind::FileWrite,scope:Some(path.to_str().unwrap().into())},Capability{entity_id:"clock".into(),kind:CapabilityKind::ClockRead,scope:None}];
    let policy=HostPolicy{schema_version:"1.0.0".into(),grants:grants.clone(),test_faults:faults.clone()};let policy=ValidatedPolicy::new(policy,&grants,true).unwrap();initialize_signals().unwrap();HostResources::new(policy,stdio,Allocator::new(faults.as_ref(),limit))
}
fn no_stdio()->InheritedStdio{InheritedStdio::new(None,None,None)}
fn read_grant(host:&HostResources)->GrantId{host.policy().grant_id("read").unwrap()}
fn write_grant(host:&HostResources)->GrantId{host.policy().grant_id("write").unwrap()}

#[test]
fn real_files_short_reads_eof_modes_and_stale_generation(){
    let dir=tempfile::tempdir().unwrap();fs::write(dir.path().join("input"),b"abcdef").unwrap();let mut host=host(dir.path(),None,no_stdio(),None);
    let h=host.open_read(read_grant(&host),"input").unwrap();assert_eq!(host.read_some(h,2,Deadline::Infinite).unwrap().as_slice(),b"ab");assert_eq!(host.read_some(h,100,Deadline::Infinite).unwrap().as_slice(),b"cdef");assert!(host.read_some(h,1,Deadline::Infinite).unwrap().is_empty());assert_eq!(host.write_some(h,b"x",0,Deadline::Infinite),Err(IoError::PermissionDenied.into()));host.close(h).unwrap();
    let new=host.open_read(read_grant(&host),"input").unwrap();assert_eq!(h.slot,new.slot);assert_ne!(h.generation,new.generation);assert_eq!(host.close(h),Err(IoError::Closed.into()));assert_eq!(host.read_some(new,1,Deadline::Infinite).unwrap().as_slice(),b"a");host.drop_handle(new).unwrap();assert_eq!(host.live_handles(),0);
    let write=host.open_write(write_grant(&host),"output").unwrap();assert_eq!(host.write_some(write,b"xyz",1,Deadline::Infinite),Ok(2));assert_eq!(host.write_some(write,b"xyz",3,Deadline::Infinite),Ok(0));assert_eq!(host.write_some(write,b"xyz",4,Deadline::Infinite),Err(IoError::InvalidData.into()));host.close(write).unwrap();assert_eq!(fs::read(dir.path().join("output")).unwrap(),b"yz");
    let events=host.take_events();assert!(events.iter().any(|event|event.kind==HandleEventKind::Dropped));assert!(events.iter().any(|event|event.error==Some(IoError::Closed)));
}

#[test]
fn containment_and_held_directory_authority(){
    let base=tempfile::tempdir().unwrap();let scope=base.path().join("scope");fs::create_dir(&scope).unwrap();fs::write(base.path().join("secret"),b"secret").unwrap();fs::write(scope.join("inside"),b"original").unwrap();symlink("../secret",scope.join("escape")).unwrap();let mut host=host(&scope,None,no_stdio(),None);
    for path in ["../secret","/etc/passwd","x\0y","escape"]{assert_eq!(host.open_read(read_grant(&host),path),Err(IoError::PermissionDenied.into()),"{path:?}");}
    assert_eq!(host.open_read(write_grant(&host),"inside"),Err(IoError::PermissionDenied.into()));assert_eq!(host.open_read(read_grant(&host),"absent"),Err(IoError::Read.into()));
    fs::rename(&scope,base.path().join("moved")).unwrap();fs::create_dir(&scope).unwrap();fs::write(scope.join("inside"),b"replacement").unwrap();let h=host.open_read(read_grant(&host),"inside").unwrap();assert_eq!(host.read_some(h,100,Deadline::Infinite).unwrap().as_slice(),b"original");host.close(h).unwrap();
}

#[test]
fn actual_partial_io_and_failure_counters(){
    let dir=tempfile::tempdir().unwrap();let(r,w)=pipe(true);let faults=Faults{allocation_fail_after:None,io_max_chunk:Some(2),io_fail_after:Some(2)};let mut host=host(dir.path(),Some(faults),InheritedStdio::new(Some(r),Some(w),None),None);
    assert_eq!(host.stdout_write(b"hello",0,Deadline::Infinite),Ok(2));assert_eq!(host.stdin_read(5,Deadline::Infinite).unwrap().as_slice(),b"he");assert_eq!(host.successful_io_calls(),2);assert_eq!(host.stdout_write(b"hello",2,Deadline::Infinite),Err(IoError::Write.into()));assert_eq!(host.stdin_read(1,Deadline::Infinite).err(),Some(IoError::Read.into()));
}

#[test]
fn allocation_fault_is_typed_and_quota_is_distinct(){
    let dir=tempfile::tempdir().unwrap();fs::write(dir.path().join("input"),b"x").unwrap();let faults=Faults{allocation_fail_after:Some(0),io_max_chunk:None,io_fail_after:None};let mut failing=host(dir.path(),Some(faults),no_stdio(),None);let h=failing.open_read(read_grant(&failing),"input").unwrap();assert_eq!(failing.read_some(h,1,Deadline::Infinite).err(),Some(IoError::Other.into()));failing.close(h).unwrap();
    let mut quota=host(dir.path(),None,no_stdio(),Some(0));let h=quota.open_read(read_grant(&quota),"input").unwrap();assert_eq!(quota.read_some(h,1,Deadline::Infinite).err(),Some(IoFailure::ResourceLimit));quota.close(h).unwrap();
}

#[test]
fn nonblocking_timeout_and_blocking_stream_rejection(){
    let dir=tempfile::tempdir().unwrap();let(r,_w)=pipe(true);let mut resource=host(dir.path(),None,InheritedStdio::new(Some(r),None,None),None);let start=Instant::now();let at=monotonic_now().unwrap()+30_000_000;assert_eq!(resource.stdin_read(10,Deadline::At(at)).err(),Some(IoError::Timeout.into()));assert!(start.elapsed()>=Duration::from_millis(25));assert!(start.elapsed()<Duration::from_secs(1));
    let(r,_w)=pipe(false);let fd=r.as_raw_fd();let flags=unsafe{libc::fcntl(fd,libc::F_GETFL)};let mut resource=host(dir.path(),None,InheritedStdio::new(Some(r),None,None),None);assert_eq!(resource.stdin_read(1,Deadline::At(monotonic_now().unwrap()+1_000_000_000)).err(),Some(IoError::InvalidData.into()));assert_eq!(unsafe{libc::fcntl(fd,libc::F_GETFL)},flags);
}

static SIGNALS:AtomicUsize=AtomicUsize::new(0);
extern "C" fn interrupted(_:i32){SIGNALS.fetch_add(1,Ordering::SeqCst);}
#[test]
fn real_eintr_preserves_absolute_deadline(){
    let dir=tempfile::tempdir().unwrap();let(r,_w)=pipe(true);let mut resource=host(dir.path(),None,InheritedStdio::new(Some(r),None,None),None);
    unsafe{
        let mut action:libc::sigaction=std::mem::zeroed();action.sa_sigaction=interrupted as usize;libc::sigemptyset(&mut action.sa_mask);action.sa_flags=0;let mut old=std::mem::zeroed();assert_eq!(libc::sigaction(libc::SIGUSR1,&action,&mut old),0);
        let target=libc::pthread_self();let sender=std::thread::spawn(move||{for _ in 0..3{std::thread::sleep(Duration::from_millis(20));assert_eq!(libc::pthread_kill(target,libc::SIGUSR1),0);}});
        let start=Instant::now();let deadline=monotonic_now().unwrap()+100_000_000;let result=resource.stdin_read(1,Deadline::At(deadline));let elapsed=start.elapsed();sender.join().unwrap();assert_eq!(libc::sigaction(libc::SIGUSR1,&old,std::ptr::null_mut()),0);
        assert_eq!(result.err(),Some(IoError::Timeout.into()));assert_eq!(SIGNALS.load(Ordering::SeqCst),3);assert!(elapsed>=Duration::from_millis(90));assert!(elapsed<Duration::from_millis(150),"deadline reset on signal: {elapsed:?}");
        eprintln!("{{\"gate\":\"real_eintr_preserves_absolute_deadline\",\"signals\":3,\"deadline_ms\":100,\"elapsed_ms\":{}}}",elapsed.as_millis());
    }
}

#[test]
fn context_drop_closes_remaining_resources_and_clock_requires_its_grant(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("leak");fs::write(&path,b"x").unwrap();
    let count=||fs::read_dir("/proc/self/fd").unwrap().filter_map(|entry|entry.ok()).filter(|entry|fs::read_link(entry.path()).ok().as_ref()==Some(&path)).count();
    assert_eq!(count(),0);let mut resource=host(dir.path(),None,no_stdio(),None);resource.open_read(read_grant(&resource),"leak").unwrap();assert_eq!(count(),1);assert!(resource.clock_now(resource.policy().grant_id("clock").unwrap()).unwrap()>0);assert_eq!(resource.clock_now(read_grant(&resource)),Err(IoError::PermissionDenied.into()));drop(resource);assert_eq!(count(),0);
}

#[test]
fn whole_file_helpers_close_on_every_path_and_allocate_one_guest_result(){
    let dir=tempfile::tempdir().unwrap();let data=vec![b'x';20001];fs::write(dir.path().join("large"),&data).unwrap();let mut resource=host(dir.path(),None,no_stdio(),Some(30000));
    let output=resource.read_all(read_grant(&resource),"large").unwrap();assert_eq!(output.as_slice(),data);assert_eq!(resource.allocator().attempts(),1);assert_eq!(resource.allocator().live_allocations(),1);assert_eq!(resource.allocator().peak_bytes(),data.len()as u64);assert_eq!(resource.live_handles(),0);drop(output);
    let mut quota=host(dir.path(),None,no_stdio(),Some(10));assert_eq!(quota.read_all(read_grant(&quota),"large").err(),Some(IoFailure::ResourceLimit));assert_eq!(quota.live_handles(),0);assert_eq!(quota.allocator().attempts(),0);
    let faults=Faults{allocation_fail_after:None,io_max_chunk:Some(2),io_fail_after:Some(2)};let mut failing=host(dir.path(),Some(faults),no_stdio(),None);assert_eq!(failing.write_all(write_grant(&failing),"partial",b"abcdef"),Err(IoError::Write.into()));assert_eq!(fs::read(dir.path().join("partial")).unwrap(),b"abcd");assert_eq!(failing.live_handles(),0);
    let faults=Faults{allocation_fail_after:Some(0),io_max_chunk:None,io_fail_after:None};let mut failing=host(dir.path(),Some(faults),no_stdio(),None);assert_eq!(failing.read_all(read_grant(&failing),"large").err(),Some(IoError::Other.into()));assert_eq!(failing.live_handles(),0);assert_eq!(failing.allocator().attempts(),1);
}

#[test]
fn captured_outputs_reject_finite_deadlines_and_drain_large_writes(){
    let dir=tempfile::tempdir().unwrap();let mut resource=host(dir.path(),None,InheritedStdio::captured_output().unwrap(),None);let deadline=Deadline::At(monotonic_now().unwrap()+1_000_000_000);assert_eq!(resource.stdout_write(b"x",0,deadline),Err(IoError::InvalidData.into()));assert_eq!(resource.stderr_write(b"x",0,deadline),Err(IoError::InvalidData.into()));let bytes=vec![b'x';262144];let mut offset=0;while offset<bytes.len(){offset+=resource.stdout_write(&bytes,offset,Deadline::Infinite).unwrap();}assert_eq!(offset,bytes.len());drop(resource);
}
