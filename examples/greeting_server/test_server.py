#!/usr/bin/env python3
"""Test script for greeting_server."""

import requests
import json
import time
import sys


def test_health(base_url):
    """Test health endpoint."""
    print("[TEST] GET /health")
    try:
        response = requests.get(f"{base_url}/health", timeout=5)
        print(f"  Status: {response.status_code}")
        print(f"  Response: {response.json()}")

        if response.status_code == 200:
            data = response.json()
            if "status" in data and "timestamp" in data:
                print("  [PASS] Health check successful")
                return True
            else:
                print("  [FAIL] Missing required fields")
                return False
        else:
            print("  [FAIL] Wrong status code")
            return False
    except Exception as e:
        print(f"  [FAIL] {e}")
        return False


def test_greet_success(base_url):
    """Test successful greet."""
    print("\n[TEST] POST /greet (success)")
    try:
        response = requests.post(
            f"{base_url}/greet",
            json={"name": "Alice"},
            headers={"Content-Type": "application/json"},
            timeout=5
        )
        print(f"  Status: {response.status_code}")
        print(f"  Response: {response.json()}")

        if response.status_code == 200:
            data = response.json()
            if "message" in data and "timestamp" in data:
                if "Alice" in data["message"]:
                    print("  [PASS] Greet successful")
                    return True
                else:
                    print("  [FAIL] Name not in message")
                    return False
            else:
                print("  [FAIL] Missing required fields")
                return False
        else:
            print("  [FAIL] Wrong status code")
            return False
    except Exception as e:
        print(f"  [FAIL] {e}")
        return False


def test_greet_error(base_url):
    """Test greet with missing name."""
    print("\n[TEST] POST /greet (error - missing name)")
    try:
        response = requests.post(
            f"{base_url}/greet",
            json={},
            headers={"Content-Type": "application/json"},
            timeout=5
        )
        print(f"  Status: {response.status_code}")
        print(f"  Response: {response.json()}")

        if response.status_code == 400:
            data = response.json()
            if "error" in data:
                print("  [PASS] Error handling works")
                return True
            else:
                print("  [FAIL] Missing error field")
                return False
        else:
            print("  [FAIL] Wrong status code (expected 400)")
            return False
    except Exception as e:
        print(f"  [FAIL] {e}")
        return False


def test_invalid_route(base_url):
    """Test invalid route."""
    print("\n[TEST] GET /invalid (404)")
    try:
        response = requests.get(f"{base_url}/invalid", timeout=5)
        print(f"  Status: {response.status_code}")

        if response.status_code == 404:
            print("  [PASS] 404 handling works")
            return True
        else:
            print("  [FAIL] Wrong status code (expected 404)")
            return False
    except Exception as e:
        print(f"  [FAIL] {e}")
        return False


def wait_for_server(base_url, timeout=30):
    """Wait for server to become available."""
    print(f"[INFO] Waiting for server at {base_url} (timeout: {timeout}s)")

    start = time.time()
    while time.time() - start < timeout:
        try:
            response = requests.get(f"{base_url}/health", timeout=1)
            if response.status_code == 200:
                print("[OK] Server is ready")
                return True
        except:
            pass
        time.sleep(1)

    print("[FAIL] Server did not become ready in time")
    return False


def run_all_tests(base_url):
    """Run all tests."""
    print("=" * 60)
    print("GREETING SERVER TEST SUITE")
    print("=" * 60)
    print(f"Target: {base_url}\n")

    results = []

    # Test health
    results.append(("Health Check", test_health(base_url)))

    # Test successful greet
    results.append(("Greet Success", test_greet_success(base_url)))

    # Test error handling
    results.append(("Error Handling", test_greet_error(base_url)))

    # Test 404
    results.append(("404 Handling", test_invalid_route(base_url)))

    # Summary
    print("\n" + "=" * 60)
    print("TEST SUMMARY")
    print("=" * 60)

    passed = 0
    failed = 0

    for name, result in results:
        status = "[PASS]" if result else "[FAIL]"
        print(f"{status} {name}")
        if result:
            passed += 1
        else:
            failed += 1

    print()
    print(f"Total: {len(results)} tests")
    print(f"Passed: {passed}")
    print(f"Failed: {failed}")
    print()

    if failed == 0:
        print("[SUCCESS] All tests passed!")
        return 0
    else:
        print("[FAILURE] Some tests failed")
        return 1


def main():
    import argparse

    parser = argparse.ArgumentParser(description="Test greeting_server")
    parser.add_argument("--url", default="http://localhost:8080", help="Server URL")
    parser.add_argument("--wait", action="store_true", help="Wait for server to start")
    parser.add_argument("--timeout", type=int, default=30, help="Wait timeout")

    args = parser.parse_args()

    if args.wait:
        if not wait_for_server(args.url, args.timeout):
            sys.exit(1)

    sys.exit(run_all_tests(args.url))


if __name__ == "__main__":
    main()
