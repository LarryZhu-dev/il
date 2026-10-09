//! Bounded excerpts from compiler-locked specifications, never request paths.
use il_graph::hash_bytes;
use serde_json::{json, Value};

const FRAGMENT_LIMIT: usize = 900;
struct Reference {
    source: &'static str,
    text: &'static str,
    anchor: &'static str,
    lines: usize,
}
macro_rules! reference {
    ($source:literal, $anchor:literal, $lines:literal) => {
        Reference { source:$source, text:include_str!(concat!("../../../",$source)), anchor:$anchor, lines:$lines }
    };
}

fn excerpt(reference: Reference, kind: &str) -> Value {
    let lines: Vec<_> = reference.text.lines().collect();
    let Some(start) = lines.iter().position(|line|line.contains(reference.anchor)) else {
        return json!({"source":reference.source,"source_sha256":hash_bytes(reference.text.as_bytes()),
            "reference_kind":kind,"available":false,"start_line":null,"fragment":null,"truncated":false});
    };
    let mut fragment = String::new();
    let mut truncated = false;
    for line in lines.iter().skip(start).take(reference.lines) {
        if fragment.len()+line.len()+1>FRAGMENT_LIMIT { truncated=true;break; }
        fragment.push_str(line);fragment.push('\n');
    }
    json!({"source":reference.source,"source_sha256":hash_bytes(reference.text.as_bytes()),
        "reference_kind":kind,"available":true,"start_line":start+1,"fragment":fragment,"truncated":truncated})
}

/// Returns actual source excerpts. Unregistered codes receive an explicitly
/// general error-contract reference, not a guessed code-specific explanation.
pub fn rules(code: &str) -> Value {
    let selected: Vec<Reference> = match code {
        "E_TYPE_MISMATCH" => vec![
            reference!("spec/types.yaml","record_identity:",12),
            reference!("rfc/0006-semantic-acceptance.md","Positive cases cover",7)],
        "E_NON_EXHAUSTIVE_MATCH" => vec![reference!("rfc/0006-semantic-acceptance.md","A sum switch",6)],
        "E_MISSING_RETURN"|"E_NAME_NOT_FOUND"|"E_DUPLICATE_NAME"|"E_UNREACHABLE_BLOCK" =>
            vec![reference!("rfc/0006-semantic-acceptance.md","Negative cases require",7)],
        "E_USE_AFTER_MOVE"|"E_DOUBLE_DROP"|"E_OWNERSHIP_JOIN"|"E_OWNERSHIP_METADATA"|
        "E_BORROW_ESCAPE"|"E_BORROW_CONFLICT" => vec![
            reference!("spec/memory.yaml","owned_heap_value:",15),
            reference!("spec/memory.yaml","states:",12)],
        "E_EFFECT_UNDECLARED" => vec![reference!("spec/effects.yaml","effects:",4)],
        "E_CAPABILITY_MISSING"|"E_HOST_POLICY_INVALID" =>
            vec![reference!("spec/effects.yaml","capabilities:",14)],
        "E_STALE_REVISION"|"E_INVALID_SCOPE" => vec![reference!("spec/versioning.yaml","transaction:",9)],
        "E_STATE_INCONSISTENT"|"E_COMMIT_DURABILITY_UNCERTAIN" =>
            vec![reference!("rfc/0004-store-publication.md","No reader treats",6),
                reference!("rfc/0004-store-publication.md","If the directory sync",6)],
        "E_CONTEXT_INSUFFICIENT" =>
            vec![reference!("rfc/0022-ai-tool-protocol.md","One conservative token unit",7),
                reference!("rfc/0022-ai-tool-protocol.md","An insufficient context response",8)],
        "E_EVIDENCE_INCOMPLETE" =>
            vec![reference!("rfc/0022-ai-tool-protocol.md","evidence resolves selected receipts",7)],
        "E_ROUTE_COLLISION"|"E_HTTP_DECLARATION" =>
            vec![reference!("rfc/0021-http-library-and-declarative-routing.md","Each route has",10)],
        "E_HTTP_MALFORMED" => vec![reference!("rfc/0003-checked-values-and-http.md","HTTP rules and exact limits",9)],
        "E_INTEGER_OVERFLOW"|"E_DIVIDE_BY_ZERO"|"E_INVALID_SHIFT" =>
            vec![reference!("rfc/0003-checked-values-and-http.md","All fixed-width integer",7)],
        "E_INDEX_OUT_OF_BOUNDS"|"E_ALLOCATION_FAILED" =>
            vec![reference!("rfc/0020-network-resources-and-byte-primitives.md","bytes_get out of bounds",7)],
        "E_SCHEMA_INVALID" => vec![reference!("rfc/0022-ai-tool-protocol.md","The registry below is closed",6)],
        "E_NATIVE_EXECUTION_FAILED"|"E_NATIVE_BUILD_FAILED"|"E_TOOLCHAIN_FAILURE" =>
            vec![reference!("rfc/0017-native-tools.md","Each build creates",8),
                reference!("rfc/0017-native-tools.md","Captured native execution",6)],
        _ => vec![],
    };
    if selected.is_empty() {
        json!([excerpt(reference!("spec/errors.yaml","business_errors:",16),"general")])
    } else {
        Value::Array(selected.into_iter().map(|item|excerpt(item,"code_specific")).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_registered_excerpts_exist_and_are_bounded() {
        for code in ["E_TYPE_MISMATCH","E_NON_EXHAUSTIVE_MATCH","E_MISSING_RETURN","E_USE_AFTER_MOVE",
            "E_EFFECT_UNDECLARED","E_CAPABILITY_MISSING","E_STALE_REVISION","E_STATE_INCONSISTENT",
            "E_CONTEXT_INSUFFICIENT","E_EVIDENCE_INCOMPLETE","E_ROUTE_COLLISION","E_HTTP_MALFORMED",
            "E_INTEGER_OVERFLOW","E_INDEX_OUT_OF_BOUNDS","E_SCHEMA_INVALID","E_NATIVE_EXECUTION_FAILED"] {
            let references=rules(code);
            for item in references.as_array().unwrap() {
                assert_eq!(item["available"],true,"{code}: {item}");
                assert_eq!(item["reference_kind"],"code_specific");
                let fragment=item["fragment"].as_str().unwrap();
                assert!(!fragment.is_empty() && fragment.len()<=FRAGMENT_LIMIT);
                assert!(item["start_line"].as_u64().unwrap()>0);
                assert!(item["source_sha256"].as_str().unwrap().starts_with("sha256:"));
            }
            assert!(serde_json::to_vec(&references).unwrap().len()<3000);
        }
    }
    #[test]
    fn type_error_quotes_real_nominal_and_conversion_rules() {
        let value=rules("E_TYPE_MISMATCH");
        let fragment=value[0]["fragment"].as_str().unwrap();
        assert!(fragment.contains("record_identity: nominal_entity_id"));
        assert!(fragment.contains("implicit_numeric_conversion: false"));
        assert_eq!(value[0]["source_sha256"],hash_bytes(include_bytes!("../../../spec/types.yaml")));
    }
    #[test]
    fn unknown_code_is_explicit_general_guidance_without_interpolation() {
        let value=rules("UNREGISTERED_SHELL_FRAGMENT");
        assert_eq!(value[0]["reference_kind"],"general");
        assert_eq!(value[0]["source"],"spec/errors.yaml");
        assert!(!value.to_string().contains("UNREGISTERED_SHELL_FRAGMENT"));
    }
}
