#!/usr/bin/env python3
"""
Demo il HTTP server that uses test fixtures instead of live il binary.
Demonstrates the HTTP API without requiring a compiled il binary.
"""
import sys
import json
from pathlib import Path
from typing import Optional, Dict, Any
from fastapi import FastAPI, HTTPException
from fastapi.responses import JSONResponse
import uvicorn

# Add tools to path
sys.path.insert(0, str(Path(__file__).parent / "tools" / "http_server"))

app = FastAPI(title="il Demo Server", version="0.1.0")

# Mock graph state based on test fixtures
MOCK_STATE = {
    "nodes": {
        "greeting": {
            "type": "constant",
            "value": {"type": "text", "content": "Hello, World!"}
        }
    },
    "edges": []
}

MOCK_CAPABILITIES = {
    "http": {"enabled": True, "version": "1.1"},
    "graph": {"enabled": True, "max_nodes": 1000},
    "persistence": {"enabled": False}
}


@app.get("/")
async def root():
    """Root endpoint."""
    return {
        "service": "il",
        "version": "0.1.0-demo",
        "mode": "demo",
        "description": "Demo server using test fixtures"
    }


@app.get("/health")
async def health():
    """Health check endpoint."""
    return {
        "status": "healthy",
        "mode": "demo",
        "message": "Demo server running with test fixtures"
    }


@app.get("/capabilities")
async def capabilities():
    """Get system capabilities."""
    return MOCK_CAPABILITIES


@app.get("/graph/state")
async def get_state():
    """Get current graph state."""
    return MOCK_STATE


@app.post("/graph/node")
async def create_node(node_data: Dict[str, Any]):
    """Create a new node."""
    node_id = node_data.get("id")
    if not node_id:
        raise HTTPException(status_code=400, detail="Node ID required")

    if node_id in MOCK_STATE["nodes"]:
        raise HTTPException(status_code=409, detail=f"Node {node_id} already exists")

    MOCK_STATE["nodes"][node_id] = {
        "type": node_data.get("type", "constant"),
        "value": node_data.get("value")
    }

    return {"id": node_id, "status": "created"}


@app.get("/graph/node/{node_id}")
async def get_node(node_id: str):
    """Get a specific node."""
    if node_id not in MOCK_STATE["nodes"]:
        raise HTTPException(status_code=404, detail=f"Node {node_id} not found")

    return {
        "id": node_id,
        **MOCK_STATE["nodes"][node_id]
    }


@app.post("/graph/edge")
async def create_edge(edge_data: Dict[str, Any]):
    """Create a new edge."""
    source = edge_data.get("source")
    target = edge_data.get("target")

    if not source or not target:
        raise HTTPException(status_code=400, detail="Source and target required")

    if source not in MOCK_STATE["nodes"]:
        raise HTTPException(status_code=404, detail=f"Source node {source} not found")

    if target not in MOCK_STATE["nodes"]:
        raise HTTPException(status_code=404, detail=f"Target node {target} not found")

    edge = {
        "source": source,
        "target": target,
        "label": edge_data.get("label", "")
    }

    MOCK_STATE["edges"].append(edge)

    return {"status": "created", "edge": edge}


@app.post("/graph/execute")
async def execute_graph(execution_data: Dict[str, Any]):
    """Execute the graph."""
    node_id = execution_data.get("node_id", "greeting")

    if node_id not in MOCK_STATE["nodes"]:
        raise HTTPException(status_code=404, detail=f"Node {node_id} not found")

    node = MOCK_STATE["nodes"][node_id]

    return {
        "status": "completed",
        "node_id": node_id,
        "result": node.get("value")
    }


@app.delete("/graph/reset")
async def reset_graph():
    """Reset graph to initial state."""
    global MOCK_STATE
    MOCK_STATE = {
        "nodes": {
            "greeting": {
                "type": "constant",
                "value": {"type": "text", "content": "Hello, World!"}
            }
        },
        "edges": []
    }
    return {"status": "reset", "message": "Graph reset to initial state"}


def main():
    """Start the demo server."""
    print("=" * 60)
    print("il Demo Server")
    print("=" * 60)
    print(f"Mode: Demo (using test fixtures)")
    print(f"Starting server on http://127.0.0.1:8000")
    print()
    print("Available endpoints:")
    print("  GET  /              - Service info")
    print("  GET  /health        - Health check")
    print("  GET  /capabilities  - System capabilities")
    print("  GET  /graph/state   - Current graph state")
    print("  POST /graph/node    - Create node")
    print("  GET  /graph/node/:id - Get node")
    print("  POST /graph/edge    - Create edge")
    print("  POST /graph/execute - Execute graph")
    print("  DELETE /graph/reset - Reset graph")
    print("=" * 60)
    print()

    uvicorn.run(app, host="127.0.0.1", port=8000, log_level="info")


if __name__ == "__main__":
    main()
