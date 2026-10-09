use std::{io::Read,process::{Command,Stdio},sync::{Arc,atomic::{AtomicBool,Ordering}},time::{Duration,Instant}};
pub const OUTPUT_LIMIT:u64=8*1024*1024;
pub struct ProcessFailure{pub message:String,pub exit_code:Option<i32>,pub stdout:Vec<u8>,pub stderr:Vec<u8>}
/// Keep an owned command from surviving a killed worker, even in its own group.
/// The parent check closes the fork-to-prctl race; Linux does not deliver the
/// configured signal retroactively when the parent has already died.
pub fn kill_on_parent_death(command:&mut Command){
    #[cfg(target_os="linux")]{
        use std::os::unix::process::CommandExt;
        let parent=std::process::id() as i32;
        unsafe{command.pre_exec(move||{
            unsafe extern "C"{fn prctl(option:i32,...)->i32;fn getppid()->i32;}
            if prctl(1,9usize,0usize,0usize,0usize)<0{return Err(std::io::Error::last_os_error())}
            if getppid()!=parent{return Err(std::io::Error::from_raw_os_error(3))}
            Ok(())
        });}
    }
    #[cfg(not(target_os="linux"))]let _=command;
}
pub fn bounded_command(mut command:Command,timeout:Duration)->Result<(Option<i32>,Vec<u8>,Vec<u8>),ProcessFailure>{
    bounded_command_for_stage(command, timeout, None)
}
pub fn bounded_command_for_stage(mut command:Command,timeout:Duration,stage:Option<&str>)->Result<(Option<i32>,Vec<u8>,Vec<u8>),ProcessFailure>{
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]{use std::os::unix::process::CommandExt;command.process_group(0);}
    kill_on_parent_death(&mut command);
    let process_directory=command.get_current_dir().map(|path| path.to_path_buf());
    let mut child=command.spawn().map_err(|e|ProcessFailure{message:e.to_string(),exit_code:None,stdout:vec![],stderr:vec![]})?;
    #[cfg(all(feature = "process-test-barriers", target_os = "linux"))]
    if let Some(stage) = stage { process_barrier(stage, child.id(), process_directory); }
    let out=child.stdout.take().unwrap();let err=child.stderr.take().unwrap();
    let capped=Arc::new(AtomicBool::new(false));
    let read=|mut stream:Box<dyn Read+Send>,capped:Arc<AtomicBool>|{let mut bytes=vec![];let result=stream.by_ref().take(OUTPUT_LIMIT+1).read_to_end(&mut bytes);if bytes.len() as u64>OUTPUT_LIMIT{capped.store(true,Ordering::Release);} (result,bytes)};
    let out_cap=capped.clone();let err_cap=capped.clone();
    let stdout=std::thread::spawn(move||read(Box::new(out),out_cap));let stderr=std::thread::spawn(move||read(Box::new(err),err_cap));
    let start=Instant::now();let mut failed=None;
    let status=loop{
        match child.try_wait(){Ok(Some(s))=>break Some(s),Ok(None)=>{},Err(e)=>{failed=Some(e.to_string());terminate(&mut child);break child.wait().ok();}}
        if start.elapsed()>=timeout {failed=Some("native process timeout".into());terminate(&mut child);break child.wait().ok();}
        if capped.load(Ordering::Acquire){failed=Some("native output limit exceeded".into());terminate(&mut child);break child.wait().ok();}
        std::thread::sleep(Duration::from_millis(5));
    };
    // No descendant may keep inherited output pipes alive after the command leader exits.
    terminate(&mut child);
    let (out_read,mut out)=stdout.join().unwrap_or_else(|_|(Err(std::io::Error::other("stdout reader panicked")),vec![]));
    let (err_read,mut err)=stderr.join().unwrap_or_else(|_|(Err(std::io::Error::other("stderr reader panicked")),vec![]));
    let code=status.and_then(|s|s.code());
    if out.len() as u64>OUTPUT_LIMIT||err.len() as u64>OUTPUT_LIMIT{out.truncate(OUTPUT_LIMIT as usize);err.truncate(OUTPUT_LIMIT as usize);failed=Some("native output limit exceeded".into())}
    if out_read.is_err()||err_read.is_err(){failed=Some("cannot read native output".into())}
    if let Some(message)=failed{Err(ProcessFailure{message,exit_code:code,stdout:out,stderr:err})}else{Ok((code,out,err))}
}

#[cfg(all(feature = "process-test-barriers", target_os = "linux"))]
fn process_barrier(stage: &str, child: u32, directory: Option<std::path::PathBuf>) {
    use std::io::{BufRead, BufReader, Write};
    use std::os::fd::FromRawFd;
    const FD: i32 = 198;
    if unsafe { libc::fcntl(FD, libc::F_GETFD) } < 0 { return; }
    let stream = unsafe { std::fs::File::from_raw_fd(FD) };
    let Ok(read_stream) = stream.try_clone() else { return; };
    let mut reader = BufReader::new(read_stream);
    let mut phase = String::new();
    if reader.read_line(&mut phase).is_err() || phase.trim_end() != stage { return; }
    let mut stream = stream;
    let event = format!("{{\"stage\":\"{stage}\",\"parent_pid\":{},\"worker_pid\":{child}}}\n", std::process::id());
    let marker = directory.unwrap_or_else(|| std::env::current_dir().unwrap_or_default()).join("process-interrupted.json");
    let _ = std::fs::write(marker, format!("{{\"schema_version\":\"1.0.0\",\"status\":\"INTERRUPTED\",\"diagnostics\":[{{\"code\":\"E_PROCESS_INTERRUPTED\",\"stage\":\"{stage}\",\"worker_pid\":{child}}}]}}\n"));
    if stream.write_all(event.as_bytes()).is_ok() {
        let mut release = String::new();
        let _ = reader.read_line(&mut release);
    }
    std::mem::forget(stream);
}

fn terminate(child:&mut std::process::Child){
    #[cfg(unix)]{unsafe extern "C"{fn kill(pid:i32,signal:i32)->i32;}unsafe{kill(-(child.id() as i32),9);}}
    let _=child.kill();
}

#[cfg(all(test,target_os="linux"))]
#[path="process_test_support.rs"]
mod process_test_support;

#[cfg(all(test,target_os="linux"))]
mod tests{
    use super::*;
    #[test]
    fn bounded_child_dies_and_is_reaped_after_parent_sigkill(){
        if let Some(directory)=std::env::var_os("IL_PARENT_DEATH_WORKER"){
            let mut command=Command::new("/usr/bin/python3");
            command.current_dir(directory).args(["-c","import os,time;open('child.pid','w').write(str(os.getpid()));time.sleep(30)"]);
            let _=bounded_command(command,Duration::from_secs(30));
            panic!("worker returned before supervisor killed it");
        }
        process_test_support::assert_parent_death("process::tests::bounded_child_dies_and_is_reaped_after_parent_sigkill",|_|{});
    }
}
