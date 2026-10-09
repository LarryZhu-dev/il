//! Trusted host-policy parsing and Linux descriptor authority. No guest evaluator.
use il_graph::{Capability,CapabilityKind};
use serde::{Deserialize,Serialize};
use std::{collections::BTreeSet,ffi::CString,fs::File,io::Read,os::fd::{AsFd,AsRawFd,BorrowedFd,FromRawFd,OwnedFd,RawFd},path::Path,net::SocketAddr};

pub const POLICY_MAX_BYTES:usize=65536;
fn required_nullable<'de,D:serde::Deserializer<'de>,T:Deserialize<'de>>(d:D)->Result<Option<T>,D::Error>{Option::<T>::deserialize(d)}

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct Faults{
    #[serde(deserialize_with="required_nullable")]
    pub allocation_fail_after:Option<u64>,
    #[serde(deserialize_with="required_nullable")]
    pub io_max_chunk:Option<u64>,
    #[serde(deserialize_with="required_nullable")]
    pub io_fail_after:Option<u64>,
}

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct HostPolicy{
    pub schema_version:String,
    pub grants:Vec<Capability>,
    #[serde(deserialize_with="required_nullable")]
    pub test_faults:Option<Faults>,
}
impl Default for HostPolicy{fn default()->Self{Self{schema_version:"1.0.0".into(),grants:vec![],test_faults:None}}}

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct StartupError{pub code:&'static str,pub message:String}
impl std::fmt::Display for StartupError{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{write!(f,"{}: {}",self.code,self.message)}}
impl std::error::Error for StartupError{}
fn invalid(message:impl Into<String>)->StartupError{StartupError{code:"E_HOST_POLICY_INVALID",message:message.into()}}
fn denied(message:impl Into<String>)->StartupError{StartupError{code:"E_CAPABILITY_MISSING",message:message.into()}}

impl HostPolicy{
    pub fn empty()->Self{Self::default()}
    pub fn parse(bytes:&[u8])->Result<Self,StartupError>{
        if bytes.len()>POLICY_MAX_BYTES{return Err(invalid("host policy exceeds 64 KiB"))}
        let policy:Self=serde_json::from_slice(bytes).map_err(|e|invalid(e.to_string()))?;policy.validate()?;Ok(policy)
    }
    pub fn validate(&self)->Result<(),StartupError>{
        if self.schema_version!="1.0.0"{return Err(invalid("unknown host policy version"))}
        if self.test_faults.as_ref().is_some_and(|f|f.io_max_chunk==Some(0)){return Err(invalid("io_max_chunk must be positive"))}
        let mut ids=BTreeSet::new();
        for grant in &self.grants{
            if !il_graph::valid_id(&grant.entity_id)||!ids.insert(&grant.entity_id){return Err(invalid("invalid or duplicate grant identifier"))}
            match grant.kind{
                CapabilityKind::FileRead|CapabilityKind::FileWrite=>{
                    let scope=grant.scope.as_ref().ok_or_else(||invalid("directory grant requires scope"))?;
                    if scope.contains('\0')||!Path::new(scope).is_absolute(){return Err(invalid("directory scope must be an absolute NUL-free path"))}
                }
                CapabilityKind::Listen|CapabilityKind::Connect=>{parse_endpoint(grant.scope.as_deref().ok_or_else(||invalid("network grant requires endpoint scope"))?)?;}
                CapabilityKind::ClockRead=>if grant.scope.is_some(){return Err(invalid("clock grant has no scope"))},
                _=>return Err(StartupError{code:"E_UNSUPPORTED_FEATURE",message:"host capability kind requires its implementation stage".into()}),
            }
        }
        Ok(())
    }
}

#[repr(transparent)]
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct GrantId(pub u64);
struct CheckedGrant{declaration:Capability,directory:Option<OwnedFd>}
pub struct ValidatedPolicy{grants:Vec<CheckedGrant>,faults:Option<Faults>}
impl ValidatedPolicy{
    /// Requirement indices are sorted by stable ID and contain only exact host-authorized grants.
    pub fn new(policy:HostPolicy,requirements:&[Capability],captured:bool)->Result<Self,StartupError>{
        policy.validate()?;
        if !captured&&policy.test_faults.is_some(){return Err(invalid("test fault injection is forbidden for applications"))}
        let mut sorted=requirements.to_vec();sorted.sort_by(|a,b|a.entity_id.cmp(&b.entity_id));
        if sorted.windows(2).any(|pair|pair[0].entity_id==pair[1].entity_id){return Err(invalid("duplicate capability requirement"))}
        let mut grants=vec![];
        for declaration in sorted{
            if !policy.grants.iter().any(|grant|*grant==declaration){return Err(denied(format!("no exact host grant for {}",declaration.entity_id)))}
            let directory=match declaration.kind{
                CapabilityKind::FileRead|CapabilityKind::FileWrite=>{
                    let path=CString::new(declaration.scope.as_deref().unwrap()).map_err(|_|invalid("scope contains NUL"))?;
                    let fd=unsafe{libc::open(path.as_ptr(),libc::O_PATH|libc::O_DIRECTORY|libc::O_CLOEXEC)};
                    if fd<0{return Err(denied(format!("cannot open directory scope {}: {}",declaration.entity_id,std::io::Error::last_os_error())))}
                    let directory=unsafe{OwnedFd::from_raw_fd(fd)};
                    // Held authority must not become an accidentally inherited standard stream.
                    Some(if fd<5{duplicate(fd)?.ok_or_else(||denied("directory descriptor disappeared"))?}else{directory})
                }
                CapabilityKind::ClockRead|CapabilityKind::Listen|CapabilityKind::Connect=>None,
                _=>return Err(invalid("unsupported grant requirement")),
            };
            grants.push(CheckedGrant{declaration,directory});
        }
        Ok(Self{grants,faults:policy.test_faults})
    }
    pub fn requirements(&self)->Vec<Capability>{self.grants.iter().map(|g|g.declaration.clone()).collect()}
    pub fn grant_id(&self,entity:&str)->Option<GrantId>{self.grants.iter().position(|g|g.declaration.entity_id==entity).map(|n|GrantId(n as u64))}
    pub fn faults(&self)->Option<&Faults>{self.faults.as_ref()}
    pub fn check(&self,id:GrantId,kind:CapabilityKind)->Result<(),StartupError>{
        self.grants.get(id.0 as usize).filter(|g|g.declaration.kind==kind).ok_or_else(||denied("invalid grant index or kind"))?;Ok(())
    }
    pub fn endpoint(&self,id:GrantId,kind:CapabilityKind)->Result<SocketAddr,StartupError>{self.check(id,kind)?;if !matches!(kind,CapabilityKind::Listen|CapabilityKind::Connect){return Err(denied("grant is not a network endpoint"))}parse_endpoint(self.grants[id.0 as usize].declaration.scope.as_deref().ok_or_else(||invalid("missing endpoint"))?)}
    pub fn directory(&self,id:GrantId,kind:CapabilityKind)->Result<BorrowedFd<'_>,StartupError>{
        self.check(id,kind)?;self.grants[id.0 as usize].directory.as_ref().map(AsFd::as_fd).ok_or_else(||denied("grant is not a directory grant"))
    }
}

fn parse_endpoint(scope:&str)->Result<SocketAddr,StartupError>{let endpoint:SocketAddr=scope.parse().map_err(|_|invalid("network scope requires canonical numeric address:port"))?;if endpoint.port()==0||endpoint.to_string()!=scope{return Err(invalid("network scope requires nonzero canonical numeric endpoint"))}Ok(endpoint)}

pub struct InheritedStdio{pub stdin:Option<OwnedFd>,pub stdout:Option<OwnedFd>,pub stderr:Option<OwnedFd>,drains:Vec<std::thread::JoinHandle<()>>}
impl InheritedStdio{
    pub fn new(stdin:Option<OwnedFd>,stdout:Option<OwnedFd>,stderr:Option<OwnedFd>)->Self{Self{stdin,stdout,stderr,drains:vec![]}}
    /// Duplicate descriptors without changing the shared open-file-description flags.
    pub fn capture()->Result<Self,StartupError>{Ok(Self::new(duplicate(0)?,duplicate(1)?,duplicate(2)?))}
    /// Private blocking pipes preserve native captured-stream semantics. Drainers discard
    /// bytes with bounded stack storage; execution engines retain only successful writes.
    pub fn captured_output()->Result<Self,StartupError>{
        fn sink()->Result<(OwnedFd,std::thread::JoinHandle<()>),StartupError>{
            let mut fds=[-1;2];if unsafe{libc::pipe2(fds.as_mut_ptr(),libc::O_CLOEXEC)}<0{return Err(invalid("cannot create captured output pipe"))}
            let read=unsafe{OwnedFd::from_raw_fd(fds[0])};let write=unsafe{OwnedFd::from_raw_fd(fds[1])};
            let reader=duplicate(read.as_raw_fd())?.ok_or_else(||invalid("captured reader disappeared"))?;
            let writer=duplicate(write.as_raw_fd())?.ok_or_else(||invalid("captured writer disappeared"))?;drop(read);drop(write);
            let drain=std::thread::Builder::new().name("il-output-drain".into()).spawn(move||{let mut reader=File::from(reader);let mut scratch=[0u8;8192];loop{match reader.read(&mut scratch){Ok(0)=>break,Ok(_)=>{},Err(error)if error.kind()==std::io::ErrorKind::Interrupted=>{},Err(_)=>break}}}).map_err(|error|invalid(error.to_string()))?;
            Ok((writer,drain))
        }
        let mut result=Self::new(duplicate(0)?,None,None);let(stdout,drain)=sink()?;result.stdout=Some(stdout);result.drains.push(drain);let(stderr,drain)=sink()?;result.stderr=Some(stderr);result.drains.push(drain);Ok(result)
    }
}
impl Drop for InheritedStdio{fn drop(&mut self){self.stdout.take();self.stderr.take();for drain in self.drains.drain(..){let _=drain.join();}}}

fn duplicate(fd:RawFd)->Result<Option<OwnedFd>,StartupError>{
    let result=unsafe{libc::fcntl(fd,libc::F_DUPFD_CLOEXEC,5)};
    if result>=0{Ok(Some(unsafe{OwnedFd::from_raw_fd(result)}))}else if std::io::Error::last_os_error().raw_os_error()==Some(libc::EBADF){Ok(None)}else{Err(invalid(std::io::Error::last_os_error().to_string()))}
}

/// Descriptor 4 is a bounded regular policy input; absence supplies no authority.
pub fn load_policy_fd(fd:RawFd)->Result<HostPolicy,StartupError>{
    let Some(fd)=duplicate(fd)?else{return Ok(HostPolicy::default())};
    let mut stat=std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe{libc::fstat(fd.as_raw_fd(),stat.as_mut_ptr())}<0{return Err(invalid("cannot stat policy descriptor"))}
    let stat=unsafe{stat.assume_init()};if stat.st_mode&libc::S_IFMT!=libc::S_IFREG||stat.st_size<0||stat.st_size as usize>POLICY_MAX_BYTES{return Err(invalid("policy descriptor must be a regular file of at most 64 KiB"))}
    if unsafe{libc::lseek(fd.as_raw_fd(),0,libc::SEEK_SET)}<0{return Err(invalid("cannot rewind policy input"))}
    let mut bytes=vec![];File::from(fd).take((POLICY_MAX_BYTES+1)as u64).read_to_end(&mut bytes).map_err(|e|invalid(e.to_string()))?;HostPolicy::parse(&bytes)
}

/// Called by full startup before exposing standard output to guest code.
pub fn initialize_signals()->Result<(),StartupError>{
    unsafe{let mut action:libc::sigaction=std::mem::zeroed();action.sa_sigaction=libc::SIG_IGN;
        if libc::sigemptyset(&mut action.sa_mask)!=0||libc::sigaction(libc::SIGPIPE,&action,std::ptr::null_mut())!=0{return Err(invalid("cannot install SIGPIPE policy"))}}
    Ok(())
}
