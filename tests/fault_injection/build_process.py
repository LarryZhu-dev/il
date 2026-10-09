"""Kill the real pinned LLVM link worker and verify retained retry evidence."""
from __future__ import annotations
import argparse, json, os, signal, socket, subprocess, tempfile, time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
SOURCE='''@id("app") module app { @id("app.main") fn main()->I32 effects [process] { runtime.print_i64(7); return 0; } }'''
def main(binary):
    # This suite is Linux-only: it requires the fixed /usr/bin/clang-14 worker.
    if os.name != "posix": raise SystemExit("build process acceptance requires Linux")
    with tempfile.TemporaryDirectory(prefix="il-p09-build-") as name:
        root=Path(name); repo=root/"source"; store=root/"application"
        # Reuse the canonical P08 fixture builder and create a real application store.
        import sys
        sys.path.insert(0, str(Path(__file__).parent))
        from transaction_process import fixture
        repo,store=fixture(root)
        tx={"task_id":"P09-build","base_revision":0,"scope":["program"],"operations":[{"op":"import_text","source":SOURCE}],"required_checks":["schema","names","types","ownership","effects","capabilities","contracts"]}
        subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"transact"],input=json.dumps(tx),text=True,capture_output=True,check=True)
        request={"revision":1,"target":"x86_64-unknown-linux-gnu","profile":"release"}
        old=subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"build"],input=json.dumps(request),text=True,capture_output=True)
        if old.returncode: raise RuntimeError(f"initial build failed: {old.stdout} {old.stderr}")
        old_response=json.loads(old.stdout); old_exe=Path(old_response["result"]["native"]["artifacts"]["executable"]["path"]); old_hash=__import__('hashlib').sha256(old_exe.read_bytes()).hexdigest()
        # The feature binary blocks after spawning fixed clang; parent SIGKILL leaves build-failure.json.
        parent,child=socket.socketpair(); child_fd=child.fileno()
        os.dup2(child_fd,198)
        build=subprocess.Popen([str(binary),"--repository",str(repo),"--store",str(store),"build"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,pass_fds=(198,))
        build.stdin.write(json.dumps(request).encode()); build.stdin.close()
        child.close(); os.close(198); parent.sendall(b"link_runtime\n"); event=b""
        while not event.endswith(b"\n"):
            chunk=parent.recv(256)
            if not chunk: break
            event += chunk
        parent.close()
        assert event, "link worker barrier was not reached"
        info=json.loads(event); os.kill(build.pid,signal.SIGKILL); build.wait(timeout=20)
        candidates=list((store/".il/builds/candidates").glob("native-*/")); assert candidates
        assert any((p/"build-failure.json").exists() or (p/"process-interrupted.json").exists() for p in candidates)
        assert __import__('hashlib').sha256(old_exe.read_bytes()).hexdigest() == old_hash
        assert subprocess.run([str(old_exe)],capture_output=True).returncode == 0
        retry=subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"build"],input=json.dumps(request),text=True,capture_output=True)
        assert retry.returncode==0, retry.stderr
        print(json.dumps({"schema_version":"1.0.0","suite":"build_process","status":"PASSED","worker":info,"retry":"VERIFIED"}))
if __name__ == "__main__":
    p=argparse.ArgumentParser(); p.add_argument("--binary",type=Path,required=True); main(p.parse_args().binary)
