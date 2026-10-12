#!/usr/bin/env python3
"""Test client for greeting server."""
import http.client
import sys
import time


def test_greeting():
    """Test the /greeting endpoint."""
    print("Testing greeting server...")

    try:
        conn = http.client.HTTPConnection("127.0.0.1", 8080, timeout=5)

        # Test GET /greeting
        print("\n1. GET /greeting")
        conn.request("GET", "/greeting")
        response = conn.getresponse()

        print(f"   Status: {response.status} {response.reason}")
        print(f"   Headers: {dict(response.headers)}")

        body = response.read().decode('utf-8')
        print(f"   Body: {body}")

        if response.status == 200 and body == "hi, this is a server based il":
            print("   ✅ PASS")
        else:
            print("   ❌ FAIL")
            return False

        conn.close()
        return True

    except ConnectionRefusedError:
        print("❌ Cannot connect to server. Is it running?")
        return False
    except Exception as e:
        print(f"❌ Error: {e}")
        return False


def test_invalid_path():
    """Test an invalid path (should get 404)."""
    print("\n2. GET /invalid")

    try:
        conn = http.client.HTTPConnection("127.0.0.1", 8080, timeout=5)
        conn.request("GET", "/invalid")
        response = conn.getresponse()

        print(f"   Status: {response.status} {response.reason}")
        body = response.read().decode('utf-8')
        print(f"   Body: {body[:100]}")

        if response.status == 404:
            print("   ✅ PASS (expected 404)")
        else:
            print(f"   ⚠️  Got {response.status}, expected 404")

        conn.close()
        return True

    except Exception as e:
        print(f"❌ Error: {e}")
        return False


def test_invalid_method():
    """Test an invalid method (should get 405 or similar)."""
    print("\n3. POST /greeting")

    try:
        conn = http.client.HTTPConnection("127.0.0.1", 8080, timeout=5)
        conn.request("POST", "/greeting")
        response = conn.getresponse()

        print(f"   Status: {response.status} {response.reason}")
        body = response.read().decode('utf-8')
        print(f"   Body: {body[:100]}")

        if response.status in (404, 405):
            print(f"   ✅ PASS (got {response.status})")
        else:
            print(f"   ⚠️  Got {response.status}, expected 404 or 405")

        conn.close()
        return True

    except Exception as e:
        print(f"❌ Error: {e}")
        return False


def benchmark():
    """Simple benchmark of the server."""
    print("\n4. Benchmark (1000 requests)")

    try:
        start = time.time()
        success = 0

        for i in range(1000):
            conn = http.client.HTTPConnection("127.0.0.1", 8080, timeout=5)
            conn.request("GET", "/greeting")
            response = conn.getresponse()
            response.read()

            if response.status == 200:
                success += 1

            conn.close()

            if (i + 1) % 100 == 0:
                print(f"   Progress: {i + 1}/1000")

        elapsed = time.time() - start
        rps = 1000 / elapsed

        print(f"\n   Completed: {success}/1000 successful")
        print(f"   Time: {elapsed:.2f}s")
        print(f"   Throughput: {rps:.2f} req/s")
        print(f"   Latency: {elapsed/1000*1000:.2f}ms per request")

        return success == 1000

    except Exception as e:
        print(f"❌ Error: {e}")
        return False


def main():
    """Run all tests."""
    print("=" * 60)
    print("Greeting Server Test Suite")
    print("=" * 60)

    tests = [
        ("Basic greeting", test_greeting),
        ("Invalid path", test_invalid_path),
        ("Invalid method", test_invalid_method),
    ]

    results = []
    for name, test_fn in tests:
        try:
            passed = test_fn()
            results.append((name, passed))
        except KeyboardInterrupt:
            print("\n\n⚠️  Tests interrupted")
            sys.exit(1)

    # Run benchmark if basic tests pass
    if all(r[1] for r in results):
        print("\n" + "=" * 60)
        print("Basic tests passed! Running benchmark...")
        print("=" * 60)
        try:
            benchmark()
        except KeyboardInterrupt:
            print("\n\n⚠️  Benchmark interrupted")

    # Summary
    print("\n" + "=" * 60)
    print("Test Summary")
    print("=" * 60)
    for name, passed in results:
        status = "✅ PASS" if passed else "❌ FAIL"
        print(f"{status} - {name}")

    passed_count = sum(1 for _, p in results if p)
    total = len(results)
    print(f"\nPassed: {passed_count}/{total}")

    sys.exit(0 if passed_count == total else 1)


if __name__ == "__main__":
    main()
