use crate::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub fn lower(hir: &il_hir::Program) -> Result<Program, Vec<Diagnostic>> { lower_with_capabilities(hir, &[]) }

pub fn lower_with_capabilities(hir: &il_hir::Program, capabilities: &[Capability]) -> Result<Program, Vec<Diagnostic>> {
    let graph = &hir.graph;
    let mut diagnostics = il_checker::check_with_capabilities(graph, capabilities);
    if hir.schema_version != "1.0.0" || hir.compiler_version != env!("CARGO_PKG_VERSION") || graph.hash().ok().as_ref() != Some(&hir.input_hash) {
        diagnostics.push(Diagnostic::error("E_SCHEMA_INVALID", None, "HIR version or source hash does not match its payload", graph.revision));
    }
    if !diagnostics.is_empty() { return Err(diagnostics); }
    let mut functions = vec![];
    for function in &graph.functions { functions.push(lower_function(graph, function)?); }
    let program = Program { schema_version: "1.0.0".into(), compiler_version: env!("CARGO_PKG_VERSION").into(),
        input_hash: hir.hash().map_err(|error| vec![Diagnostic::error("E_SCHEMA_INVALID", None, error.to_string(), graph.revision)])?, source_revision: graph.revision,
        target: graph.target.clone(), types: graph.types.clone(), capabilities: graph.capabilities.clone(),
        public_functions: graph.functions.iter().filter(|function|graph.modules.iter().any(|module|module.visibility==Visibility::Public&&module.declarations.contains(&function.entity_id))).map(|function|function.entity_id.clone()).collect(), functions };
    let diagnostics = crate::verify::verify_with_capabilities(&program, capabilities);
    if diagnostics.is_empty() { Ok(program) } else { Err(diagnostics) }
}

fn lower_function(graph: &Graph, function: &il_graph::Function) -> Result<Function, Vec<Diagnostic>> {
    let Some(entry) = function.blocks.first() else { return Err(vec![Diagnostic::error("E_MISSING_RETURN", Some(&function.entity_id), "function has no entry", graph.revision)]); };
    let owned = |ty: &str| il_checker::is_owned_type(graph, ty);
    let parameter_types: BTreeMap<_, _> = function.parameters.iter().map(|parameter| (parameter.entity_id.clone(), parameter.type_ref.clone())).collect();
    let initial: BTreeSet<_> = function.parameters.iter().filter(|parameter| owned(&parameter.type_ref)).map(|parameter| parameter.entity_id.clone()).collect();
    let mut incoming = BTreeMap::from([(entry.entity_id.clone(), initial)]);
    let source_blocks: BTreeMap<_, _> = function.blocks.iter().map(|block| (block.entity_id.as_str(), block)).collect();
    let mut queue = VecDeque::from([entry.entity_id.clone()]);
    let mut blocks = BTreeMap::new();
    while let Some(id) = queue.pop_front() {
        if blocks.contains_key(&id) { continue; }
        let Some(source) = source_blocks.get(id.as_str()) else { continue; };
        let mut live = incoming.get(&id).cloned().unwrap_or_default();
        let mut types = parameter_types.clone();
        let mut order: Vec<_> = function.parameters.iter().filter(|parameter| owned(&parameter.type_ref)).map(|parameter| parameter.entity_id.clone()).collect();
        let parameter_count = order.len();
        for argument in &source.arguments {
            types.insert(argument.entity_id.clone(), argument.type_ref.clone());
            if owned(&argument.type_ref) { live.insert(argument.entity_id.clone()); order.push(argument.entity_id.clone()); }
        }
        for operation in &source.operations {
            for input in &operation.consumes { live.remove(input); }
            for output in &operation.outputs {
                types.insert(output.entity_id.clone(), output.type_ref.clone());
                if owned(&output.type_ref) && !matches!(operation.opcode, Opcode::Borrow | Opcode::BorrowMut) { live.insert(output.entity_id.clone()); order.push(output.entity_id.clone()); }
            }
        }
        let cleanup = |current: &BTreeSet<String>, all: bool| -> Vec<DropAction> {
            order.iter().skip(if all { 0 } else { parameter_count }).rev().filter(|id| current.contains(*id)).filter_map(|id| types.get(id).map(|ty| DropAction { value_id: id.clone(), type_ref: ty.clone() })).collect()
        };
        let edge = |target: &str, arguments: &[String]| -> Edge {
            let mut remaining = live.clone();
            for argument in arguments { remaining.remove(argument); }
            Edge { target: target.into(), arguments: arguments.to_vec(), cleanup: cleanup(&remaining, false) }
        };
        let operation = &source.terminator;
        let entity_id = operation.entity_id.clone();
        let terminator = match (&operation.opcode, &operation.attributes) {
            (Opcode::Return, _) => {
                let value = operation.inputs.first().cloned();
                let mut remaining = live.clone();
                if let Some(value) = &value { remaining.remove(value); }
                Terminator::Return { entity_id, value, cleanup: cleanup(&remaining, true) }
            }
            (Opcode::Trap, Attributes::Trap { code }) => Terminator::Trap { entity_id, code: code.clone() },
            (Opcode::Branch, Attributes::Branch { target }) => Terminator::Branch { entity_id, edge: edge(target, &operation.inputs) },
            (Opcode::CondBranch, Attributes::CondBranch { then_block, else_block, then_arguments, else_arguments }) => Terminator::CondBranch {
                entity_id, condition: operation.inputs.first().cloned().unwrap_or_default(), then_edge: edge(then_block, then_arguments), else_edge: edge(else_block, else_arguments),
            },
            (Opcode::Switch, Attributes::Switch { cases, default, default_arguments }) => Terminator::Switch {
                entity_id, value: operation.inputs.first().cloned().unwrap_or_default(),
                cases: cases.iter().map(|case| SwitchEdge { tag: case.tag.clone(), edge: edge(&case.target, &case.arguments) }).collect(), default: edge(default, default_arguments),
            },
            _ => return Err(vec![Diagnostic::error("E_SCHEMA_INVALID", Some(&operation.entity_id), "unlowered or invalid graph terminator", graph.revision)]),
        };
        for edge in terminator.edges() {
            let mut parameters: BTreeSet<_> = live.iter().filter(|id| parameter_types.contains_key(*id)).cloned().collect();
            for argument in &edge.arguments { parameters.remove(argument); }
            match incoming.get(&edge.target) {
                Some(previous) if previous != &parameters => return Err(vec![Diagnostic::error("E_OWNERSHIP_JOIN", Some(&edge.target), "inconsistent parameter lifetime during MIR lowering", graph.revision)]),
                None => { incoming.insert(edge.target.clone(), parameters); queue.push_back(edge.target.clone()); }
                _ => {}
            }
        }
        blocks.insert(id, Block { entity_id: source.entity_id.clone(), arguments: source.arguments.clone(), operations: source.operations.clone(), terminator });
    }
    let blocks = function.blocks.iter().filter_map(|block| blocks.remove(&block.entity_id)).collect();
    Ok(Function { entity_id: function.entity_id.clone(), name: function.name.clone(), parameters: function.parameters.clone(), result: function.result.clone(),
        effects: function.effects.clone(), capabilities: function.capabilities.clone(), blocks })
}
