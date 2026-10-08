use il_graph::*;
use serde::Serialize;

fn json<T: Serialize>(value: &T) -> String { serde_json::to_string(value).unwrap() }
fn id(value: &str) -> String { format!("@id({})", json(&value)) }
fn values(values: &[ValueDef]) -> String {
    format!("[{}]", values.iter().map(|value| format!("{} {}", id(&value.entity_id), json(&value.type_ref))).collect::<Vec<_>>().join(", "))
}

pub fn format(graph: &Graph) -> Result<String, Vec<Diagnostic>> {
    let diagnostics = graph.validate_structural();
    if !diagnostics.is_empty() { return Err(diagnostics); }
    let mut text = String::from("// Intelligent language canonical graph projection\n");
    for module in &graph.modules {
        text.push_str(&format!("{} module {} visibility {} imports {} declarations {} {{}}\n", id(&module.entity_id), json(&module.path), json(&module.visibility).trim_matches('"'), json(&module.imports), json(&module.declarations)));
    }
    for ty in &graph.types {
        text.push_str(&format!("{} type {} kind {} parameters {} layout {} integer {} fields {} variants {};\n",
            id(&ty.entity_id), json(&ty.entity_id), json(&ty.kind).trim_matches('"'), json(&ty.parameters),
            json(&ty.layout).trim_matches('"'), json(&ty.integer), json(&ty.fields), json(&ty.variants)));
    }
    for function in &graph.functions {
        let parameters = function.parameters.iter().map(|parameter| format!("{} {}: {}", id(&parameter.entity_id), json(&parameter.name), json(&parameter.type_ref))).collect::<Vec<_>>().join(", ");
        text.push_str(&format!("{} fn {}({parameters}) -> {} effects {} capabilities {} contracts {} {{\n",
            id(&function.entity_id), json(&function.name), json(&function.result), json(&function.effects), json(&function.capabilities), json(&function.contracts)));
        for (index, block) in function.blocks.iter().enumerate() {
            let parameters = block.arguments.iter().enumerate().map(|(index, value)| format!("{} v{index}: {}", id(&value.entity_id), json(&value.type_ref))).collect::<Vec<_>>().join(", ");
            text.push_str(&format!("  {} block b{index}({parameters}) {{\n", id(&block.entity_id)));
            for operation in block.operations.iter().chain(std::iter::once(&block.terminator)) {
                text.push_str(&format!("    {} op {} inputs {} outputs {} attributes {} effects {} consumes {} produces {};\n",
                    id(&operation.entity_id), json(&operation.opcode).trim_matches('"'), json(&operation.inputs), values(&operation.outputs),
                    json(&operation.attributes), json(&operation.effects), json(&operation.consumes), values(&operation.produces)));
            }
            text.push_str("  }\n");
        }
        text.push_str("}\n");
    }
    for capability in &graph.capabilities {
        text.push_str(&format!("{} capability {}: {} scope {};\n", id(&capability.entity_id), json(&capability.entity_id), json(&capability.kind).trim_matches('"'), json(&capability.scope)));
    }
    for package in &graph.packages {
        text.push_str(&format!("{} package {} version {} modules {} effects {} capabilities {};\n", id(&package.entity_id), json(&package.name), json(&package.version), json(&package.modules), json(&package.effects), json(&package.capabilities)));
    }
    for contract in &graph.contracts {
        text.push_str(&format!("{} contract subject {} predicates {};\n", id(&contract.entity_id), json(&contract.subject), json(&contract.predicates)));
    }
    Ok(text)
}
