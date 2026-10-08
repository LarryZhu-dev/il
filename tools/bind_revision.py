"""Bind a graph revision to a Git commit in Git's external metadata journal."""
import argparse
import json
import os
import subprocess
from pathlib import Path


def git(*args):
    return subprocess.check_output(['git', *args], text=True).strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--revision', type=int)
    parser.add_argument('--commit', default='HEAD')
    args = parser.parse_args()
    state = json.loads(Path('repository_state.json').read_text())
    revision = state['head_revision'] if args.revision is None else args.revision
    if revision != state['head_revision']:
        raise SystemExit('STATE_INCONSISTENT: only the current graph revision may be bound')
    commit = git('rev-parse', '--verify', args.commit + '^{commit}')
    committed_state = json.loads(git('show', commit + ':repository_state.json'))
    if committed_state != state:
        raise SystemExit('STATE_INCONSISTENT: committed and working state differ')
    record = {'revision': revision, 'git_commit': commit,
              'tree_hash': git('rev-parse', commit + '^{tree}')}
    directory = Path(git('rev-parse', '--git-dir')) / 'il'
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / 'revision_bindings.json'
    journal = json.loads(path.read_text()) if path.exists() else {'schema_version': '1.0.0', 'bindings': []}
    if record not in journal['bindings']:
        journal['bindings'].append(record)
        temporary = path.with_suffix('.tmp')
        with temporary.open('w', encoding='utf-8', newline='\n') as file:
            json.dump(journal, file, indent=2)
            file.write('\n')
            file.flush()
            os.fsync(file.fileno())
        os.replace(temporary, path)
    print(json.dumps({'ok': True, 'tool': 'bind_revision', 'binding': record}))


if __name__ == '__main__':
    main()
