//! Linux resource ownership and actual partial I/O, shared by the interpreter and native facade.
use il_graph::{CapabilityKind,ResourceKind};
use il_runtime_allocator::{AllocFailure,Allocator,OwnedBuffer};
use il_runtime_startup::{GrantId,InheritedStdio,ValidatedPolicy};
use std::{ffi::CString,os::fd::{AsRawFd,FromRawFd,IntoRawFd,OwnedFd,RawFd}};

#[repr(C)]
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct HandleId{pub slot:u64,pub generation:u64}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Deadline{Infinite,At(u64)}
#[repr(u32)]
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum IoError{Read=0,Write=1,Closed=2,Timeout=3,PermissionDenied=4,InvalidData=5,Other=6}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum IoFailure{Error(IoError),ResourceLimit,Unsupported}
impl From<IoError> for IoFailure{fn from(value:IoError)->Self{Self::Error(value)}}
impl From<AllocFailure> for IoFailure{fn from(value:AllocFailure)->Self{match value{AllocFailure::ResourceLimit=>Self::ResourceLimit,_=>IoError::Other.into()}}}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum HandleEventKind{Opened,Closed,Dropped}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct HandleEvent{pub kind:HandleEventKind,pub handle:HandleId,pub error:Option<IoError>}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Access{Read,Write,Duplex}
struct Resource{fd:OwnedFd,kind:ResourceKind,access:Access,regular:bool,opened_at:u64}
struct Slot{generation:u64,resource:Option<Resource>}
struct IoFaults{successful:u64,max_chunk:Option<u64>,fail_after:Option<u64>,accepts:u64,accept_fail_after:Option<u64>}
pub struct HostResources{policy:ValidatedPolicy,stdio:InheritedStdio,allocator:Allocator,slots:Vec<Slot>,events:Vec<HandleEvent>,faults:IoFaults}

impl HostResources{
    pub fn new(policy:ValidatedPolicy,stdio:InheritedStdio,allocator:Allocator)->Self{
        let faults=IoFaults{successful:0,max_chunk:policy.faults().and_then(|f|f.io_max_chunk),fail_after:policy.faults().and_then(|f|f.io_fail_after),accepts:0,accept_fail_after:policy.faults().and_then(|f|f.accept_fail_after)};
        Self{policy,stdio,allocator,slots:vec![],events:vec![],faults}
    }
    pub fn allocator(&self)->&Allocator{&self.allocator}
    pub fn policy(&self)->&ValidatedPolicy{&self.policy}
    pub fn live_handles(&self)->u64{self.slots.iter().filter(|slot|slot.resource.is_some()).count()as u64}
    pub fn take_events(&mut self)->Vec<HandleEvent>{std::mem::take(&mut self.events)}
    pub fn successful_io_calls(&self)->u64{self.faults.successful}
    pub fn validate_handle(&self,handle:HandleId,kind:ResourceKind)->Result<(),IoFailure>{let resource=self.lookup(handle)?;if resource.kind!=kind{return Err(IoError::InvalidData.into())}Ok(())}
    fn lookup(&self,handle:HandleId)->Result<&Resource,IoFailure>{self.slots.get(handle.slot as usize).filter(|slot|slot.generation==handle.generation).and_then(|slot|slot.resource.as_ref()).ok_or(IoFailure::Error(IoError::Closed))}
    fn insert(&mut self,resource:Resource)->HandleId{let slot=if let Some(index)=self.slots.iter().position(|slot|slot.resource.is_none()&&slot.generation<u64::MAX){self.slots[index].resource=Some(resource);index}else{self.slots.push(Slot{generation:1,resource:Some(resource)});self.slots.len()-1};let handle=HandleId{slot:slot as u64,generation:self.slots[slot].generation};self.events.push(HandleEvent{kind:HandleEventKind::Opened,handle,error:None});handle}

    pub fn net_listen(&mut self,grant:GrantId)->Result<HandleId,IoFailure>{
        let endpoint=self.policy.endpoint(grant,CapabilityKind::Listen).map_err(|_|IoError::PermissionDenied)?;let fd=socket(endpoint)?;let one:libc::c_int=1;
        if endpoint.is_ipv6()&&unsafe{libc::setsockopt(fd.as_raw_fd(),libc::IPPROTO_IPV6,libc::IPV6_V6ONLY,(&one as *const libc::c_int).cast(),std::mem::size_of_val(&one)as libc::socklen_t)}<0{return Err(last_error(Access::Write))}
        if unsafe{libc::setsockopt(fd.as_raw_fd(),libc::SOL_SOCKET,libc::SO_REUSEADDR,(&one as *const libc::c_int).cast(),std::mem::size_of_val(&one)as libc::socklen_t)}<0{return Err(last_error(Access::Write))}
        let(address,len)=socket_address(endpoint);if unsafe{libc::bind(fd.as_raw_fd(),(&address as *const libc::sockaddr_storage).cast(),len)}<0||unsafe{libc::listen(fd.as_raw_fd(),128)}<0{return Err(last_error(Access::Write))}
        Ok(self.insert(Resource{fd,kind:ResourceKind::Listener,access:Access::Read,regular:false,opened_at:0}))
    }
    pub fn net_connect(&mut self,grant:GrantId,deadline:Deadline)->Result<HandleId,IoFailure>{
        check_deadline(deadline)?;let endpoint=self.policy.endpoint(grant,CapabilityKind::Connect).map_err(|_|IoError::PermissionDenied)?;let fd=socket(endpoint)?;let(address,len)=socket_address(endpoint);
        let result=unsafe{libc::connect(fd.as_raw_fd(),(&address as *const libc::sockaddr_storage).cast(),len)};
        if result<0{let error=std::io::Error::last_os_error().raw_os_error().unwrap_or(0);if !matches!(error,libc::EINPROGRESS|libc::EALREADY|libc::EINTR|libc::EISCONN){return Err(map_errno(error,Access::Write).into())}
            if error!=libc::EISCONN{loop{wait(fd.as_raw_fd(),false,true,deadline)?;let mut error:libc::c_int=0;let mut size=std::mem::size_of_val(&error)as libc::socklen_t;if unsafe{libc::getsockopt(fd.as_raw_fd(),libc::SOL_SOCKET,libc::SO_ERROR,(&mut error as *mut libc::c_int).cast(),&mut size)}<0{return Err(last_error(Access::Write))}if error==0{break}if error==libc::EINPROGRESS||error==libc::EALREADY{continue}return Err(map_errno(error,Access::Write).into())}}
        }
        Ok(self.insert(Resource{fd,kind:ResourceKind::Stream,access:Access::Duplex,regular:false,opened_at:monotonic_now()?}))
    }
    pub fn net_accept(&mut self,listener:HandleId,deadline:Deadline)->Result<HandleId,IoFailure>{
        self.validate_handle(listener,ResourceKind::Listener)?;let raw=self.lookup(listener)?.fd.as_raw_fd();
        loop{wait(raw,false,false,deadline)?;
            if self.faults.accept_fail_after.is_some_and(|after|self.faults.accepts>=after){return Err(IoError::Read.into())}
            let fd=unsafe{libc::accept4(raw,std::ptr::null_mut(),std::ptr::null_mut(),libc::SOCK_NONBLOCK|libc::SOCK_CLOEXEC)};
            if fd>=0{self.faults.accepts+=1;return Ok(self.insert(Resource{fd:unsafe{OwnedFd::from_raw_fd(fd)},kind:ResourceKind::Stream,access:Access::Duplex,regular:false,opened_at:monotonic_now()?}))}
            let error=std::io::Error::last_os_error().raw_os_error().unwrap_or(0);if error==libc::EINTR||error==libc::EAGAIN{continue}return Err(map_errno(error,Access::Read).into())
        }
    }
    pub fn net_opened_at(&self,stream:HandleId)->Result<u64,IoFailure>{self.validate_handle(stream,ResourceKind::Stream)?;Ok(self.lookup(stream)?.opened_at)}
    pub fn net_read(&mut self,stream:HandleId,maximum:usize,deadline:Deadline)->Result<OwnedBuffer,IoFailure>{
        self.validate_handle(stream,ResourceKind::Stream)?;let fd=self.lookup(stream)?.fd.as_raw_fd();let maximum=self.faults.chunk(maximum);let mut buffer=self.allocator.allocate(maximum)?;if maximum==0{return Ok(buffer)}
        loop{self.faults.check(Access::Read)?;wait(fd,false,false,deadline)?;let count=unsafe{libc::recv(fd,buffer.as_mut_ptr().cast(),maximum,0)};if count>=0{let count=count as usize;buffer.truncate(count);self.faults.completed(count);return Ok(buffer)}let error=std::io::Error::last_os_error().raw_os_error().unwrap_or(0);if error==libc::EINTR||error==libc::EAGAIN{continue}return Err(map_errno(error,Access::Read).into())}
    }
    pub fn net_write(&mut self,stream:HandleId,bytes:&[u8],offset:usize,deadline:Deadline)->Result<usize,IoFailure>{
        self.validate_handle(stream,ResourceKind::Stream)?;let fd=self.lookup(stream)?.fd.as_raw_fd();if offset>bytes.len(){return Err(IoError::InvalidData.into())}if offset==bytes.len(){return Ok(0)}let count=self.faults.chunk(bytes.len()-offset);
        loop{self.faults.check(Access::Write)?;wait(fd,false,true,deadline)?;let wrote=unsafe{libc::send(fd,bytes[offset..].as_ptr().cast(),count,libc::MSG_NOSIGNAL)};if wrote>=0{let wrote=wrote as usize;self.faults.completed(wrote);return Ok(wrote)}let error=std::io::Error::last_os_error().raw_os_error().unwrap_or(0);if error==libc::EINTR||error==libc::EAGAIN{continue}return Err(map_errno(error,Access::Write).into())}
    }
    pub fn open_read(&mut self,grant:GrantId,path:&str)->Result<HandleId,IoFailure>{self.open(grant,path,Access::Read)}
    pub fn open_write(&mut self,grant:GrantId,path:&str)->Result<HandleId,IoFailure>{self.open(grant,path,Access::Write)}
    fn open(&mut self,grant:GrantId,path:&str,access:Access)->Result<HandleId,IoFailure>{
        if path.is_empty(){return Err(IoError::InvalidData.into())}
        if path.starts_with('/')||path.contains('\0')||path.split('/').any(|part|part==".."){return Err(IoError::PermissionDenied.into())}
        let kind=if access==Access::Read{CapabilityKind::FileRead}else{CapabilityKind::FileWrite};
        let directory=self.policy.directory(grant,kind).map_err(|_|IoFailure::Error(IoError::PermissionDenied))?;
        let path=CString::new(path).map_err(|_|IoError::PermissionDenied)?;
        #[repr(C)]struct OpenHow{flags:u64,mode:u64,resolve:u64}
        let flags=libc::O_CLOEXEC|libc::O_NONBLOCK|if access==Access::Read{libc::O_RDONLY}else{libc::O_WRONLY|libc::O_CREAT|libc::O_TRUNC};
        let how=OpenHow{flags:flags as u64,mode:if access==Access::Write{0o600}else{0},resolve:0x08|0x02};
        let fd=loop{
            let fd=unsafe{libc::syscall(libc::SYS_openat2,directory.as_raw_fd(),path.as_ptr(),&how,std::mem::size_of::<OpenHow>())};
            if fd>=0{break fd as RawFd}
            let error=std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if error==libc::EINTR{continue}
            if error==libc::ENOSYS||error==libc::E2BIG||error==libc::EINVAL{return Err(IoFailure::Unsupported)}
            return Err(map_errno(error,access).into())
        };
        let fd=unsafe{OwnedFd::from_raw_fd(fd)};let regular=regular(fd.as_raw_fd())?;
        Ok(self.insert(Resource{fd,kind:ResourceKind::File,access,regular,opened_at:0}))
    }

    fn resource(&self,handle:HandleId,access:Access)->Result<(RawFd,bool),IoFailure>{
        let resource=self.lookup(handle)?;if resource.kind!=ResourceKind::File{return Err(IoError::InvalidData.into())}
        if resource.access!=access{return Err(IoError::PermissionDenied.into())}Ok((resource.fd.as_raw_fd(),resource.regular))
    }
    pub fn close(&mut self,handle:HandleId,kind:ResourceKind)->Result<(),IoFailure>{self.release(handle,kind,HandleEventKind::Closed)}
    pub fn drop_handle(&mut self,handle:HandleId,kind:ResourceKind)->Result<(),IoFailure>{self.release(handle,kind,HandleEventKind::Dropped)}
    fn release(&mut self,handle:HandleId,expected:ResourceKind,kind:HandleEventKind)->Result<(),IoFailure>{
        if let Err(error)=self.validate_handle(handle,expected){let IoFailure::Error(error)=error else{unreachable!()};self.events.push(HandleEvent{kind,handle,error:Some(error)});return Err(error.into())}
        let Some(slot)=self.slots.get_mut(handle.slot as usize).filter(|slot|slot.generation==handle.generation&&slot.resource.is_some())else{
            self.events.push(HandleEvent{kind,handle,error:Some(IoError::Closed)});return Err(IoError::Closed.into())
        };
        let resource=slot.resource.take().unwrap();slot.generation=slot.generation.saturating_add(1);
        let raw=resource.fd.into_raw_fd();let result=unsafe{libc::close(raw)};
        // Linux may release a descriptor even when close reports EINTR: retrying could close a replacement.
        let error=if result<0{Some(map_errno(std::io::Error::last_os_error().raw_os_error().unwrap_or(0),resource.access))}else{None};
        self.events.push(HandleEvent{kind,handle,error});if let Some(error)=error{Err(error.into())}else{Ok(())}
    }
    pub fn read_some(&mut self,handle:HandleId,maximum:usize,deadline:Deadline)->Result<OwnedBuffer,IoFailure>{let(fd,regular)=self.resource(handle,Access::Read)?;self.read(fd,regular,maximum,deadline)}
    pub fn write_some(&mut self,handle:HandleId,bytes:&[u8],offset:usize,deadline:Deadline)->Result<usize,IoFailure>{let(fd,regular)=self.resource(handle,Access::Write)?;self.write(fd,regular,bytes,offset,deadline)}
    pub fn stdin_read(&mut self,maximum:usize,deadline:Deadline)->Result<OwnedBuffer,IoFailure>{let fd=self.stdio.stdin.as_ref().ok_or(IoError::Closed)?.as_raw_fd();self.read(fd,regular(fd)?,maximum,deadline)}
    pub fn stdout_write(&mut self,bytes:&[u8],offset:usize,deadline:Deadline)->Result<usize,IoFailure>{let fd=self.stdio.stdout.as_ref().ok_or(IoError::Closed)?.as_raw_fd();self.write(fd,regular(fd)?,bytes,offset,deadline)}
    pub fn stderr_write(&mut self,bytes:&[u8],offset:usize,deadline:Deadline)->Result<usize,IoFailure>{let fd=self.stdio.stderr.as_ref().ok_or(IoError::Closed)?.as_raw_fd();self.write(fd,regular(fd)?,bytes,offset,deadline)}
    pub fn clock_now(&self,grant:GrantId)->Result<u64,IoFailure>{self.policy.check(grant,CapabilityKind::ClockRead).map_err(|_|IoError::PermissionDenied)?;monotonic_now()}
    pub fn read_all(&mut self,grant:GrantId,path:&str)->Result<OwnedBuffer,IoFailure>{
        let handle=self.open_read(grant,path)?;
        let result=(||{
            let(fd,regular)=self.resource(handle,Access::Read)?;let mut data=Vec::new();let mut scratch=[0u8;8192];
            loop{let count=self.read_into(fd,regular,&mut scratch,Deadline::Infinite)?;if count==0{break}
                let length=data.len().checked_add(count).ok_or(IoError::Other)?;self.allocator.check_capacity(length)?;
                data.try_reserve(count).map_err(|_|IoError::Other)?;data.extend_from_slice(&scratch[..count]);}
            self.allocator.copy(&data).map_err(IoFailure::from)
        })();let close=self.close(handle,ResourceKind::File);match result{Err(error)=>Err(error),Ok(buffer)=>close.map(|_|buffer)}
    }
    pub fn write_all(&mut self,grant:GrantId,path:&str,bytes:&[u8])->Result<usize,IoFailure>{
        let handle=self.open_write(grant,path)?;let result=(||{let mut offset=0;while offset<bytes.len(){let count=self.write_some(handle,bytes,offset,Deadline::Infinite)?;if count==0{return Err(IoError::Write.into())}offset+=count;}Ok(offset)})();
        let close=self.close(handle,ResourceKind::File);match result{Err(error)=>Err(error),Ok(count)=>close.map(|_|count)}
    }
    fn read(&mut self,fd:RawFd,regular:bool,maximum:usize,deadline:Deadline)->Result<OwnedBuffer,IoFailure>{
        let maximum=self.faults.chunk(maximum);let mut buffer=self.allocator.allocate(maximum)?;
        let count=self.read_into(fd,regular,buffer.as_mut_slice(),deadline)?;buffer.truncate(count);Ok(buffer)
    }
    fn read_into(&mut self,fd:RawFd,regular:bool,buffer:&mut[u8],deadline:Deadline)->Result<usize,IoFailure>{
        let maximum=self.faults.chunk(buffer.len());if maximum==0{return Ok(0)}
        loop{
            self.faults.check(Access::Read)?;wait(fd,regular,false,deadline)?;
            let count=unsafe{libc::read(fd,buffer.as_mut_ptr().cast(),maximum)};
            if count>=0{let count=count as usize;self.faults.completed(count);return Ok(count)}
            let error=std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if error==libc::EINTR||error==libc::EAGAIN{continue}return Err(map_errno(error,Access::Read).into())
        }
    }
    fn write(&mut self,fd:RawFd,regular:bool,bytes:&[u8],offset:usize,deadline:Deadline)->Result<usize,IoFailure>{
        if offset>bytes.len(){return Err(IoError::InvalidData.into())}if offset==bytes.len(){return Ok(0)}
        let count=self.faults.chunk(bytes.len()-offset);
        loop{
            self.faults.check(Access::Write)?;wait(fd,regular,true,deadline)?;
            let wrote=unsafe{libc::write(fd,bytes[offset..].as_ptr().cast(),count)};
            if wrote>=0{let wrote=wrote as usize;self.faults.completed(wrote);return Ok(wrote)}
            let error=std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if error==libc::EINTR||error==libc::EAGAIN{continue}return Err(map_errno(error,Access::Write).into())
        }
    }
}
impl IoFaults{
    fn check(&self,access:Access)->Result<(),IoFailure>{if self.fail_after.is_some_and(|limit|self.successful>=limit){Err(if access==Access::Read{IoError::Read}else{IoError::Write}.into())}else{Ok(())}}
    fn chunk(&self,bytes:usize)->usize{self.max_chunk.map_or(bytes,|limit|bytes.min(limit.min(usize::MAX as u64)as usize))}
    fn completed(&mut self,bytes:usize){if bytes>0{self.successful=self.successful.saturating_add(1);}}
}
fn map_errno(error:i32,access:Access)->IoError{match error{libc::EBADF=>IoError::Closed,libc::EACCES|libc::EPERM|libc::EXDEV|libc::ELOOP=>IoError::PermissionDenied,libc::ETIMEDOUT=>IoError::Timeout,libc::EINVAL=>IoError::InvalidData,_=>if access==Access::Read{IoError::Read}else{IoError::Write}}}
fn regular(fd:RawFd)->Result<bool,IoFailure>{let mut stat=std::mem::MaybeUninit::<libc::stat>::uninit();if unsafe{libc::fstat(fd,stat.as_mut_ptr())}<0{return Err(map_errno(std::io::Error::last_os_error().raw_os_error().unwrap_or(0),Access::Read).into())}Ok(unsafe{stat.assume_init()}.st_mode&libc::S_IFMT==libc::S_IFREG)}
pub fn monotonic_now()->Result<u64,IoFailure>{let mut time=libc::timespec{tv_sec:0,tv_nsec:0};if unsafe{libc::clock_gettime(libc::CLOCK_MONOTONIC,&mut time)}<0{return Err(IoError::Other.into())}u64::try_from(time.tv_sec).ok().and_then(|seconds|seconds.checked_mul(1_000_000_000)).and_then(|n|n.checked_add(time.tv_nsec as u64)).ok_or(IoFailure::Error(IoError::Other))}
fn wait(fd:RawFd,regular:bool,writing:bool,deadline:Deadline)->Result<(),IoFailure>{
    let flags=unsafe{libc::fcntl(fd,libc::F_GETFL)};if flags<0{return Err(IoError::Closed.into())}
    if !regular&&matches!(deadline,Deadline::At(_))&&flags&libc::O_NONBLOCK==0{return Err(IoError::InvalidData.into())}
    loop{
        let timeout=match deadline{Deadline::Infinite=>-1,Deadline::At(at)=>{let now=monotonic_now()?;if now>=at{return Err(IoError::Timeout.into())}((at-now).div_ceil(1_000_000)).min(i32::MAX as u64)as i32}};
        if regular{return Ok(())}
        let mut poll=libc::pollfd{fd,events:if writing{libc::POLLOUT}else{libc::POLLIN},revents:0};let result=unsafe{libc::poll(&mut poll,1,timeout)};
        if result<0{let error=std::io::Error::last_os_error().raw_os_error().unwrap_or(0);if error==libc::EINTR{continue}return Err(map_errno(error,if writing{Access::Write}else{Access::Read}).into())}
        if result==0{continue}
        if poll.revents&libc::POLLNVAL!=0{return Err(IoError::Closed.into())}
        // HUP/ERR still require read/write to surface EOF or the concrete OS failure.
        if let Deadline::At(at)=deadline{if monotonic_now()?>=at{return Err(IoError::Timeout.into())}}
        return Ok(())
    }
}

fn check_deadline(deadline:Deadline)->Result<(),IoFailure>{if let Deadline::At(at)=deadline{if monotonic_now()?>=at{return Err(IoError::Timeout.into())}}Ok(())}
fn last_error(access:Access)->IoFailure{map_errno(std::io::Error::last_os_error().raw_os_error().unwrap_or(0),access).into()}
fn socket(endpoint:std::net::SocketAddr)->Result<OwnedFd,IoFailure>{
    let fd=unsafe{libc::socket(if endpoint.is_ipv4(){libc::AF_INET}else{libc::AF_INET6},libc::SOCK_STREAM|libc::SOCK_NONBLOCK|libc::SOCK_CLOEXEC,libc::IPPROTO_TCP)};if fd<0{return Err(last_error(Access::Write))}let fd=unsafe{OwnedFd::from_raw_fd(fd)};
    Ok(fd)
}
fn socket_address(endpoint:std::net::SocketAddr)->(libc::sockaddr_storage,libc::socklen_t){
    let mut storage:libc::sockaddr_storage=unsafe{std::mem::zeroed()};let size=match endpoint{
        std::net::SocketAddr::V4(endpoint)=>{let value=libc::sockaddr_in{sin_family:libc::AF_INET as libc::sa_family_t,sin_port:endpoint.port().to_be(),sin_addr:libc::in_addr{s_addr:u32::from_ne_bytes(endpoint.ip().octets())},sin_zero:[0;8]};unsafe{std::ptr::write((&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr_in>(),value);}std::mem::size_of::<libc::sockaddr_in>()},
        std::net::SocketAddr::V6(endpoint)=>{let value=libc::sockaddr_in6{sin6_family:libc::AF_INET6 as libc::sa_family_t,sin6_port:endpoint.port().to_be(),sin6_flowinfo:endpoint.flowinfo(),sin6_addr:libc::in6_addr{s6_addr:endpoint.ip().octets()},sin6_scope_id:endpoint.scope_id()};unsafe{std::ptr::write((&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr_in6>(),value);}std::mem::size_of::<libc::sockaddr_in6>()},
    };(storage,size as libc::socklen_t)
}
