//! Isolated supervisor: orphan adoption must not change the parallel test host.
use std::{fs,path::Path,process::Command,time::{Duration,SystemTime,UNIX_EPOCH}};

pub fn assert_parent_death(test_name:&str,prepare:impl FnOnce(&Path)){
    let directory=std::env::temp_dir().join(format!("il-parent-death-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir(&directory).unwrap();
    struct Directory(std::path::PathBuf);
    impl Drop for Directory{fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
    let _cleanup=Directory(directory.clone());
    prepare(&directory);
    let script=r#"
import ctypes,errno,os,pathlib,signal,subprocess,sys,time
libc=ctypes.CDLL(None,use_errno=True)
assert libc.prctl(36,1,0,0,0)==0, ctypes.get_errno()
root=pathlib.Path(sys.argv[3])
env=dict(os.environ,IL_PARENT_DEATH_WORKER=str(root))
worker=subprocess.Popen([sys.argv[1],'--exact',sys.argv[2],'--nocapture'],env=env)
child=None
reaped=False
try:
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        marker=root/'child.pid'
        if marker.exists():
            text=marker.read_text()
            if text:
                child=int(text)
                break
        assert worker.poll() is None, 'worker exited before publishing owned child'
        time.sleep(.01)
    assert child is not None, 'child startup timed out'
    assert os.getpgid(child)==child, 'owned child has no independent process group'
    assert os.getpgid(worker.pid)!=child, 'child incorrectly shares worker group'
    assert pathlib.Path('/proc',str(child)).exists(), 'child was not running'
    worker.kill()
    assert worker.wait(timeout=5)==-signal.SIGKILL
    deadline=time.monotonic()+5
    while time.monotonic()<deadline:
        pid,status=os.waitpid(child,os.WNOHANG)
        if pid:
            reaped=True
            assert os.WIFSIGNALED(status) and os.WTERMSIG(status)==signal.SIGKILL, status
            break
        time.sleep(.01)
    assert reaped, 'owned child survived parent SIGKILL'
    assert not pathlib.Path('/proc',str(child)).exists(), 'owned child was not reaped'
    print('PASS owned child died by SIGKILL and was reaped',flush=True)
finally:
    if worker.poll() is None:
        worker.kill()
    worker.wait(timeout=5)
    # Reap every adopted process, including a child created just before a
    # failing worker managed to publish its PID.
    children=pathlib.Path('/proc/self/task',str(os.getpid()),'children')
    for pid in [int(x) for x in children.read_text().split()]:
        try: os.kill(pid,signal.SIGKILL)
        except ProcessLookupError: pass
        try: os.waitpid(pid,0)
        except ChildProcessError: pass
"#;
    let mut command=Command::new("/usr/bin/python3");
    command.args(["-c",script]).arg(std::env::current_exe().unwrap()).arg(test_name).arg(&directory);
    let output=il_bounded_command(command,Duration::from_secs(25));
    let (code,stdout,stderr)=output.unwrap_or_else(|message|panic!("{message}"));
    assert_eq!(code,Some(0),"supervisor failed: stdout={} stderr={}",String::from_utf8_lossy(&stdout),String::from_utf8_lossy(&stderr));
    assert!(String::from_utf8_lossy(&stdout).contains("PASS owned child died by SIGKILL and was reaped"));
}

fn il_bounded_command(command:Command,timeout:Duration)->Result<(Option<i32>,Vec<u8>,Vec<u8>),String>{
    // The including module supplies the same production process boundary in
    // both crates, without exporting test-only hooks in the library API.
    super::bounded_command(command,timeout).map_err(|error|error.message)
}
