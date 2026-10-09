"""P09 real linker-worker interruption and candidate recovery acceptance."""
from __future__ import annotations
import argparse, hashlib, json, os, signal, socket, subprocess, tempfile
from pathlib import Path

def digest(data: bytes) -> str: return "sha256:" + hashlib.sha256(data).hexdigest()
def file_digest(path: Path) -> str: return digest(path.read_bytes())
def canonical(value) -> bytes: return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()

SOURCE='''@id("app") module app { @id("app.main") fn main()->I32 effects [process] { runtime.print_i64(7); return 0; } }'''

def wait_line(parent):
    parent.settimeout(20); data=bytearray()
    while not data.endswith(b"\n"):
        chunk=parent.recv(512)
        if not chunk: break
        data.extend(chunk)
    if not data: raise AssertionError("link worker barrier was not reached")
    return json.loads(bytes(data))

def main(binary: Path) -> int:
    if os.name != "posix": raise SystemExit("build process acceptance requires Linux")
    with tempfile.TemporaryDirectory(prefix="il-p09-build-") as name:
        root=Path(name); repo=root/"source"; store=root/"application"; import sys
        sys.path.insert(0,str(Path(__file__).parent)); from transaction_process import fixture
        repo,store=fixture(root); tx={"task_id":"P09-build","base_revision":0,"scope":["program"],"operations":[{"op":"import_text","source":SOURCE}],"required_checks":["schema","names","types","ownership","effects","capabilities","contracts"]}
        created=subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"transact"],input=json.dumps(tx),text=True,capture_output=True,check=True); created_response=json.loads(created.stdout)
        if not created_response.get("ok"): raise AssertionError(f"build fixture transaction failed: {created_response}")
        request={"revision":1,"target":"x86_64-unknown-linux-gnu","profile":"release"}; old=subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"build"],input=json.dumps(request),text=True,capture_output=True)
        if old.returncode: raise RuntimeError(f"initial build failed: {old.stdout} {old.stderr}")
        old_response=json.loads(old.stdout); old_exe=Path(old_response["result"]["native"]["artifacts"]["executable"]["path"]); old_hash=file_digest(old_exe); old_input_hash=digest(canonical(request))
        parent,child=socket.socketpair(); os.dup2(child.fileno(),198); build=None
        try:
            build=subprocess.Popen([str(binary),"--repository",str(repo),"--store",str(store),"build"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,pass_fds=(198,)); build.stdin.write(canonical(request)); build.stdin.close(); child.close(); os.close(198); parent.sendall(b"link_runtime\n"); event=wait_line(parent); pid=build.pid; executable_hash=file_digest(binary); os.kill(pid,signal.SIGKILL); build.wait(timeout=20)
        finally:
            parent.close()
            try: child.close()
            except OSError: pass
        candidates=list((store/".il/builds/candidates").glob("native-*/"));
        if not candidates: raise AssertionError("interrupted build did not retain a candidate")
        failed=[]
        for candidate in candidates:
            for name in ("build-failure.json","process-interrupted.json"):
                path=candidate/name
                if path.is_file(): failed.append({"path":str(path),"sha256":file_digest(path),"schema_valid":bool(json.loads(path.read_text(encoding="utf-8")))})
        if not failed: raise AssertionError("interrupted build did not retain a structured failure report")
        if file_digest(old_exe) != old_hash: raise AssertionError("verified executable changed after interrupted build")
        old_run=subprocess.run([str(old_exe)],capture_output=True)
        if old_run.returncode != 0: raise AssertionError("previous verified executable no longer runs")
        retry=subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"build"],input=json.dumps(request),text=True,capture_output=True)
        if retry.returncode: raise AssertionError(f"clean retry failed: {retry.stdout} {retry.stderr}")
        retry_response=json.loads(retry.stdout); retry_exe=Path(retry_response["result"]["native"]["artifacts"]["executable"]["path"]); retry_hash=file_digest(retry_exe)
        if retry_exe == old_exe: raise AssertionError("retry did not create a distinct candidate executable")
        report={"schema_version":"1.0.0","suite":"build_process","status":"PASSED","worker":event,"signal":"SIGKILL","owned_pid":pid,"executable_hash":executable_hash,"input_hash":old_input_hash,"verified_executable":{"path":str(old_exe),"sha256":old_hash,"exit_code":old_run.returncode},"failure_reports":failed,"retry":{"path":str(retry_exe),"sha256":retry_hash,"verified":retry_response.get("result",{}).get("native",{}).get("verified",True)}}
        print(json.dumps(report,sort_keys=True)); return 0

if __name__ == "__main__":
    parser=argparse.ArgumentParser(); parser.add_argument("--binary",type=Path,required=True); raise SystemExit(main(parser.parse_args().binary.resolve()))
