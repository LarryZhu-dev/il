use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Check { Schema, Names, References, Types, Ownership, Effects, Capabilities, Contracts }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub task_id: String,
    pub base_revision: u64,
    pub scope: Vec<EntityId>,
    pub operations: Vec<TransactionOperation>,
    pub required_checks: Vec<Check>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransactionOperation {
    AddModule { module: Module },
    RemoveModule { entity_id: EntityId },
    RenameModule { entity_id: EntityId, path: String },
    SetModuleVisibility { entity_id: EntityId, visibility: Visibility },
    SetModuleImports { entity_id: EntityId, imports: Vec<EntityId> },
    SetModuleDeclarations { entity_id: EntityId, declarations: Vec<EntityId> },
    AddType { type_definition: TypeDef },
    AddFunction { function: Function },
    ReplaceFunction { function: Function },
    RemoveEntity { entity_id: EntityId },
    ReplaceProgram { graph: Graph },
}

impl TransactionOperation {
    pub fn entity_id(&self) -> &str {
        match self {
            Self::AddModule { module } => &module.entity_id,
            Self::RemoveModule { entity_id } | Self::RenameModule { entity_id, .. } |
            Self::SetModuleVisibility { entity_id, .. } | Self::SetModuleImports { entity_id, .. } |
            Self::SetModuleDeclarations { entity_id, .. } | Self::RemoveEntity { entity_id } => entity_id,
            Self::AddType { type_definition } => &type_definition.entity_id,
            Self::AddFunction { function } | Self::ReplaceFunction { function } => &function.entity_id,
            Self::ReplaceProgram { .. } => "program",
        }
    }

}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TransactionOutcome {
    pub ok: bool,
    pub base_revision: u64,
    pub result_revision: u64,
    pub diagnostics: Vec<Diagnostic>,
    pub graph_hash: String,
    pub candidate: Option<String>,
}

/// Apply edits to a private candidate. Publication additionally requires the caller's
/// mandatory semantic checker; requested checks can never disable validation.
pub fn apply_transaction(base: &Graph, transaction: &Transaction) -> (Graph, Vec<Diagnostic>) {
    let mut candidate = base.clone();
    let mut diagnostics = vec![];
    let mut reject = |code, entity: Option<&str>, reason: &str| diagnostics.push(Diagnostic::error(code, entity, reason, base.revision));
    if transaction.base_revision != base.revision { reject("E_STALE_REVISION", None, "base revision differs from published HEAD"); }
    if !valid_id(&transaction.task_id) { reject("E_SCHEMA_INVALID", None, "invalid task identifier"); }
    if transaction.operations.is_empty() { reject("E_SCHEMA_INVALID", None, "transaction operations cannot be empty"); }
    let scopes: BTreeSet<_> = transaction.scope.iter().collect();
    if scopes.len() != transaction.scope.len() || transaction.scope.iter().any(|id| !valid_id(id)) { reject("E_SCHEMA_INVALID", None, "scope must contain unique exact logical IDs"); }
    if transaction.required_checks.iter().enumerate().any(|(index, check)| transaction.required_checks[..index].contains(check)) {
        reject("E_SCHEMA_INVALID", None, "required checks must not contain duplicates");
    }
    for operation in &transaction.operations {
        if !scopes.contains(&operation.entity_id().to_owned()) { reject("E_INVALID_SCOPE", Some(operation.entity_id()), "modified entity is outside the exact transaction scope"); }
        if let TransactionOperation::ReplaceProgram { graph } = operation {
            if graph.revision != base.revision { reject("E_STALE_REVISION", Some("program"), "imported program must identify the transaction base revision"); }
        }
    }
    drop(reject);
    if !diagnostics.is_empty() { return (candidate, diagnostics); }
    for operation in &transaction.operations {
        let id = operation.entity_id();
        let existing = candidate.modules.iter().position(|module| module.entity_id == id);
        match operation {
            TransactionOperation::AddModule { module } => candidate.modules.push(module.clone()),
            TransactionOperation::RemoveModule { .. } => {
                if let Some(index) = existing { candidate.modules.remove(index); }
                else { diagnostics.push(Diagnostic::error("E_NAME_NOT_FOUND", Some(id), "module to remove does not exist", base.revision)); }
            }
            TransactionOperation::RenameModule { path, .. } => {
                if let Some(index) = existing { candidate.modules[index].path = path.clone(); }
                else { diagnostics.push(Diagnostic::error("E_NAME_NOT_FOUND", Some(id), "module to rename does not exist", base.revision)); }
            }
            TransactionOperation::SetModuleVisibility { visibility, .. } => {
                if let Some(index) = existing { candidate.modules[index].visibility = *visibility; }
                else { diagnostics.push(Diagnostic::error("E_NAME_NOT_FOUND", Some(id), "module does not exist", base.revision)); }
            }
            TransactionOperation::SetModuleImports { imports, .. } => {
                if let Some(index) = existing { candidate.modules[index].imports = imports.clone(); }
                else { diagnostics.push(Diagnostic::error("E_NAME_NOT_FOUND", Some(id), "module does not exist", base.revision)); }
            }
            TransactionOperation::SetModuleDeclarations { declarations, .. } => {
                if let Some(index) = existing { candidate.modules[index].declarations = declarations.clone(); }
                else { diagnostics.push(Diagnostic::error("E_NAME_NOT_FOUND", Some(id), "module does not exist", base.revision)); }
            }
            TransactionOperation::AddType { type_definition } => candidate.types.push(type_definition.clone()),
            TransactionOperation::AddFunction { function } => candidate.functions.push(function.clone()),
            TransactionOperation::ReplaceFunction { function } => {
                if let Some(existing) = candidate.functions.iter_mut().find(|value| value.entity_id == function.entity_id) { *existing = function.clone(); }
                else { diagnostics.push(Diagnostic::error("E_NAME_NOT_FOUND", Some(id), "function to replace does not exist", base.revision)); }
            }
            TransactionOperation::RemoveEntity { entity_id } => {
                let before = candidate.modules.len() + candidate.types.len() + candidate.functions.len() + candidate.contracts.len() + candidate.packages.len();
                candidate.modules.retain(|v| v.entity_id != *entity_id);
                candidate.types.retain(|v| v.entity_id != *entity_id);
                candidate.functions.retain(|v| v.entity_id != *entity_id);
                candidate.contracts.retain(|v| v.entity_id != *entity_id);
                candidate.packages.retain(|v| v.entity_id != *entity_id);
                let after = candidate.modules.len() + candidate.types.len() + candidate.functions.len() + candidate.contracts.len() + candidate.packages.len();
                if before == after { diagnostics.push(Diagnostic::error("E_NAME_NOT_FOUND", Some(id), "removable entity does not exist", base.revision)); }
            }
            TransactionOperation::ReplaceProgram { graph } => candidate = graph.clone(),
        }
    }
    diagnostics.extend(candidate.validate_structural());
    (candidate, diagnostics)
}
