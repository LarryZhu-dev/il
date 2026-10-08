use crate::lexer::{lex, Kind, Token};
use il_graph::*;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
mod surface;
mod lowering;

type Result<T> = std::result::Result<T, Diagnostic>;

pub fn parse(source: &str, revision: u64) -> std::result::Result<Graph, Vec<Diagnostic>> {
    let tokens = lex(source).map_err(|(offset, message)| vec![diagnostic(revision, "E_SCHEMA_INVALID", offset, &message)])?;
    let mut parser = Parser { tokens, at: 0, revision, counter: 0, graph: Graph::empty(),
        names: BTreeMap::new(), pending: vec![], depth: 0 };
    parser.graph.revision = revision;
    parser.document().map_err(|error| vec![error])?;
    parser.finish().map_err(|error| vec![error])?;
    Ok(parser.graph)
}

fn diagnostic(revision: u64, code: &str, offset: usize, message: &str) -> Diagnostic {
    let mut result = Diagnostic::error(code, None, format!("byte {offset}: {message}"), revision);
    result.stage = "text_parse".into();
    result
}

struct Parser {
    tokens: Vec<Token>, at: usize, revision: u64, counter: u64,
    graph: Graph, names: BTreeMap<String, String>, pending: Vec<Pending>, depth: usize,
}

struct Pending { function: usize, start: usize, end: usize, scope: String }

impl Parser {
    fn token(&self) -> &Token { &self.tokens[self.at] }
    fn error(&self, code: &str, message: &str) -> Diagnostic { diagnostic(self.revision, code, self.token().offset, message) }
    fn is(&self, expected: &str) -> bool {
        matches!(&self.token().kind, Kind::Name(value) | Kind::Symbol(value) if value == expected)
    }
    fn eat(&mut self, expected: &str) -> bool { if self.is(expected) { self.at += 1; true } else { false } }
    fn expect(&mut self, expected: &str) -> Result<()> {
        if self.eat(expected) { Ok(()) } else { Err(self.error("E_SCHEMA_INVALID", &format!("expected {expected}"))) }
    }
    fn name(&mut self) -> Result<String> {
        match self.token().kind.clone() {
            Kind::Name(value) | Kind::String(value) => { self.at += 1; Ok(value) }
            _ => Err(self.error("E_SCHEMA_INVALID", "expected a name or quoted name")),
        }
    }
    fn annotation(&mut self) -> Result<Option<String>> {
        if !self.eat("@") { return Ok(None); }
        self.expect("id")?; self.expect("(")?;
        let id = match self.token().kind.clone() {
            Kind::String(value) => { self.at += 1; value },
            _ => return Err(self.error("E_SCHEMA_INVALID", "@id requires a JSON string")),
        };
        self.expect(")")?;
        Ok(Some(id))
    }
    fn allocate(&mut self, owner: &str) -> String {
        let value = format!("{owner}.new_r{}_{}", self.revision, self.counter);
        self.counter += 1;
        value
    }
    fn id(&mut self, annotation: Option<String>, owner: &str) -> String { annotation.unwrap_or_else(|| self.allocate(owner)) }
    fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > 32 { Err(self.error("E_RESOURCE_LIMIT", "nesting depth exceeds 32")) } else { Ok(()) }
    }
    fn json(&mut self) -> Result<Value> {
        self.enter()?;
        let result = self.json_inner();
        self.depth -= 1;
        result
    }
    fn json_inner(&mut self) -> Result<Value> {
        if self.eat("[") {
            let mut values = vec![];
            if !self.eat("]") { loop {
                values.push(self.json()?);
                if self.eat("]") { break; }
                self.expect(",")?;
            }}
            return Ok(Value::Array(values));
        }
        if self.eat("{") {
            let mut values = Map::new();
            if !self.eat("}") { loop {
                let key = match self.token().kind.clone() {
                    Kind::String(value) => { self.at += 1; value },
                    _ => return Err(self.error("E_SCHEMA_INVALID", "JSON object keys must be quoted")),
                };
                self.expect(":")?;
                if values.contains_key(&key) { return Err(self.error("E_SCHEMA_INVALID", "duplicate JSON object key")); }
                values.insert(key, self.json()?);
                if self.eat("}") { break; }
                self.expect(",")?;
            }}
            return Ok(Value::Object(values));
        }
        if self.eat("null") { return Ok(Value::Null); }
        if self.eat("true") { return Ok(Value::Bool(true)); }
        if self.eat("false") { return Ok(Value::Bool(false)); }
        if let Kind::String(value) = self.token().kind.clone() { self.at += 1; return Ok(Value::String(value)); }
        let negative = self.eat("-");
        if let Kind::Number(value) = self.token().kind.clone() {
            self.at += 1;
            let text = if negative { format!("-{value}") } else { value };
            return serde_json::from_str(&text).map_err(|_| self.error("E_SCHEMA_INVALID", "integer literal exceeds supported range"));
        }
        Err(self.error("E_SCHEMA_INVALID", "expected a JSON value"))
    }
    fn typed_json<T: DeserializeOwned>(&mut self) -> Result<T> {
        let value = self.json()?;
        serde_json::from_value(value).map_err(|error| self.error("E_SCHEMA_INVALID", &error.to_string()))
    }
    fn list(&mut self) -> Result<Vec<String>> {
        self.expect("[")?;
        let mut values = vec![];
        if !self.eat("]") { loop {
            values.push(self.name()?);
            if self.eat("]") { break; } self.expect(",")?;
        }}
        Ok(values)
    }
    fn effects(&mut self) -> Result<Vec<Effect>> {
        self.list()?.into_iter().map(|name| serde_json::from_value(Value::String(name))
            .map_err(|_| self.error("E_SCHEMA_INVALID", "unknown effect"))).collect()
    }
    fn enum_name<T: DeserializeOwned>(&mut self) -> Result<T> {
        let name = self.name()?;
        serde_json::from_value(Value::String(name)).map_err(|error| self.error("E_SCHEMA_INVALID", &error.to_string()))
    }
    fn register(&mut self, scope: &str, name: &str, id: &str) -> Result<()> {
        let key = format!("{scope}::{name}");
        if self.names.insert(key, id.into()).is_some() { return Err(self.error("E_DUPLICATE_NAME", "duplicate declaration name in scope")); }
        Ok(())
    }
    fn reference(&self, scope: &str, name: String) -> String {
        if is_builtin_type(&name) { name } else { format!("?{scope}::{name}") }
    }
    fn type_name(&mut self, scope: &str) -> Result<String> {
        let raw = matches!(self.token().kind, Kind::String(_));
        let name = self.name()?;
        Ok(if raw { name } else { self.reference(scope, name) })
    }
    fn document(&mut self) -> Result<()> {
        while self.token().kind != Kind::End { self.declaration("global", None)?; }
        Ok(())
    }
    fn declaration(&mut self, scope: &str, module: Option<usize>) -> Result<()> {
        let annotation = self.annotation()?;
        if self.eat("module") {
            if module.is_some() { return Err(self.error("E_UNSUPPORTED_FEATURE", "nested modules require explicit top-level module declarations")); }
            let path = self.name()?; let id = self.id(annotation, "il");
            self.register("global", &path, &id)?;
            let mut visibility = Visibility::Private; let mut imports = vec![]; let mut declarations = vec![];
            if self.eat("visibility") { visibility = self.enum_name()?; }
            if self.eat("imports") { imports = self.list()?; }
            if self.eat("declarations") { declarations = self.list()?; }
            self.expect("{")?; self.enter()?;
            let index = self.graph.modules.len();
            self.graph.modules.push(Module { entity_id: id.clone(), path, imports, declarations, visibility });
            while !self.eat("}") { self.declaration(&id, Some(index))?; }
            self.depth -= 1; self.eat(";");
        } else if self.eat("type") {
            let id = self.type_decl(annotation, scope)?;
            if let Some(index) = module { self.graph.modules[index].declarations.push(id); }
        } else if self.eat("fn") {
            let id = self.function(annotation, scope)?;
            if let Some(index) = module { self.graph.modules[index].declarations.push(id); }
        } else if self.eat("capability") {
            let name = self.name()?; let id = self.id(annotation, scope);
            self.register(scope, &name, &id)?;
            self.expect(":")?; let kind = self.enum_name()?;
            self.expect("scope")?; let scope = self.typed_json()?; self.expect(";")?;
            self.graph.capabilities.push(Capability { entity_id: id, kind, scope });
        } else if self.eat("package") {
            let name = self.name()?; let id = self.id(annotation, scope);
            self.expect("version")?; let version = self.name()?;
            self.expect("modules")?; let modules = self.list()?;
            self.expect("effects")?; let effects = self.effects()?;
            self.expect("capabilities")?; let capabilities = self.list()?; self.expect(";")?;
            self.graph.packages.push(Package { entity_id: id, name, version, modules, effects, capabilities });
        } else if self.eat("contract") {
            let id = self.id(annotation, scope); self.expect("subject")?; let subject = self.name()?;
            self.expect("predicates")?; let predicates = self.typed_json()?; self.expect(";")?;
            self.graph.contracts.push(Contract { entity_id: id, subject, predicates });
        } else { return Err(self.error("E_UNSUPPORTED_FEATURE", "expected a supported declaration")); }
        Ok(())
    }
    fn type_decl(&mut self, annotation: Option<String>, scope: &str) -> Result<String> {
        let name = self.name()?; let id = self.id(annotation, scope); self.register(scope, &name, &id)?;
        let mut ty = TypeDef { entity_id: id.clone(), kind: TypeKind::Unit, parameters: vec![], layout: Layout::Inferred,
            integer: None, fields: vec![], variants: vec![] };
        if self.eat("kind") {
            ty.kind = self.enum_name()?; self.expect("parameters")?; ty.parameters = self.list()?;
            self.expect("layout")?; ty.layout = self.enum_name()?;
            self.expect("integer")?; ty.integer = self.typed_json()?;
            self.expect("fields")?; ty.fields = self.typed_json()?;
            self.expect("variants")?; ty.variants = self.typed_json()?;
        } else {
            self.expect("=")?; let kind = self.name()?;
            match kind.as_str() {
                "record" => {
                    ty.kind = TypeKind::Record; self.expect("{")?;
                    if !self.eat("}") { loop {
                        let name = self.name()?; self.expect(":")?; let type_ref = self.type_name(scope)?;
                        ty.fields.push(Field { name, type_ref });
                        if self.eat("}") { break; } self.expect(",")?;
                    }}
                }
                "sum" => {
                    ty.kind = TypeKind::Sum; self.expect("{")?;
                    if !self.eat("}") { loop {
                        let name = self.name()?; let mut fields = vec![];
                        if self.eat("(") { if !self.eat(")") { loop {
                            fields.push(self.type_name(scope)?); if self.eat(")") { break; } self.expect(",")?;
                        }}}
                        ty.variants.push(Variant { name, fields });
                        if self.eat("}") { break; } self.expect(",")?;
                    }}
                }
                "Option" | "Result" | "Tuple" => {
                    ty.kind = match kind.as_str() { "Option" => TypeKind::Option, "Result" => TypeKind::Result, _ => TypeKind::Tuple };
                    self.expect("<")?;
                    if !self.eat(">") { loop { ty.parameters.push(self.type_name(scope)?); if self.eat(">") { break; } self.expect(",")?; }}
                }
                "Unit" => ty.kind = TypeKind::Unit, "Never" => ty.kind = TypeKind::Never,
                "Bool" => ty.kind = TypeKind::Bool, "String" => ty.kind = TypeKind::String,
                "Bytes" => ty.kind = TypeKind::Bytes, "Usize" => ty.kind = TypeKind::Usize,
                name if is_builtin_type(name) => {
                    ty.kind = TypeKind::Int;
                    ty.integer = Some(IntegerLayout { signed: name.starts_with('I'), bits: name[1..].parse().map_err(|_| self.error("E_SCHEMA_INVALID", "invalid integer width"))? });
                }
                _ => return Err(self.error("E_UNSUPPORTED_FEATURE", "only concrete core type declarations are supported")),
            }
            if self.eat("layout") { ty.layout = self.enum_name()?; }
        }
        self.expect(";")?; self.graph.types.push(ty); Ok(id)
    }
    fn parameters(&mut self, owner: &str, scope: &str) -> Result<Vec<Parameter>> {
        self.expect("(")?; let mut parameters = vec![];
        if !self.eat(")") { loop {
            let annotation = self.annotation()?; let name = self.name()?; self.expect(":")?;
            let type_ref = self.type_name(scope)?; let entity_id = self.id(annotation, owner);
            parameters.push(Parameter { entity_id, name, type_ref });
            if self.eat(")") { break; } self.expect(",")?;
        }}
        Ok(parameters)
    }
    fn value_list(&mut self, owner: &str) -> Result<Vec<ValueDef>> {
        self.expect("[")?; let mut values = vec![];
        if !self.eat("]") { loop {
            let annotation = self.annotation()?; let type_ref = self.name()?;
            values.push(ValueDef { entity_id: self.id(annotation, owner), type_ref });
            if self.eat("]") { break; } self.expect(",")?;
        }}
        Ok(values)
    }
    fn function(&mut self, annotation: Option<String>, scope: &str) -> Result<String> {
        let name = self.name()?; let id = self.id(annotation, scope);
        let scope = if scope == "global" {
            self.graph.modules.iter().find(|module| module.declarations.contains(&id)).map(|module| module.entity_id.clone()).unwrap_or_else(|| scope.into())
        } else { scope.into() };
        self.register(&scope, &name, &id)?;
        let parameters = self.parameters(&id, &scope)?;
        self.expect("->")?; let result = self.type_name(&scope)?;
        let mut effects = vec![]; let mut capabilities = vec![]; let mut contracts = vec![];
        if self.eat("effects") { effects = self.effects()?; }
        if self.eat("capabilities") { capabilities = self.list()?; }
        if self.eat("contracts") { contracts = self.list()?; }
        self.expect("{")?;
        let start = self.at; let mut braces = 1;
        while braces > 0 {
            if self.token().kind == Kind::End { return Err(self.error("E_SCHEMA_INVALID", "unterminated function body")); }
            if self.is("{") { braces += 1; }
            if self.is("}") { braces -= 1; }
            if braces > 32 { return Err(self.error("E_RESOURCE_LIMIT", "function nesting exceeds 32")); }
            self.at += 1;
        }
        let index = self.graph.functions.len();
        self.graph.functions.push(Function { entity_id: id.clone(), name, parameters, result, effects, capabilities, contracts, blocks: vec![] });
        self.pending.push(Pending { function: index, start, end: self.at - 1, scope });
        Ok(id)
    }
    fn core_operation(&mut self, id: String) -> Result<Operation> {
        let opcode = self.enum_name()?;
        self.expect("inputs")?; let inputs = self.list()?;
        self.expect("outputs")?; let outputs = self.value_list(&id)?;
        self.expect("attributes")?; let attributes = self.typed_json()?;
        self.expect("effects")?; let effects = self.effects()?;
        self.expect("consumes")?; let consumes = self.list()?;
        self.expect("produces")?; let produces = self.value_list(&id)?;
        self.expect(";")?;
        Ok(Operation { entity_id: id, opcode, inputs, outputs, attributes, effects, consumes, produces })
    }
    fn resolve_name(&self, scope: &str, name: &str) -> String {
        resolve_name(&self.graph, &self.names, scope, name)
    }
    fn finish(&mut self) -> Result<()> {
        let graph = self.graph.clone();
        for ty in &mut self.graph.types {
            for reference in ty.parameters.iter_mut().chain(ty.fields.iter_mut().map(|field| &mut field.type_ref)).chain(ty.variants.iter_mut().flat_map(|variant| variant.fields.iter_mut())) {
                *reference = resolve_name(&graph, &self.names, "global", reference);
            }
        }
        for function in &mut self.graph.functions {
            function.result = resolve_name(&graph, &self.names, "global", &function.result);
            for parameter in &mut function.parameters { parameter.type_ref = resolve_name(&graph, &self.names, "global", &parameter.type_ref); }
        }
        let pending = std::mem::take(&mut self.pending);
        let mut bodies = vec![];
        for pending in &pending {
            self.at = pending.start;
            let function = self.graph.functions[pending.function].clone();
            bodies.push(surface::body(self, pending.end, &pending.scope, &function.entity_id)?);
        }
        for (pending, body) in pending.into_iter().zip(bodies) {
            let function = self.graph.functions[pending.function].clone();
            let blocks = lowering::lower(&mut self.graph, &self.names, &pending.scope, &function, body)?;
            self.graph.functions[pending.function].blocks = blocks;
        }
        Ok(())
    }
}

fn resolve_name(graph: &Graph, names: &BTreeMap<String, String>, scope: &str, raw: &str) -> String {
    let (scope, name) = if let Some(reference) = raw.strip_prefix('?') { reference.rsplit_once("::").unwrap_or((scope, reference)) } else { (scope, raw) };
    if is_builtin_type(name) || graph.types.iter().any(|ty| ty.entity_id == name) || graph.functions.iter().any(|f| f.entity_id == name) { return name.into(); }
    if let Some(id) = names.get(&format!("{scope}::{name}")) { return id.clone(); }
    if let Some((module_name, member)) = name.rsplit_once('.') {
        if let Some(module) = graph.modules.iter().find(|module| module.path == module_name || module.entity_id == module_name) {
            if let Some(id) = names.get(&format!("{}::{member}", module.entity_id)) { return id.clone(); }
        }
    }
    names.get(&format!("global::{name}")).cloned().unwrap_or_else(|| name.into())
}

fn op(entity_id: String, opcode: Opcode, inputs: Vec<String>, outputs: Vec<ValueDef>, attributes: Attributes) -> Operation {
    Operation { entity_id, opcode, inputs, outputs, attributes, effects: vec![], consumes: vec![], produces: vec![] }
}
