"""Record actual gate output and artifact identities; never invent passed checks."""
import argparse
import gzip
import hashlib
import json
import subprocess
from pathlib import Path


def digest(data):
    return 'sha256:' + hashlib.sha256(data).hexdigest()


def archive_artifact(path):
    data = Path(path).read_bytes()
    identity = digest(data)
    archive = Path('eval/artifacts') / (identity.removeprefix('sha256:') + '.gz')
    archive.parent.mkdir(parents=True, exist_ok=True)
    if archive.exists():
        if gzip.decompress(archive.read_bytes()) != data:
            raise SystemExit('E_EVIDENCE_INCOMPLETE: corrupt immutable artifact archive')
    else:
        archive.write_bytes(gzip.compress(data, compresslevel=9, mtime=0))
    return {'path': path, 'sha256': identity}


def validate_report(report, source_commit):
    checks = [gate for gate in report.get('gates', []) if 'exit_code' in gate]
    if report.get('source_commit') != source_commit:
        raise ValueError('E_EVIDENCE_INCOMPLETE: report does not describe the current source commit')
    if report.get('active_gate') is not None or not checks or report.get('status') != 'PASSED' or any(g['exit_code'] != 0 for g in checks):
        raise ValueError('E_EVIDENCE_INCOMPLETE: only complete actual passing gate reports can be recorded')
    if not any(g.get('is_test') and g.get('test_count', 0) > 0 for g in checks):
        raise ValueError('E_EVIDENCE_INCOMPLETE: report contains no executed test cases')
    return checks


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
    if subprocess.check_output(['git', 'status', '--porcelain'], text=True).strip():
        raise SystemExit('E_EVIDENCE_INCOMPLETE: evidence requires a clean source checkout')
    report = json.loads(Path(args.report).read_text())
    source_commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
    try:
        checks = validate_report(report, source_commit)
    except ValueError as error:
        raise SystemExit(str(error))
    state = json.loads(Path('repository_state.json').read_text())
    lock = json.loads(Path('toolchain.lock').read_text())
    unavailable = [name for name in ['compiler', 'runtime'] if getattr(args, name) is None]
    evidence = {
        'evidence_id': f'ev_{args.task}_{state["head_revision"]}',
        'task_id': args.task, 'phase': args.phase,
        'revision': state['head_revision'],
        'git_commit': source_commit,
        'graph_hash': digest(Path(args.graph).read_bytes()),
        'compiler_hash': digest(Path(args.compiler).read_bytes()) if args.compiler else None,
        'runtime_hash': digest(Path(args.runtime).read_bytes()) if args.runtime else None,
        'host_compiler_hash': lock['host_compiler_hash'],
        'unavailable_components': unavailable,
        'target': state['target'], 'toolchain_lock_hash': digest(Path('toolchain.lock').read_bytes()),
        'commands': [{'id': g['name'], 'argv_hash': digest(json.dumps(g['command'], separators=(',', ':')).encode()), 'exit_code': g['exit_code']} for g in checks],
        'artifacts': [archive_artifact(p) for p in args.artifact],
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
    path.write_text(json.dumps(evidence, indent=2) + '\n', newline='\n')
    print(json.dumps({'ok': True, 'evidence': str(path)}))


if __name__ == '__main__':
    main()
