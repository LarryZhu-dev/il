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
    AddType { type_definition: TypeDef },
    AddFunction { function: Function },
    ReplaceFunction { function: Function },
    RemoveEntity { entity_id: EntityId },
}

impl TransactionOperation {
    pub fn entity_id(&self) -> &str {
        match self {
            Self::AddModule { module } => &module.entity_id,
            Self::RemoveModule { entity_id } | Self::RenameModule { entity_id, .. } |
            Self::SetModuleVisibility { entity_id, .. } | Self::SetModuleImports { entity_id, .. } |
            Self::RemoveEntity { entity_id } => entity_id,
            Self::AddType { type_definition } => &type_definition.entity_id,
            Self::AddFunction { function } | Self::ReplaceFunction { function } => &function.entity_id,
        }
    }

    pub fn semantic(&self) -> bool {
        matches!(self, Self::AddType { .. } | Self::AddFunction { .. } | Self::ReplaceFunction { .. } | Self::RemoveEntity { .. }) ||
            matches!(self, Self::AddModule { module } if !module.declarations.is_empty())
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

/// P02 is deliberately limited to operations fully checked without P03.
pub fn apply_structural(base: &Graph, transaction: &Transaction) -> (Graph, Vec<Diagnostic>) {
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
    if transaction.required_checks.iter().any(|check| !matches!(check, Check::Schema | Check::Names | Check::References)) {
        reject("E_UNSUPPORTED_FEATURE", None, "semantic checks require the VERIFIED P03 checker");
    }
    if base.has_semantics() { reject("E_UNSUPPORTED_FEATURE", None, "semantic graph requires the VERIFIED P03 checker"); }
    for operation in &transaction.operations {
        if !scopes.contains(&operation.entity_id().to_owned()) { reject("E_INVALID_SCOPE", Some(operation.entity_id()), "modified entity is outside the exact transaction scope"); }
        if operation.semantic() { reject("E_UNSUPPORTED_FEATURE", Some(operation.entity_id()), "semantic edits require the VERIFIED P03 checker"); }
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
            _ => unreachable!("semantic operations were rejected before applying edits"),
        }
    }
    diagnostics.extend(candidate.validate_structural());
    (candidate, diagnostics)
}
