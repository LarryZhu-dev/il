#!/usr/bin/env python3
"""
Test script for il demo server.
"""
import requests
import json
import sys
from typing import Dict, Any


BASE_URL = "http://127.0.0.1:8000"


def print_section(title: str):
    """Print a section header."""
    print(f"\n{'=' * 60}")
    print(f"  {title}")
    print('=' * 60)


def print_response(response: requests.Response):
    """Print a formatted response."""
    print(f"Status: {response.status_code}")
    try:
        data = response.json()
        print(json.dumps(data, indent=2))
    except:
        print(response.text)


def test_basic_endpoints():
    """Test basic server endpoints."""
    print_section("Basic Endpoints")

    print("\n1. Root endpoint:")
    r = requests.get(f"{BASE_URL}/")
    print_response(r)

    print("\n2. Health check:")
    r = requests.get(f"{BASE_URL}/health")
    print_response(r)

    print("\n3. Capabilities:")
    r = requests.get(f"{BASE_URL}/capabilities")
    print_response(r)


def test_graph_operations():
    """Test graph operations."""
    print_section("Graph Operations")

    print("\n1. Get initial state:")
    r = requests.get(f"{BASE_URL}/graph/state")
    print_response(r)

    print("\n2. Execute greeting node:")
    r = requests.post(f"{BASE_URL}/graph/execute",
                     json={"node_id": "greeting"})
    print_response(r)

    print("\n3. Create new node 'user_name':")
    r = requests.post(f"{BASE_URL}/graph/node",
                     json={
                         "id": "user_name",
                         "type": "constant",
                         "value": {"type": "text", "content": "Alice"}
                     })
    print_response(r)

    print("\n4. Get node 'user_name':")
    r = requests.get(f"{BASE_URL}/graph/node/user_name")
    print_response(r)

    print("\n5. Create edge from greeting to user_name:")
    r = requests.post(f"{BASE_URL}/graph/edge",
                     json={
                         "source": "greeting",
                         "target": "user_name",
                         "label": "uses"
                     })
    print_response(r)

    print("\n6. Get updated state:")
    r = requests.get(f"{BASE_URL}/graph/state")
    print_response(r)


def test_error_handling():
    """Test error handling."""
    print_section("Error Handling")

    print("\n1. Get non-existent node:")
    r = requests.get(f"{BASE_URL}/graph/node/nonexistent")
    print_response(r)

    print("\n2. Create duplicate node:")
    r = requests.post(f"{BASE_URL}/graph/node",
                     json={
                         "id": "greeting",
                         "type": "constant",
                         "value": {"type": "text", "content": "Duplicate"}
                     })
    print_response(r)

    print("\n3. Create edge with invalid source:")
    r = requests.post(f"{BASE_URL}/graph/edge",
                     json={
                         "source": "invalid",
                         "target": "greeting",
                         "label": "test"
                     })
    print_response(r)


def test_reset():
    """Test graph reset."""
    print_section("Graph Reset")

    print("\n1. Reset graph:")
    r = requests.delete(f"{BASE_URL}/graph/reset")
    print_response(r)

    print("\n2. Verify state after reset:")
    r = requests.get(f"{BASE_URL}/graph/state")
    print_response(r)


def main():
    """Run all tests."""
    print("=" * 60)
    print("  il Demo Server Test Suite")
    print("=" * 60)
    print(f"Testing server at: {BASE_URL}")

    try:
        # Check if server is running
        r = requests.get(f"{BASE_URL}/health", timeout=2)
        if r.status_code != 200:
            print(f"\nError: Server returned status {r.status_code}")
            sys.exit(1)
    except requests.exceptions.ConnectionError:
        print("\nError: Cannot connect to server.")
        print("Please start the server with: python demo_server.py")
        sys.exit(1)

    try:
        test_basic_endpoints()
        test_graph_operations()
        test_error_handling()
        test_reset()

        print("\n" + "=" * 60)
        print("  All tests completed successfully!")
        print("=" * 60)

    except Exception as e:
        print(f"\nError during testing: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()
