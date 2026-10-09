"""Trusted, persistent P08 task worker. Untrusted stdin is only {tool, request}."""
from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools'))
import bootstrap_check as schema

TOOLS = frozenset('state schema-check inspect callers dependencies slice validate transact diff build test blackbox explain restore evidence'.split())
ACCEPTANCE = frozenset('add_route_without_full_source_dump stale_transaction_rejected failed_transaction_keeps_old_revision diagnostic_is_machine_parseable evidence_can_reconstruct_build'.split())
PHASES = ('contract', 'target_entities', 'direct_dependencies', 'callers', 'related_types', 'related_tests', 'current_diagnostics', 'relevant_rfc_fragments')
MAX_BYTES = 262144
BUILTIN_TYPES = frozenset('Unit Bool I8 I16 I32 I64 U8 U16 U32 U64 Usize String Bytes'.split())


def encoded(value):
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':')) + '\n').encode('utf-8')


def digest(value):
    return 'sha256:' + hashlib.sha256(value).hexdigest()


def strict(text):
    def pairs(items):
        value = {}
        for key, item in items:
            if key in value:
                raise Rejected('E_SCHEMA_INVALID', 'duplicate JSON key: ' + key)
            value[key] = item
        return value
    def constant(value):
        raise Rejected('E_SCHEMA_INVALID', 'non-JSON number: ' + value)
    try:
        return json.loads(text, object_pairs_hook=pairs, parse_constant=constant)
    except (ValueError, UnicodeError) as error:
        raise Rejected('E_SCHEMA_INVALID', 'invalid JSON') from error


class Rejected(Exception):
    def __init__(self, code, cause, status='BLOCKED', missing=()):
        super().__init__(cause)
        self.code, self.cause, self.status, self.missing = code, cause, status, list(missing)


def require(condition, code, cause, **kwargs):
    if not condition:
        raise Rejected(code, cause, **kwargs)


def atomic(path, value):
    pending = path.with_suffix('.pending')
    with pending.open('wb') as stream:
        stream.write(encoded(value)); stream.flush(); os.fsync(stream.fileno())
    os.replace(pending, path)
    descriptor = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


class Session:
    """All paths are trusted launcher inputs; a JSON step cannot override them."""
    def __init__(self, *, binary, repository, store, policy, task, directory):
        self.binary, self.repository, self.store, self.policy, self.task_path, self.directory = (
            Path(path).resolve() for path in (binary, repository, store, policy, task, directory))
        self.path = self.directory / 'session.json'
        self.state = None

    @contextlib.contextmanager
    def locked(self):
        require(sys.platform == 'linux', 'E_UNSUPPORTED_TARGET', 'task process enforcement requires Linux')
        import fcntl
        self.directory.mkdir(parents=True, exist_ok=True)
        with (self.directory / 'lock').open('a+b') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            yield

    def save(self):
        self.validate_session(self.state)
        atomic(self.path, self.state)

    def validate_session(self, state):
        path = ROOT / 'schema/ai_session.schema.json'
        try:
            schema.validate_instance(state, schema.read_json(path), path)
        except schema.ValidationError as error:
            raise Rejected('E_STATE_INCONSISTENT', 'invalid persisted session: ' + str(error)) from error
        history = state['history']
        require(state['phases'] == list(PHASES[:len(state['phases'])]), 'E_STATE_INCONSISTENT', 'session phases are out of order')
        require(state['runs'] == [item['run_id'] for item in history], 'E_STATE_INCONSISTENT', 'session run ledger differs')
        require([item['index'] for item in history] == list(range(1, len(history) + 1)), 'E_STATE_INCONSISTENT', 'session call ledger has gaps')
        pending = state['pending']
        require(state['tool_calls'] == len(history) + (1 if pending is not None else 0),
                'E_STATE_INCONSISTENT', 'session call counter differs from its ledger')
        require(pending is None or pending['index'] == state['tool_calls'], 'E_STATE_INCONSISTENT', 'pending admission index differs')
        require(state['base_revision'] == self.task['base_revision'] and state['current_revision'] >= state['base_revision'],
                'E_STATE_INCONSISTENT', 'session revision differs from task base')
        if state['status'] in {'RUNNING', 'DESIGN_REQUIRED'}:
            require(state['tool_calls'] <= self.budget['tool_calls'] and state['context_bytes'] <= self.budget['context_tokens'],
                    'E_STATE_INCONSISTENT', 'active session exceeds its budget')
            require(state['context_bytes'] >= sum(item['response_bytes'] for item in history),
                    'E_STATE_INCONSISTENT', 'active session context counter was reduced')

    def validate_task(self):
        self.task = strict(self.task_path.read_bytes())
        try:
            path = ROOT / 'schema/task.schema.json'
            schema.validate_instance(self.task, schema.read_json(path), path)
        except schema.ValidationError as error:
            raise Rejected('E_SCHEMA_INVALID', str(error)) from error
        require(set(self.task['acceptance']) <= ACCEPTANCE, 'E_SCHEMA_INVALID', 'unknown acceptance predicate')
        require(self.task['status'] in {'READY', 'RUNNING'}, 'E_STATE_INCONSISTENT', 'task is not executable')
        require('program' not in self.task['scope'], 'E_INVALID_SCOPE', 'whole-program scope is forbidden')
        for dependency in self.task['requires']:
            require(re.fullmatch(r'(P0[0-9]|P10|E0[1-5])', dependency), 'E_SCHEMA_INVALID', 'unknown prerequisite')
            document = schema.read_json(ROOT / 'eval/tasks' / (dependency + '.json'))
            require(document['status'] == 'VERIFIED', 'E_STATE_INCONSISTENT', 'prerequisite is not VERIFIED: ' + dependency)
        routes = [op for op in self.task['operations'] if op.get('op') == 'add_route']
        require(len(routes) == 1, 'E_SCHEMA_INVALID', 'registered HTTP task requires exactly one planned add_route')
        self.route = routes[0]
        self.contract, self.handler = self.route['route_table_id'], self.route['route']['handler']
        require(self.contract in self.task['scope'] and self.route['route']['entity_id'] in self.task['scope'],
                'E_INVALID_SCOPE', 'planned route and contract must be in task scope')
        rfc_paths = {path.relative_to(ROOT).as_posix() for path in (ROOT / 'rfc').glob('0022-*.md')}
        allowed_inputs = {self.contract, self.handler, 'spec/http.yaml'} | rfc_paths
        require(set(self.task['inputs']) <= allowed_inputs, 'E_SCHEMA_INVALID', 'input is not in the task context registry')
        self.budget = self.task['resource_budget']
        self.identity = {'task_hash': digest(self.task_path.read_bytes()), 'compiler_hash': digest(self.binary.read_bytes()),
                         'repository': str(self.repository), 'store': str(self.store), 'policy_hash': digest(self.policy.read_bytes())}

    def initialize(self):
        self.validate_task()
        if self.path.exists():
            loaded = strict(self.path.read_bytes())
            self.validate_session(loaded)
            self.state = loaded
            require(self.state['identity'] == self.identity, 'E_STATE_INCONSISTENT', 'session inputs changed')
            if self.state.get('pending') is not None:
                self.state['status'] = 'BLOCKED'; self.state['reason'] = 'interrupted admitted call requires explicit reconciliation'
                self.save()
            require(self.state['status'] in {'RUNNING', 'DESIGN_REQUIRED'}, 'E_STATE_INCONSISTENT', 'session is ' + self.state['status'])
            return
        self.state = {'schema_version': '1.0.0', 'identity': self.identity, 'task_id': self.task['task_id'],
                      'base_revision': self.task['base_revision'], 'current_revision': self.task['base_revision'],
                      'status': 'RUNNING', 'tool_calls': 0, 'context_bytes': 0, 'cpu_seconds': 0.0,
                      'graph_nodes': 4096, 'environment_retries': 0, 'phases': [], 'runs': [],
                      'history': [], 'pending': None, 'related_types': [],
                      'known_entities': sorted(set(self.task['scope']) | {self.handler}),
                      'base_authority': None, 'effects_delta': [], 'capabilities_delta': [], 'authority_revision': None}
        self.save()
        state = self.call('state', {}, 'preflight')
        require(state['ok'] and state['result_revision'] == self.task['base_revision'],
                'E_STALE_REVISION', 'task base differs from application HEAD')
        self.state['base_authority'] = self.authority(state['result']['run_id'])
        self.state['effects_delta'] = []
        self.state['capabilities_delta'] = []
        self.save()
        created = {self.route['route']['entity_id']}
        for entity_id in self.task['scope']:
            response = self.call('inspect', {'entity_id': entity_id, 'revision': self.task['base_revision'],
                                 'fields': ['entity_id'], 'budget': 2048}, 'preflight')
            absent = not response['ok'] and any(d['code'] == 'E_NAME_NOT_FOUND' for d in response['diagnostics'])
            require(response['ok'] or (absent and entity_id in created), 'E_INVALID_SCOPE', 'task scope entity unavailable: ' + entity_id)
            require(not (response['ok'] and entity_id in created), 'E_INVALID_SCOPE', 'planned new route already exists')
        for entity_id in sorted(set(self.task['inputs']) & {self.handler, self.contract} - set(self.task['scope'])):
            response = self.call('inspect', {'entity_id': entity_id, 'revision': self.task['base_revision'],
                                 'fields': ['entity_id'], 'budget': 2048}, 'preflight')
            require(response['ok'], 'E_NAME_NOT_FOUND', 'declared task input is unavailable: ' + entity_id)

    def process(self, tool, request, index):
        import ctypes
        import resource
        require(ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0) == 0,
                'E_UNSUPPORTED_TARGET', 'cannot establish owned descendant reaping')
        allowance = self.budget['cpu_seconds'] - self.state['cpu_seconds']
        require(allowance > 0, 'E_RESOURCE_LIMIT', 'task CPU budget exhausted')
        argv = [str(self.binary), '--repository', str(self.repository), '--store', str(self.store),
                '--host-policy', str(self.policy), tool]
        output = self.directory / f'{index:06}-stdout.json'
        error = self.directory / f'{index:06}-stderr.log'
        def limits():
            os.setsid()
            resource.setrlimit(resource.RLIMIT_CPU, (max(1, math.ceil(allowance)), max(1, math.ceil(allowance))))
            resource.setrlimit(resource.RLIMIT_AS, (self.budget['memory_bytes'], self.budget['memory_bytes']))
            resource.setrlimit(resource.RLIMIT_FSIZE, (64 * 1024 * 1024, 64 * 1024 * 1024))
            resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
        before = resource.getrusage(resource.RUSAGE_CHILDREN)
        started = time.monotonic()
        group_cpu = 0.0
        reason = None
        owned_pids = set()
        with output.open('wb') as stdout, error.open('wb') as stderr:
            process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=stdout, stderr=stderr,
                                       preexec_fn=limits, cwd=self.repository, env={'PATH': '/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin'})
            try:
                process.stdin.write(encoded(request)); process.stdin.close()
                while process.poll() is None:
                    owned_pids.update(pid for pid, _ in self.session_processes(process.pid))
                    cpu, memory = self.session_usage(process.pid)
                    group_cpu = max(group_cpu, cpu)
                    if cpu > allowance or memory > self.budget['memory_bytes']:
                        reason = 'task process-group CPU/memory budget exhausted'; break
                    if time.monotonic() - started > max(10, min(600, allowance * 4)):
                        reason = 'task worker wall deadline exceeded'; break
                    if output.stat().st_size > MAX_BYTES or error.stat().st_size > MAX_BYTES:
                        reason = 'task worker stream limit exceeded'; break
                    time.sleep(.02)
            finally:
                # LLVM and HTTP launchers may create nested process groups; they
                # remain in this dedicated POSIX session. Signal only those groups.
                owned_pids.update(pid for pid, _ in self.session_processes(process.pid))
                for group in self.owned_groups(process.pid) | {process.pid}:
                    try:
                        os.killpg(group, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                process.wait()
                # As a subreaper, this launcher adopts grandchildren whose worker
                # exited. Reap only recorded members of the task's own session.
                deadline = time.monotonic() + 2
                while owned_pids and time.monotonic() < deadline:
                    for pid in list(owned_pids):
                        try:
                            waited, _ = os.waitpid(pid, os.WNOHANG)
                            if waited:
                                owned_pids.remove(pid)
                        except ChildProcessError:
                            owned_pids.remove(pid)
                    if owned_pids:
                        time.sleep(.01)
        after = resource.getrusage(resource.RUSAGE_CHILDREN)
        self.state['cpu_seconds'] += max(group_cpu, after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime)
        require(reason is None, 'E_RESOURCE_LIMIT', reason or '')
        require(output.stat().st_size <= MAX_BYTES and error.stat().st_size <= MAX_BYTES,
                'E_RESOURCE_LIMIT', 'task worker output limit exceeded')
        require(output.stat().st_size > 0, 'E_TOOLCHAIN_FAILURE', 'task worker exited without a structured response')
        response = strict(output.read_bytes())
        require(isinstance(response, dict) and response.get('tool') == tool and isinstance(response.get('ok'), bool),
                'E_STATE_INCONSISTENT', 'worker returned malformed protocol response')
        require((process.returncode == 0) == response['ok'], 'E_STATE_INCONSISTENT', 'worker exit disagrees with response')
        return response

    @staticmethod
    def session_processes(session):
        for entry in Path('/proc').iterdir():
            if not entry.name.isdecimal():
                continue
            try:
                fields = (entry / 'stat').read_text().rsplit(')', 1)[1].split()
                if int(fields[3]) == session:
                    yield int(entry.name), fields
            except (FileNotFoundError, ProcessLookupError, PermissionError):
                continue

    @staticmethod
    def owned_groups(session):
        return {int(fields[2]) for _, fields in Session.session_processes(session)}

    @staticmethod
    def session_usage(session):
        cpu = 0; memory = 0
        ticks = os.sysconf('SC_CLK_TCK'); page = os.sysconf('SC_PAGE_SIZE')
        for _, fields in Session.session_processes(session):
            cpu += sum(int(fields[i]) for i in (11, 12, 13, 14)) / ticks
            memory += int(fields[21]) * page
        return cpu, memory

    def charge(self, value):
        size = len(encoded(value)); remaining = self.budget['context_tokens'] - self.state['context_bytes']
        require(size <= remaining, 'E_CONTEXT_INSUFFICIENT', 'cumulative task context budget exhausted')
        self.state['context_bytes'] += size

    def call(self, tool, request, kind='step'):
        require(self.state['tool_calls'] < self.budget['tool_calls'], 'E_RESOURCE_LIMIT', 'task tool-call budget exhausted')
        self.state['tool_calls'] += 1
        index = self.state['tool_calls']
        self.state['pending'] = {'index': index, 'tool': tool, 'request': request, 'kind': kind}
        self.save()
        response = self.process(tool, request, index)
        run_id = (response.get('result') or {}).get('run_id')
        require(isinstance(run_id, str) and re.fullmatch(r'run_[0-9a-f]{64}', run_id),
                'E_STATE_INCONSISTENT', 'admitted tool result has no immutable run ID')
        self.state['runs'].append(run_id)
        self.state['history'].append({'index': index, 'tool': tool, 'kind': kind, 'run_id': run_id,
                                      'request_hash': digest(encoded(request)),
                                      'response_hash': digest(encoded(response)), 'response_bytes': len(encoded(response)),
                                      'ok': response['ok'], 'subject_revision': response['result_revision']})
        if tool in {'transact', 'restore'} and response['ok']:
            self.state['current_revision'] = response['result_revision']
        self.state['pending'] = None
        self.charge(response)
        self.save()
        return response

    def authority(self, run_id):
        require(re.fullmatch(r'run_[0-9a-f]{64}', run_id), 'E_STATE_INCONSISTENT', 'invalid authority receipt identity')
        journal = self.store / '.il-tools'
        path = journal / 'runs' / (run_id + '.json')
        require(not path.is_symlink() and path.resolve().is_relative_to(journal.resolve()),
                'E_STATE_INCONSISTENT', 'authority receipt escapes journal')
        data = path.read_bytes()
        require(digest(data)[7:] == run_id[4:], 'E_STATE_INCONSISTENT', 'authority receipt hash differs')
        receipt = strict(data)
        refs = [item for item in receipt['artifacts'] if item['role'] == 'graph']
        require(len(refs) == 1, 'E_STATE_INCONSISTENT', 'authority receipt has no unique published graph')
        item = refs[0]
        require(re.fullmatch(r'sha256:[0-9a-f]{64}', item['sha256']) and item['path'] == 'blobs/' + item['sha256'][7:],
                'E_STATE_INCONSISTENT', 'authority graph locator is invalid')
        graph_path = journal / item['path']
        require(not graph_path.is_symlink() and graph_path.resolve().is_relative_to(journal.resolve()),
                'E_STATE_INCONSISTENT', 'authority graph escapes journal')
        graph_bytes = graph_path.read_bytes()
        require(digest(graph_bytes) == item['sha256'], 'E_STATE_INCONSISTENT', 'authority graph hash differs')
        graph = strict(graph_bytes)
        return {'effects': {function['entity_id']: sorted(function['effects']) for function in graph['functions']},
                'capabilities': {capability['entity_id']: capability for capability in graph['capabilities']}}

    def record_delta(self, run_id):
        current = self.authority(run_id)
        base = self.state['base_authority']
        for name, output in [('effects', 'effects_delta'), ('capabilities', 'capabilities_delta')]:
            changes = []
            for entity_id in sorted(set(base[name]) | set(current[name])):
                before, after = base[name].get(entity_id), current[name].get(entity_id)
                if before != after:
                    changes.append({'entity_id': entity_id, 'before': before, 'after': after})
            self.state[output] = changes
        self.state['authority_revision'] = self.state['current_revision']

    def validate_step(self, step):
        require(isinstance(step, dict) and set(step) == {'tool', 'request'}, 'E_SCHEMA_INVALID', 'step requires only tool and request')
        tool, request = step['tool'], step['request']
        require(isinstance(tool, str) and tool in TOOLS, 'E_SCHEMA_INVALID', 'tool is outside the closed registry')
        require(isinstance(request, dict), 'E_SCHEMA_INVALID', 'tool request must be an object')
        path = ROOT / 'schema/tool.schema.json'; definition = schema.read_json(path)
        try:
            schema.validate_instance(request, {'$ref': '#/$defs/' + tool}, path, root_schema=definition)
        except schema.ValidationError as error:
            raise Rejected('E_SCHEMA_INVALID', str(error)) from error
        require(len(encoded(request)) <= MAX_BYTES, 'E_RESOURCE_LIMIT', 'request exceeds transport limit')
        require(tool != 'schema-check' and request.get('entity_id') != 'program' and 'program' not in request.get('root_entities', []),
                'E_INVALID_SCOPE', 'whole-source and whole-program queries are forbidden')
        if tool == 'validate':
            require(isinstance(request['graph_or_revision'], int), 'E_INVALID_SCOPE', 'inline program validation is forbidden')
        if tool == 'slice':
            require(request['max_nodes'] <= self.state['graph_nodes'], 'E_RESOURCE_LIMIT', 'slice exceeds task graph-node limit')
            require(set(request['root_entities']) <= set(self.state['known_entities']), 'E_INVALID_SCOPE', 'slice roots were not acquired as context')
        if tool in {'inspect', 'callers', 'dependencies'}:
            require(request['entity_id'] in self.state['known_entities'], 'E_INVALID_SCOPE', 'entity was not acquired as task context')
        if tool in {'inspect', 'slice', 'callers', 'dependencies', 'build', 'test', 'blackbox', 'evidence'}:
            require('revision' in request, 'E_SCHEMA_INVALID', 'task reads require an explicit revision')
        if tool == 'transact':
            require(request['task_id'] == self.task['task_id'], 'E_INVALID_SCOPE', 'transaction task differs')
            require(set(request['scope']) <= set(self.task['scope']), 'E_INVALID_SCOPE', 'transaction exceeds task scope')
            require(not any(op['op'] in {'replace_program', 'import_text'} for op in request['operations']),
                    'E_INVALID_SCOPE', 'whole-program mutation is forbidden')
            require(request['base_revision'] == self.state['current_revision'], 'E_STALE_REVISION', 'transaction differs from session revision')
            for operation in request['operations']:
                require(operation['op'] == 'add_route', 'E_INVALID_SCOPE', 'this registered task only admits route operations')
                require(operation['route_table_id'] == self.contract
                        and operation['route']['handler'] in self.state['known_entities'],
                        'E_CONTEXT_INSUFFICIENT', 'route references were not acquired as context')
        if tool == 'restore':
            require(request['revision'] == self.task['rollback_revision'], 'E_INVALID_SCOPE', 'restore differs from task rollback revision')
        if tool == 'explain':
            require(request['run_id'] in self.state['runs'], 'E_INVALID_SCOPE', 'diagnostic run is outside this session')
        if tool == 'evidence':
            require(request['task_id'] == self.task['task_id'], 'E_INVALID_SCOPE', 'evidence task differs')
            require(set(request['tests']) <= set(self.state['runs'])
                    and all(item['run_id'] in self.state['runs'] for item in request['artifacts']),
                    'E_INVALID_SCOPE', 'evidence references a run outside this session')
        return tool, request

    @staticmethod
    def validate_admission(step):
        require(isinstance(step, dict) and set(step) == {'tool', 'request'}, 'E_SCHEMA_INVALID', 'step requires only tool and request')
        tool, request = step['tool'], step['request']
        require(isinstance(tool, str) and tool in TOOLS, 'E_SCHEMA_INVALID', 'tool is outside the closed registry')
        require(isinstance(request, dict), 'E_SCHEMA_INVALID', 'tool request must be an object')
        path = ROOT / 'schema/tool.schema.json'; definition = schema.read_json(path)
        try:
            schema.validate_instance(request, {'$ref': '#/$defs/' + tool}, path, root_schema=definition)
        except schema.ValidationError as error:
            raise Rejected('E_SCHEMA_INVALID', str(error)) from error
        require(tool != 'schema-check' and request.get('entity_id') != 'program'
                and 'program' not in request.get('root_entities', [])
                and 'program' not in request.get('scope', [])
                and not any(op.get('op') in {'replace_program', 'import_text'} for op in request.get('operations', [])),
                'E_INVALID_SCOPE', 'whole-program context is forbidden')

    def expected_context(self, tool, request):
        phase = len(self.state['phases'])
        if phase >= 5:
            return None
        expected = [('inspect', self.contract), ('inspect', self.handler), ('dependencies', self.handler),
                    ('callers', self.handler), ('slice', None)][phase]
        require(tool == expected[0] and (expected[1] is None or request.get('entity_id') == expected[1]),
                'E_CONTEXT_INSUFFICIENT', 'context acquisition phase is ' + PHASES[phase], missing=[expected[1] or 'related_types'])
        require(request.get('revision') == self.state['current_revision'], 'E_STALE_REVISION', 'context must describe task current revision')
        if phase == 1:
            require(set(request.get('fields', [])) == {'entity_id', 'parameters', 'result', 'effects', 'capabilities', 'contracts'},
                    'E_INVALID_SCOPE', 'handler context must be a bounded signature projection')
        if phase == 2:
            require(request.get('direction') == 'forward', 'E_SCHEMA_INVALID', 'direct dependencies require forward direction')
        if phase == 4:
            require(set(request['root_entities']) == set(self.state['related_types']),
                    'E_CONTEXT_INSUFFICIENT', 'related types must come from the inspected signature', missing=self.state['related_types'])
        return PHASES[phase]

    def registered_context(self):
        contract = (ROOT / 'spec/http.yaml').read_bytes()
        candidates = sorted((ROOT / 'rfc').glob('0022-*.md'))
        require(len(candidates) == 1, 'E_STATE_INCONSISTENT', 'RFC registry is ambiguous')
        text = candidates[0].read_text(encoding='utf-8')
        fragment = text.split('## Bounded route mutation', 1)[1].split('\n## ', 1)[0]
        return [
            {'phase': 'related_tests', 'reference': 'spec/http.yaml', 'sha256': digest(contract), 'content': contract.decode('utf-8')},
            {'phase': 'current_diagnostics', 'diagnostics': [], 'reason': 'no task mutation or failed tool has occurred'},
            {'phase': 'relevant_rfc_fragments', 'reference': candidates[0].relative_to(ROOT).as_posix(),
             'sha256': digest(candidates[0].read_bytes()), 'section': 'Bounded route mutation', 'content': fragment},
        ]

    def step(self, value):
        with self.locked():
            try:
                self.validate_admission(value)
                self.initialize()
                tool, request = self.validate_step(value)
                if self.state['status'] == 'DESIGN_REQUIRED' and tool in {'transact', 'restore', 'build', 'test', 'blackbox', 'evidence'}:
                    failure = self.state.get('design_failure', {})
                    require(tool == 'transact' and failure.get('tool') == 'transact'
                            and failure.get('explained') is True
                            and digest(encoded(request)) != failure.get('request_hash'),
                            'E_STATE_INCONSISTENT', 'nonretryable failure requires explanation and a different repair transaction', status='DESIGN_REQUIRED')
                    # A changed, explicitly submitted repair is a new design. The failed
                    # request is never replayed and remains in the immutable transcript.
                    self.state['status'] = 'RUNNING'
                phase = self.expected_context(tool, request)
                if tool in {'transact', 'restore'}:
                    require(len(self.state['phases']) == len(PHASES), 'E_CONTEXT_INSUFFICIENT', 'required context phases incomplete')
                    current = self.call('state', {}, 'mutation_preflight')
                    require(current['ok'] and current['result_revision'] == self.state['current_revision'],
                            'E_STALE_REVISION', 'application HEAD advanced outside session')
                response = self.call(tool, request)
                if tool == 'explain' and response['ok'] and self.state.get('design_failure', {}).get('run_id') == request['run_id']:
                    self.state['design_failure']['explained'] = True
                if tool in {'transact', 'restore', 'evidence'} and response['ok']:
                    self.record_delta(response['result']['run_id'])
                context = []
                if phase and response['ok']:
                    self.state['phases'].append(phase)
                    if phase == 'contract':
                        entity = response['result']['entity']
                        require(entity['subject'] in self.task['scope'],
                                'E_INVALID_SCOPE', 'dispatcher is outside task scope')
                        references = {entity['subject']}
                        for predicate in entity.get('predicates', []):
                            if predicate.get('kind') == 'http_server':
                                references.update(route['handler'] for route in predicate['routes'])
                        self.state['known_entities'] = sorted(set(self.state['known_entities']) | references)
                    if phase == 'target_entities':
                        entity = response['result']['entity']
                        types = [param['type'] for param in entity['parameters']] + [entity['result']]
                        self.state['related_types'] = sorted(set(types) - BUILTIN_TYPES)
                        self.state['known_entities'] = sorted(set(self.state['known_entities']) | set(self.state['related_types']))
                    if phase == 'direct_dependencies':
                        self.state['known_entities'] = sorted(set(self.state['known_entities']) | set(response['result']['entity_ids']))
                    if phase == 'related_types':
                        context = self.registered_context()
                        self.charge(context)
                        self.state['phases'].extend(PHASES[5:])
                if not response['ok']:
                    codes = {diagnostic['code'] for diagnostic in response['diagnostics']}
                    if codes & {'E_CONTEXT_INSUFFICIENT', 'E_RESOURCE_LIMIT', 'E_TOOLCHAIN_FAILURE'}:
                        self.state['status'] = 'BLOCKED'
                    elif any(not diagnostic['retryable'] for diagnostic in response['diagnostics']):
                        self.state['status'] = 'DESIGN_REQUIRED'
                        self.state['design_failure'] = {'tool': tool, 'request_hash': digest(encoded(request)),
                                                        'run_id': response['result']['run_id'], 'explained': False}
                result = {'status': self.state['status'], 'response': response, 'context': context,
                          'usage': {key: self.state[key] for key in ['tool_calls', 'context_bytes', 'cpu_seconds']}}
                # Context charges already include CLI responses and registry entries. Charge
                # the additional runner envelope as well, including its final counter digits.
                previous = self.state['context_bytes']
                while True:
                    extra = max(0, len(encoded(result)) - len(encoded(response)) - (len(encoded(context)) if context else 0))
                    require(previous + extra <= self.budget['context_tokens'], 'E_CONTEXT_INSUFFICIENT', 'runner envelope exceeds remaining context budget')
                    if result['usage']['context_bytes'] == previous + extra:
                        break
                    result['usage']['context_bytes'] = previous + extra
                self.state['context_bytes'] = result['usage']['context_bytes']
                self.save()
                return result
            except (Rejected, OSError) as error:
                if not isinstance(error, Rejected):
                    error = Rejected('E_TOOLCHAIN_FAILURE', str(error))
                result = {'status': error.status, 'code': error.code, 'cause': error.cause, 'missing': error.missing}
                if self.state is not None:
                    self.state['status'] = error.status
                    self.state['reason'] = error.cause
                    # An impossibly small remainder may be exceeded only by this bounded
                    # terminal error; no withheld entity or log context is delivered.
                    self.state['context_bytes'] += len(encoded(result))
                    self.save()
                return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['binary', 'repository', 'store', 'policy', 'task', 'directory']:
        parser.add_argument('--' + name, type=Path, required=True)
    options = vars(parser.parse_args())
    raw = sys.stdin.buffer.read(MAX_BYTES + 1)
    try:
        require(len(raw) <= MAX_BYTES, 'E_RESOURCE_LIMIT', 'step exceeds transport limit')
        result = Session(**options).step(strict(raw))
    except Rejected as error:
        result = {'status': error.status, 'code': error.code, 'cause': error.cause, 'missing': error.missing}
    sys.stdout.buffer.write(encoded(result))
    return 0 if result['status'] == 'RUNNING' else 1


if __name__ == '__main__':
    raise SystemExit(main())
