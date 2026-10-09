# HTTP demo

Compose core, alloc, io, time, net, json, http, test, tracing and this directory's `main.il`. The graph contract materializes a checked dispatcher for `GET /health` and `GET /hello/{name}`. The parser, route matching, handlers and response loop execute compiled il code.

Launch a Full native build with an explicit `demo.listen` Listen grant for `127.0.0.1:8080` and a `demo.clock` ClockRead grant. Native policy input is descriptor 4. `main` serves sequential requests; the reusable `demo.serve(maximum)` bounds the request count, and `demo.serve_once` serves one request for captured external-client tests. Services are stopped by their launcher.

`spec/http.yaml` is the independent wire contract. Tests use a real TCP client and retain build hashes and process receipts.
