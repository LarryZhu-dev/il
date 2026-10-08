"""Record actual gate output and artifact identities; never invent passed checks."""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path


def digest(data):
    return 'sha256:' + hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--task', required=True)
    parser.add_argument('--phase', required=True)
    parser.add_argument('--report', required=True)
    parser.add_argument('--graph', default='examples/bootstrap/graph.json')
    parser.add_argument('--compiler')
    parser.add_argument('--runtime')
    parser.add_argument('--artifact', action='append', default=[])
    args = parser.parse_args()
    report = json.loads(Path(args.report).read_text())
    gates = report.get('gates', [])
    checks = [g for g in gates if 'exit_code' in g]
    if not checks or report.get('status') != 'PASSED' or any(g['exit_code'] != 0 for g in checks):
        raise SystemExit('E_EVIDENCE_INCOMPLETE: only actual passing gate reports can be recorded')
    state = json.loads(Path('repository_state.json').read_text())
    lock = json.loads(Path('toolchain.lock').read_text())
    unavailable = [name for name in ['compiler', 'runtime'] if getattr(args, name) is None]
    evidence = {
        'evidence_id': f'ev_{args.task}_{state["head_revision"]}',
        'task_id': args.task, 'phase': args.phase,
        'revision': state['head_revision'],
        'git_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'graph_hash': digest(Path(args.graph).read_bytes()),
        'compiler_hash': digest(Path(args.compiler).read_bytes()) if args.compiler else None,
        'runtime_hash': digest(Path(args.runtime).read_bytes()) if args.runtime else None,
        'host_compiler_hash': lock['host_compiler_hash'],
        'unavailable_components': unavailable,
        'target': state['target'], 'toolchain_lock_hash': digest(Path('toolchain.lock').read_bytes()),
        'commands': [{'id': g['name'], 'argv_hash': digest(json.dumps(g['command'], separators=(',', ':')).encode()), 'exit_code': g['exit_code']} for g in checks],
        'artifacts': [{'path': p, 'sha256': digest(Path(p).read_bytes())} for p in args.artifact],
        'tests': [{'suite': g['name'], 'passed': True, 'count': g.get('test_count', 1)} for g in checks if g.get('is_test', False)],
        'effects_delta': [], 'capabilities_delta': [],
        'known_limits': ['Only the recorded task and gates are verified; later tasks are not verified.'],
    }
    if not evidence['tests']:
        raise SystemExit('E_EVIDENCE_INCOMPLETE: report has no designated test suites')
    path = Path('eval/evidence') / (evidence['evidence_id'] + '.json')
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        raise SystemExit('Evidence is immutable: ' + str(path))
    path.write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps({'ok': True, 'evidence': str(path)}))


if __name__ == '__main__':
    main()
