use crate::protocol::Failure;
use il_graph::{canonical_bytes, hash_bytes, valid_id, Graph};
use il_hir::StageRecord;
use il_interpreter::execute_with_policy;
use il_runtime_startup::HostPolicy;
use il_execution_model::{Limits, Value};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub entry: String,
    pub arguments: Vec<Value>,
    pub limits: Limits,
}

pub fn run(graph: &Graph, suite: Suite, policy: &HostPolicy) -> Result<serde_json::Value, Failure> {
    if !valid_id(&suite.entry) { return Err(Failure::input("entry must be a stable function ID")); }
    let hir = il_hir::lower_with_capabilities(graph, &policy.grants).map_err(Failure::diagnostics)?;
    let mir = il_mir::lower_with_capabilities(&hir, &policy.grants).map_err(Failure::diagnostics)?;
    let mut stages = vec![hir.stage_record().map_err(Failure::json)?, mir.stage_record().map_err(Failure::json)?];
    let diagnostics = il_mir::verify_with_capabilities(&mir, &policy.grants);
    if !diagnostics.is_empty() { return Err(Failure::diagnostics(diagnostics)); }
    let mir_hash = mir.hash().map_err(Failure::json)?;
    stages.push(StageRecord { stage: "verify_mir".into(), input_hash: mir_hash.clone(), output_hash: mir_hash.clone(),
        compiler_version: env!("CARGO_PKG_VERSION").into(), diagnostics: vec![] });
    let execution_input = json!({"mir_hash": mir_hash, "entry": suite.entry,
        "arguments": suite.arguments, "limits": suite.limits, "isolation": "captured", "policy_hash": hash_bytes(&canonical_bytes(&serde_json::to_value(policy).map_err(Failure::json)?).map_err(Failure::json)?)});
    let execution = execute_with_policy(&mir, &suite.entry, &suite.arguments, suite.limits, policy);
    stages.push(StageRecord { stage: "execute_mir".into(), input_hash: hash_bytes(&canonical_bytes(&execution_input).map_err(Failure::json)?),
        output_hash: hash_bytes(&canonical_bytes(&execution).map_err(Failure::json)?),
        compiler_version: env!("CARGO_PKG_VERSION").into(), diagnostics: execution.diagnostics.clone() });
    Ok(json!({"execution": execution, "execution_input": execution_input, "stages": stages}))
}
