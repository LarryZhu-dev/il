"""Build and execute P01's real LLVM/object/link/runtime acceptance pipeline."""

import json
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
REPORT = ROOT / 'build' / 'probe_gate_report.json'


def main():
    if len(sys.argv) != 1:
        raise SystemExit('test_native_probe.py accepts no arguments')
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    report = {'schema_version': '1.0.0', 'task_id': 'P01', 'status': 'RUNNING', 'gates': []}
    try:
        command = ['cargo', 'build', '--locked', '-p', 'il-native-probe', '-p', 'il-probe-runtime']
        built = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=300)
        report['gates'].append({
            'name': 'build-native-probe', 'command': command, 'exit_code': built.returncode,
            'status': 'PASSED' if built.returncode == 0 else 'FAILED', 'is_test': False,
            'test_count': 0, 'stdout': built.stdout, 'stderr': built.stderr,
        })
        if built.returncode:
            raise RuntimeError('native probe build failed: ' + built.stderr)
        # Replace any earlier report before executing, so a killed probe cannot
        # accidentally reuse a previous passing result.
        REPORT.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
        executed = subprocess.run(
            [str(ROOT / 'target' / 'debug' / 'il-native-probe')], cwd=ROOT,
            capture_output=True, text=True, timeout=60,
        )
        native = json.loads(REPORT.read_text(encoding='utf-8'))
        native['gates'] = report['gates'] + native['gates']
        report = native
        if executed.returncode or report.get('status') != 'PASSED':
            raise RuntimeError('native probe execution failed: ' + executed.stderr)
        print(json.dumps({'status': 'PASSED', 'checks': report['checks'], 'report': str(REPORT.relative_to(ROOT))}))
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report['status'] = 'FAILED'
        report['error'] = str(error)
        print(str(error), file=sys.stderr)
        return 1
    finally:
        REPORT.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    sys.exit(main())
