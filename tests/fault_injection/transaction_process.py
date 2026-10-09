"""P09 real-process graph publication recovery acceptance."""
from __future__ import annotations
import argparse, hashlib, json, os, signal, socket, subprocess, tempfile
from pathlib import Path

def sha256_bytes(data: bytes) -> str: return "sha256:" + hashlib.sha256(data).hexdigest()
def sha256_file(path: Path) -> str: return sha256_bytes(path.read_bytes())
def canonical(value) -> bytes: return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
def write(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True); path.write_text(json.dumps(value, sort_keys=True) + "\n", encoding="utf-8")

def fixture(root: Path):
    repo, store = root / "source", root / "application"; repo.mkdir(parents=True); store.mkdir(parents=True)
    subprocess.run(["git", "-C", str(repo), "init", "--initial-branch=main"], check=True, stdout=subprocess.DEVNULL)
    state = {"project_id":"il","schema_version":"1.0.0","head_revision":0,"last_verified_revision":0,"compiler_version":None,"runtime_version":None,"target":"x86_64-unknown-linux-gnu","open_tasks":["P09"],"blocked_tasks":[],"failed_tasks":[],"locks":{}}
    graph = {"project_id":"il","graph_version":"1.0.0","revision":0,"target":"x86_64-unknown-linux-gnu","modules":[],"types":[],"functions":[],"capabilities":[],"packages":[],"contracts":[]}
    write(repo/"repository_state.json", state); write(repo/"examples/bootstrap/graph.json", graph); write(repo/"toolchain.lock", {"target":"x86_64-unknown-linux-gnu","rust":"1.90.0","llvm":"14.0.6"})
    subprocess.run(["git","-C",str(repo),"add","."],check=True,stdout=subprocess.DEVNULL); subprocess.run(["git","-C",str(repo),"-c","user.name=il test","-c","user.email=test@invalid.local","commit","-m","test: fixture"],check=True,stdout=subprocess.DEVNULL)
    commit=subprocess.check_output(["git","-C",str(repo),"rev-parse","HEAD"],text=True).strip(); tree=subprocess.check_output(["git","-C",str(repo),"rev-parse","HEAD^{tree}"],text=True).strip(); write(repo/".git/il/revision_bindings.json", {"schema_version":"1.0.0","bindings":[{"revision":0,"git_commit":commit,"tree_hash":tree}]})
    return repo, store

def transaction_payload(task_id: str, entity: str):
    return {"task_id":task_id,"base_revision":1,"scope":[entity],"operations":[{"op":"add_module","module":{"entity_id":entity,"path":entity,"imports":[],"declarations":[],"visibility":"private"}}],"required_checks":["schema","names"]}

def initialize(binary: Path, repo: Path, store: Path) -> None:
    payload={"task_id":"P09-initialize","base_revision":0,"scope":["seed"],"operations":[{"op":"add_module","module":{"entity_id":"seed","path":"seed","imports":[],"declarations":[],"visibility":"private"}}],"required_checks":["schema","names"]}
    result=subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"transact"],input=json.dumps(payload),text=True,capture_output=True)
    if result.returncode or not json.loads(result.stdout).get("ok"): raise RuntimeError(f"store initialization failed: {result.stdout} {result.stderr}")

def wait_line(parent: socket.socket) -> dict:
    parent.settimeout(15); event=bytearray()
    while not event.endswith(b"\n"):
        chunk=parent.recv(512)
        if not chunk: break
        event.extend(chunk)
    if not event: raise AssertionError("child did not reach private publication barrier")
    value=json.loads(bytes(event));
    if not isinstance(value,dict) or not value.get("stage"): raise AssertionError(f"invalid barrier event: {value!r}")
    return value

def crash_case(binary: Path, repo: Path, store: Path, payload: dict, stage: str) -> dict:
    parent, child=socket.socketpair(); os.set_inheritable(child.fileno(),True); os.dup2(child.fileno(),198); os.set_inheritable(198,True); process=None
    try:
        process=subprocess.Popen([str(binary),"--repository",str(repo),"--store",str(store),"transact"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,close_fds=False)
        process.stdin.write(canonical(payload)); process.stdin.close(); process.stdin = None; child.close(); os.close(198)
        try:
            barriers = ["transaction_before_snapshot", "transaction_after_snapshot",
                        "transaction_before_head_rename", "transaction_after_head_rename"]
            event = None
            for current in barriers[:barriers.index(stage) + 1]:
                parent.sendall((current + "\n").encode())
                event = wait_line(parent)
                if current != stage:
                    parent.sendall(b"release\n")
        except AssertionError as error:
            process.wait(timeout=5)
            stdout = process.stdout.read() if process.stdout else b""
            stderr = process.stderr.read() if process.stderr else b""
            raise AssertionError(f"{error}: stdout={stdout[:2000]!r} stderr={stderr[:2000]!r}")
        pid=process.pid
        result={"stage":stage,"event":event,"pid":pid,"executable_hash":sha256_file(binary),"input_hash":sha256_bytes(canonical(payload)),"signal":"SIGKILL"}
        os.kill(pid,signal.SIGKILL); process.wait(timeout=15); result["exit_code"]=process.returncode; result["cleanup"]={"owned_process_reaped":process.poll() is not None,"descriptor_closed":True,"process_group_terminated":process.returncode == -signal.SIGKILL}
        if not result["cleanup"]["owned_process_reaped"]: raise AssertionError(f"owned process {pid} was not reaped")
        return result
    finally:
        parent.close()
        try: child.close()
        except OSError: pass

def reopened(binary: Path, repo: Path, store: Path) -> dict:
    result=subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"state"],input="{}",text=True,capture_output=True,check=True); response=json.loads(result.stdout)
    return {"head_revision":response["result"]["head_revision"],"graph_hash":response["result"]["graph_hash"],"response_hash":sha256_bytes(result.stdout.encode())}

def main(binary: Path) -> int:
    stages=["transaction_before_snapshot","transaction_after_snapshot","transaction_before_head_rename","transaction_after_head_rename"]; cases=[]
    with tempfile.TemporaryDirectory(prefix="il-p09-transaction-") as name:
        root=Path(name)
        for index,stage in enumerate(stages):
            repo,store=fixture(root/stage); initialize(binary,repo,store); payload=transaction_payload(f"P09-crash-{index}",f"app_{index}"); crashed=crash_case(binary,repo,store,payload,stage)
            if stage != "transaction_after_head_rename":
                observed=reopened(binary,repo,store)
                if observed["head_revision"] != 1: raise AssertionError(f"pre-rename crash changed verified HEAD: {observed}")
                if stage == "transaction_after_snapshot":
                    orphan=store/".il/revisions/00000000000000000002"
                    if not (orphan/"graph.json").is_file() or not (orphan/"manifest.json").is_file(): raise AssertionError("durable orphan snapshot was not retained")
                retry=transaction_payload(payload["task_id"]+"-retry", payload["scope"][0]+"_retry"); retry_result=subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"transact"],input=json.dumps(retry),text=True,capture_output=True,check=True); retry_response=json.loads(retry_result.stdout)
                expected_retry = 2 if stage == "transaction_before_snapshot" else 3
                if not retry_response.get("ok") or retry_response["result_revision"] != expected_retry: raise AssertionError(f"retry revision was not deterministic after {stage}: {retry_response}")
            else:
                observed=reopened(binary,repo,store)
                if observed["head_revision"] != 2: raise AssertionError(f"post-rename crash lost committed snapshot: {observed}")
                retry=transaction_payload(payload["task_id"]+"-retry", payload["scope"][0]+"_retry"); retry["base_revision"]=2; retry_result=subprocess.run([str(binary),"--repository",str(repo),"--store",str(store),"transact"],input=json.dumps(retry),text=True,capture_output=True,check=True); retry_response=json.loads(retry_result.stdout)
                if not retry_response.get("ok") or retry_response["result_revision"] != 3: raise AssertionError(f"post-rename retry did not advance revision: {retry_response}")
            crashed["reopen"]=observed; crashed["retry_revision"]=retry_response["result_revision"]; crashed["store_head_hash"]=sha256_file(store/".il/HEAD"); cases.append(crashed)
    print(json.dumps({"schema_version":"1.0.0","suite":"transaction_process","status":"PASSED","process_termination":"SIGKILL","cases":cases,"case_count":len(cases)},sort_keys=True)); return 0

if __name__ == "__main__":
    parser=argparse.ArgumentParser(); parser.add_argument("--binary",type=Path,required=True); raise SystemExit(main(parser.parse_args().binary.resolve()))
