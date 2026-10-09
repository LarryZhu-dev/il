# RFC 0021: HTTP library and declarative routing

Status: Accepted under the approved implementation plan. Stage: P07.
Prerequisites: P06 VERIFIED and RFC0020. The existing `spec/http.yaml` remains
authoritative. HTTP parsing, path matching, JSON quoting, response encoding and
the serving loop are actual il package source compiled through ordinary MIR and
LLVM. Host primitives only implement general bytes, allocation and sockets.

## Typed package surface

MVP has no user generics: the development document's Request/Response examples
are represented by explicitly named monomorphic sum types, with owned fields
extracted by exhaustive match. E01 will add user-defined generic abstractions.

* http.HttpError = Protocol(U16) | Io(core.IoError) | Handler.
* http.Request = Parsed(Bytes method, Bytes decoded_path).
* http.ParseResult = Result<http.Request,http.HttpError>.
* http.EmptyRequest = Empty; http.PathRequest = Path(String).
* http.Response = Response(U16 status, String content_type, Bytes body).
* http.ResponseResult = Result<http.Response,http.HttpError>.
* http.PathMatch = Miss | Match(String), with empty String for a static route.
* http.Read = Read(net.Stream, http.ParseResult).

`http.parse(Bytes)->ParseResult` and
`http.match_path(Bytes path,Bytes pattern)->Result<PathMatch,HttpError>` allocate.
`http.read_request(net.Stream,core.Deadline)->http.Read` preserves the connection
on every protocol/read error so that a response can be attempted (net, alloc).
`http.encode(Response)->Result<Bytes,core.IoError>` allocates.
`http.write_response(net.Stream,Response,core.Deadline)->Result<Unit,core.IoError>`
consumes and closes the stream on all outcomes (net, alloc).
`http.error_response(HttpError)->Response` allocates and creates an empty body;
Protocol carries its status, Io Timeout maps to 408, other failures to 500.
The serving loop emits structured tracing diagnostics for peer disconnect and
failed writes; a failed diagnostic write must still release the stream.

## Declaration and graph contract

An ordinary contract predicate `http_server` carries `entry`, `bind`,
`capability`, and `routes`. The contract's stable entity_id is the server ID;
its subject is a dispatcher function with two owned Bytes parameters (method,
decoded_path) returning http.ResponseResult. `entry` names the ordinary il
serving function, which requires the Listen grant `capability`. Its reachable
call graph must contain a call to the dispatcher and at least one `net_listen`
using that grant; ordinary helper functions are permitted, and graph traversal
visits each function at most once even with recursive calls. `bind` must equal
that grant's canonical numeric endpoint scope.
The demo uses 127.0.0.1:8080. The server loop is sequential and bounded by an
explicit maximum request count; an application chooses its loop count and tests
can choose one request without a separate runtime HTTP implementation.

Each route has an `entity_id`, `method`, `path`, `handler`, and `parameters`.
MVP methods are GET. Path patterns are absolute decoded paths, matched by whole
case-sensitive segments. A route has at most one `{name}` segment; parameters
are either empty or exactly one record with name, type_ref=String, source=path,
max_utf8_bytes=128. Static routes require a handler
`http.EmptyRequest -> http.ResponseResult`; parameter routes require
`http.PathRequest -> http.ResponseResult`. Handler effects and capabilities
propagate to the dispatcher and entry by ordinary checks. Overlapping patterns
(including static vs parameter) are rejected with E_ROUTE_COLLISION.
Malformed declarations use E_HTTP_DECLARATION; missing IDs/type mismatches keep
the ordinary stable diagnostics. At most 128 routes and 4096 bytes per pattern.

Text parsing and graph transactions deterministically materialize the dispatcher
as ordinary graph operations from this contract. This is package declaration
elaboration, not a new HTTP opcode or a runtime handler evaluator. Generated IDs
use dispatcher/route stable IDs, not source positions or route ordinals. Graph
transactions require program scope or exact dispatcher scope for all generated
changes. Canonical projections include the full generated body. Static checking
reconstructs the expected body and rejects a mismatched dispatcher; callers
cannot attach metadata to an unrelated implementation. Handler calls are normal
statically checked calls. Builds never generate C, Rust or another target language.

A dispatcher may use a function signature ending in `;`, for example
`fn dispatch(method:Bytes,path:Bytes)->http.ResponseResult effects [alloc]
contracts [server];`. Such a declaration must explicitly reference an
`http_server` contract whose subject is that function. Parsing materializes its
ordinary body after all declarations and source bodies have been resolved;
bodyless functions without an implementing contract are rejected. Canonical
text always includes the complete generated body. Expansion is bounded by the
256-type and 100,000-node compilation budgets before generated nodes are allocated.

## Protocol boundary details

The 4096 request-line limit excludes its CRLF; the 16384 header-section limit
counts all bytes after that CRLF, including the terminating empty CRLF. A line
or section exceeding its bound is rejected as soon as the limit is exceeded.
The parser requires exactly three nonempty request-line tokens separated by
single SP; methods use ASCII token syntax. Unsupported syntactically valid
HTTP versions produce 505; malformed version spelling produces 400. CONNECT
produces 405. Header names use ASCII token syntax, no whitespace before colon,
and values reject NUL/control bytes except horizontal tab. Obsolete folding is
rejected. Host must be exactly one nonempty trimmed value.

Duplicate Content-Length is rejected even when values agree. All framing
validation precedes the nonzero-body decision, so conflicting/duplicate framing
is 400, then a valid nonzero Content-Length is 413. A missing Content-Length
means zero. Transfer-Encoding and Upgrade are 400. Bytes after the header
terminator are not another request: this connection always closes. Request
targets use origin-form, with query excluded before decoding. Raw fragment
markers/control bytes are invalid. Decode percent escapes once in each segment;
reject decoded slash, NUL and invalid UTF-8. UTF-8 parameter limit is checked on
decoded bytes. Empty parameter segments do not match; trailing slash matters.

Read deadline is stored stream opening instant + 5,000,000,000ns; response write
deadline is response-start monotonic instant + 5,000,000,000ns. Partial I/O keeps
the same deadline. Full application mode still rejects captured fault policies;
external TCP clients may exercise a bounded captured native server to inject
short writes while using the same package code. Each test owns and closes its
listener/client/server processes. No unrelated listener is stopped or displaced.

## Acceptance

The composed standard packages produce a canonical projection larger than the
bootstrap-only 1 MiB source ceiling in RFC0005. From P07 the frontend accepts up
to 16 MiB of UTF-8 source and 1,048,576 lexical tokens, excluding the end marker;
nesting remains 32 and semantic graph checking remains bounded at 100,000 nodes.
These replace the two bootstrap source limits for every frontend caller, with no
special header or trusted-source bypass. The formatter refuses projections that
exceed the same byte/token limits. Limit violations return E_RESOURCE_LIMIT;
malformed syntax continues to return E_SCHEMA_INVALID. CLI request/response and
AI context budgets remain independent limits; no unbounded transport is added.

Independent external clients read locked expectations from spec/http.yaml and
exercise both native optimization profiles, happy paths, Unicode/JSON escaping,
404/405/Allow, framing/encoding/size/version failures, fragmented requests,
absolute timeout, partial writes and disconnect cleanup. Pure package behavior
also has interpreter/native parity tests. Graph/text roundtrip, deterministic
IDs, collisions, wrong handler types, missing grants, graph-only edits and
transaction rollback receive positive and negative coverage. HTTP stage is not
VERIFIED until actual service execution and all dependencies pass.
