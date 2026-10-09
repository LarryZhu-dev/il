"""Real process crash acceptance for atomic graph publication.

The barrier is an inherited, private fd used only by the feature-enabled test
binary.  The production CLI has no barrier option and cannot activate it.
"""
from __future__ import annotations
import argparse, json, os, signal, socket, subprocess, tempfile, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True) + "\n", encoding="utf-8")

def fixture(root):
    repo, store = root / "source", root / "application"
    repo.mkdir(parents=True); store.mkdir(parents=True)
    subprocess.run(["git", "-C", str(repo), "init", "--initial-branch=main"], check=True, stdout=subprocess.DEVNULL)
    state = {"project_id":"il","schema_version":"1.0.0","head_revision":0,"last_verified_revision":0,
             "compiler_version":None,"runtime_version":None,"target":"x86_64-unknown-linux-gnu",
             "open_tasks":["P09"],"blocked_tasks":[],"failed_tasks":[],"locks":{}}
    graph = {"project_id":"il","graph_version":"1.0.0","revision":0,"target":"x86_64-unknown-linux-gnu",
             "modules":[],"types":[],"functions":[],"capabilities":[],"packages":[],"contracts":[]}
    write(repo/"repository_state.json", state); write(repo/"examples/bootstrap/graph.json", graph)
    write(repo/"toolchain.lock", {"target":"x86_64-unknown-linux-gnu","rust":"1.90.0","llvm":"14.0.6"})
    subprocess.run(["git","-C",str(repo),"add","."],check=True,stdout=subprocess.DEVNULL)
    subprocess.run(["git","-C",str(repo),"-c","user.name=il test","-c","user.email=test@invalid.local","commit","-m","test: fixture"],check=True,stdout=subprocess.DEVNULL)
    commit=subprocess.check_output(["git","-C",str(repo),"rev-parse","HEAD"],text=True).strip()
    tree=subprocess.check_output(["git","-C",str(repo),"rev-parse","HEAD^{tree}"],text=True).strip()
    write(repo/".git/il/revision_bindings.json", {"schema_version":"1.0.0","bindings":[{"revision":0,"git_commit":commit,"tree_hash":tree}]})
    return repo, store

def request(binary, repo, store, payload, stage):
    parent, child = socket.socketpair(); child_fd=child.fileno()
    os.dup2(child_fd, 198)
    proc = subprocess.Popen([str(binary),"--repository",str(repo),"--store",str(store),"transact"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, pass_fds=(198,))
    proc.stdin.write(json.dumps(payload).encode()); proc.stdin.close(); proc.stdin = None
    child.close(); os.close(198)
    parent.sendall(("transaction_before_head_rename\n" if stage == "transaction_after_head_rename" else stage+"\n").encode()); event = b""
    try:
        while not event.endswith(b"\n"):
            chunk = parent.recv(256)
            if not chunk: break
            event += chunk
    except ConnectionResetError:
        pass
    if not event:
        stdout, stderr = proc.communicate(timeout=10)
        raise RuntimeError(f"barrier not reached; exit={proc.returncode} stdout={stdout!r} stderr={stderr!r}")
    if not event: raise AssertionError("child did not reach private publication barrier")
    if stage == "transaction_after_head_rename":
        parent.sendall(b"release\ntransaction_after_head_rename\n")
        event = b""
        while not event.endswith(b"\n"):
            chunk = parent.recv(256)
            if not chunk: break
            event += chunk
        if not event: raise AssertionError("child did not reach post-rename barrier")
    os.kill(proc.pid, signal.SIGKILL); proc.wait(timeout=10)
    parent.close()
    return json.loads(event)

def main(binary):
    with tempfile.TemporaryDirectory(prefix="il-p09-transaction-") as name:
        tx={"task_id":"P09-crash","base_revision":0,"scope":["app"],"operations":[{"op":"add_module","module":{"entity_id":"app","path":"app","imports":[],"declarations":[],"visibility":"private"}}],"required_checks":["schema","names"]}
        repo, store = fixture(Path(name)/"before")
        before=request(binary,repo,store,tx,"transaction_before_head_rename")
        assert before["stage"] == "transaction_before_head_rename"
        assert not (store/".il"/"HEAD").exists()
        repo, store = fixture(Path(name)/"after")
        after=request(binary,repo,store,tx,"transaction_after_head_rename")
        assert after["stage"] == "transaction_after_head_rename"
        state=subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"state"],input="{}",text=True,capture_output=True,check=True)
        observed=json.loads(state.stdout)["result_revision"]
        assert observed in (0, 1), "reopened store must retain a complete prior or newly published HEAD"
        # A fresh process retry must allocate a revision greater than every snapshot.
        tx2=dict(tx, base_revision=observed, task_id="P09-crash-retry", scope=["other"], operations=[{"op":"add_module","module":{"entity_id":"other","path":"other","imports":[],"declarations":[],"visibility":"private"}}])
        retry=subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"transact"],input=json.dumps(tx2),text=True,capture_output=True,check=True)
        result=json.loads(retry.stdout); assert result["ok"] and result["result_revision"] > observed
        print(json.dumps({"schema_version":"1.0.0","suite":"transaction_process","status":"PASSED","events":[before,after],"revision":result["result_revision"]}))

if __name__ == "__main__":
    parser=argparse.ArgumentParser(); parser.add_argument("--binary",type=Path,required=True); main(parser.parse_args().binary)
