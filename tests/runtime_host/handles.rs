use il_graph::{Capability,CapabilityKind,ResourceKind};
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
    let h=host.open_read(read_grant(&host),"input").unwrap();assert_eq!(host.read_some(h,2,Deadline::Infinite).unwrap().as_slice(),b"ab");assert_eq!(host.read_some(h,100,Deadline::Infinite).unwrap().as_slice(),b"cdef");assert!(host.read_some(h,1,Deadline::Infinite).unwrap().is_empty());assert_eq!(host.write_some(h,b"x",0,Deadline::Infinite),Err(IoError::PermissionDenied.into()));host.close(h,ResourceKind::File).unwrap();
    let new=host.open_read(read_grant(&host),"input").unwrap();assert_eq!(h.slot,new.slot);assert_ne!(h.generation,new.generation);assert_eq!(host.close(h,ResourceKind::File),Err(IoError::Closed.into()));assert_eq!(host.read_some(new,1,Deadline::Infinite).unwrap().as_slice(),b"a");host.drop_handle(new,ResourceKind::File).unwrap();assert_eq!(host.live_handles(),0);
    let write=host.open_write(write_grant(&host),"output").unwrap();assert_eq!(host.write_some(write,b"xyz",1,Deadline::Infinite),Ok(2));assert_eq!(host.write_some(write,b"xyz",3,Deadline::Infinite),Ok(0));assert_eq!(host.write_some(write,b"xyz",4,Deadline::Infinite),Err(IoError::InvalidData.into()));host.close(write,ResourceKind::File).unwrap();assert_eq!(fs::read(dir.path().join("output")).unwrap(),b"yz");
    let events=host.take_events();assert!(events.iter().any(|event|event.kind==HandleEventKind::Dropped));assert!(events.iter().any(|event|event.error==Some(IoError::Closed)));
}

#[test]
fn containment_and_held_directory_authority(){
    let base=tempfile::tempdir().unwrap();let scope=base.path().join("scope");fs::create_dir(&scope).unwrap();fs::write(base.path().join("secret"),b"secret").unwrap();fs::write(scope.join("inside"),b"original").unwrap();symlink("../secret",scope.join("escape")).unwrap();let mut host=host(&scope,None,no_stdio(),None);
    for path in ["../secret","/etc/passwd","x\0y","escape"]{assert_eq!(host.open_read(read_grant(&host),path),Err(IoError::PermissionDenied.into()),"{path:?}");}
    assert_eq!(host.open_read(write_grant(&host),"inside"),Err(IoError::PermissionDenied.into()));assert_eq!(host.open_read(read_grant(&host),"absent"),Err(IoError::Read.into()));
    fs::rename(&scope,base.path().join("moved")).unwrap();fs::create_dir(&scope).unwrap();fs::write(scope.join("inside"),b"replacement").unwrap();let h=host.open_read(read_grant(&host),"inside").unwrap();assert_eq!(host.read_some(h,100,Deadline::Infinite).unwrap().as_slice(),b"original");host.close(h,ResourceKind::File).unwrap();
}

#[test]
fn actual_partial_io_and_failure_counters(){
    let dir=tempfile::tempdir().unwrap();let(r,w)=pipe(true);let faults=Faults{allocation_fail_after:None,io_max_chunk:Some(2),io_fail_after:Some(2)};let mut host=host(dir.path(),Some(faults),InheritedStdio::new(Some(r),Some(w),None),None);
    assert_eq!(host.stdout_write(b"hello",0,Deadline::Infinite),Ok(2));assert_eq!(host.stdin_read(5,Deadline::Infinite).unwrap().as_slice(),b"he");assert_eq!(host.successful_io_calls(),2);assert_eq!(host.stdout_write(b"hello",2,Deadline::Infinite),Err(IoError::Write.into()));assert_eq!(host.stdin_read(1,Deadline::Infinite).err(),Some(IoError::Read.into()));
}

#[test]
fn allocation_fault_is_typed_and_quota_is_distinct(){
    let dir=tempfile::tempdir().unwrap();fs::write(dir.path().join("input"),b"x").unwrap();let faults=Faults{allocation_fail_after:Some(0),io_max_chunk:None,io_fail_after:None};let mut failing=host(dir.path(),Some(faults),no_stdio(),None);let h=failing.open_read(read_grant(&failing),"input").unwrap();assert_eq!(failing.read_some(h,1,Deadline::Infinite).err(),Some(IoError::Other.into()));failing.close(h,ResourceKind::File).unwrap();
    let mut quota=host(dir.path(),None,no_stdio(),Some(0));let h=quota.open_read(read_grant(&quota),"input").unwrap();assert_eq!(quota.read_some(h,1,Deadline::Infinite).err(),Some(IoFailure::ResourceLimit));quota.close(h,ResourceKind::File).unwrap();
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

fn network_host(ip:&str,faults:Option<Faults>)->HostResources{
    let probe=std::net::TcpListener::bind(format!("{ip}:0")).unwrap();let endpoint=probe.local_addr().unwrap().to_string();drop(probe);
    let grants=vec![Capability{entity_id:"connect".into(),kind:CapabilityKind::Connect,scope:Some(endpoint.clone())},Capability{entity_id:"listen".into(),kind:CapabilityKind::Listen,scope:Some(endpoint)}];let policy=ValidatedPolicy::new(HostPolicy{schema_version:"1.0.0".into(),grants:grants.clone(),test_faults:faults.clone()},&grants,true).unwrap();HostResources::new(policy,no_stdio(),Allocator::new(faults.as_ref(),Some(100000)))
}
fn within()->Deadline{Deadline::At(monotonic_now().unwrap()+1_000_000_000)}
fn connected(host:&mut HostResources)->(HandleId,HandleId,HandleId){let listener=host.net_listen(host.policy().grant_id("listen").unwrap()).unwrap();let client=host.net_connect(host.policy().grant_id("connect").unwrap(),within()).unwrap();let server=host.net_accept(listener,within()).unwrap();assert!(host.net_opened_at(client).unwrap()<=host.net_opened_at(server).unwrap());assert!(host.net_opened_at(server).unwrap()<=monotonic_now().unwrap());assert_eq!(host.net_opened_at(listener),Err(IoError::InvalidData.into()));(listener,client,server)}
#[test]
fn actual_ipv4_ipv6_partial_tcp_io_and_resource_kind_guards(){
    for ip in ["127.0.0.1","[::1]"]{let mut host=network_host(ip,Some(Faults{allocation_fail_after:None,io_max_chunk:Some(2),io_fail_after:None}));let(listener,client,server)=connected(&mut host);assert_eq!(host.live_handles(),3);
        assert_eq!(host.close(client,ResourceKind::File),Err(IoError::InvalidData.into()));assert_eq!(host.read_some(client,1,within()).err(),Some(IoError::InvalidData.into()));assert_eq!(host.net_read(listener,1,within()).err(),Some(IoError::InvalidData.into()));assert_eq!(host.net_write(client,b"abcde",0,within()),Ok(2));assert_eq!(host.net_read(server,10,within()).unwrap().as_slice(),b"ab");assert_eq!(host.net_write(server,b"reply",0,within()),Ok(2));assert_eq!(host.net_read(client,10,within()).unwrap().as_slice(),b"re");assert_eq!(host.successful_io_calls(),4);assert_eq!(host.net_write(client,b"abcde",5,within()),Ok(0));assert_eq!(host.net_write(client,b"abcde",6,within()),Err(IoError::InvalidData.into()));
        host.close(client,ResourceKind::Stream).unwrap();assert!(host.net_read(server,10,within()).unwrap().is_empty());host.drop_handle(server,ResourceKind::Stream).unwrap();host.close(listener,ResourceKind::Listener).unwrap();assert_eq!(host.live_handles(),0);assert_eq!(host.close(listener,ResourceKind::Listener),Err(IoError::Closed.into()));
    }
}
#[test]
fn tcp_deadlines_and_shared_positive_syscall_fault_counter(){
    let mut host=network_host("127.0.0.1",None);let listener=host.net_listen(host.policy().grant_id("listen").unwrap()).unwrap();let start=Instant::now();assert_eq!(host.net_accept(listener,Deadline::At(monotonic_now().unwrap()+30_000_000)),Err(IoError::Timeout.into()));assert!(start.elapsed()>=Duration::from_millis(25));assert!(start.elapsed()<Duration::from_secs(1));assert_eq!(host.net_connect(host.policy().grant_id("connect").unwrap(),Deadline::At(0)),Err(IoError::Timeout.into()));assert_eq!(host.live_handles(),1);host.close(listener,ResourceKind::Listener).unwrap();
    let mut host=network_host("127.0.0.1",Some(Faults{allocation_fail_after:None,io_max_chunk:Some(2),io_fail_after:Some(2)}));let(listener,client,server)=connected(&mut host);assert_eq!(host.successful_io_calls(),0);assert_eq!(host.net_write(client,b"abc",0,within()),Ok(2));assert_eq!(host.net_read(server,8,within()).unwrap().as_slice(),b"ab");assert_eq!(host.net_write(server,b"x",0,within()),Err(IoError::Write.into()));assert_eq!(host.net_read(client,1,within()).err(),Some(IoError::Read.into()));for(handle,kind)in [(listener,ResourceKind::Listener),(client,ResourceKind::Stream),(server,ResourceKind::Stream)]{host.close(handle,kind).unwrap();}assert_eq!(host.live_handles(),0);
}

#[test]
fn tcp_broken_peer_child(){
    if std::env::var_os("IL_TEST_TCP_BROKEN_PEER").is_none(){return}
    use std::io::Write;let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let endpoint=listener.local_addr().unwrap();let grant=Capability{entity_id:"connect".into(),kind:CapabilityKind::Connect,scope:Some(endpoint.to_string())};let policy=ValidatedPolicy::new(HostPolicy{schema_version:"1.0.0".into(),grants:vec![grant.clone()],test_faults:None},&[grant],true).unwrap();let mut host=HostResources::new(policy,no_stdio(),Allocator::new(None,Some(1000)));
    unsafe{let mut action:libc::sigaction=std::mem::zeroed();action.sa_sigaction=libc::SIG_DFL;assert_eq!(libc::sigemptyset(&mut action.sa_mask),0);assert_eq!(libc::sigaction(libc::SIGPIPE,&action,std::ptr::null_mut()),0);}
    let client=host.net_connect(GrantId(0),within()).unwrap();let(mut peer,_)=listener.accept().unwrap();peer.write_all(b"ready").unwrap();let linger=libc::linger{l_onoff:1,l_linger:0};assert_eq!(unsafe{libc::setsockopt(peer.as_raw_fd(),libc::SOL_SOCKET,libc::SO_LINGER,(&linger as *const libc::linger).cast(),std::mem::size_of_val(&linger)as libc::socklen_t)},0);drop(peer);drop(listener);
    let _=host.net_read(client,100,within());let _=host.net_read(client,100,within());assert!(host.net_write(client,b"x",0,within()).is_err());assert!(host.net_write(client,b"x",0,within()).is_err());host.close(client,ResourceKind::Stream).unwrap();assert_eq!(host.live_handles(),0);
}
#[test]
fn msg_nosignal_preserves_typed_errors_with_default_sigpipe(){let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","tcp_broken_peer_child"]).env("IL_TEST_TCP_BROKEN_PEER","1").output().unwrap();assert!(result.status.success(),"{result:?}");}

#[test]
fn numeric_ipv4_mapped_ipv6_connect_uses_its_exact_grant(){
    use std::io::Write;let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let endpoint=format!("[::ffff:127.0.0.1]:{}",listener.local_addr().unwrap().port());let grant=Capability{entity_id:"connect".into(),kind:CapabilityKind::Connect,scope:Some(endpoint)};let policy=ValidatedPolicy::new(HostPolicy{schema_version:"1.0.0".into(),grants:vec![grant.clone()],test_faults:None},&[grant],true).unwrap();let mut host=HostResources::new(policy,no_stdio(),Allocator::new(None,Some(100)));let stream=host.net_connect(GrantId(0),within()).unwrap();let(mut peer,_)=listener.accept().unwrap();peer.write_all(b"x").unwrap();assert_eq!(host.net_read(stream,1,within()).unwrap().as_slice(),b"x");host.close(stream,ResourceKind::Stream).unwrap();
}
