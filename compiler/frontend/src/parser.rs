use crate::lexer::{lex, Kind, Token};
use il_graph::*;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, Diagnostic>;

pub fn parse(source: &str, revision: u64) -> std::result::Result<Graph, Vec<Diagnostic>> {
    let tokens = lex(source).map_err(|(offset, message)| vec![diagnostic(revision, "E_SCHEMA_INVALID", offset, &message)])?;
    let mut parser = Parser { tokens, at: 0, revision, counter: 0, graph: Graph::empty(),
        names: BTreeMap::new(), sugar: BTreeSet::new(), depth: 0 };
    parser.graph.revision = revision;
    parser.document().map_err(|error| vec![error])?;
    parser.resolve();
    Ok(parser.graph)
}

fn diagnostic(revision: u64, code: &str, offset: usize, message: &str) -> Diagnostic {
    let mut result = Diagnostic::error(code, None, format!("byte {offset}: {message}"), revision);
    result.stage = "text_parse".into();
    result
}

struct Parser {
    tokens: Vec<Token>, at: usize, revision: u64, counter: u64,
    graph: Graph, names: BTreeMap<String, String>, sugar: BTreeSet<String>, depth: usize,
}

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
        let effective_scope = if scope == "global" {
            self.graph.modules.iter().find(|module| module.declarations.contains(&id)).map(|module| module.entity_id.clone()).unwrap_or_else(|| scope.into())
        } else { scope.into() };
        let scope = effective_scope.as_str();
        self.register(scope, &name, &id)?;
        let parameters = self.parameters(&id, scope)?;
        self.expect("->")?; let result = self.type_name(scope)?;
        let mut effects = vec![]; let mut capabilities = vec![]; let mut contracts = vec![];
        if self.eat("effects") { effects = self.effects()?; }
        if self.eat("capabilities") { capabilities = self.list()?; }
        if self.eat("contracts") { contracts = self.list()?; }
        self.expect("{")?; self.enter()?;
        let mut blocks = vec![]; let mut block_names = BTreeMap::new();
        while !self.eat("}") {
            let annotation = self.annotation()?;
            if self.eat("block") {
                let name = self.name()?; let block_id = self.id(annotation, &id);
                if block_names.insert(name, block_id.clone()).is_some() { return Err(self.error("E_DUPLICATE_NAME", "duplicate block name")); }
                let args = if self.is("(") { self.parameters(&block_id, scope)? } else { vec![] };
                self.expect("{")?;
                let symbols = parameters.iter().chain(&args).map(|p| (p.name.clone(), ValueDef { entity_id: p.entity_id.clone(), type_ref: p.type_ref.clone() })).collect();
                let block = self.statements(&id, block_id, args.into_iter().map(|p| ValueDef { entity_id: p.entity_id, type_ref: p.type_ref }).collect(), scope, &result, symbols, None)?;
                blocks.push(block);
            } else {
                if !blocks.is_empty() { return Err(self.error("E_SCHEMA_INVALID", "cannot mix implicit statements and explicit blocks")); }
                let block_id = self.allocate(&id);
                let symbols = parameters.iter().map(|p| (p.name.clone(), ValueDef { entity_id: p.entity_id.clone(), type_ref: p.type_ref.clone() })).collect();
                let block = self.statements(&id, block_id, vec![], scope, &result, symbols, annotation)?;
                blocks.push(block);
                break;
            }
        }
        self.depth -= 1;
        for block in &mut blocks {
            if !self.sugar.contains(&block.terminator.entity_id) { continue; }
            match &mut block.terminator.attributes {
                Attributes::Branch { target } => if let Some(id) = block_names.get(target) { *target = id.clone(); },
                Attributes::CondBranch { then_block, else_block, .. } => { if let Some(id) = block_names.get(then_block) { *then_block = id.clone(); } if let Some(id) = block_names.get(else_block) { *else_block = id.clone(); } },
                _ => {}
            }
        }
        self.graph.functions.push(Function { entity_id: id.clone(), name, parameters, result, effects, capabilities, blocks, contracts });
        Ok(id)
    }
    fn statements(&mut self, function: &str, block: String, arguments: Vec<ValueDef>, scope: &str, result_type: &str,
                  mut symbols: BTreeMap<String, ValueDef>, first_annotation: Option<String>) -> Result<Block> {
        let mut operations = vec![]; let mut terminator = None; let mut first = Some(first_annotation);
        while !self.eat("}") {
            if terminator.is_some() { return Err(self.error("E_SCHEMA_INVALID", "statement after terminator")); }
            let annotation = match first.take() { Some(Some(id)) => Some(id), _ => self.annotation()? };
            let id = self.id(annotation, &block);
            if self.eat("op") {
                let operation = self.core_operation(id)?;
                for output in &operation.outputs { symbols.insert(output.entity_id.clone(), output.clone()); }
                if operation.opcode.is_terminator() { terminator = Some(operation); } else { operations.push(operation); }
            } else if self.eat("let") {
                let output_annotation = self.annotation()?; let name = self.name()?; self.expect(":")?;
                let ty = self.type_name(scope)?; self.expect("=")?;
                let output = ValueDef { entity_id: output_annotation.unwrap_or_else(|| format!("{id}.value")), type_ref: ty.clone() };
                let expression = self.expression(scope, &symbols, &mut operations, &ty, 0)?;
                self.expect(";")?;
                let mut operation = expression.finish(id.clone(), output.clone());
                self.sugar.insert(id); operation.outputs = vec![output.clone()]; operations.push(operation);
                if symbols.insert(name, output).is_some() { return Err(self.error("E_DUPLICATE_NAME", "duplicate local name")); }
            } else if self.eat("return") {
                let mut inputs = vec![];
                if !self.is(";") {
                    let expression = self.expression(scope, &symbols, &mut operations, result_type, 0)?;
                    inputs.push(self.materialize(expression, &mut operations, &id, result_type));
                }
                self.expect(";")?;
                self.sugar.insert(id.clone());
                terminator = Some(op(id, Opcode::Return, inputs, vec![], Attributes::Empty {}));
            } else if self.eat("branch") {
                let target = self.name()?; let inputs = self.arguments(&symbols)?; self.expect(";")?;
                self.sugar.insert(id.clone());
                terminator = Some(op(id, Opcode::Branch, inputs, vec![], Attributes::Branch { target }));
            } else if self.eat("cond_branch") {
                let condition = self.value_reference(&symbols)?; self.expect(",")?;
                let then_block = self.name()?; let then_arguments = self.arguments(&symbols)?; self.expect(",")?;
                let else_block = self.name()?; let else_arguments = self.arguments(&symbols)?; self.expect(";")?;
                self.sugar.insert(id.clone());
                terminator = Some(op(id, Opcode::CondBranch, vec![condition], vec![], Attributes::CondBranch { then_block, else_block, then_arguments, else_arguments }));
            } else { return Err(self.error("E_UNSUPPORTED_FEATURE", &format!("unsupported statement in function {function}; use an explicit core operation"))); }
        }
        let terminator = terminator.ok_or_else(|| self.error("E_MISSING_RETURN", "block requires an explicit terminator"))?;
        Ok(Block { entity_id: block, arguments, operations, terminator })
    }
    fn core_operation(&mut self, id: String) -> Result<Operation> {
        let opcode: Opcode = self.enum_name()?;
        self.expect("inputs")?; let inputs = self.list()?;
        self.expect("outputs")?; let outputs = self.value_list(&id)?;
        self.expect("attributes")?; let attributes = self.typed_json()?;
        self.expect("effects")?; let effects = self.effects()?;
        self.expect("consumes")?; let consumes = self.list()?;
        self.expect("produces")?; let produces = self.value_list(&id)?;
        self.expect(";")?;
        Ok(Operation { entity_id: id, opcode, inputs, outputs, attributes, effects, consumes, produces })
    }
    fn value_reference(&mut self, symbols: &BTreeMap<String, ValueDef>) -> Result<String> {
        let name = self.name()?;
        Ok(symbols.get(&name).map(|value| value.entity_id.clone()).unwrap_or(name))
    }
    fn arguments(&mut self, symbols: &BTreeMap<String, ValueDef>) -> Result<Vec<String>> {
        self.expect("(")?; let mut args = vec![];
        if !self.eat(")") { loop { args.push(self.value_reference(symbols)?); if self.eat(")") { break; } self.expect(",")?; }}
        Ok(args)
    }
    fn materialize(&mut self, expression: Expression, operations: &mut Vec<Operation>, owner: &str, expected: &str) -> String {
        match expression {
            Expression::Value(value) => value.entity_id,
            expression => {
                let id = self.allocate(owner); let output = ValueDef { entity_id: format!("{id}.value"), type_ref: expected.into() };
                self.sugar.insert(id.clone()); operations.push(expression.finish(id, output.clone())); output.entity_id
            }
        }
    }
    fn expression(&mut self, scope: &str, symbols: &BTreeMap<String, ValueDef>, operations: &mut Vec<Operation>, expected: &str, precedence: u8) -> Result<Expression> {
        self.enter()?;
        let result = self.expression_inner(scope, symbols, operations, expected, precedence);
        self.depth -= 1;
        result
    }
    fn expression_inner(&mut self, scope: &str, symbols: &BTreeMap<String, ValueDef>, operations: &mut Vec<Operation>, expected: &str, precedence: u8) -> Result<Expression> {
        let mut left = if self.eat("(") {
            if self.eat(")") { Expression::Literal(Literal::Unit(())) }
            else { let value = self.expression(scope, symbols, operations, expected, 0)?; self.expect(")")?; value }
        } else if self.eat("!") || self.eat("not") {
            let inner = self.expression(scope, symbols, operations, expected, 10)?;
            let input = self.materialize(inner, operations, "il.expr", expected);
            Expression::Operation(Opcode::Not, vec![input], Attributes::Empty {})
        } else if self.eat("cast") {
            self.expect("<")?; let target_type = self.type_name(scope)?; self.expect(">")?; self.expect("(")?;
            let inner = self.expression(scope, symbols, operations, "I64", 0)?; self.expect(")")?;
            let input = self.materialize(inner, operations, "il.expr", "I64");
            Expression::Operation(Opcode::Cast, vec![input], Attributes::Cast { target_type })
        } else if self.is("-") || matches!(self.token().kind, Kind::Number(_) | Kind::String(_)) || self.is("true") || self.is("false") {
            let value = self.json()?;
            let literal: Literal = serde_json::from_value(value).map_err(|error| self.error("E_SCHEMA_INVALID", &error.to_string()))?;
            Expression::Literal(literal)
        } else {
            let name = self.name()?;
            if self.eat("(") {
                let mut inputs = vec![];
                if !self.eat(")") { loop {
                    let value = self.expression(scope, symbols, operations, "I64", 0)?;
                    inputs.push(self.materialize(value, operations, "il.expr", "I64"));
                    if self.eat(")") { break; } self.expect(",")?;
                }}
                Expression::Operation(Opcode::Call, inputs, Attributes::Call { callee: self.reference(scope, name) })
            } else { Expression::Value(symbols.get(&name).cloned().unwrap_or(ValueDef { entity_id: name, type_ref: expected.into() })) }
        };
        loop {
            let operator = match &self.token().kind { Kind::Symbol(value) => value.as_str(), _ => break };
            let (level, opcode) = match operator {
                "|" => (1, Opcode::BitOr), "^" => (2, Opcode::BitXor), "&" => (3, Opcode::BitAnd),
                "==" => (4, Opcode::Eq), "!=" => (4, Opcode::Ne), "<" => (5, Opcode::Lt), "<=" => (5, Opcode::Le), ">" => (5, Opcode::Gt), ">=" => (5, Opcode::Ge),
                "<<" => (6, Opcode::Shl), ">>" => (6, Opcode::Shr), "+" => (7, Opcode::Add), "-" => (7, Opcode::Sub),
                "*" => (8, Opcode::Mul), "/" => (8, Opcode::Div), "%" => (8, Opcode::Rem), _ => break,
            };
            if level < precedence { break; }
            self.at += 1;
            let input_type = match &left { Expression::Value(value) => value.type_ref.clone(), _ if expected == "Bool" => "I64".into(), _ => expected.into() };
            let right = self.expression(scope, symbols, operations, &input_type, level + 1)?;
            let left_id = self.materialize(left, operations, "il.expr", &input_type);
            let right_id = self.materialize(right, operations, "il.expr", &input_type);
            left = Expression::Operation(opcode, vec![left_id, right_id], Attributes::Empty {});
        }
        Ok(left)
    }
    fn resolve(&mut self) {
        let resolve = |name: &mut String| {
            if let Some(reference) = name.strip_prefix('?') {
                if let Some(id) = self.names.get(reference).or_else(|| reference.rsplit_once("::").and_then(|(_, name)| self.names.get(&format!("global::{name}")))) {
                    *name = id.clone();
                } else { *name = reference.rsplit_once("::").map(|(_, name)| name).unwrap_or(reference).into(); }
            }
        };
        for ty in &mut self.graph.types {
            for reference in ty.parameters.iter_mut().chain(ty.fields.iter_mut().map(|field| &mut field.type_ref)).chain(ty.variants.iter_mut().flat_map(|variant| variant.fields.iter_mut())) { resolve(reference); }
        }
        for function in &mut self.graph.functions {
            resolve(&mut function.result);
            for parameter in &mut function.parameters { resolve(&mut parameter.type_ref); }
            for block in &mut function.blocks {
                for argument in &mut block.arguments { resolve(&mut argument.type_ref); }
                for operation in block.operations.iter_mut().chain(std::iter::once(&mut block.terminator)) {
                    for output in operation.outputs.iter_mut().chain(&mut operation.produces) { resolve(&mut output.type_ref); }
                    match &mut operation.attributes { Attributes::Call { callee } => resolve(callee), Attributes::Cast { target_type } => resolve(target_type), _ => {} }
                }
            }
        }
        let graph = self.graph.clone();
        for function in &mut self.graph.functions {
            for block in &mut function.blocks {
                let mut values: BTreeMap<_, _> = function.parameters.iter().map(|p| (p.entity_id.clone(), p.type_ref.clone())).chain(block.arguments.iter().map(|v| (v.entity_id.clone(), v.type_ref.clone()))).collect();
                for operation in block.operations.iter_mut().chain(std::iter::once(&mut block.terminator)) {
                    if self.sugar.contains(&operation.entity_id) {
                        if matches!(operation.opcode, Opcode::Call | Opcode::Move | Opcode::Return | Opcode::Branch | Opcode::CondBranch | Opcode::Switch) {
                            operation.consumes = operation.inputs.iter().chain(operation.branch_arguments()).filter(|id| values.get(*id).is_some_and(|ty| owned(&graph, ty, &mut BTreeSet::new()))).cloned().collect();
                            operation.consumes.sort(); operation.consumes.dedup();
                        }
                        operation.produces = operation.outputs.iter().filter(|value| owned(&graph, &value.type_ref, &mut BTreeSet::new())).cloned().collect();
                        if operation.opcode == Opcode::Const && !operation.produces.is_empty() { operation.effects = vec![Effect::Alloc]; }
                        if let Attributes::Call { callee } = &operation.attributes {
                            if let Some(callee) = graph.functions.iter().find(|function| function.entity_id == *callee) { operation.effects = callee.effects.clone(); }
                        }
                    }
                    for value in &operation.outputs { values.insert(value.entity_id.clone(), value.type_ref.clone()); }
                }
            }
        }
    }
}

fn owned(graph: &Graph, name: &str, visited: &mut BTreeSet<String>) -> bool {
    let mut pending = vec![name.to_owned()];
    while let Some(name) = pending.pop() {
        if matches!(name.as_str(), "String" | "Bytes") { return true; }
        if !visited.insert(name.clone()) { continue; }
        if let Some(ty) = graph.types.iter().find(|ty| ty.entity_id == name) {
            if matches!(ty.kind, TypeKind::String | TypeKind::Bytes) || ty.layout == Layout::Opaque { return true; }
            pending.extend(ty.parameters.iter().chain(ty.fields.iter().map(|field| &field.type_ref)).chain(ty.variants.iter().flat_map(|variant| &variant.fields)).cloned());
        }
    }
    false
}

fn op(entity_id: String, opcode: Opcode, inputs: Vec<String>, outputs: Vec<ValueDef>, attributes: Attributes) -> Operation {
    Operation { entity_id, opcode, inputs, outputs, attributes, effects: vec![], consumes: vec![], produces: vec![] }
}

enum Expression { Value(ValueDef), Literal(Literal), Operation(Opcode, Vec<String>, Attributes) }
impl Expression {
    fn finish(self, id: String, output: ValueDef) -> Operation {
        match self {
            Self::Value(value) => op(id, Opcode::Move, vec![value.entity_id], vec![output], Attributes::Empty {}),
            Self::Literal(value) => op(id, Opcode::Const, vec![], vec![output], Attributes::Constant { value }),
            Self::Operation(opcode, inputs, attributes) => op(id, opcode, inputs, vec![output], attributes),
        }
    }
}
