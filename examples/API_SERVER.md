# il API Server Example

This example demonstrates a complete HTTP API server written in pure il, showcasing:

- HTTP request handling with routing
- JSON response generation
- Content negotiation and headers
- System time and capabilities introspection
- Docker containerization

## Architecture

The server is implemented in `api_server.il` using il's native HTTP capabilities:

```
api_server.il (168 lines)
  |
  +-- HTTP listener on port 8080
  +-- Route dispatcher (4 endpoints)
  +-- JSON response builders
  +-- System capability queries
```

## Endpoints

### GET /health
Health check endpoint returning server status.

**Response:**
```json
{
  "status": "healthy",
  "service": "il-api-server",
  "version": "0.1.0"
}
```

### GET /hello
Simple greeting endpoint with optional name parameter.

**Query Parameters:**
- `name` (optional): Custom name for greeting

**Response:**
```json
{
  "message": "Hello, World!",
  "timestamp": 1728691200
}
```

### GET /time
Current system time in multiple formats.

**Response:**
```json
{
  "unix_timestamp": 1728691200,
  "iso_8601": "2024-10-12T00:00:00Z",
  "formatted": "2024-10-12 00:00:00 UTC"
}
```

### GET /capabilities
Server and runtime capabilities report.

**Response:**
```json
{
  "server": {
    "language": "il",
    "version": "0.1.0",
    "http_server": true,
    "json_support": true
  },
  "runtime": {
    "platform": "x86_64-unknown-linux-gnu",
    "container": true
  }
}
```

## Building and Running

### Prerequisites

- Docker (for Windows/macOS development)
- il toolchain in Docker image `il-server:latest`

### Quick Start

Use the PowerShell scripts provided:

```powershell
# 1. Compile the server
.\compile_api_server.ps1

# 2. Run the server
.\run_api_server.ps1

# 3. Test endpoints (in another terminal)
.\test_api_server.ps1
```

### Manual Steps

1. **Compile** (imports source to graph, compiles to ELF):
   ```powershell
   docker run --rm `
     -v "${PWD}:/workspace" `
     -w /workspace `
     il-server:latest `
     bash -c "
       il import api_server.il && \
       il compile --release -o api_server api_server
     "
   ```

2. **Run** (in Docker container):
   ```powershell
   docker run --rm -p 8080:8080 `
     -v "${PWD}:/workspace" `
     -w /workspace `
     il-server:latest `
     ./api_server
   ```

3. **Test**:
   ```powershell
   curl http://localhost:8080/health
   curl http://localhost:8080/hello?name=Alice
   curl http://localhost:8080/time
   curl http://localhost:8080/capabilities
   ```

## Development Workflow

### Hot Reload
Changes to `api_server.il` require recompilation. The workflow is:

1. Edit `api_server.il`
2. Stop the running container (Ctrl+C)
3. Run `.\compile_api_server.ps1`
4. Run `.\run_api_server.ps1`

### Testing
The `test_api_server.ps1` script validates all endpoints:

```powershell
.\test_api_server.ps1
```

Output shows each endpoint status, response JSON, and overall result.

### Debugging

View server logs in the Docker container output:
```
Starting il API server...
Listening on 0.0.0.0:8080
[INFO] GET /health -> 200
[INFO] GET /hello?name=Alice -> 200
```

## File Structure

```
examples/
├── api_server.il              # Server source (168 lines)
├── compile_api_server.ps1     # Build script
├── run_api_server.ps1         # Run script
├── test_api_server.ps1        # Test script
├── API_SERVER.md              # This file
└── README.md                  # Overview of all examples
```

## Implementation Notes

### HTTP Routing
Routes are matched by exact path prefix. Query parameters are parsed from the request URI.

### JSON Generation
Responses are hand-crafted JSON strings (no JSON library dependency). Production code would use il's standard JSON builder when available.

### Error Handling
- 404 for unknown routes
- 400 for malformed requests
- 500 for internal errors

All errors return JSON:
```json
{
  "error": "Not Found",
  "path": "/unknown"
}
```

### Performance
The server handles requests sequentially in this demo. Production servers would use il's async runtime for concurrent request handling.

## Next Steps

- Add POST/PUT/DELETE endpoints
- Implement request body parsing
- Add middleware (logging, auth, CORS)
- Connect to il database/storage
- Deploy to production with proper Docker image

## Related Examples

- `greeting_server/` - Simpler TCP server
- `http_demo/` - HTTP client examples
- See `examples/README.md` for complete list
