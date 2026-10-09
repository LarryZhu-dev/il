use il_frontend::parse;
use il_graph::*;

const TYPES: &str = r#"
@id("core") module core visibility public {
 @id("core.File") type File=record{slot:U64,generation:U64} layout opaque;
 @id("core.Deadline") type Deadline=sum{Infinite,At(U64)};
}
@id("net") module net visibility public {
 @id("net.Listener") type Listener=record{slot:U64,generation:U64} layout opaque;
 @id("net.Stream") type Stream=record{slot:U64,generation:U64} layout opaque;
}
"#;

fn graph(body: &str) -> Graph { parse(&format!("{TYPES}\n{body}"),0).unwrap() }
fn codes(graph: &Graph) -> Vec<String> {
    il_checker::check_with_capabilities(graph,&graph.capabilities).into_iter().map(|error|error.code).collect()
}

#[test]
fn nested_file_and_socket_cleanup_require_both_effects() {
    for (effects, valid) in [("fs,net",true),("fs",false),("net",false),("",false)] {
        let program=graph(&format!("module app imports[core,net]{{ type Bundle=sum{{Both(core.File,net.Stream)}};fn consume(value:Bundle)->Unit effects[{effects}]{{return;}} }}"));
        let diagnostics=codes(&program);
        assert_eq!(diagnostics.is_empty(),valid,"{effects}: {diagnostics:?}");
        if !valid { assert!(diagnostics.iter().any(|code|code=="E_EFFECT_UNDECLARED")); }
    }
    let program=graph("module app imports[core,net]{type Bundle=sum{Both(core.File,net.Stream)};fn transfer(value:Bundle)->Bundle{return value;}}");
    assert!(codes(&program).is_empty(),"{:?}",codes(&program));
}

#[test]
fn network_handles_are_nominal_owned_and_unforgeable() {
    for ty in ["net.Listener","net.Stream"] {
        let program=graph(&format!("module app imports[net]{{fn forge()->{ty}{{return {ty}{{slot:0,generation:1}};}}}}"));
        assert!(codes(&program).iter().any(|code|code=="E_TYPE_MISMATCH"));
        let program=graph(&format!("module app imports[net]{{fn duplicate(value:{ty})->{ty} effects[alloc,net]{{return clone(value);}}}}"));
        assert!(codes(&program).iter().any(|code|code=="E_UNSUPPORTED_FEATURE"));
    }
    let program=graph("module app imports[net]{fn wrong(listener:net.Listener)->Unit effects[net]{runtime.net_close_stream(listener);return;}}");
    assert!(codes(&program).iter().any(|code|code=="E_TYPE_MISMATCH"));
}

#[test]
fn closing_socket_consumes_it_but_shared_timestamp_does_not() {
    let program=graph("module app imports[net]{fn consume(stream:net.Stream)->U64 effects[net]{let when:U64=runtime.net_opened_at(stream);runtime.net_close_stream(stream);return when;}}");
    assert!(codes(&program).is_empty(),"{:?}",codes(&program));
    let source=format!("{TYPES}\nmodule app imports[net]{{fn invalid(stream:net.Stream)->U64 effects[net]{{runtime.net_close_stream(stream);return runtime.net_opened_at(stream);}}}}");
    let diagnostics=parse(&source,0).unwrap_err();
    assert!(diagnostics.iter().any(|error|error.code=="E_USE_AFTER_MOVE"));
}

#[test]
fn network_selector_kind_and_trusted_injection_are_checked() {
    let source=r#"@id("listen") capability listen:Listen scope "127.0.0.1:8080";
     module app imports[net]{fn open()->Unit effects[net] capabilities[listen]{runtime.net_listen[listen]();return;}}"#;
    let program=graph(source);
    assert!(codes(&program).is_empty(),"{:?}",codes(&program));
    assert!(il_checker::check(&program).iter().any(|error|error.code=="E_CAPABILITY_MISSING"));
    let mut wrong=program.clone();wrong.capabilities[0].kind=CapabilityKind::Connect;
    assert!(codes(&wrong).iter().any(|code|code=="E_CAPABILITY_MISSING"));
    let mut altered=program.capabilities.clone();altered[0].scope=Some("127.0.0.1:8081".into());
    assert!(il_checker::check_with_capabilities(&program,&altered).iter().any(|error|error.code=="E_CAPABILITY_MISSING"));
}
