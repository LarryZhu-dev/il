use crate::{execution::Suite, protocol::Failure, source::Context};
use il_graph::{canonical_bytes, hash_bytes, Graph, TARGET};
use il_hir::StageRecord;
use il_object_emitter::Profile;
use il_native_ir::{BuildMode, BuildOptions, RuntimeProfile};
use il_link_driver::LinkPlan;
use il_runtime_startup::HostPolicy;
use serde_json::{json, Value};
use std::{fs::{self, File, OpenOptions}, io::Write, path::{Path, PathBuf}, time::Duration};

fn write(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path).map_err(Failure::io)?;
    file.write_all(bytes).map_err(Failure::io)?;
    file.sync_all().map_err(Failure::io)
}

fn artifact(path: &Path) -> Result<Value, Failure> {
    Ok(json!({"path":path.canonicalize().map_err(Failure::io)?, "sha256":hash_bytes(&fs::read(path).map_err(Failure::io)?)}))
}

fn stage(name: &str, input: &str, output: &str) -> StageRecord {
    StageRecord { stage:name.into(), input_hash:input.into(), output_hash:output.into(),
        compiler_version:env!("CARGO_PKG_VERSION").into(), diagnostics:vec![] }
}

struct Compiled {
    response: Value,
    directory: PathBuf,
    executable: Option<PathBuf>,
    execution_input: Value,
}

fn compile(graph: &Graph, options: BuildOptions, profile: Profile, store: &Path, context: &Context, policy: &HostPolicy) -> Result<Compiled, Failure> {
    let candidates = store.join(".il/builds/candidates");
    fs::create_dir_all(&candidates).map_err(Failure::io)?;
    let directory = tempfile::Builder::new().prefix("native-").tempdir_in(&candidates).map_err(Failure::io)?.keep().canonicalize().map_err(Failure::io)?;
    write(&directory.join("graph.json"), &canonical_bytes(graph).map_err(Failure::json)?)?;
    write(&directory.join("request.json"), &canonical_bytes(&json!({"mode":options.mode,"runtime_profile":options.runtime_profile,"profile":profile})).map_err(Failure::json)?)?;
    #[cfg(unix)]
    File::open(&candidates).and_then(|f| f.sync_all()).map_err(Failure::io)?;
    compile_inner(graph, options, profile, &directory, context, policy).map_err(|mut failure| {
        let record = json!({"status":"FAILED","code":failure.code,"message":failure.message,"diagnostics":failure.diagnostics});
        if let Err(error) = canonical_bytes(&record).map_err(Failure::json).and_then(|bytes| write(&directory.join("failure.json"), &bytes)) {
            failure.message.push_str(&format!("; cannot retain failure record: {}",error.message));
        }
        failure.message.push_str(&format!("; candidate={}",directory.display()));
        failure
    })
}

fn compile_inner(graph: &Graph, options: BuildOptions, profile: Profile, directory: &Path, context: &Context, policy: &HostPolicy) -> Result<Compiled, Failure> {
    let hir = il_hir::lower_with_capabilities(graph, &policy.grants).map_err(Failure::diagnostics)?;
    let mir = il_mir::lower_with_capabilities(&hir, &policy.grants).map_err(Failure::diagnostics)?;
    let diagnostics = il_mir::verify_with_capabilities(&mir, &policy.grants);
    if !diagnostics.is_empty() { return Err(Failure::diagnostics(diagnostics)); }
    let mir_hash = mir.hash().map_err(Failure::json)?;
    let mut stages = vec![hir.stage_record().map_err(Failure::json)?, mir.stage_record().map_err(Failure::json)?,
        stage("verify_mir", &mir_hash, &mir_hash)];
    let program = il_native_ir::lower(&mir, &options, &policy.grants).map_err(Failure::diagnostics)?;
    let diagnostics = il_native_ir::verify(&program);
    if !diagnostics.is_empty() { return Err(Failure::diagnostics(diagnostics)); }
    let native_hash = program.hash().map_err(Failure::json)?;
    stages.push(program.stage_record().map_err(Failure::json)?);
    stages.push(stage("verify_native_ir", &native_hash, &native_hash));
    let llvm = il_codegen_x86_64::emit(&program).map_err(Failure::diagnostics)?;
    stages.push(llvm.stage.clone());
    let compiler = std::env::current_exe().map_err(Failure::io)?;
    let parent = compiler.parent().ok_or_else(|| Failure::input("compiler location has no parent"))?;
    let plan = match options.runtime_profile {
        RuntimeProfile::Full => LinkPlan::Full { archive: parent.join("libil_native_runtime.a") },
        RuntimeProfile::Minimal => LinkPlan::Minimal { archive: parent.join("libil_minimal_runtime.a") },
        RuntimeProfile::None => LinkPlan::None,
    };
    let compiler_hash = hash_bytes(&fs::read(&compiler).map_err(Failure::io)?);
    let runtime_hash = match &plan {
        LinkPlan::Full { archive } | LinkPlan::Minimal { archive } => Some(hash_bytes(&fs::read(archive).map_err(Failure::io)?)),
        LinkPlan::None => None,
    };
    let policy_bytes = canonical_bytes(&serde_json::to_value(policy).map_err(Failure::json)?).map_err(Failure::json)?;
    let policy_hash = hash_bytes(&policy_bytes);
    write(&directory.join("policy.json"), &policy_bytes)?;
    let mut requirements = graph.capabilities.clone();
    requirements.sort_by(|a,b| a.entity_id.cmp(&b.entity_id));
    let lock_hash = hash_bytes(&fs::read(context.repository.join("toolchain.lock")).map_err(Failure::io)?);
    let mut input = json!({"graph_hash":graph.hash().map_err(Failure::json)?, "mir_hash":mir_hash, "native_ir_hash":native_hash,
        "mode":options.mode, "runtime_profile":options.runtime_profile, "requirements":requirements, "policy_hash":policy_hash, "target":TARGET, "profile":profile, "compiler_hash":compiler_hash,
        "runtime_hash":runtime_hash, "toolchain_lock_hash":lock_hash});
    write(&directory.join("compiler-input.json"), &canonical_bytes(&input).map_err(Failure::json)?)?;
    write(&directory.join("native.json"), &program.canonical_bytes().map_err(Failure::json)?)?;
    write(&directory.join("entities.json"), &canonical_bytes(&llvm.entity_map).map_err(Failure::json)?)?;
    let built = il_link_driver::compile(&llvm.ir, directory, &plan, profile)
        .map_err(|error| Failure::new("E_TOOLCHAIN_FAILURE", &format!("{}; candidate={}", error.message, directory.display())))?;
    input["tools"] = json!(built.tools);
    write(&directory.join("input.json"), &canonical_bytes(&input).map_err(Failure::json)?)?;
    // The driver has verified LLVM, emitted the object, linked and validated ELF.
    stages.push(stage("verify_llvm", &built.llvm_ir.sha256, &built.llvm_ir.sha256));
    stages.push(stage("optimize_llvm", &built.llvm_ir.sha256, &built.optimized_ir.sha256));
    stages.push(stage("emit_object", &built.optimized_ir.sha256, &built.object.sha256));
    let link_input = hash_bytes(&canonical_bytes(&json!({"object_hash":built.object.sha256,"runtime_hash":runtime_hash,"profile":profile})).map_err(Failure::json)?);
    if let Some(executable) = &built.executable {
        stages.push(stage("link_runtime", &link_input, &executable.sha256));
        stages.push(stage("emit_elf", &executable.sha256, &executable.sha256));
    }
    let artifacts = json!({"native_ir":artifact(&directory.join("native.json"))?, "llvm_ir":artifact(&built.llvm_ir.path)?,
        "object":artifact(&built.object.path)?, "executable":built.executable.as_ref().map(|item| artifact(&item.path)).transpose()?});
    let record = json!({"schema_version":"1.0.0","status":"VERIFIED","scope":"native_compilation",
        "revision":graph.revision,"source":context.provenance,"input":input,"artifacts":artifacts,
        "driver_record":artifact(&built.record_path)?,"stages":stages});
    let pending = directory.join("verified.pending");
    write(&pending, &canonical_bytes(&record).map_err(Failure::json)?)?;
    fs::rename(&pending, directory.join("verified.json")).map_err(Failure::io)?;
    #[cfg(unix)]
    File::open(&directory).and_then(|f| f.sync_all()).map_err(Failure::io)?;
    let response = json!({"stages":stages,"native":{"profile":profile,"runtime_profile":options.runtime_profile,"target":TARGET,
        "artifacts":artifacts,"build_record":artifact(&directory.join("verified.json"))?}});
    Ok(Compiled {response,directory:directory.to_path_buf(),executable:built.executable.map(|item| item.path),execution_input:input})
}

pub fn build(graph: &Graph, profile: Profile, runtime_profile: RuntimeProfile, exports: Option<Vec<String>>, store: &Path, context: &Context, policy: &HostPolicy) -> Result<Value, Failure> {
    let mode = if runtime_profile == RuntimeProfile::None {
        let entries = exports.filter(|items| !items.is_empty()).ok_or_else(|| Failure::input("runtime_profile none requires nonempty exports"))?;
        BuildMode::Exports { entries }
    } else {
        if exports.is_some() { return Err(Failure::input("exports requires runtime_profile none")); }
        let entries: Vec<_> = graph.functions.iter().filter(|function| function.name == "main").collect();
        if entries.len() != 1 { return Err(Failure::new("E_NAME_NOT_FOUND", "application requires a unique function named main")); }
        BuildMode::Application { entry:entries[0].entity_id.clone() }
    };
    Ok(compile(graph, BuildOptions { mode, runtime_profile }, profile, store, context, policy)?.response)
}

pub fn test(graph: &Graph, suite: Suite, isolation: &str, store: &Path, context: &Context, policy: &HostPolicy) -> Result<Value, Failure> {
    let profile = match isolation { "native_debug"=>Profile::Debug,"native_release"=>Profile::Release,
        _=>return Err(Failure::input("unknown native isolation")) };
    if !il_graph::valid_id(&suite.entry) || !suite.limits.valid() { return Err(Failure::input("invalid native suite entry or limits")); }
    let mode = BuildMode::Captured { entry:suite.entry.clone(), arguments:suite.arguments.clone(), limits:suite.limits };
    let mut compiled = compile(graph, BuildOptions { mode, runtime_profile:RuntimeProfile::Full }, profile, store, context, policy)?;
    compiled.execution_input.as_object_mut().unwrap().remove("mode");
    for (key,value) in [("entry",json!(suite.entry)),("arguments",json!(suite.arguments)),("limits",json!(suite.limits)),("isolation",json!(isolation))] {
        compiled.execution_input[key] = value;
    }
    let executable = compiled.executable.as_ref().ok_or_else(|| Failure::input("captured execution requires executable"))?;
    let run = il_link_driver::run(executable, &compiled.directory.join("execution.json"), &compiled.directory.join("policy.json"), Duration::from_secs(30))
        .map_err(|error| Failure::new("E_TOOLCHAIN_FAILURE", &format!("{}; candidate={}", error.message, compiled.directory.display())))?;
    let result_hash = hash_bytes(&canonical_bytes(&run.execution).map_err(Failure::json)?);
    let input_hash = hash_bytes(&canonical_bytes(&compiled.execution_input).map_err(Failure::json)?);
    let mut execution_stage = stage("execute_native", &input_hash, &result_hash);
    execution_stage.diagnostics = run.execution.diagnostics.clone();
    compiled.response["stages"].as_array_mut().unwrap().push(json!(execution_stage));
    compiled.response["native"]["argv"] = json!([executable]);
    compiled.response["native"]["report_fd"] = json!(3);
    compiled.response["native"]["policy_fd"] = json!(4);
    compiled.response["native"]["exit_code"] = json!(run.exit_code);
    compiled.response["execution"] = json!(run.execution);
    compiled.response["execution_input"] = compiled.execution_input;
    write(&compiled.directory.join("test-result.json"), &canonical_bytes(&compiled.response).map_err(Failure::json)?)?;
    Ok(compiled.response)
}
