use il_graph::*;

pub(super) fn check(graph: &Graph) -> Vec<Diagnostic> {
    match il_graph::http::expected_dispatchers(graph) {
        Err(diagnostics) => diagnostics,
        Ok(functions) => functions.into_iter().filter_map(|expected| {
            let actual = graph.functions.iter().find(|f| f.entity_id == expected.entity_id);
            (actual != Some(&expected)).then(|| Diagnostic::error("E_HTTP_DECLARATION", Some(&expected.entity_id),
                "HTTP dispatcher differs from the canonical declaration expansion", graph.revision))
        }).collect(),
    }
}
