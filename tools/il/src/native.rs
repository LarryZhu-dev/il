use crate::{execution::Suite, protocol::Failure, source::Context};
use il_graph::{canonical_bytes, hash_bytes, Graph, TARGET};
use il_hir::StageRecord;
use il_object_emitter::Profile;
use il_native_ir::BuildMode;
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
    executable: PathBuf,
    execution_input: Value,
}

fn compile(graph: &Graph, mode: BuildMode, profile: Profile, store: &Path, context: &Context) -> Result<Compiled, Failure> {
    let candidates = store.join(".il/builds/candidates");
    fs::create_dir_all(&candidates).map_err(Failure::io)?;
    let directory = tempfile::Builder::new().prefix("native-").tempdir_in(&candidates).map_err(Failure::io)?.keep().canonicalize().map_err(Failure::io)?;
    write(&directory.join("graph.json"), &canonical_bytes(graph).map_err(Failure::json)?)?;
    write(&directory.join("request.json"), &canonical_bytes(&json!({"mode":mode,"profile":profile})).map_err(Failure::json)?)?;
    #[cfg(unix)]
    File::open(&candidates).and_then(|f| f.sync_all()).map_err(Failure::io)?;
    compile_inner(graph, mode, profile, &directory, context).map_err(|mut failure| {
        let record = json!({"status":"FAILED","code":failure.code,"message":failure.message,"diagnostics":failure.diagnostics});
        if let Err(error) = canonical_bytes(&record).map_err(Failure::json).and_then(|bytes| write(&directory.join("failure.json"), &bytes)) {
            failure.message.push_str(&format!("; cannot retain failure record: {}",error.message));
        }
        failure.message.push_str(&format!("; candidate={}",directory.display()));
        failure
    })
}

fn compile_inner(graph: &Graph, mode: BuildMode, profile: Profile, directory: &Path, context: &Context) -> Result<Compiled, Failure> {
    let hir = il_hir::lower(graph).map_err(Failure::diagnostics)?;
    let mir = il_mir::lower(&hir).map_err(Failure::diagnostics)?;
    let diagnostics = il_mir::verify(&mir);
    if !diagnostics.is_empty() { return Err(Failure::diagnostics(diagnostics)); }
    let mir_hash = mir.hash().map_err(Failure::json)?;
    let mut stages = vec![hir.stage_record().map_err(Failure::json)?, mir.stage_record().map_err(Failure::json)?,
        stage("verify_mir", &mir_hash, &mir_hash)];
    let program = il_native_ir::lower(&mir, &mode).map_err(Failure::diagnostics)?;
    let diagnostics = il_native_ir::verify(&program);
    if !diagnostics.is_empty() { return Err(Failure::diagnostics(diagnostics)); }
    let native_hash = program.hash().map_err(Failure::json)?;
    stages.push(program.stage_record().map_err(Failure::json)?);
    stages.push(stage("verify_native_ir", &native_hash, &native_hash));
    let llvm = il_codegen_x86_64::emit(&program).map_err(Failure::diagnostics)?;
    stages.push(llvm.stage.clone());
    let compiler = std::env::current_exe().map_err(Failure::io)?;
    let runtime = compiler.parent().ok_or_else(|| Failure::input("compiler location has no parent"))?.join("libil_native_runtime.a");
    let compiler_hash = hash_bytes(&fs::read(&compiler).map_err(Failure::io)?);
    let runtime_hash = hash_bytes(&fs::read(&runtime).map_err(Failure::io)?);
    let lock_hash = hash_bytes(&fs::read(context.repository.join("toolchain.lock")).map_err(Failure::io)?);
    let mut input = json!({"graph_hash":graph.hash().map_err(Failure::json)?, "mir_hash":mir_hash, "native_ir_hash":native_hash,
        "mode":mode, "target":TARGET, "profile":profile, "compiler_hash":compiler_hash,
        "runtime_hash":runtime_hash, "toolchain_lock_hash":lock_hash});
    write(&directory.join("compiler-input.json"), &canonical_bytes(&input).map_err(Failure::json)?)?;
    write(&directory.join("native.json"), &program.canonical_bytes().map_err(Failure::json)?)?;
    write(&directory.join("entities.json"), &canonical_bytes(&llvm.entity_map).map_err(Failure::json)?)?;
    let built = il_link_driver::compile(&llvm.ir, &directory, &runtime, profile)
        .map_err(|error| Failure::new("E_TOOLCHAIN_FAILURE", &format!("{}; candidate={}", error.message, directory.display())))?;
    input["tools"] = json!(built.tools);
    write(&directory.join("input.json"), &canonical_bytes(&input).map_err(Failure::json)?)?;
    // The driver has verified LLVM, emitted the object, linked and validated ELF.
    stages.push(stage("verify_llvm", &built.llvm_ir.sha256, &built.llvm_ir.sha256));
    stages.push(stage("optimize_llvm", &built.llvm_ir.sha256, &built.optimized_ir.sha256));
    stages.push(stage("emit_object", &built.optimized_ir.sha256, &built.object.sha256));
    let link_input = hash_bytes(&canonical_bytes(&json!({"object_hash":built.object.sha256,"runtime_hash":runtime_hash,"profile":profile})).map_err(Failure::json)?);
    stages.push(stage("link_runtime", &link_input, &built.executable.sha256));
    stages.push(stage("emit_elf", &built.executable.sha256, &built.executable.sha256));
    let artifacts = json!({"native_ir":artifact(&directory.join("native.json"))?, "llvm_ir":artifact(&built.llvm_ir.path)?,
        "object":artifact(&built.object.path)?, "executable":artifact(&built.executable.path)?});
    let record = json!({"schema_version":"1.0.0","status":"VERIFIED","scope":"native_compilation",
        "revision":graph.revision,"source":context.provenance,"input":input,"artifacts":artifacts,
        "driver_record":artifact(&built.record_path)?,"stages":stages});
    let pending = directory.join("verified.pending");
    write(&pending, &canonical_bytes(&record).map_err(Failure::json)?)?;
    fs::rename(&pending, directory.join("verified.json")).map_err(Failure::io)?;
    #[cfg(unix)]
    File::open(&directory).and_then(|f| f.sync_all()).map_err(Failure::io)?;
    let response = json!({"stages":stages,"native":{"profile":profile,"target":TARGET,
        "artifacts":artifacts,"build_record":artifact(&directory.join("verified.json"))?}});
    Ok(Compiled {response,directory:directory.to_path_buf(),executable:built.executable.path,execution_input:input})
}

pub fn build(graph: &Graph, profile: Profile, store: &Path, context: &Context) -> Result<Value, Failure> {
    let entries: Vec<_> = graph.functions.iter().filter(|function| function.name == "main").collect();
    if entries.len() != 1 { return Err(Failure::new("E_NAME_NOT_FOUND", "application requires a unique function named main")); }
    Ok(compile(graph, BuildMode::Application { entry:entries[0].entity_id.clone() }, profile, store, context)?.response)
}

pub fn test(graph: &Graph, suite: Suite, isolation: &str, store: &Path, context: &Context) -> Result<Value, Failure> {
    let profile = match isolation { "native_debug"=>Profile::Debug,"native_release"=>Profile::Release,
        _=>return Err(Failure::input("unknown native isolation")) };
    if !il_graph::valid_id(&suite.entry) || !suite.limits.valid() { return Err(Failure::input("invalid native suite entry or limits")); }
    let mode = BuildMode::Captured { entry:suite.entry.clone(), arguments:suite.arguments.clone(), limits:suite.limits };
    let mut compiled = compile(graph, mode, profile, store, context)?;
    compiled.execution_input.as_object_mut().unwrap().remove("mode");
    for (key,value) in [("entry",json!(suite.entry)),("arguments",json!(suite.arguments)),("limits",json!(suite.limits)),("isolation",json!(isolation))] {
        compiled.execution_input[key] = value;
    }
    let run = il_link_driver::run(&compiled.executable, &compiled.directory.join("execution.json"), Duration::from_secs(30))
        .map_err(|error| Failure::new("E_TOOLCHAIN_FAILURE", &format!("{}; candidate={}", error.message, compiled.directory.display())))?;
    let result_hash = hash_bytes(&canonical_bytes(&run.execution).map_err(Failure::json)?);
    let input_hash = hash_bytes(&canonical_bytes(&compiled.execution_input).map_err(Failure::json)?);
    let mut execution_stage = stage("execute_native", &input_hash, &result_hash);
    execution_stage.diagnostics = run.execution.diagnostics.clone();
    compiled.response["stages"].as_array_mut().unwrap().push(json!(execution_stage));
    compiled.response["native"]["argv"] = json!([compiled.executable]);
    compiled.response["native"]["report_fd"] = json!(3);
    compiled.response["native"]["exit_code"] = json!(run.exit_code);
    compiled.response["execution"] = json!(run.execution);
    compiled.response["execution_input"] = compiled.execution_input;
    write(&compiled.directory.join("test-result.json"), &canonical_bytes(&compiled.response).map_err(Failure::json)?)?;
    Ok(compiled.response)
}
