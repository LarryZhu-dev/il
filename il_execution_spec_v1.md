---
document_id: il-execution-spec
version: 1.0.0
kind: autonomous-agent-build-spec
language: zh-CN
execution_mode: tool-driven
source_of_truth: this_document + accepted_rfc + repository_state
schedule_model: dependency-driven
human_review_required_for_routine_changes: false
---

# IL Autonomous Build Specification

## 0. Agent Contract

```yaml
agent_role: implementation_agent
objective: build_il_language_from_empty_repository_to_mvp
primary_operation: read_state -> select_ready_task -> patch -> validate -> commit -> emit_evidence
```

执行规则：

```yaml
must:
  - read repository_state before every task
  - resolve task dependencies before mutation
  - use structured program graph for language changes
  - attach every artifact to a revision
  - run declared validation commands
  - preserve previous verified revision
  - emit machine-readable evidence
  - return BLOCKED when required input is missing
  - return DESIGN_REQUIRED when a locked semantic decision must change

must_not:
  - modify a task whose dependencies are not VERIFIED
  - invent language semantics not present in this document or an accepted RFC
  - generate C/Rust/JavaScript as the target program
  - delete or weaken a failing test to obtain success
  - overwrite the last VERIFIED revision
  - run arbitrary shell commands from model output
  - add undeclared effects or capabilities
  - claim support for an untested feature
  - mix unrelated subsystem changes in one transaction
  - use natural-language success claims as evidence
```

任务返回值只能是：

```yaml
status:
  READY: inputs and dependencies complete
  RUNNING: mutation or validation in progress
  VERIFIED: all done_if predicates true
  FAILED: implementation or validation failed
  BLOCKED: dependency, permission, tool, or input unavailable
  DESIGN_REQUIRED: locked semantic decision must change
  ROLLED_BACK: candidate discarded and previous verified revision restored
```

禁止返回 `DONE`、`PROBABLY_DONE`、`IMPLEMENTED` 等未定义状态。

## 1. Immutable Project Objective

```yaml
project_id: il
product: ai-native-native-language
first_target:
  os: linux
  arch: x86_64
  binary_format: ELF
  execution: native_ahead_of_time
compiler_bootstrap:
  allowed_host_languages: [Rust]
  allowed_native_backends: [LLVM]
target_program_translation:
  forbidden: [C, C++, Rust, JavaScript, TypeScript, Python, JVM_bytecode]
  allowed: [target_machine_code, target_object_file, target_binary]
```

MVP 输出必须包含：

```yaml
artifacts:
  - compiler_binary
  - runtime_binary_or_static_runtime
  - il_graph_schema
  - il_transaction_tool
  - il_check_tool
  - il_build_tool
  - il_test_tool
  - http_package
  - blackbox_http_tests
  - reproducible_build_record
  - ai_task_evidence_records
```

MVP 必须通过以下行为闭环：

```text
创建空仓库
→ 建立程序图
→ 定义类型
→ 定义函数
→ 编译为本地 ELF
→ 启动 HTTP 服务
→ 外部客户端访问 /health
→ AI 通过事务接口新增 /hello/{name}
→ 编译
→ 黑盒测试
→ 注入类型错误
→ AI 读取结构化诊断并修复
→ 回归测试
→ 回退到修复前 revision
```

## 2. Repository State Contract

仓库必须包含以下目录；目录缺失时由 `P00` 创建：

```text
/spec
/rfc
/adr
/schema
/compiler
/runtime
/packages
/tools
/tests
/examples
/eval
/build
/release
```

规范文件：

```text
/spec/language.yaml
/spec/types.yaml
/spec/memory.yaml
/spec/effects.yaml
/spec/abi.yaml
/spec/http.yaml
/spec/errors.yaml
/spec/versioning.yaml
```

图与事务文件：

```text
/schema/program_graph.schema.json
/schema/transaction.schema.json
/schema/diagnostic.schema.json
/schema/evidence.schema.json
/schema/task.schema.json
```

仓库状态文件：

```json
{
  "project_id": "il",
  "schema_version": "1.0.0",
  "head_revision": 0,
  "last_verified_revision": 0,
  "compiler_version": null,
  "runtime_version": null,
  "target": "x86_64-unknown-linux-gnu",
  "open_tasks": ["P00"],
  "blocked_tasks": [],
  "failed_tasks": [],
  "locks": {}
}
```

任何工具首先读取 `repository_state.json`。状态文件和实际 Git revision 不一致时返回 `STATE_INCONSISTENT`，禁止继续写入。

## 3. Canonical Program Representation

### 3.1 Primary Representation

源代码文本不是主数据。主数据为规范化程序图：

```yaml
ProgramGraph:
  project_id: string
  graph_version: string
  revision: uint64
  target: Target
  modules: [Module]
  types: [Type]
  functions: [Function]
  capabilities: [Capability]
  packages: [Package]
  contracts: [Contract]
```

所有实体必须有稳定 `entity_id`。`entity_id` 不得由内存地址、文件行号或随机 ID 生成。

### 3.2 Entity Types

```yaml
Module:
  entity_id: EntityId
  path: string
  imports: [EntityId]
  declarations: [EntityId]
  visibility: private|public

Type:
  entity_id: EntityId
  kind: unit|never|bool|int|usize|string|bytes|record|sum|option|result|tuple
  parameters: [TypeRef]
  layout: inferred|ffi|opaque

Function:
  entity_id: EntityId
  name: string
  parameters: [Parameter]
  result: TypeRef
  effects: [Effect]
  capabilities: [CapabilityRef]
  blocks: [Block]
  contracts: [ContractRef]

Block:
  entity_id: EntityId
  arguments: [Value]
  operations: [Operation]
  terminator: Operation

Operation:
  entity_id: EntityId
  opcode: string
  inputs: [ValueRef]
  outputs: [Value]
  attributes: object
  effects: [Effect]
  consumes: [ValueRef]
  produces: [Value]
```

### 3.3 Normalization

序列化必须固定：

```yaml
encoding: UTF-8
object_key_order: schema_order
array_order: semantic_order
number_format: decimal_without_exponent
line_ending: LF
trailing_newline: true
unknown_fields: reject
unknown_opcodes: reject
```

### 3.4 Graph Invariants

校验器必须拒绝：

```yaml
invalid:
  - duplicate_entity_id
  - dangling_entity_reference
  - value_used_before_definition
  - operation_after_terminator
  - block_without_terminator
  - function_without_return_path
  - type_cycle_without_indirection
  - effect_not_declared_by_caller
  - capability_not_injected
  - resource_used_after_move
  - resource_dropped_twice
  - borrow_escape
  - route_collision
```

## 4. Transaction Protocol

所有图变更只能通过事务接口：

```json
{
  "task_id": "P07-T04",
  "base_revision": 42,
  "scope": ["module_api", "route_table", "handler_hello"],
  "operations": [
    {
      "op": "add_route",
      "route_table_id": "route_table",
      "method": "GET",
      "path": "/hello/{name}",
      "handler_id": "handler_hello"
    }
  ],
  "required_checks": [
    "schema",
    "names",
    "types",
    "ownership",
    "effects",
    "capabilities",
    "contracts"
  ]
}
```

事务处理顺序固定：

```text
read base_revision
→ validate request schema
→ verify scope
→ clone candidate graph
→ apply operations
→ validate references
→ resolve names
→ check types
→ check ownership
→ check effects
→ check capabilities
→ check contracts
→ serialize candidate
→ atomically commit new revision
```

任一检查失败：

```yaml
commit: false
head_revision: unchanged
candidate: retained_as_experiment_only
result: diagnostics[]
```

过期 `base_revision` 必须返回 `STALE_REVISION`。v1.0 不自动解决语义冲突。

## 5. Tool Protocol

工具必须提供以下接口。接口定义以 JSON Schema 为准，模型不得自行扩展参数。

```yaml
inspect(entity_id, revision, fields, budget)
callers(entity_id, revision, budget)
dependencies(entity_id, revision, direction, budget)
slice(root_entities, revision, max_nodes, max_tokens)
validate(graph_or_revision, checks)
transact(transaction)
diff(base_revision, target_revision, scope)
build(revision, target, profile)
test(revision, suite, isolation)
run_blackbox(revision, contract)
explain_diagnostic(diagnostic_id, context_budget)
restore(revision, reason)
emit_evidence(revision, artifacts, tests)
```

工具返回统一结构：

```json
{
  "ok": true,
  "tool": "validate",
  "tool_version": "1.0.0",
  "base_revision": 42,
  "result_revision": 42,
  "diagnostics": [],
  "artifacts": [],
  "evidence_id": null
}
```

工具不得返回只有自然语言的结果。

## 6. Language Locks

以下语义在 MVP 中锁定。改变任一项必须创建 RFC，旧任务全部停止在 `DESIGN_REQUIRED`。

### 6.1 Syntax Projection

文本投影采用 C/Rust 风格块结构；文本不是编译器唯一输入。禁止自动分号插入、隐式数字转换、隐式字符串转换、原型链、运行时 eval 和用户自定义语法。

### 6.2 Types

```yaml
primitive: [Unit, Never, Bool, I8, I16, I32, I64, U8, U16, U32, U64, Usize, String, Bytes]
compound: [Tuple, Record, Sum, Option, Result]
default_integer_literal: I64
implicit_numeric_conversion: false
implicit_nullable_conversion: false
structural_record_subtyping: false
```

### 6.3 Errors

```text
Result<T,E> = Ok(T) | Err(E)
```

`?` 只表示错误传播。panic/trap 只用于不可恢复错误。业务错误不得依赖本地化文本匹配。

### 6.4 Memory

```yaml
scalar: copy
owned_heap_value: move
copy_owned_value: explicit_clone
borrow_shared: lexical_non_escaping
borrow_mut: unique_lexical_non_escaping
raw_pointer: unsafe_only
borrow_across_await: forbidden_in_mvp
field_partial_move: forbidden_in_mvp
```

资源状态：

```text
Uninitialized → Initialized → Moved | Borrowed | Dropped
Borrowed → Initialized
Moved/Dropped → terminal
```

### 6.5 Effects

MVP 效果集合：

```text
alloc
fs
net
clock
process
unsafe
```

调用者效果必须覆盖被调用者效果。效果不能被注释、命名或运行时猜测替代。

### 6.6 Capabilities

MVP 能力：

```text
FileRead(path_scope)
FileWrite(path_scope)
Listen(address_scope)
Connect(address_scope)
SpawnProcess(command_scope)
ClockRead
```

能力只能由宿主注入。普通图操作不能凭空构造能力。

## 7. Compiler Pipeline

编译阶段固定：

```text
load_graph
→ schema_validate
→ resolve_modules
→ resolve_names
→ type_check
→ effect_check
→ capability_check
→ ownership_check
→ lower_to_hir
→ lower_to_mir
→ verify_mir
→ lower_to_native_ir
→ verify_native_ir
→ codegen_x86_64
→ emit_object
→ link_runtime
→ emit_elf
```

每阶段必须输出：

```yaml
stage: string
input_hash: sha256
output_hash: sha256
compiler_version: string
diagnostics: [Diagnostic]
```

### 7.1 HIR

消除 `?`、模板字符串、语法糖和隐式返回。HIR 不得保留无法解释的表面语法。

### 7.2 MIR

显式表示基本块、控制流、move、borrow、drop、错误边和外部调用。所有资源路径在 MIR 验证后才能进入后端。

### 7.3 Native IR

必须表达定宽整数、指针、内存读写、调用约定、原子操作、分支和调试位置。语言语义不能直接继承后端未定义行为。

### 7.4 Backend

P05 仅支持 `x86_64-unknown-linux-gnu`。可以使用固定版本 LLVM 输出目标文件。不得生成中间高级语言源码。

## 8. Runtime Contract

运行时组件：

```text
startup
allocator
string_bytes
resource_handles
stdout_stderr
deadline
panic_trap
stack_trace
capability_bridge
```

运行时必须实现：

```yaml
resource_rules:
  - one_owner_per_handle
  - close_invalidates_handle
  - double_close_detectable
  - failed_allocation_is_explicit
  - partial_io_is_explicit
  - timeout_is_explicit
  - panic_exit_code_is_stable
```

运行时模式：

```text
full-runtime
minimal-runtime
no-runtime
```

MVP 服务使用 `full-runtime`；底层测试至少覆盖 `minimal-runtime`。

## 9. Standard Library Surface

MVP 包顺序由依赖决定：

```text
core
→ alloc
→ io
→ time
→ net
→ json
→ http
→ test
→ tracing
```

禁止在 `core` 依赖 `http`、`database` 或模型调用。

每个包必须提供：

```yaml
package_manifest
public_types
public_functions
error_types
effects
capabilities
unit_tests
blackbox_tests
schema_index
```

## 10. HTTP MVP Contract

HTTP 不是语言关键字；它是 `packages/http` 的类型化包。

### 10.1 Graph-Level Declaration

```yaml
server:
  entity_id: server_api
  bind: 127.0.0.1:8080
  capability: ListenCapability
  routes:
    - method: GET
      path: /health
      handler: fn_health
    - method: GET
      path: /hello/{name}
      handler: fn_hello
      parameters:
        name:
          type: String
          source: path
          max_utf8_bytes: 128
```

### 10.2 Handler Types

```text
fn_health: Request<Empty> -> Result<Response<Bytes>, HttpError>
fn_hello: Request<Path<{name: String}>> -> Result<Response<HelloBody>, HttpError>
```

### 10.3 Protocol Subset

MVP 只实现：

```yaml
protocol: HTTP/1.1_subset
connection: one_request_then_close
request_body: reject_nonzero_length
transfer_encoding: reject
upgrade: reject
connect: reject
compression: reject
tls: external_boundary_not_mvp
```

限制：

```yaml
request_line_max_bytes: 4096
header_section_max_bytes: 16384
read_deadline_ms: 5000
write_deadline_ms: 5000
path_parameter_max_utf8_bytes: 128
```

### 10.4 Blackbox Contract

```yaml
GET /health:
  status: 200
  content_type: text/plain; charset=utf-8
  body: ok

GET /hello/Larry:
  status: 200
  content_type: application/json
  body_json:
    message: Hello, Larry

GET /unknown:
  status: 404

POST /health:
  status: 405
  allow: GET
```

测试必须通过外部 TCP/HTTP 客户端执行，不能导入路由表或内部解析函数。

## 11. Task Graph

任务调度器读取以下依赖图。未列出的任务不属于 MVP。

```yaml
P00: []
P01: [P00]
P02: [P00]
P03: [P02]
P04: [P03]
P05: [P01, P03, P04]
P06: [P05]
P07: [P06]
P08: [P02, P03, P07]
P09: [P07, P08]
P10: [P09]
E01: [P10]
E02: [P10]
E03: [P10]
E04: [P10]
E05: [P10]
```

调度器只能选择 `all(requires.status == VERIFIED)` 的任务。

## 12. Task Specifications

### P00 Bootstrap Contract

```yaml
task_id: P00
requires: []
inputs: [this_document]
outputs:
  - repository_tree
  - repository_state.json
  - toolchain.lock
  - rfc_template
  - diagnostic_schema
  - evidence_schema
mutations:
  - create_directories
  - create_state_files
  - create_ci_entrypoint
checks:
  - clean_repository
  - target_is_x86_64_linux
  - state_head_equals_git_head
  - schemas_parse
  - forbidden_target_languages_not_used
fail_if:
  - target_is_not_linux_x86_64
  - arbitrary_build_commands_are_enabled
```

### P01 Native Codegen Probe

```yaml
task_id: P01
requires: [P00]
inputs: [toolchain.lock, target_description]
outputs:
  - compiler/native_probe
  - runtime/probe_runtime
  - build/probe.elf
  - build/probe_evidence.json
steps:
  - construct_minimal_native_ir
  - emit_x86_64_object
  - link_runtime
  - execute_probe
  - hash_binary
checks:
  - exit_code_is_zero
  - stdout_equals_probe_ok
  - clean_environment_execution
  - evidence_complete
```

### P02 Program Graph and Transaction Core

```yaml
task_id: P02
requires: [P00]
outputs:
  - schema/program_graph.schema.json
  - schema/transaction.schema.json
  - tools/graph_validate
  - tools/graph_inspect
  - tools/graph_transact
  - tests/graph
steps:
  - implement_entity_ids
  - implement_normalized_serialization
  - implement_revision_check
  - implement_atomic_transaction
  - implement_diff
  - implement_restore
negative_cases:
  - stale_revision
  - dangling_reference
  - duplicate_id
  - unknown_opcode
  - partial_commit
  - invalid_scope
checks:
  - failed_transaction_preserves_base_revision
  - normalized_serialization_is_deterministic
```

### P03 Static Semantics Checker

```yaml
task_id: P03
requires: [P02]
outputs:
  - compiler/schema_checker
  - compiler/name_resolver
  - compiler/type_checker
  - compiler/effect_checker
  - compiler/ownership_checker
  - compiler/diagnostics
steps:
  - implement_types
  - implement_function_signatures
  - implement_name_resolution
  - implement_result_option
  - implement_move_state
  - implement_lexical_borrow
  - implement_drop_paths
  - implement_effect_union
  - implement_capability_check
checks:
  - every_negative_case_has_stable_code
  - every_positive_fixture_passes
  - no_checker_panic_on_fuzz_input
```

### P04 Interpreter and MIR

```yaml
task_id: P04
requires: [P03]
outputs:
  - compiler/hir
  - compiler/mir
  - compiler/mir_verifier
  - runtime/interpreter
  - tests/semantic_snapshots
steps:
  - lower_graph_to_hir
  - lower_hir_to_mir
  - verify_mir
  - execute_mir
  - record_output_and_error
checks:
  - pure_fixtures_pass
  - illegal_mir_is_rejected
  - output_is_deterministic
```

### P05 Native Compiler

```yaml
task_id: P05
requires: [P01, P03, P04]
outputs:
  - compiler/native_ir
  - compiler/codegen_x86_64
  - compiler/object_emitter
  - compiler/link_driver
steps:
  - lower_constants
  - lower_integer_ops
  - lower_control_flow
  - lower_calls
  - lower_records
  - lower_sum_types
  - lower_result_paths
  - lower_string_runtime_calls
  - emit_debug_locations
  - link_elf
checks:
  - interpreter_and_native_outputs_match
  - debug_and_release_semantics_match
  - no_high_level_target_source_emitted
  - reproducible_binary_hash_for_same_inputs
```

### P06 Runtime and Core Packages

```yaml
task_id: P06
requires: [P05]
outputs:
  - runtime/startup
  - runtime/allocator
  - runtime/handles
  - packages/core
  - packages/alloc
  - packages/io
  - examples/hello
steps:
  - implement_entrypoint
  - implement_exit_codes
  - implement_owned_bytes
  - implement_string
  - implement_resource_handles
  - implement_stdout_stderr
  - implement_deadlines
  - implement_failure_injection
checks:
  - hello_native_runs_clean
  - resource_failure_is_explicit
  - double_drop_is_detected
  - leaked_handle_test_passes
```

### P07 HTTP Package and Route Graph

```yaml
task_id: P07
requires: [P06]
outputs:
  - packages/net
  - packages/http
  - examples/http_demo
  - tests/http_blackbox
steps:
  - implement_loopback_listener
  - implement_connection_deadline
  - implement_request_line_parser
  - implement_header_parser
  - implement_path_decoder
  - implement_exact_route_match
  - implement_typed_path_parameter
  - implement_response_encoder
  - implement_404_405
  - implement_json_escape
  - implement_external_blackbox_client
checks:
  - health_contract_passes
  - hello_contract_passes
  - malformed_request_cases_pass
  - fragmentation_cases_pass
  - timeout_cases_pass
  - body_and_header_limits_pass
  - route_collision_is_rejected
```

### P08 AI Tool Protocol

```yaml
task_id: P08
requires: [P02, P03, P07]
outputs:
  - tools/ai_protocol
  - schema/task.schema.json
  - schema/evidence.schema.json
  - eval/tasks
  - eval/runner
steps:
  - implement_inspect
  - implement_slice
  - implement_callers
  - implement_dependencies
  - implement_transaction_wrapper
  - implement_diagnostic_explanation
  - implement_build_wrapper
  - implement_test_wrapper
  - bind_every_result_to_revision
  - enforce_context_budget
  - enforce_command_allowlist
checks:
  - add_route_without_full_source_dump
  - stale_transaction_rejected
  - failed_transaction_keeps_old_revision
  - diagnostic_is_machine_parseable
  - evidence_can_reconstruct_build
```

### P09 Independent Validation and Fault Injection

```yaml
task_id: P09
requires: [P07, P08]
outputs:
  - tests/blackbox
  - tests/fault_injection
  - eval/evidence
steps:
  - build_external_http_client
  - run_http_contract
  - inject_allocation_failure
  - inject_accept_failure
  - inject_read_timeout
  - inject_partial_write
  - inject_peer_disconnect
  - interrupt_transaction
  - interrupt_build
  - verify_restore
checks:
  - no_internal_http_imports_in_blackbox
  - every_failure_has_diagnostic
  - old_verified_revision_still_runs
  - evidence_schema_valid
```

### P10 MVP Gate

```yaml
task_id: P10
requires: [P09]
outputs:
  - release/mvp_manifest.json
  - release/mvp_binary_hashes.json
  - release/known_limits.json
  - eval/mvp_comparison.json
checks:
  - all_G0_G4_predicates_true
  - clean_environment_rebuild_passes
  - http_blackbox_passes
  - ai_transaction_repair_passes
  - rollback_passes
  - comparison_data_complete
```

### E01 Language Extensions

```yaml
task_id: E01
requires: [P10]
allowed_order: [generics, traits, floats, const_eval, controlled_macros]
rule: one_extension_one_rfc_one_retest
```

### E02 Concurrency

```yaml
task_id: E02
requires: [P10]
order: [Thread, Mutex, Channel, Atomic, structured_tasks, async]
forbidden_until_rfc: borrow_across_await
```

### E03 Multi-Target

```yaml
task_id: E03
requires: [P10]
order: [aarch64_linux, macos, windows]
per_target_required: [abi, runtime, syscall, linker, blackbox, reproducibility]
```

### E04 Package Ecosystem

```yaml
task_id: E04
requires: [P10]
required: [manifest, lockfile, hash, capability_declaration, isolated_build, sbom]
```

### E05 Self-Hosting

```yaml
task_id: E05
requires: [P10]
order: [formatter, docs, stdlib_subset, graph_tools, frontend, compiler]
bootstrap_rule: old_compiler_builds_new_compiler_then_new_compiler_builds_itself
```

## 13. Diagnostics Contract

诊断结构固定：

```json
{
  "diagnostic_id": "diag_01",
  "code": "E_TYPE_MISMATCH",
  "stage": "type_check",
  "severity": "error",
  "entity_id": "fn_hello",
  "related_entities": ["type_string", "type_u64"],
  "expected": "String",
  "actual": "U64",
  "cause": "handler parameter does not match route parameter",
  "suggested_operations": [
    {"op": "change_parameter_type", "to": "String"}
  ],
  "retryable": true,
  "base_revision": 42
}
```

稳定错误码：

```text
E_SCHEMA_INVALID
E_STALE_REVISION
E_NAME_NOT_FOUND
E_DUPLICATE_NAME
E_TYPE_MISMATCH
E_MISSING_RETURN
E_NON_EXHAUSTIVE_MATCH
E_USE_AFTER_MOVE
E_BORROW_ESCAPE
E_DOUBLE_DROP
E_EFFECT_UNDECLARED
E_CAPABILITY_MISSING
E_ROUTE_COLLISION
E_HTTP_MALFORMED
E_RESOURCE_LIMIT
E_UNSUPPORTED_FEATURE
E_TOOLCHAIN_FAILURE
E_EVIDENCE_INCOMPLETE
```

`retryable=false` 的诊断不得由 AI 自动重试；必须改变任务状态为 `DESIGN_REQUIRED` 或 `BLOCKED`。

## 14. Test Contract

### 14.1 Test Kinds

```yaml
unit: isolated_function_or_operation
snapshot: normalized_graph_or_diagnostic
semantic: interpreter_MIR_behavior
native: compiled_binary_behavior
blackbox: external_process_or_TCP_behavior
fuzz: arbitrary_input_no_crash
fault: injected_resource_or_tool_failure
reproducibility: same_inputs_same_hash
```

### 14.2 Test Independence

黑盒测试不得导入被测实现的路由表、parser、serializer 或内部 helper。验收合同和实现必须来自不同输入路径。测试修改必须通过独立事务记录。

### 14.3 Mandatory Negative Cases

```text
unknown_symbol
wrong_type
missing_return
non_exhaustive_sum_match
use_after_move
borrow_escape
double_drop
undeclared_effect
missing_capability
duplicate_route
invalid_percent_encoding
oversized_header
conflicting_length_headers
read_timeout
partial_write
stale_revision
failed_build
```

## 15. Evidence Contract

每个 VERIFIED 任务生成：

```json
{
  "evidence_id": "ev_43",
  "task_id": "P07",
  "revision": 43,
  "graph_hash": "sha256:...",
  "compiler_hash": "sha256:...",
  "runtime_hash": "sha256:...",
  "target": "x86_64-unknown-linux-gnu",
  "toolchain_lock_hash": "sha256:...",
  "commands": [
    {"id": "build", "argv_hash": "sha256:...", "exit_code": 0}
  ],
  "artifacts": [
    {"path": "build/http_demo", "sha256": "sha256:..."}
  ],
  "tests": [
    {"suite": "http_blackbox", "passed": true, "count": 24}
  ],
  "effects_delta": [],
  "capabilities_delta": ["ListenCapability(127.0.0.1:8080)"],
  "known_limits": []
}
```

缺少 `graph_hash`、`compiler_hash`、`target`、命令退出码或测试清单时，状态不得为 VERIFIED。

## 16. Failure and Recovery

### 16.1 Mutation Failure

```text
candidate graph retained
main revision unchanged
emit diagnostics
return FAILED
```

### 16.2 Build Failure

```text
candidate revision remains non_verified
collect compiler stage and diagnostics
main verified revision remains runnable
return FAILED
```

### 16.3 Test Failure

```text
preserve failing evidence
do not alter acceptance contract
allow retry only with same base revision or explicit new transaction
```

### 16.4 Tool Failure

环境错误允许重试；语义错误不得通过重试掩盖。工具连续失败达到任务配置上限后返回 `BLOCKED`，记录环境、输入哈希和最后状态。

### 16.5 Rollback

`restore(revision)` 只能创建新的回退 revision，不能删除历史 revision。回退只恢复程序图、配置和构建输入，不撤销已发生的外部副作用。

## 17. AI Context Protocol

上下文获取顺序固定：

```text
contract
→ target entities
→ direct dependencies
→ callers
→ related types
→ related tests
→ current diagnostics
→ relevant RFC fragments
```

默认限制：

```yaml
max_context_tokens: task_defined
max_graph_nodes: task_defined
max_tool_calls: task_defined
whole_repository_dump: forbidden
unbounded_log_dump: forbidden
```

上下文不足时返回：

```json
{
  "status": "BLOCKED",
  "code": "CONTEXT_INSUFFICIENT",
  "missing": ["type_hello_body", "test_http_hello"]
}
```

不得通过猜测实体或复制全仓库解决上下文不足。

## 18. AI Task Record

每个执行任务使用以下结构：

```yaml
task_id: string
requires: [task_id]
base_revision: uint64
scope: [entity_id]
inputs: [artifact_or_entity]
operations: [transaction_operation]
acceptance: [predicate_or_command]
resource_budget:
  context_tokens: uint32
  tool_calls: uint32
  cpu_seconds: uint32
  memory_bytes: uint64
outputs: [artifact]
rollback_revision: uint64
status: READY|RUNNING|VERIFIED|FAILED|BLOCKED|DESIGN_REQUIRED|ROLLED_BACK
```

执行器必须先验证 `requires`、`base_revision`、`scope`、`resource_budget` 和 `acceptance`。验证失败时不得调用 `transact`。

## 19. MVP Command Allowlist

仅允许以下命令类别；具体路径由工具传入并校验：

```text
il state
il schema-check
il inspect
il slice
il transact
il validate
il build
il test
il blackbox
il diff
il restore
il evidence
```

禁止 AI 直接调用：

```text
shell -c
curl 任意地址
wget 任意地址
package_install 未锁定依赖
systemctl
kill 非任务进程
git reset --hard
rm -rf
```

## 20. Acceptance Predicates

### G0 Core

```yaml
G0:
  graph_schema_valid: true
  type_checker_verified: true
  ownership_checker_verified: true
  MIR_verifier_verified: true
  interpreter_semantic_suite_passed: true
```

### G1 Native Application

```yaml
G1:
  native_elf_generated: true
  clean_environment_runs: true
  no_high_level_target_source: true
  reproducible_hash: true
```

### G2 AI Modification

```yaml
G2:
  inspect_succeeds: true
  slice_succeeds: true
  transaction_add_route_succeeds: true
  stale_transaction_rejected: true
  failure_preserves_previous_revision: true
```

### G3 Evidence

```yaml
G3:
  blackbox_tests_external: true
  all_artifacts_hashed: true
  diagnostics_machine_readable: true
  rollback_verified: true
```

### G4 Decision Data

```yaml
G4:
  baseline_tasks_complete: true
  target_language_tasks_complete: true
  comparison_record_complete: true
  failure_distribution_recorded: true
```

MVP 只有在 `G0 && G1 && G2 && G3 && G4` 同时为真时才能生成 `release/mvp_manifest.json`。

## 21. Release Manifest

```json
{
  "release": "il-mvp",
  "language_spec_version": "1.0.0",
  "compiler_version": "...",
  "runtime_version": "...",
  "target": "x86_64-unknown-linux-gnu",
  "verified_revision": 0,
  "artifacts": [],
  "test_suites": [],
  "known_limits": [
    "single_request_then_close",
    "loopback_only",
    "no_tls",
    "no_async",
    "no_cross_await_borrow"
  ],
  "reproducibility": {
    "clean_rebuild": true,
    "binary_hashes": []
  }
}
```

发布文件不可修改；修订必须产生新的 release manifest 和新的 revision。

## 22. Final Execution Sequence

调度器执行以下依赖序列，不包含时间计划：

```text
P00
→ P01 + P02
→ P03
→ P04
→ P05
→ P06
→ P07
→ P08
→ P09
→ P10
→ E01/E02/E03/E04/E05 按需求分别排队
```

每个箭头表示 `requires.status == VERIFIED`，不是时间顺序承诺。任何阶段失败都回到该阶段的失败任务，不跳过验证门。

## 23. Terminal Condition

项目终态只能是：

```yaml
project_status:
  MVP_VERIFIED:
    when: G0 && G1 && G2 && G3 && G4
  BLOCKED:
    when: required_tool_or_dependency_missing
  DESIGN_REQUIRED:
    when: locked_semantics_must_change
  FAILED:
    when: unrecoverable_validation_or_build_failure
```

`MVP_VERIFIED` 必须绑定：

```text
verified_revision
compiler_hash
runtime_hash
binary_hash
blackbox_evidence
ai_transaction_evidence
rollback_evidence
```

没有完整绑定关系时，项目状态保持 `NOT_VERIFIED`。
