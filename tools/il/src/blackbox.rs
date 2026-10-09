//! Fixed external TCP contract against actual compiled il applications.
use crate::{native, protocol::Failure, source::Context};
use il_graph::{canonical_bytes, hash_bytes, Diagnostic, Graph};
use il_native_ir::RuntimeProfile;
use il_object_emitter::Profile;
use il_runtime_startup::HostPolicy;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs::{self, File, OpenOptions}, io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpStream}, path::{Path, PathBuf},
    process::{Child, Command, Stdio}, time::{Duration, Instant}};

const CONTRACT: &[u8] = include_bytes!("../../../spec/http.yaml");
const WIRE_LIMIT: usize = 65536;
const LOG_LIMIT: u64 = 1_048_576;

fn save(path: &Path, value: &Value) -> Result<(), Failure> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path).map_err(Failure::io)?;
    file.write_all(&canonical_bytes(value).map_err(Failure::json)?).and_then(|_|file.sync_all()).map_err(Failure::io)
}
fn artifact(path: &Path) -> Result<Value, Failure> {
    Ok(json!({"path":path.canonicalize().map_err(Failure::io)?,"sha256":hash_bytes(&fs::read(path).map_err(Failure::io)?)}))
}
fn failure(message: impl AsRef<str>) -> Failure { Failure::new("E_NATIVE_EXECUTION_FAILED", message.as_ref()) }

// Only the byte-identical compiler-locked contract is accepted. This extracts
// scalar expectations from its mapping; it is not a general YAML evaluator.
fn rules(context: &Context) -> Result<BTreeMap<String, String>, Failure> {
    let bytes = fs::read(context.repository.join("spec/http.yaml")).map_err(Failure::io)?;
    if bytes != CONTRACT { return Err(Failure::new("E_STATE_INCONSISTENT", "HTTP contract differs from compiler-locked contract")); }
    let text = std::str::from_utf8(&bytes).map_err(|_|Failure::input("HTTP contract is not UTF-8"))?;
    let mut stack: Vec<(usize, &str)> = vec![];
    let mut values = BTreeMap::new();
    for line in text.lines() {
        if line.trim().is_empty() { continue; }
        let indent = line.len()-line.trim_start().len();
        let (key,value) = line.trim().split_once(':').ok_or_else(||Failure::input("invalid locked HTTP contract mapping"))?;
        while stack.last().is_some_and(|(level,_)| *level>=indent) { stack.pop(); }
        let value = value.trim();
        if value.is_empty() { stack.push((indent,key)); continue; }
        let path = stack.iter().map(|(_,key)|*key).chain(std::iter::once(key)).collect::<Vec<_>>().join(".");
        if values.insert(path,value.trim_matches('\'').to_owned()).is_some() { return Err(Failure::input("duplicate locked HTTP contract key")); }
    }
    Ok(values)
}
fn rule<'a>(rules: &'a BTreeMap<String,String>, name: &str) -> Result<&'a str, Failure> {
    rules.get(name).map(String::as_str).ok_or_else(||Failure::input("missing locked HTTP contract expectation"))
}
fn number(rules: &BTreeMap<String,String>, name: &str) -> Result<u64, Failure> {
    rule(rules,name)?.parse().map_err(|_|Failure::input("invalid numeric HTTP contract expectation"))
}

struct Server { child: Child, directory: PathBuf, finished: bool, started: Instant }
impl Server {
    fn alive(&mut self) -> Result<(), Failure> {
        if self.started.elapsed()>Duration::from_secs(60){return Err(failure("HTTP server wall-time limit exceeded"))}
        if self.child.try_wait().map_err(Failure::io)?.is_some() { return Err(failure("HTTP application exited before contract completed")); }
        for name in ["stdout.log","stderr.log"] {
            if fs::metadata(self.directory.join(name)).map_err(Failure::io)?.len()>=LOG_LIMIT {
                return Err(failure("HTTP application output limit exceeded"));
            }
        }
        Ok(())
    }
    fn stop(&mut self) -> Result<Value,Failure> {
        #[cfg(unix)] { unsafe extern "C" { fn kill(pid:i32,signal:i32)->i32; }
            unsafe { kill(-(self.child.id() as i32),9); }
        }
        let _ = self.child.kill();
        let status = self.child.wait().map_err(Failure::io)?;
        self.finished=true;
        #[cfg(unix)] let signal = { use std::os::unix::process::ExitStatusExt; status.signal() };
        #[cfg(not(unix))] let signal: Option<i32> = None;
        Ok(json!({"exit_code":status.code(),"signal":signal,"termination":"owned_server_cleanup",
            "stdout":artifact(&self.directory.join("stdout.log"))?,"stderr":artifact(&self.directory.join("stderr.log"))?}))
    }
}
impl Drop for Server { fn drop(&mut self) { if !self.finished { let _=self.stop(); } } }

#[cfg(target_os="linux")]
fn start(executable:&Path, policy:&Path, directory:&Path, endpoint:SocketAddr) -> Result<Server,Failure> {
    use std::os::{fd::{AsRawFd,FromRawFd,OwnedFd},unix::process::CommandExt};
    // Match the application's SO_REUSEADDR rule without connecting to an
    // unrelated service. Keep this probe alive until immediately before spawn.
    unsafe extern "C" { fn socket(domain:i32,kind:i32,protocol:i32)->i32; fn setsockopt(fd:i32,level:i32,name:i32,value:*const i32,len:u32)->i32; fn bind(fd:i32,address:*const u8,len:u32)->i32; }
    #[repr(C)] struct Address { family:u16, port:u16, address:[u8;4], zero:[u8;8] }
    let SocketAddr::V4(endpoint)=endpoint else{return Err(Failure::input("locked HTTP endpoint must be IPv4"))};
    let raw=unsafe{socket(2,1|0x80000,6)};
    if raw<0{return Err(Failure::io(std::io::Error::last_os_error()))}
    let probe=unsafe{OwnedFd::from_raw_fd(raw)};let one=1;
    if unsafe{setsockopt(raw,1,2,&one,4)}<0{return Err(Failure::io(std::io::Error::last_os_error()))}
    let address=Address{family:2,port:endpoint.port().to_be(),address:endpoint.ip().octets(),zero:[0;8]};
    if unsafe{bind(raw,(&address as *const Address).cast(),16)}<0{return Err(Failure::new("E_TOOLCHAIN_FAILURE","locked HTTP endpoint unavailable; no unrelated process was changed"))}
    let policy=File::open(policy).map_err(Failure::io)?;let policy_fd=policy.as_raw_fd();
    let out=OpenOptions::new().write(true).create_new(true).open(directory.join("stdout.log")).map_err(Failure::io)?;
    let err=OpenOptions::new().write(true).create_new(true).open(directory.join("stderr.log")).map_err(Failure::io)?;
    let mut command=Command::new(executable);
    command.env_clear().current_dir(directory).stdin(Stdio::null()).stdout(out).stderr(err).process_group(0);
    il_object_emitter::process::kill_on_parent_death(&mut command);
    unsafe { command.pre_exec(move || {
        unsafe extern "C" {fn dup2(old:i32,new:i32)->i32;fn fcntl(fd:i32,cmd:i32,...)->i32;fn close(fd:i32)->i32;fn setrlimit(resource:i32,limit:*const[u64;2])->i32;}
        let saved=fcntl(policy_fd,1030,5);if saved<0{return Err(std::io::Error::last_os_error())}
        if dup2(saved,4)<0||fcntl(4,2,0)<0{close(saved);return Err(std::io::Error::last_os_error())}close(saved);
        for (resource,limit) in [(0,30),(1,LOG_LIMIT),(9,536_870_912)] {
            if setrlimit(resource,&[limit,limit])<0{return Err(std::io::Error::last_os_error())}
        }
        Ok(())
    }); }
    drop(probe);
    let child=command.spawn().map_err(Failure::io)?;
    Ok(Server{child,directory:directory.to_owned(),finished:false,started:Instant::now()})
}
#[cfg(not(target_os="linux"))]
fn start(_: &Path,_: &Path,_: &Path,_: SocketAddr) -> Result<Server,Failure> {
    Err(Failure::new("E_UNSUPPORTED_TARGET","HTTP native blackbox requires Linux execution"))
}

fn request(method:&str,target:&str)->Vec<u8>{format!("{method} {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").into_bytes()}
fn exchange(endpoint:SocketAddr, fragments:&[Vec<u8>], server:&mut Server)->Result<(Vec<u8>,u128),Failure>{
    server.alive()?;
    let started=Instant::now();let deadline=started+Duration::from_secs(8);
    let mut stream=TcpStream::connect_timeout(&endpoint,Duration::from_secs(1)).map_err(Failure::io)?;
    for fragment in fragments {
        let remaining=deadline.checked_duration_since(Instant::now()).ok_or_else(||failure("HTTP client absolute timeout"))?;
        stream.set_write_timeout(Some(remaining)).map_err(Failure::io)?;
        stream.write_all(fragment).map_err(Failure::io)?;
        if fragments.len()>1 {std::thread::sleep(Duration::from_millis(2));}
    }
    let mut wire=vec![];let mut buffer=[0;4096];
    loop {
        server.alive()?;
        let remaining=deadline.checked_duration_since(Instant::now()).ok_or_else(||failure("HTTP client absolute timeout"))?;
        stream.set_read_timeout(Some(remaining)).map_err(Failure::io)?;
        match stream.read(&mut buffer) {
            Ok(0)=>break,
            Ok(count)=>{if wire.len()+count>WIRE_LIMIT{return Err(failure("HTTP response wire limit exceeded"))}wire.extend_from_slice(&buffer[..count]);}
            Err(error) if error.kind()==std::io::ErrorKind::ConnectionReset&&!wire.is_empty()=>break,
            Err(error)=>return Err(Failure::io(error)),
        }
    }
    Ok((wire,started.elapsed().as_nanos()))
}
fn parse(wire:&[u8])->Result<(u16,BTreeMap<String,String>,Vec<u8>),Failure>{
    let boundary=wire.windows(4).position(|part|part==b"\r\n\r\n").ok_or_else(||failure("HTTP response lacks CRLF header terminator"))?;
    let head=std::str::from_utf8(&wire[..boundary]).map_err(|_|failure("HTTP response header is not UTF-8"))?;
    let mut lines=head.split("\r\n");let status=lines.next().unwrap().splitn(3,' ').collect::<Vec<_>>();
    if status.len()!=3||status[0]!="HTTP/1.1"{return Err(failure("invalid HTTP response status line"))}
    let code=status[1].parse().map_err(|_|failure("invalid HTTP response status"))?;let mut headers=BTreeMap::new();
    for line in lines {let(name,value)=line.split_once(':').ok_or_else(||failure("invalid HTTP response header"))?;
        if headers.insert(name.to_ascii_lowercase(),value.trim().to_owned()).is_some(){return Err(failure("duplicate HTTP response header"))}
    }
    let body=wire[boundary+4..].to_vec();
    if headers.get("connection").map(String::as_str)!=Some("close")||headers.get("content-length").and_then(|n|n.parse::<usize>().ok())!=Some(body.len()){
        return Err(failure("HTTP response violates Connection/Content-Length contract"));
    }
    Ok((code,headers,body))
}

fn profile(executable:&Path,policy:&Path,directory:&Path,rules:&BTreeMap<String,String>,rows:&mut Vec<Value>)->Result<Value,Failure>{
    let endpoint:SocketAddr=rule(rules,"bind")?.parse().map_err(|_|Failure::input("invalid locked HTTP endpoint"))?;
    let mut server=start(executable,policy,directory,endpoint)?;
    let begin=Instant::now();
    loop {
        server.alive()?;
        if let Ok(stream)=TcpStream::connect_timeout(&endpoint,Duration::from_millis(50)){drop(stream);break;}
        if begin.elapsed()>Duration::from_secs(5){return Err(failure("HTTP application did not listen before startup deadline"))}
        std::thread::sleep(Duration::from_millis(10));
    }
    let health=number(rules,"responses.health.status")? as u16;
    let hello=number(rules,"responses.hello.status")? as u16;
    let mut cases=vec![("health",request("GET","/health"),health),("hello",request("GET","/hello/Larry"),hello),
        ("unknown",request("GET","/unknown"),number(rules,"responses.errors.unknown_path")? as u16),
        ("method",request("POST","/health"),number(rules,"responses.errors.wrong_method")? as u16),
        ("invalid_percent",request("GET","/hello/%GG"),number(rules,"responses.errors.invalid_percent_encoding")? as u16),
        ("invalid_utf8",request("GET","/hello/%FF"),number(rules,"responses.errors.invalid_utf8")? as u16),
        ("encoded_slash",request("GET","/hello/%2F"),number(rules,"responses.errors.invalid_percent_encoding")? as u16),
        ("fragmented",request("GET","/hello/Larry"),hello),
        ("read_timeout",b"GET /health HTTP/1.1\r\nHost:".to_vec(),number(rules,"responses.errors.read_timeout")? as u16)];
    cases.push(("after_disconnect",request("GET","/health"),health));
    let outcome: Result<(),Failure>=(||{
        for (name,raw,expected) in cases {
            let mut disconnect_log_offset=None;
            if name=="after_disconnect" {let mut stream=TcpStream::connect_timeout(&endpoint,Duration::from_secs(1)).map_err(Failure::io)?;
                disconnect_log_offset=Some(fs::metadata(directory.join("stderr.log")).map_err(Failure::io)?.len() as usize);
                stream.write_all(b"GET /health").map_err(Failure::io)?;stream.shutdown(Shutdown::Both).map_err(Failure::io)?;}
            let fragments=if name=="fragmented"{raw.chunks(1).map(|part|part.to_vec()).collect()}else{vec![raw.clone()]};
            let (wire,elapsed)=match exchange(endpoint,&fragments,&mut server){
                Ok(value)=>value,
                Err(error)=>{rows.push(json!({"id":name,"passed":false,"request":raw,"wire":null,
                    "expected_status":expected,"failure":error.message}));
                    save(&directory.join(format!("case-{name}.json")),rows.last().unwrap())?;return Err(error);}
            };
            let parsed=parse(&wire);
            let check=(||{
                let(status,headers,body)=parsed?;
                if status!=expected{return Err(failure(format!("{name}: expected status {expected}, received {status}")))}
                if expected>=400&&!body.is_empty(){return Err(failure("HTTP error response body must be empty"))}
                if name=="health"||name=="after_disconnect"{
                    if body!=rule(rules,"responses.health.body")?.as_bytes()||headers.get("content-type").map(String::as_str)!=Some(rule(rules,"responses.health.content_type")?){return Err(failure("health body or content type differs from locked contract"))}
                }
                if name=="hello"||name=="fragmented"{
                    let body:Value=serde_json::from_slice(&body).map_err(Failure::json)?;
                    let inline=rule(rules,"responses.hello.body_json")?;
                    let message=inline.strip_prefix("{message: '").and_then(|s|s.strip_suffix("'}")).ok_or_else(||failure("unsupported locked hello body expectation"))?;
                    if body!=json!({"message":message})||headers.get("content-type").map(String::as_str)!=Some(rule(rules,"responses.hello.content_type")?){return Err(failure("hello JSON or content type differs from locked contract"))}
                }
                if name=="method"{let(name,value)=rule(rules,"responses.method_not_allowed_header")?.split_once(':').ok_or_else(||failure("invalid locked method header"))?;
                    if headers.get(&name.to_ascii_lowercase()).map(String::as_str)!=Some(value.trim()){return Err(failure("405 response omitted locked Allow header"))}
                }
                if name=="read_timeout"{let ms=elapsed/1_000_000;let expected=number(rules,"limits.read_deadline_ms")? as u128;
                    if ms+500<expected||ms>expected+2500{return Err(failure("read timeout differs from locked absolute deadline"))}
                }
                if let Some(offset)=disconnect_log_offset{
                    let log=fs::read(directory.join("stderr.log")).map_err(Failure::io)?;
                    let recent=log.get(offset..).ok_or_else(||failure("HTTP diagnostic log was truncated"))?;
                    let mut diagnosed=false;
                    for line in recent.split(|byte|*byte==b'\n').filter(|line|!line.is_empty()){
                        let event:Value=serde_json::from_slice(line).map_err(|_|failure("HTTP disconnect diagnostic is not structured JSON"))?;
                        if matches!(event["code"].as_str(),Some("E_HTTP_REQUEST_FAILED"|"E_HTTP_PEER_DISCONNECT")){diagnosed=true;}
                    }
                    if !diagnosed{return Err(failure("HTTP disconnect produced no structured diagnostic"))}
                }
                Ok(())
            })();
            rows.push(json!({"id":name,"passed":check.is_ok(),"request":raw,"wire":wire,"wire_sha256":hash_bytes(&wire),"elapsed_ns":elapsed.to_string(),"expected_status":expected,
                "failure":check.as_ref().err().map(|e|e.message.clone())}));
            save(&directory.join(format!("case-{name}.json")),rows.last().unwrap())?;
            check?;
        }
        Ok(())
    })();
    let process=server.stop()?;
    save(&directory.join("process.json"),&process)?;
    outcome?;
    Ok(process)
}

pub fn run(graph:&Graph,contract:&str,store:&Path,context:&Context,policy:&HostPolicy)->Result<Value,Failure>{
    if contract!="http_mvp"{return Err(Failure::input("unknown blackbox contract"))}
    if policy.test_faults.is_some(){return Err(Failure::input("blackbox application does not accept captured fault injection"))}
    let rules=rules(context)?;
    let root=store.join(".il/blackbox/candidates");fs::create_dir_all(&root).map_err(Failure::io)?;
    let directory=tempfile::Builder::new().prefix("http-").tempdir_in(&root).map_err(Failure::io)?.keep().canonicalize().map_err(Failure::io)?;
    {let mut locked=OpenOptions::new().write(true).create_new(true).open(directory.join("http.yaml")).map_err(Failure::io)?;
        locked.write_all(CONTRACT).and_then(|_|locked.sync_all()).map_err(Failure::io)?;}
    save(&directory.join("input.json"),&json!({"revision":graph.revision,"graph_hash":graph.hash().map_err(Failure::json)?,"contract":contract,"contract_hash":hash_bytes(CONTRACT),"policy":policy}))?;
    let mut profiles=vec![];let mut diagnostics=vec![];let mut count=0;
    for optimization in [Profile::Debug,Profile::Release]{
        let label=if optimization==Profile::Debug{"debug"}else{"release"};
        let profile_dir=directory.join(label);fs::create_dir(&profile_dir).map_err(Failure::io)?;
        let built=native::build(graph,optimization,RuntimeProfile::Full,None,store,context,policy)?;
        let executable=Path::new(built["native"]["artifacts"]["executable"]["path"].as_str().ok_or_else(||failure("native build has no executable"))?);
        let policy_path=profile_dir.join("policy.json");save(&policy_path,&serde_json::to_value(policy).map_err(Failure::json)?)?;
        let mut rows=vec![];
        let outcome=profile(executable,&policy_path,&profile_dir,&rules,&mut rows);
        count+=rows.len();
        if let Err(error)=&outcome{let mut diagnostic=Diagnostic::error(&error.code,None,&error.message,graph.revision);diagnostic.stage="blackbox".into();diagnostic.retryable=error.code=="E_TOOLCHAIN_FAILURE";diagnostics.push(diagnostic);}
        let process=if profile_dir.join("process.json").exists(){
            serde_json::from_slice::<Value>(&fs::read(profile_dir.join("process.json")).map_err(Failure::io)?).map_err(Failure::json)?
        }else{Value::Null};
        let process_record=if profile_dir.join("process.json").exists(){artifact(&profile_dir.join("process.json"))?}else{Value::Null};
        profiles.push(json!({"profile":label,"passed":outcome.is_ok(),"build":built,"cases":rows,"process":process,"process_record":process_record}));
    }
    let result=json!({"revision":graph.revision,"graph_hash":graph.hash().map_err(Failure::json)?,"contract":contract,"contract_hash":hash_bytes(CONTRACT),
        "passed":diagnostics.is_empty(),"count":count,"profiles":profiles,"diagnostics":diagnostics,
        "tests":[{"suite":"http_mvp_wire_subset","passed":diagnostics.is_empty(),"count":count,"contract_hash":hash_bytes(CONTRACT)}],
        "scope":"http_mvp_wire_subset","known_limits":["Partial-write and backpressure fault injection remain in the independent P07 suite."]});
    save(&directory.join("report.json"),&result)?;
    let mut response=result;response["report"]=artifact(&directory.join("report.json"))?;
    response["contract_artifact"]=artifact(&directory.join("http.yaml"))?;
    Ok(response)
}

#[cfg(all(test,target_os="linux"))]
use il_object_emitter::process::bounded_command;

#[cfg(all(test,target_os="linux"))]
#[path="../../../compiler/object_emitter/src/process_test_support.rs"]
mod process_test_support;

#[cfg(all(test,target_os="linux"))]
mod tests{
    use super::*;
    #[test]
    fn blackbox_child_dies_and_is_reaped_after_parent_sigkill(){
        if let Some(directory)=std::env::var_os("IL_PARENT_DEATH_WORKER"){
            let directory=PathBuf::from(directory);
            // Port zero only probes availability; this payload never listens.
            let server=start(&directory.join("payload"),&directory.join("policy.json"),&directory,"127.0.0.1:0".parse().unwrap())
                .unwrap_or_else(|error|panic!("{}",error.message));
            fs::write(directory.join("child.pid"),server.child.id().to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(30));
            panic!("worker survived supervisor SIGKILL");
        }
        process_test_support::assert_parent_death("blackbox::tests::blackbox_child_dies_and_is_reaped_after_parent_sigkill",|directory|{
            fs::write(directory.join("policy.json"),b"{}").unwrap();
            fs::write(directory.join("payload.ll"),b"declare i32 @pause()\ndefine i32 @main() {\nentry:\n br label %wait\nwait:\n %r = call i32 @pause()\n br label %wait\n}\n").unwrap();
            let mut command=Command::new("/usr/bin/clang-14");
            command.current_dir(directory).args(["payload.ll","-o","payload"]);
            let (code,_,stderr)=il_object_emitter::process::bounded_command(command,Duration::from_secs(15)).map_err(|e|e.message).unwrap();
            assert_eq!(code,Some(0),"{}",String::from_utf8_lossy(&stderr));
        });
    }
}
