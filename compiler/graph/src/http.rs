//! HTTP declarations elaborate to ordinary, auditable graph functions.
use crate::*;
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;

fn error(graph: &Graph, code: &str, id: &str, cause: impl Into<String>) -> Diagnostic {
    Diagnostic::error(code, Some(id), cause, graph.revision)
}

fn parameter_segment(segment: &str) -> Option<&str> {
    segment.strip_prefix('{').and_then(|s| s.strip_suffix('}'))
}

fn pattern_parameter(path: &str) -> Result<Option<&str>, ()> {
    if !path.starts_with('/') || path.len() > 4096 || path.chars().any(char::is_control) { return Err(()); }
    let mut parameter = None;
    for segment in path.split('/') {
        if segment.contains(['{', '}']) {
            let name = parameter_segment(segment).ok_or(())?;
            let mut bytes = name.bytes();
            if !matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
                || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_') || parameter.is_some() { return Err(()); }
            parameter = Some(name);
        }
    }
    Ok(parameter)
}

fn overlaps(left: &str, right: &str) -> bool {
    let left: Vec<_> = left.split('/').collect();
    let right: Vec<_> = right.split('/').collect();
    left.len() == right.len() && left.iter().zip(right).all(|(a, b)|
        *a == b || parameter_segment(a).is_some() && !b.is_empty() || parameter_segment(b).is_some() && !a.is_empty())
}

fn entry_reaches(graph: &Graph, entry: &str, matches: impl Fn(&Operation) -> bool) -> bool {
    let mut pending = vec![entry];
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) { continue; }
        let Some(function) = graph.functions.iter().find(|f| f.entity_id == id) else { continue; };
        for operation in function.blocks.iter().flat_map(|b| b.operations.iter().chain(std::iter::once(&b.terminator))) {
            if matches(operation) { return true; }
            if let Attributes::Call { callee } = &operation.attributes { pending.push(callee); }
        }
    }
    false
}

fn body_nodes(function: &Function) -> usize {
    function.blocks.iter().fold(0usize, |count, block| count.saturating_add(1 + block.arguments.len())
        .saturating_add(block.operations.iter().chain(std::iter::once(&block.terminator))
            .fold(0usize, |count, op| count.saturating_add(1 + op.outputs.len()))))
}

fn expansion_budget(graph: &Graph) -> Result<(), Diagnostic> {
    const MAX_NODES: usize = 100_000;
    let limit = || error(graph, "E_RESOURCE_LIMIT", "program", "HTTP declaration expansion budget exceeded");
    if graph.types.len() > 256 { return Err(limit()); }
    let mut nodes = 0usize;
    let mut ids = BTreeSet::new();
    for id in graph.modules.iter().map(|v| &v.entity_id).chain(graph.types.iter().map(|v| &v.entity_id))
        .chain(graph.capabilities.iter().map(|v| &v.entity_id)).chain(graph.packages.iter().map(|v| &v.entity_id))
        .chain(graph.contracts.iter().map(|v| &v.entity_id)).chain(graph.functions.iter().flat_map(|function| {
            std::iter::once(&function.entity_id).chain(function.parameters.iter().map(|p| &p.entity_id))
                .chain(function.blocks.iter().flat_map(|block| std::iter::once(&block.entity_id)
                    .chain(block.arguments.iter().map(|a| &a.entity_id)).chain(block.operations.iter()
                        .chain(std::iter::once(&block.terminator)).flat_map(|op| std::iter::once(&op.entity_id).chain(op.outputs.iter().map(|v| &v.entity_id))))))
        })) {
        nodes += 1;
        if nodes > MAX_NODES { return Err(limit()); }
        ids.insert(id.as_str());
    }
    for ty in &graph.types {
        nodes = nodes.saturating_add(ty.parameters.len()).saturating_add(ty.fields.len());
        for variant in &ty.variants { nodes = nodes.saturating_add(1).saturating_add(variant.fields.len()); }
        if nodes > MAX_NODES { return Err(limit()); }
    }
    let mut subjects = BTreeSet::new();
    let mut added = 0usize;
    let mut replaced = 0usize;
    for contract in &graph.contracts {
        nodes = nodes.saturating_add(contract.predicates.len());
        if nodes > MAX_NODES { return Err(limit()); }
        for predicate in &contract.predicates {
            if let ContractPredicate::HttpServer { routes, .. } = predicate {
                if !subjects.insert(&contract.subject) { return Err(error(graph, "E_HTTP_DECLARATION", &contract.entity_id, "dispatcher has more than one HTTP declaration")); }
                if routes.is_empty() || routes.len() > 128 { return Err(error(graph, "E_HTTP_DECLARATION", &contract.entity_id, "HTTP declaration requires between 1 and 128 routes")); }
                for route in routes {
                    if !valid_id(&route.entity_id) || route.entity_id == "program" { return Err(error(graph, "E_SCHEMA_INVALID", &route.entity_id, "invalid HTTP route entity identifier")); }
                    if !ids.insert(&route.entity_id) { return Err(error(graph, "E_DUPLICATE_NAME", &route.entity_id, "duplicate HTTP route entity identifier")); }
                    nodes = nodes.saturating_add(1).saturating_add(route.parameters.len());
                }
                // Includes operation outputs and block arguments, with an upper bound
                // independent of the user-provided body that will be replaced.
                added = added.saturating_add(routes.len().saturating_mul(64)).saturating_add(16);
                if let Some(function) = graph.functions.iter().find(|f| f.entity_id == contract.subject) { replaced = replaced.saturating_add(body_nodes(function)); }
                if nodes > MAX_NODES || added > MAX_NODES { return Err(limit()); }
            }
        }
    }
    if nodes.saturating_sub(replaced).saturating_add(added) > MAX_NODES { return Err(limit()); }
    Ok(())
}

/// Validate the bounded declaration before any generated graph is allocated.
pub fn validate_declarations(graph: &Graph) -> Vec<Diagnostic> {
    if !graph.contracts.iter().any(|c| c.predicates.iter().any(|p| matches!(p, ContractPredicate::HttpServer { .. }))) { return vec![]; }
    if let Err(diagnostic) = expansion_budget(graph) { return vec![diagnostic]; }
    let mut diagnostics = vec![];
    let mut subjects = BTreeSet::new();
    for contract in &graph.contracts {
        for predicate in &contract.predicates {
            let ContractPredicate::HttpServer { entry, bind, capability, routes } = predicate else { continue; };
            let mut reject = |code, id: &str, cause: &str| diagnostics.push(error(graph, code, id, cause));
            if !subjects.insert(&contract.subject) { reject("E_HTTP_DECLARATION", &contract.entity_id, "dispatcher has more than one HTTP declaration"); }
            let subject = graph.functions.iter().find(|f| f.entity_id == contract.subject);
            match subject {
                None => reject("E_NAME_NOT_FOUND", &contract.entity_id, "HTTP dispatcher function does not exist"),
                Some(function) if function.parameters.len() != 2 || function.parameters.iter().any(|p| p.type_ref != "Bytes") || function.result != "http.ResponseResult" =>
                    reject("E_TYPE_MISMATCH", &contract.entity_id, "HTTP dispatcher requires (Bytes, Bytes) -> http.ResponseResult"),
                _ => {}
            }
            match graph.functions.iter().find(|f| &f.entity_id == entry) {
                None => reject("E_NAME_NOT_FOUND", &contract.entity_id, "HTTP entry function does not exist"),
                Some(function) => {
                    if !function.capabilities.contains(capability) { reject("E_CAPABILITY_MISSING", &contract.entity_id, "HTTP entry must declare its Listen capability"); }
                    if !entry_reaches(graph, entry, |op| matches!(&op.attributes, Attributes::RuntimeCall { symbol, capability: Some(grant) } if symbol == "net_listen" && grant == capability)) {
                        reject("E_HTTP_DECLARATION", &contract.entity_id, "HTTP entry must reach net_listen using the declared Listen grant");
                    }
                    if !entry_reaches(graph, entry, |op| matches!(&op.attributes, Attributes::Call { callee } if callee == &contract.subject)) {
                        reject("E_HTTP_DECLARATION", &contract.entity_id, "HTTP entry must reach a call to the declared dispatcher");
                    }
                }
            }
            let canonical_bind = bind.parse::<SocketAddr>().ok().is_some_and(|address| address.port() != 0
                && address.to_string() == *bind
                && match address { SocketAddr::V6(v6) => v6.scope_id() == 0 && v6.flowinfo() == 0, _ => true });
            if !canonical_bind { reject("E_HTTP_DECLARATION", &contract.entity_id, "HTTP bind must be a canonical numeric endpoint with a nonzero port"); }
            match graph.capabilities.iter().find(|c| &c.entity_id == capability) {
                None => reject("E_NAME_NOT_FOUND", &contract.entity_id, "HTTP Listen capability does not exist"),
                Some(grant) if grant.kind != CapabilityKind::Listen || grant.scope.as_ref() != Some(bind) =>
                    reject("E_CAPABILITY_MISSING", &contract.entity_id, "HTTP bind must equal the selected Listen grant scope"),
                _ => {}
            }
            if routes.is_empty() || routes.len() > 128 { reject("E_HTTP_DECLARATION", &contract.entity_id, "HTTP declaration requires between 1 and 128 routes"); continue; }
            for route in routes {
                if route.method != "GET" { reject("E_HTTP_DECLARATION", &route.entity_id, "MVP HTTP route method must be GET"); }
                let parameter = match pattern_parameter(&route.path) {
                    Ok(parameter) => parameter,
                    Err(()) => { reject("E_HTTP_DECLARATION", &route.entity_id, "HTTP pattern must be an absolute decoded path with at most one whole-segment parameter"); continue; }
                };
                let parameters_valid = match parameter {
                    None => route.parameters.is_empty(),
                    Some(name) => route.parameters.len() == 1 && route.parameters[0] == (HttpParameter {
                        name: name.into(), type_ref: "String".into(), source: "path".into(), max_utf8_bytes: 128,
                    }),
                };
                if !parameters_valid { reject("E_HTTP_DECLARATION", &route.entity_id, "HTTP parameter schema must exactly describe the String path capture with a 128-byte bound"); }
                let request = if parameter.is_some() { "http.PathRequest" } else { "http.EmptyRequest" };
                match graph.functions.iter().find(|f| f.entity_id == route.handler) {
                    None => reject("E_NAME_NOT_FOUND", &route.entity_id, "HTTP route handler does not exist"),
                    Some(handler) if handler.parameters.len() != 1 || handler.parameters[0].type_ref != request || handler.result != "http.ResponseResult" =>
                        reject("E_TYPE_MISMATCH", &route.entity_id, "HTTP handler signature does not match the declared request and response types"),
                    _ => {}
                }
            }
            for (index, route) in routes.iter().enumerate() {
                for previous in &routes[..index] {
                    if pattern_parameter(&route.path).is_ok() && pattern_parameter(&previous.path).is_ok() && overlaps(&route.path, &previous.path) {
                        reject("E_ROUTE_COLLISION", &route.entity_id, "HTTP route patterns overlap");
                    }
                }
            }
            if !graph.functions.iter().any(|f| f.entity_id == "http.match_path") { reject("E_NAME_NOT_FOUND", &contract.entity_id, "HTTP declaration requires http.match_path"); }
            for type_id in ["http.ResponseResult", "http.HttpError", "http.PathMatch", "http.MatchResult", "http.PathRequest", "http.EmptyRequest"] {
                if !graph.types.iter().any(|ty| ty.entity_id == type_id) { reject("E_NAME_NOT_FOUND", &contract.entity_id, &format!("HTTP declaration requires {type_id}")); }
            }
        }
    }
    diagnostics
}

pub fn expected_dispatchers(graph: &Graph) -> Result<Vec<Function>, Vec<Diagnostic>> {
    let diagnostics = validate_declarations(graph);
    if !diagnostics.is_empty() { return Err(diagnostics); }
    let mut result = vec![];
    for contract in &graph.contracts {
        for predicate in &contract.predicates {
            if let ContractPredicate::HttpServer { routes, .. } = predicate {
                let function = graph.functions.iter().find(|f| f.entity_id == contract.subject).expect("validated dispatcher");
                result.push(dispatcher(graph, function, routes));
            }
        }
    }
    Ok(result)
}

/// Replace only generated bodies. Effect and capability declarations remain explicit.
pub fn elaborate(graph: &mut Graph) -> Vec<Diagnostic> {
    match expected_dispatchers(graph) {
        Err(diagnostics) => diagnostics,
        Ok(functions) => {
            for function in functions {
                let target = graph.functions.iter_mut().find(|f| f.entity_id == function.entity_id).expect("validated dispatcher");
                *target = function;
            }
            vec![]
        }
    }
}

struct Builder<'a> {
    graph: &'a Graph,
    values: BTreeMap<String, String>,
    owned: BTreeSet<String>,
    blocks: Vec<Block>,
}

impl<'a> Builder<'a> {
    fn new(graph: &'a Graph, function: &Function) -> Self {
        let mut owned: BTreeSet<_> = ["String".to_owned(), "Bytes".to_owned()].into_iter().collect();
        for _ in 0..=graph.types.len() {
            let mut changed = false;
            for ty in &graph.types {
                if matches!(ty.kind, TypeKind::String | TypeKind::Bytes) || ty.layout == Layout::Opaque ||
                    ty.parameters.iter().chain(ty.fields.iter().map(|f| &f.type_ref)).chain(ty.variants.iter().flat_map(|v| &v.fields)).any(|t| owned.contains(t)) {
                    changed |= owned.insert(ty.entity_id.clone());
                }
            }
            if !changed { break; }
        }
        Self { graph, values: function.parameters.iter().map(|p| (p.entity_id.clone(), p.type_ref.clone())).collect(), owned, blocks: vec![] }
    }

    fn value(&mut self, id: &str, ty: &str) -> ValueDef {
        self.values.insert(id.into(), ty.into());
        ValueDef { entity_id: id.into(), type_ref: ty.into() }
    }

    fn op(&mut self, id: &str, opcode: Opcode, inputs: Vec<String>, output: Option<&str>, attributes: Attributes) -> Operation {
        let outputs: Vec<_> = output.map(|ty| self.value(&format!("{id}.value"), ty)).into_iter().collect();
        let produces: Vec<_> = outputs.iter().filter(|v| self.owned.contains(&v.type_ref)).cloned().collect();
        let mut effects = vec![];
        if opcode == Opcode::Const && !produces.is_empty() || opcode == Opcode::Clone && inputs.iter().any(|id| self.owned.contains(&self.values[id])) { effects.push(Effect::Alloc); }
        if let Attributes::Call { callee } = &attributes { effects = self.graph.functions.iter().find(|f| &f.entity_id == callee).expect("validated callee").effects.clone(); }
        let mut transfers = match opcode {
            Opcode::Call | Opcode::Payload | Opcode::Variant | Opcode::Return | Opcode::Branch => inputs.clone(),
            _ => vec![],
        };
        match &attributes {
            Attributes::Switch { cases, default_arguments, .. } => transfers.extend(cases.iter().flat_map(|c| c.arguments.clone()).chain(default_arguments.clone())),
            Attributes::CondBranch { then_arguments, else_arguments, .. } => transfers.extend(then_arguments.iter().chain(else_arguments).cloned()),
            _ => {}
        }
        let consumes = transfers.into_iter().filter(|id| self.owned.contains(&self.values[id])).collect::<BTreeSet<_>>().into_iter().collect();
        Operation { entity_id: id.into(), opcode, inputs, outputs, attributes, effects, consumes, produces }
    }

    fn block(&mut self, id: String, arguments: Vec<ValueDef>, operations: Vec<Operation>, terminator: Operation) {
        self.blocks.push(Block { entity_id: id, arguments, operations, terminator });
    }
}

fn output(operation: &Operation) -> String { operation.outputs[0].entity_id.clone() }
fn variant(ty: &str, tag: &str) -> Attributes { Attributes::Variant { type_id: ty.into(), variant: tag.into() } }

fn dispatcher(graph: &Graph, function: &Function, routes: &[HttpRoute]) -> Function {
    let mut builder = Builder::new(graph, function);
    let prefix = format!("{}.http", function.entity_id);
    let route_prefix = |route: &HttpRoute| format!("{prefix}.r{}", &hash_bytes(format!("{}\0{}", function.entity_id, route.entity_id).as_bytes())[7..]);
    let missing = format!("{prefix}.not_found");
    let method_error = format!("{prefix}.method_error");
    let invalid = format!("{prefix}.invalid");
    let method = &function.parameters[0].entity_id;
    let path = &function.parameters[1].entity_id;
    let mut routes: Vec<_> = routes.iter().collect();
    routes.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
    for (index, route) in routes.iter().enumerate() {
        let prefix = route_prefix(route);
        let test = format!("{prefix}.test");
        let ok = format!("{prefix}.ok");
        let err = format!("{prefix}.err");
        let matched = format!("{prefix}.matched");
        let invoke = format!("{prefix}.invoke");
        let next = routes.get(index + 1).map(|r| format!("{}.test", route_prefix(r))).unwrap_or_else(|| missing.clone());
        let copy = builder.op(&format!("{test}.copy"), Opcode::Clone, vec![path.clone()], Some("Bytes"), Attributes::Empty {});
        let pattern = builder.op(&format!("{test}.pattern"), Opcode::Const, vec![], Some("Bytes"), Attributes::Constant { value: Literal::Bytes(route.path.as_bytes().to_vec()) });
        let call = builder.op(&format!("{test}.match"), Opcode::Call, vec![output(&copy), output(&pattern)], Some("http.MatchResult"), Attributes::Call { callee: "http.match_path".into() });
        let switch = builder.op(&format!("{test}.switch"), Opcode::Switch, vec![output(&call)], None, Attributes::Switch {
            cases: vec![SwitchCase { tag: "Ok".into(), target: ok.clone(), arguments: vec![output(&call)] }, SwitchCase { tag: "Err".into(), target: err.clone(), arguments: vec![output(&call)] }],
            default: invalid.clone(), default_arguments: vec![],
        });
        builder.block(test, vec![], vec![copy, pattern, call], switch);

        let argument = builder.value(&format!("{ok}.input"), "http.MatchResult");
        let payload = builder.op(&format!("{ok}.payload"), Opcode::Payload, vec![argument.entity_id.clone()], Some("http.PathMatch"), Attributes::Empty {});
        let switch = builder.op(&format!("{ok}.switch"), Opcode::Switch, vec![output(&payload)], None, Attributes::Switch {
            cases: vec![SwitchCase { tag: "Miss".into(), target: next, arguments: vec![] }, SwitchCase { tag: "Match".into(), target: matched.clone(), arguments: vec![output(&payload)] }],
            default: invalid.clone(), default_arguments: vec![],
        });
        builder.block(ok, vec![argument], vec![payload], switch);

        let argument = builder.value(&format!("{err}.input"), "http.MatchResult");
        let payload = builder.op(&format!("{err}.payload"), Opcode::Payload, vec![argument.entity_id.clone()], Some("http.HttpError"), Attributes::Empty {});
        let result = builder.op(&format!("{err}.result"), Opcode::Variant, vec![output(&payload)], Some("http.ResponseResult"), variant("http.ResponseResult", "Err"));
        let ret = builder.op(&format!("{err}.return"), Opcode::Return, vec![output(&result)], None, Attributes::Empty {});
        builder.block(err, vec![argument], vec![payload, result], ret);

        let argument = builder.value(&format!("{matched}.input"), "http.PathMatch");
        let capture = builder.op(&format!("{matched}.capture"), Opcode::Payload, vec![argument.entity_id.clone()], Some("String"), Attributes::Empty {});
        let get = builder.op(&format!("{matched}.get"), Opcode::Const, vec![], Some("Bytes"), Attributes::Constant { value: Literal::Bytes(b"GET".to_vec()) });
        let equal = builder.op(&format!("{matched}.equal"), Opcode::RuntimeCall, vec![method.clone(), output(&get)], Some("Bool"), Attributes::RuntimeCall { symbol: "bytes_equal".into(), capability: None });
        let branch = builder.op(&format!("{matched}.branch"), Opcode::CondBranch, vec![output(&equal)], None, Attributes::CondBranch {
            then_block: invoke.clone(), else_block: method_error.clone(), then_arguments: if route.parameters.is_empty() { vec![] } else { vec![output(&capture)] }, else_arguments: vec![],
        });
        builder.block(matched, vec![argument], vec![capture, get, equal], branch);

        let arguments: Vec<_> = if route.parameters.is_empty() { vec![] } else { vec![builder.value(&format!("{invoke}.capture"), "String")] };
        let (ty, tag) = if arguments.is_empty() { ("http.EmptyRequest", "Empty") } else { ("http.PathRequest", "Path") };
        let request = builder.op(&format!("{invoke}.request"), Opcode::Variant, arguments.iter().map(|v| v.entity_id.clone()).collect(), Some(ty), variant(ty, tag));
        let response = builder.op(&format!("{invoke}.handler"), Opcode::Call, vec![output(&request)], Some("http.ResponseResult"), Attributes::Call { callee: route.handler.clone() });
        let ret = builder.op(&format!("{invoke}.return"), Opcode::Return, vec![output(&response)], None, Attributes::Empty {});
        builder.block(invoke, arguments, vec![request, response], ret);
    }
    for (block, status) in [(missing, 404), (method_error, 405)] {
        let status = builder.op(&format!("{block}.status"), Opcode::Const, vec![], Some("U16"), Attributes::Constant { value: Literal::Integer(status) });
        let protocol = builder.op(&format!("{block}.protocol"), Opcode::Variant, vec![output(&status)], Some("http.HttpError"), variant("http.HttpError", "Protocol"));
        let result = builder.op(&format!("{block}.result"), Opcode::Variant, vec![output(&protocol)], Some("http.ResponseResult"), variant("http.ResponseResult", "Err"));
        let ret = builder.op(&format!("{block}.return"), Opcode::Return, vec![output(&result)], None, Attributes::Empty {});
        builder.block(block, vec![], vec![status, protocol, result], ret);
    }
    let trap = builder.op(&format!("{invalid}.trap"), Opcode::Trap, vec![], None, Attributes::Trap { code: "E_INVALID_DISCRIMINANT".into() });
    builder.block(invalid, vec![], vec![], trap);
    let mut result = function.clone();
    result.blocks = builder.blocks;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function(id: &str, parameters: &[&str], result: &str) -> Function {
        Function {
            entity_id: id.into(), name: id.into(), parameters: parameters.iter().enumerate().map(|(i, ty)| Parameter {
                entity_id: format!("{id}.p{i}"), name: format!("p{i}"), type_ref: (*ty).into(),
            }).collect(), result: result.into(), effects: vec![Effect::Alloc], capabilities: vec![], contracts: vec![],
            blocks: vec![Block { entity_id: format!("{id}.body"), arguments: vec![], operations: vec![], terminator: Operation {
                entity_id: format!("{id}.trap"), opcode: Opcode::Trap, inputs: vec![], outputs: vec![], attributes: Attributes::Trap { code: "E_TEST".into() }, effects: vec![], consumes: vec![], produces: vec![],
            } }],
        }
    }

    fn fixture() -> Graph {
        let mut graph = Graph::empty();
        for (id, kind, parameters, variants) in [
            ("http.EmptyRequest", TypeKind::Sum, vec![], vec![("Empty", vec![])]),
            ("http.PathRequest", TypeKind::Sum, vec![], vec![("Path", vec!["String"])]),
            ("http.HttpError", TypeKind::Sum, vec![], vec![("Protocol", vec!["U16"])]),
            ("http.Response", TypeKind::Sum, vec![], vec![("Response", vec!["U16", "String", "Bytes"])]),
            ("http.PathMatch", TypeKind::Sum, vec![], vec![("Miss", vec![]), ("Match", vec!["String"])]),
            ("http.MatchResult", TypeKind::Result, vec!["http.PathMatch", "http.HttpError"], vec![]),
            ("http.ResponseResult", TypeKind::Result, vec!["http.Response", "http.HttpError"], vec![]),
        ] {
            graph.types.push(TypeDef { entity_id: id.into(), kind, parameters: parameters.into_iter().map(str::to_owned).collect(), layout: Layout::Inferred, integer: None,
                fields: vec![], variants: variants.into_iter().map(|(name, fields)| Variant { name: name.into(), fields: fields.into_iter().map(str::to_owned).collect() }).collect() });
        }
        graph.functions = vec![function("app.dispatch", &["Bytes", "Bytes"], "http.ResponseResult"), function("http.match_path", &["Bytes", "Bytes"], "http.MatchResult"),
            function("app.health", &["http.EmptyRequest"], "http.ResponseResult"), function("app.hello", &["http.PathRequest"], "http.ResponseResult")];
        let mut entry = function("app.entry", &["Bytes", "Bytes"], "http.ResponseResult");
        entry.capabilities = vec!["listen".into()];
        let mut builder = Builder::new(&graph, &entry);
        let call = builder.op("app.entry.call", Opcode::Call, entry.parameters.iter().map(|p| p.entity_id.clone()).collect(), Some("http.ResponseResult"), Attributes::Call { callee: "app.dispatch".into() });
        let ret = builder.op("app.entry.return", Opcode::Return, vec![output(&call)], None, Attributes::Empty {});
        let listen = builder.op("app.entry.listen", Opcode::RuntimeCall, vec![], None, Attributes::RuntimeCall { symbol: "net_listen".into(), capability: Some("listen".into()) });
        entry.blocks[0].operations = vec![listen, call]; entry.blocks[0].terminator = ret;
        graph.functions.push(entry);
        graph.capabilities.push(Capability { entity_id: "listen".into(), kind: CapabilityKind::Listen, scope: Some("127.0.0.1:8080".into()) });
        graph.contracts.push(Contract { entity_id: "server".into(), subject: "app.dispatch".into(), predicates: vec![ContractPredicate::HttpServer {
            entry: "app.entry".into(), bind: "127.0.0.1:8080".into(), capability: "listen".into(), routes: vec![
                HttpRoute { entity_id: "health".into(), method: "GET".into(), path: "/health".into(), handler: "app.health".into(), parameters: vec![] },
                HttpRoute { entity_id: "hello".into(), method: "GET".into(), path: "/hello/{name}".into(), handler: "app.hello".into(), parameters: vec![HttpParameter { name: "name".into(), type_ref: "String".into(), source: "path".into(), max_utf8_bytes: 128 }] },
            ],
        }] });
        graph
    }

    fn routes(graph: &mut Graph) -> &mut Vec<HttpRoute> {
        let ContractPredicate::HttpServer { routes, .. } = &mut graph.contracts[0].predicates[0] else { unreachable!() }; routes
    }

    #[test]
    fn generated_body_has_stable_ids_and_recursive_ownership_metadata() {
        let mut graph = fixture();
        assert!(elaborate(&mut graph).is_empty());
        assert!(graph.validate_structural().is_empty(), "{:?}", graph.validate_structural());
        let expected = graph.functions[0].clone();
        routes(&mut graph).reverse();
        assert!(elaborate(&mut graph).is_empty());
        assert_eq!(graph.functions[0], expected);
        for block in &expected.blocks {
            if block.terminator.opcode == Opcode::Return { assert_eq!(block.terminator.consumes, block.terminator.inputs); }
        }
        let index = entity_index(&graph).unwrap();
        assert!(index.contains_key("hello"));
        assert_eq!(entity_references(&graph, "hello"), ["app.hello"]);
    }

    #[test]
    fn collisions_and_closed_parameter_contract_are_rejected() {
        let mut graph = fixture();
        routes(&mut graph)[0].path = "/hello/world".into();
        assert!(validate_declarations(&graph).iter().any(|d| d.code == "E_ROUTE_COLLISION"));
        routes(&mut graph)[0].path = "/hello/".into();
        assert!(validate_declarations(&graph).is_empty());
        routes(&mut graph)[1].parameters[0].max_utf8_bytes = 129;
        assert!(validate_declarations(&graph).iter().any(|d| d.code == "E_HTTP_DECLARATION"));
    }

    #[test]
    fn endpoint_and_handler_signatures_are_checked_before_expansion() {
        let mut graph = fixture();
        let ContractPredicate::HttpServer { bind, .. } = &mut graph.contracts[0].predicates[0] else { unreachable!() };
        *bind = "localhost:8080".into();
        assert!(elaborate(&mut graph).iter().any(|d| d.code == "E_HTTP_DECLARATION"));
        let mut graph = fixture();
        graph.functions[3].parameters[0].type_ref = "http.EmptyRequest".into();
        assert!(elaborate(&mut graph).iter().any(|d| d.code == "E_TYPE_MISMATCH"));
        assert_eq!(graph.functions[0].blocks.len(), 1);
    }

    #[test]
    fn transitive_generated_changes_require_explicit_dispatcher_scope() {
        let mut graph = fixture();
        assert!(elaborate(&mut graph).is_empty());
        let mut handler = graph.functions[2].clone(); handler.effects.push(Effect::Clock);
        let mut tx = Transaction { task_id: "edit".into(), base_revision: 0, scope: vec!["app.health".into()], operations: vec![TransactionOperation::ReplaceFunction { function: handler }], required_checks: vec![] };
        let (_, diagnostics) = apply_transaction(&graph, &tx);
        assert!(diagnostics.iter().any(|d| d.code == "E_INVALID_SCOPE"));
        tx.scope.push("app.dispatch".into());
        let (candidate, diagnostics) = apply_transaction(&graph, &tx);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(candidate.functions[0].effects, vec![Effect::Alloc]);
        assert!(candidate.functions[0].blocks.iter().flat_map(|b| &b.operations).any(|op| op.effects.contains(&Effect::Clock)));
    }

    #[test]
    fn empty_routes_and_unbound_entry_are_rejected() {
        let mut graph = fixture(); routes(&mut graph).clear();
        assert!(elaborate(&mut graph).iter().any(|d| d.code == "E_HTTP_DECLARATION"));
        assert_eq!(graph.functions[0].blocks.len(), 1);
        let mut graph = fixture();
        graph.functions.last_mut().unwrap().blocks[0].operations.remove(0);
        assert!(elaborate(&mut graph).iter().any(|d| d.cause.contains("net_listen")));
    }

    #[test]
    fn expansion_budget_and_route_identity_are_checked_before_generation() {
        let mut graph = fixture();
        routes(&mut graph)[0].entity_id = "app.health".into();
        assert!(elaborate(&mut graph).iter().any(|d| d.code == "E_DUPLICATE_NAME"));
        let mut graph = fixture();
        let template = graph.contracts[0].clone();
        graph.contracts.clear();
        for server in 0..13 {
            let mut contract = template.clone();
            contract.entity_id = format!("server{server}");
            contract.subject = format!("dispatch{server}");
            let ContractPredicate::HttpServer { routes, .. } = &mut contract.predicates[0] else { unreachable!() };
            let template = routes[0].clone();
            *routes = (0..128).map(|index| HttpRoute { entity_id: format!("route{server}_{index}"), path: format!("/{index}"), ..template.clone() }).collect();
            graph.contracts.push(contract);
        }
        let diagnostics = elaborate(&mut graph);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "E_RESOURCE_LIMIT");
        assert_eq!(graph.functions[0].blocks.len(), 1);
    }
}
