use std::{io::Read,process::{Command,Stdio},sync::{Arc,atomic::{AtomicBool,Ordering}},time::{Duration,Instant}};
pub const OUTPUT_LIMIT:u64=8*1024*1024;
pub struct ProcessFailure{pub message:String,pub exit_code:Option<i32>,pub stdout:Vec<u8>,pub stderr:Vec<u8>}
pub fn bounded_command(mut command:Command,timeout:Duration)->Result<(Option<i32>,Vec<u8>,Vec<u8>),ProcessFailure>{
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]{use std::os::unix::process::CommandExt;command.process_group(0);}
    let mut child=command.spawn().map_err(|e|ProcessFailure{message:e.to_string(),exit_code:None,stdout:vec![],stderr:vec![]})?;
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

fn terminate(child:&mut std::process::Child){
    #[cfg(unix)]{unsafe extern "C"{fn kill(pid:i32,signal:i32)->i32;}unsafe{kill(-(child.id() as i32),9);}}
    let _=child.kill();
}

