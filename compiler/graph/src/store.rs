use crate::model::*;
use crate::transaction::*;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Provenance { pub source_git_commit: String, pub source_tree_hash: String }

impl Provenance {
    pub fn validate(&self) -> StoreResult<()> {
        let valid_hash = |value: &str| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
        if !valid_hash(&self.source_git_commit) || !valid_hash(&self.source_tree_hash) {
            return Err(StoreError::new("E_SCHEMA_INVALID", "source provenance requires exact lowercase Git object IDs"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Head { pub revision: u64, pub graph_hash: String, pub manifest_hash: String }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub revision: u64,
    pub parent_revision: Option<u64>,
    pub parent_manifest_hash: Option<String>,
    pub graph_hash: String,
    pub provenance: Provenance,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committed_revision: Option<u64>,
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(formatter, "{}: {}", self.code, self.message) }
}
impl std::error::Error for StoreError {}
impl From<std::io::Error> for StoreError { fn from(error: std::io::Error) -> Self { Self::new("E_TOOLCHAIN_FAILURE", error.to_string()) } }
impl From<serde_json::Error> for StoreError { fn from(error: serde_json::Error) -> Self { Self::new("E_SCHEMA_INVALID", error.to_string()) } }
impl StoreError {
    pub fn new(code: &str, message: impl Into<String>) -> Self { Self { code: code.into(), message: message.into(), committed_revision: None } }
    fn durability(revision: u64, message: impl Into<String>) -> Self {
        Self { code: "E_COMMIT_DURABILITY_UNCERTAIN".into(), message: message.into(), committed_revision: Some(revision) }
    }
}
pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone, Debug)]
pub struct Store { root: PathBuf }

#[derive(Debug, Clone, Serialize)]
pub struct DiffEntry { pub entity_id: String, pub change: String, pub before: Option<serde_json::Value>, pub after: Option<serde_json::Value> }

#[derive(Debug, Clone, Serialize)]
pub struct SliceResult { pub revision: u64, pub entities: Vec<serde_json::Value>, pub truncated: bool, pub missing: Vec<String> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultPoint { BeforeSnapshot, AfterSnapshot, BeforeHeadPublication, AfterHeadPublication }

fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> StoreResult<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

impl Store {
    pub fn initialize(root: impl AsRef<Path>, provenance: Provenance) -> StoreResult<Self> {
        provenance.validate()?;
        let store = Self { root: root.as_ref().to_path_buf() };
        fs::create_dir_all(store.metadata())?;
        let _lock = store.lock()?;
        if store.metadata().join("HEAD").exists() { return Err(StoreError::new("E_STATE_INCONSISTENT", "store already initialized")); }
        fs::create_dir_all(store.metadata().join("revisions"))?;
        fs::create_dir_all(store.metadata().join("experiments"))?;
        // A deterministic initial snapshot can resume missing files; divergent
        // existing bytes are never overwritten by initialization recovery.
        store.publish(&Graph::empty(), None, provenance, "initialize".into(), None)?;
        Ok(store)
    }

    pub fn open(root: impl AsRef<Path>) -> StoreResult<Self> {
        let store = Self { root: root.as_ref().to_path_buf() };
        store.read_head()?;
        Ok(store)
    }

    pub fn root(&self) -> &Path { &self.root }
    fn metadata(&self) -> PathBuf { self.root.join(".il") }
    fn snapshot(&self, revision: u64) -> PathBuf { self.metadata().join("revisions").join(format!("{revision:020}")) }

    fn lock(&self) -> StoreResult<File> {
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(self.metadata().join("lock"))?;
        file.lock_exclusive()?;
        Ok(file)
    }

    fn read_head_unchecked(&self) -> StoreResult<Head> {
        let bytes = fs::read(self.metadata().join("HEAD"))?;
        serde_json::from_slice(&bytes).map_err(|error| StoreError::new("E_STATE_INCONSISTENT", error.to_string()))
    }

    pub fn read_head(&self) -> StoreResult<Head> {
        let head = self.read_head_unchecked()?;
        self.history(&head)?;
        Ok(head)
    }

    fn history(&self, head: &Head) -> StoreResult<Vec<(Manifest, Graph)>> {
        let mut expected_revision = head.revision;
        let mut expected_manifest_hash = head.manifest_hash.clone();
        let mut result = vec![];
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(expected_revision) { return Err(StoreError::new("E_STATE_INCONSISTENT", "snapshot ancestry cycle")); }
            let snapshot = self.snapshot(expected_revision);
            let manifest_bytes = fs::read(snapshot.join("manifest.json")).map_err(|error| StoreError::new("E_STATE_INCONSISTENT", error.to_string()))?;
            if hash_bytes(&manifest_bytes) != expected_manifest_hash { return Err(StoreError::new("E_STATE_INCONSISTENT", "snapshot manifest hash mismatch")); }
            let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|error| StoreError::new("E_STATE_INCONSISTENT", error.to_string()))?;
            manifest.provenance.validate().map_err(|error| StoreError::new("E_STATE_INCONSISTENT", error.message))?;
            let graph_bytes = fs::read(snapshot.join("graph.json")).map_err(|error| StoreError::new("E_STATE_INCONSISTENT", error.to_string()))?;
            if hash_bytes(&graph_bytes) != manifest.graph_hash || manifest.revision != expected_revision {
                return Err(StoreError::new("E_STATE_INCONSISTENT", "snapshot graph hash or revision mismatch"));
            }
            if result.is_empty() && head.graph_hash != manifest.graph_hash { return Err(StoreError::new("E_STATE_INCONSISTENT", "HEAD graph hash mismatch")); }
            let graph = Graph::parse(&graph_bytes).map_err(|error| StoreError::new("E_STATE_INCONSISTENT", error.to_string()))?;
            if graph.revision != expected_revision || graph.canonical_bytes()? != graph_bytes || !graph.validate_structural().is_empty() {
                return Err(StoreError::new("E_STATE_INCONSISTENT", "snapshot graph is invalid or noncanonical"));
            }
            let parent = manifest.parent_revision.zip(manifest.parent_manifest_hash.clone());
            if manifest.parent_revision.is_some() != manifest.parent_manifest_hash.is_some() { return Err(StoreError::new("E_STATE_INCONSISTENT", "incomplete parent binding")); }
            result.push((manifest, graph));
            match parent {
                Some((revision, hash)) if revision < expected_revision => { expected_revision = revision; expected_manifest_hash = hash; }
                None if expected_revision == 0 => break,
                _ => return Err(StoreError::new("E_STATE_INCONSISTENT", "nonmonotonic or missing snapshot parent")),
            }
        }
        Ok(result)
    }

    pub fn provenance(&self) -> StoreResult<Provenance> {
        let head = self.read_head_unchecked()?;
        Ok(self.history(&head)?.remove(0).0.provenance)
    }

    pub fn load(&self, revision: u64) -> StoreResult<Graph> {
        let head = self.read_head_unchecked()?;
        self.history(&head)?.into_iter().find_map(|(manifest, graph)| (manifest.revision == revision).then_some(graph))
            .ok_or_else(|| StoreError::new("E_NAME_NOT_FOUND", "revision is not in published history"))
    }

    fn next_revision(&self, head: u64) -> StoreResult<u64> {
        let mut maximum = head;
        for entry in fs::read_dir(self.metadata().join("revisions"))? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str() {
                if let Ok(revision) = name.parse::<u64>() { maximum = maximum.max(revision); }
            }
        }
        maximum.checked_add(1).ok_or_else(|| StoreError::new("E_RESOURCE_LIMIT", "revision counter exhausted"))
    }

    pub fn transact(&self, transaction: &Transaction, provenance: Provenance, checker: &dyn Fn(&Graph) -> Vec<Diagnostic>) -> StoreResult<TransactionOutcome> {
        self.transact_with_fault(transaction, provenance, checker, None)
    }

    /// Explicit fault-injection seam for durability tests; never accepted from user JSON.
    pub fn transact_with_fault(&self, transaction: &Transaction, provenance: Provenance, checker: &dyn Fn(&Graph) -> Vec<Diagnostic>, fault: Option<FaultPoint>) -> StoreResult<TransactionOutcome> {
        provenance.validate()?;
        let _lock = self.lock()?;
        let head = self.read_head()?;
        let base = self.load(head.revision)?;
        let (mut candidate, mut diagnostics) = apply_transaction(&base, transaction);
        if diagnostics.is_empty() { diagnostics.extend(checker(&candidate)); }
        if !diagnostics.is_empty() {
            let candidate_path = self.retain_candidate(&candidate, transaction, &diagnostics)?;
            return Ok(TransactionOutcome { ok: false, base_revision: transaction.base_revision, result_revision: head.revision,
                diagnostics, graph_hash: head.graph_hash, candidate: Some(candidate_path) });
        }
        candidate.revision = self.next_revision(head.revision)?;
        let new_head = self.publish(&candidate, Some(&head), provenance, format!("transaction:{}", transaction.task_id), fault)?;
        Ok(TransactionOutcome { ok: true, base_revision: head.revision, result_revision: new_head.revision,
            diagnostics: vec![], graph_hash: new_head.graph_hash, candidate: None })
    }

    fn retain_candidate(&self, graph: &Graph, transaction: &Transaction, diagnostics: &[Diagnostic]) -> StoreResult<String> {
        let mut identity = canonical_bytes(transaction)?;
        identity.extend(graph.canonical_bytes()?);
        identity.extend(canonical_bytes(&diagnostics)?);
        let relative = format!(".il/experiments/{}", &hash_bytes(&identity)[7..]);
        let directory = self.root.join(&relative);
        fs::create_dir_all(&directory)?;
        for (name, bytes) in [("graph.json", graph.canonical_bytes()?), ("transaction.json", canonical_bytes(transaction)?), ("diagnostics.json", canonical_bytes(&diagnostics)?)] {
            let path = directory.join(name);
            if path.exists() {
                if fs::read(&path)? != bytes { return Err(StoreError::new("E_STATE_INCONSISTENT", "retained candidate content differs from its identity")); }
            } else {
                write_new(&path, &bytes)?;
            }
        }
        sync_directory(&directory)?;
        sync_directory(&self.metadata().join("experiments"))?;
        Ok(relative)
    }

    /// Keep rejected text input even when parsing could not construct a graph.
    /// The published revision is never changed by this operation.
    pub fn retain_rejected_text(&self, base_revision: u64, request: &serde_json::Value, diagnostics: Vec<Diagnostic>) -> StoreResult<TransactionOutcome> {
        if diagnostics.is_empty() { return Err(StoreError::new("E_SCHEMA_INVALID", "rejected input requires diagnostics")); }
        let _lock = self.lock()?;
        let head = self.read_head()?;
        let request_bytes = canonical_bytes(request)?;
        let diagnostic_bytes = canonical_bytes(&diagnostics)?;
        let mut identity = request_bytes.clone();
        identity.extend(&diagnostic_bytes);
        let relative = format!(".il/experiments/{}", &hash_bytes(&identity)[7..]);
        let directory = self.root.join(&relative);
        fs::create_dir_all(&directory)?;
        for (name, bytes) in [("request.json", request_bytes), ("diagnostics.json", diagnostic_bytes)] {
            let path = directory.join(name);
            if path.exists() {
                if fs::read(&path)? != bytes { return Err(StoreError::new("E_STATE_INCONSISTENT", "rejected input differs from its retained identity")); }
            } else { write_new(&path, &bytes)?; }
        }
        sync_directory(&directory)?;
        sync_directory(&self.metadata().join("experiments"))?;
        Ok(TransactionOutcome { ok: false, base_revision, result_revision: head.revision,
            diagnostics, graph_hash: head.graph_hash, candidate: Some(relative) })
    }

    fn publish(&self, graph: &Graph, parent: Option<&Head>, provenance: Provenance, reason: String, fault: Option<FaultPoint>) -> StoreResult<Head> {
        if fault == Some(FaultPoint::BeforeSnapshot) { return Err(StoreError::new("E_TOOLCHAIN_FAILURE", "injected interruption before snapshot")); }
        let directory = self.snapshot(graph.revision);
        if parent.is_none() { fs::create_dir_all(&directory)?; }
        else { fs::create_dir(&directory)?; }
        let bytes = graph.canonical_bytes()?;
        let graph_hash = hash_bytes(&bytes);
        let manifest = Manifest { revision: graph.revision, parent_revision: parent.map(|head| head.revision),
            parent_manifest_hash: parent.map(|head| head.manifest_hash.clone()), graph_hash: graph_hash.clone(), provenance, reason };
        let manifest_bytes = canonical_bytes(&manifest)?;
        for (name, content) in [("graph.json", bytes.as_slice()), ("manifest.json", manifest_bytes.as_slice())] {
            let path = directory.join(name);
            if parent.is_none() && path.exists() {
                if fs::read(&path)? != content { return Err(StoreError::new("E_STATE_INCONSISTENT", "interrupted initialization differs from required graph or provenance")); }
            } else { write_new(&path, content)?; }
        }
        sync_directory(&directory)?;
        sync_directory(&self.metadata().join("revisions"))?;
        if fault == Some(FaultPoint::AfterSnapshot) { return Err(StoreError::new("E_TOOLCHAIN_FAILURE", "injected interruption after snapshot")); }
        let head = Head { revision: graph.revision, graph_hash, manifest_hash: hash_bytes(&manifest_bytes) };
        let temporary = self.metadata().join("HEAD.next");
        // A prior interrupted publication may leave only this uncommitted pointer.
        if temporary.exists() { fs::remove_file(&temporary)?; }
        write_new(&temporary, &canonical_bytes(&head)?)?;
        if fault == Some(FaultPoint::BeforeHeadPublication) { return Err(StoreError::new("E_TOOLCHAIN_FAILURE", "injected interruption before HEAD publication")); }
        fs::rename(temporary, self.metadata().join("HEAD"))?;
        if fault == Some(FaultPoint::AfterHeadPublication) {
            return Err(StoreError::durability(graph.revision, "HEAD published; injected interruption before directory synchronization; reload HEAD before taking further action"));
        }
        sync_directory(&self.metadata()).map_err(|error| StoreError::durability(graph.revision, format!("HEAD published but directory synchronization failed: {error}; reload HEAD before taking further action")))?;
        Ok(head)
    }

    pub fn restore(&self, revision: u64, reason: &str, provenance: Provenance, checker: &dyn Fn(&Graph) -> Vec<Diagnostic>) -> StoreResult<TransactionOutcome> {
        provenance.validate()?;
        if reason.trim().is_empty() { return Err(StoreError::new("E_SCHEMA_INVALID", "restore reason cannot be empty")); }
        let _lock = self.lock()?;
        let head = self.read_head()?;
        let mut graph = self.load(revision)?;
        let diagnostics = checker(&graph);
        if !diagnostics.is_empty() {
            return Ok(TransactionOutcome { ok: false, base_revision: head.revision, result_revision: head.revision,
                diagnostics, graph_hash: head.graph_hash, candidate: None });
        }
        graph.revision = self.next_revision(head.revision)?;
        let restored = self.publish(&graph, Some(&head), provenance, format!("restore:{revision}:{reason}"), None)?;
        Ok(TransactionOutcome { ok: true, base_revision: head.revision, result_revision: restored.revision,
            diagnostics: vec![], graph_hash: restored.graph_hash, candidate: None })
    }

    pub fn inspect(&self, entity_id: &str, revision: u64) -> StoreResult<serde_json::Value> {
        entity_index(&self.load(revision)?)?.remove(entity_id).ok_or_else(|| StoreError::new("E_NAME_NOT_FOUND", "entity not found at revision"))
    }

    pub fn diff(&self, base: u64, target: u64, scope: &[String]) -> StoreResult<Vec<DiffEntry>> {
        let left = entity_index(&self.load(base)?)?;
        let right = entity_index(&self.load(target)?)?;
        let keys: BTreeSet<_> = left.keys().chain(right.keys()).collect();
        Ok(keys.into_iter().filter(|id| scope.is_empty() || scope.contains(id)).filter_map(|id| {
            let before = left.get(id);
            let after = right.get(id);
            (before != after).then(|| DiffEntry { entity_id: id.clone(), change: match (before, after) { (None, _) => "added", (_, None) => "removed", _ => "modified" }.into(), before: before.cloned(), after: after.cloned() })
        }).collect())
    }

    pub fn slice(&self, roots: &[String], revision: u64, max_nodes: usize, max_bytes: usize) -> StoreResult<SliceResult> {
        if max_nodes == 0 || max_bytes == 0 { return Err(StoreError::new("E_SCHEMA_INVALID", "slice budgets must be positive")); }
        let graph = self.load(revision)?;
        let index = entity_index(&graph)?;
        let mut result = SliceResult { revision, entities: vec![], truncated: false, missing: vec![] };
        let mut queue: VecDeque<_> = roots.iter().cloned().collect();
        let mut visited = BTreeSet::new();
        let mut used = 0usize;
        while let Some(id) = queue.pop_front() {
            if !visited.insert(id.clone()) { continue; }
            let Some(entity) = index.get(&id) else { result.missing.push(id); continue; };
            let size = serde_json::to_vec(entity)?.len();
            if result.entities.len() >= max_nodes || size > max_bytes.saturating_sub(used) {
                result.truncated = true;
                result.missing.push(id);
                continue;
            }
            used += size;
            result.entities.push(entity.clone());
            queue.extend(entity_references(&graph, &id));
        }
        Ok(result)
    }
}

pub fn entity_index(graph: &Graph) -> StoreResult<BTreeMap<String, serde_json::Value>> {
    let mut index = BTreeMap::new();
    fn add<T: Serialize>(index: &mut BTreeMap<String, serde_json::Value>, id: &str, value: &T) -> StoreResult<()> {
        index.insert(id.into(), serde_json::to_value(value)?);
        Ok(())
    }
    for value in &graph.modules { add(&mut index, &value.entity_id, value)?; }
    for value in &graph.types { add(&mut index, &value.entity_id, value)?; }
    for value in &graph.capabilities { add(&mut index, &value.entity_id, value)?; }
    for value in &graph.packages { add(&mut index, &value.entity_id, value)?; }
    for value in &graph.contracts {
        add(&mut index, &value.entity_id, value)?;
        for predicate in &value.predicates {
            if let ContractPredicate::HttpServer { routes, .. } = predicate {
                for route in routes { add(&mut index, &route.entity_id, route)?; }
            }
        }
    }
    for function in &graph.functions {
        add(&mut index, &function.entity_id, function)?;
        for parameter in &function.parameters { add(&mut index, &parameter.entity_id, parameter)?; }
        for block in &function.blocks {
            add(&mut index, &block.entity_id, block)?;
            for argument in &block.arguments { add(&mut index, &argument.entity_id, argument)?; }
            for operation in block.operations.iter().chain(std::iter::once(&block.terminator)) {
                add(&mut index, &operation.entity_id, operation)?;
                for output in &operation.outputs { add(&mut index, &output.entity_id, output)?; }
            }
        }
    }
    Ok(index)
}

pub fn entity_references(graph: &Graph, id: &str) -> Vec<String> {
    for contract in &graph.contracts {
        let mut references = vec![contract.subject.clone()];
        for predicate in &contract.predicates {
            match predicate {
                ContractPredicate::HttpServer { entry, capability, routes, .. } => {
                    references.extend([entry.clone(), capability.clone()]);
                    references.extend(routes.iter().map(|r| r.entity_id.clone()));
                    if let Some(route) = routes.iter().find(|r| r.entity_id == id) { return vec![route.handler.clone()]; }
                }
                ContractPredicate::RequiresCapability { capability } => references.push(capability.clone()),
                ContractPredicate::Returns { type_ref } if !is_builtin_type(type_ref) => references.push(type_ref.clone()),
                _ => {}
            }
        }
        if contract.entity_id == id { references.sort(); references.dedup(); return references; }
    }
    if let Some(module) = graph.modules.iter().find(|module| module.entity_id == id) { return module.imports.iter().chain(&module.declarations).cloned().collect(); }
    if let Some(ty) = graph.types.iter().find(|ty| ty.entity_id == id) { return ty.parameters.iter().chain(ty.fields.iter().map(|f| &f.type_ref)).chain(ty.variants.iter().flat_map(|v| &v.fields)).filter(|name| !is_builtin_type(name)).cloned().collect(); }
    if let Some(function) = graph.functions.iter().find(|function| function.entity_id == id) {
        let mut references: Vec<String> = function.parameters.iter().map(|p| &p.type_ref).chain(std::iter::once(&function.result)).filter(|name| !is_builtin_type(name)).chain(&function.capabilities).chain(&function.contracts).cloned().collect();
        for block in &function.blocks {
            for operation in block.operations.iter().chain(std::iter::once(&block.terminator)) {
                if let Attributes::Call { callee } = &operation.attributes { references.push(callee.clone()); }
                for value in &operation.outputs {
                    if !is_builtin_type(&value.type_ref) { references.push(value.type_ref.clone()); }
                }
            }
        }
        references.sort(); references.dedup();
        return references;
    }
    vec![]
}
