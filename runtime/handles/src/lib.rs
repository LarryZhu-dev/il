//! Linux resource ownership and actual partial I/O, shared by the interpreter and native facade.
use il_graph::CapabilityKind;
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
enum Access{Read,Write}
struct Resource{fd:OwnedFd,access:Access,regular:bool}
struct Slot{generation:u64,resource:Option<Resource>}
struct IoFaults{successful:u64,max_chunk:Option<u64>,fail_after:Option<u64>}
pub struct HostResources{policy:ValidatedPolicy,stdio:InheritedStdio,allocator:Allocator,slots:Vec<Slot>,events:Vec<HandleEvent>,faults:IoFaults}

impl HostResources{
    pub fn new(policy:ValidatedPolicy,stdio:InheritedStdio,allocator:Allocator)->Self{
        let faults=IoFaults{successful:0,max_chunk:policy.faults().and_then(|f|f.io_max_chunk),fail_after:policy.faults().and_then(|f|f.io_fail_after)};
        Self{policy,stdio,allocator,slots:vec![],events:vec![],faults}
    }
    pub fn allocator(&self)->&Allocator{&self.allocator}
    pub fn policy(&self)->&ValidatedPolicy{&self.policy}
    pub fn live_handles(&self)->u64{self.slots.iter().filter(|slot|slot.resource.is_some()).count()as u64}
    pub fn take_events(&mut self)->Vec<HandleEvent>{std::mem::take(&mut self.events)}
    pub fn successful_io_calls(&self)->u64{self.faults.successful}
    pub fn validate_handle(&self,handle:HandleId)->Result<(),IoFailure>{self.slots.get(handle.slot as usize).filter(|slot|slot.generation==handle.generation&&slot.resource.is_some()).map(|_|()).ok_or(IoFailure::Error(IoError::Closed))}
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
        let resource=Resource{fd,access,regular};
        let slot=if let Some(index)=self.slots.iter().position(|slot|slot.resource.is_none()&&slot.generation<u64::MAX){self.slots[index].resource=Some(resource);index}else{self.slots.push(Slot{generation:1,resource:Some(resource)});self.slots.len()-1};
        let handle=HandleId{slot:slot as u64,generation:self.slots[slot].generation};self.events.push(HandleEvent{kind:HandleEventKind::Opened,handle,error:None});Ok(handle)
    }
    fn resource(&self,handle:HandleId,access:Access)->Result<(RawFd,bool),IoFailure>{
        let resource=self.slots.get(handle.slot as usize).filter(|slot|slot.generation==handle.generation).and_then(|slot|slot.resource.as_ref()).ok_or(IoError::Closed)?;
        if resource.access!=access{return Err(IoError::PermissionDenied.into())}Ok((resource.fd.as_raw_fd(),resource.regular))
    }
    pub fn close(&mut self,handle:HandleId)->Result<(),IoFailure>{self.release(handle,HandleEventKind::Closed)}
    pub fn drop_handle(&mut self,handle:HandleId)->Result<(),IoFailure>{self.release(handle,HandleEventKind::Dropped)}
    fn release(&mut self,handle:HandleId,kind:HandleEventKind)->Result<(),IoFailure>{
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
        })();let close=self.close(handle);match result{Err(error)=>Err(error),Ok(buffer)=>close.map(|_|buffer)}
    }
    pub fn write_all(&mut self,grant:GrantId,path:&str,bytes:&[u8])->Result<usize,IoFailure>{
        let handle=self.open_write(grant,path)?;let result=(||{let mut offset=0;while offset<bytes.len(){let count=self.write_some(handle,bytes,offset,Deadline::Infinite)?;if count==0{return Err(IoError::Write.into())}offset+=count;}Ok(offset)})();
        let close=self.close(handle);match result{Err(error)=>Err(error),Ok(count)=>close.map(|_|count)}
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
