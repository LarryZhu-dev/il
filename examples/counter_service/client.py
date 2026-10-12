#!/usr/bin/env python3
"""
Counter service client - Example usage of the HTTP API.
"""
import requests
import sys
import json


class CounterClient:
    """Client for counter service HTTP API."""

    def __init__(self, base_url: str = "http://127.0.0.1:8000"):
        self.base_url = base_url

    def _request(self, method: str, endpoint: str, **kwargs):
        """Make HTTP request."""
        url = f"{self.base_url}{endpoint}"
        try:
            response = requests.request(method, url, **kwargs)
            response.raise_for_status()
            return response.json()
        except requests.exceptions.RequestException as e:
            print(f"✗ Request failed: {e}")
            if hasattr(e, 'response') and e.response is not None:
                print(f"  Response: {e.response.text}")
            sys.exit(1)

    def health(self):
        """Check server health."""
        return self._request("GET", "/health")

    def state(self):
        """Get repository state."""
        return self._request("GET", "/api/state")

    def inspect_counter(self, revision=None):
        """Inspect counter entity."""
        payload = {"entity_id": "counter"}
        if revision is not None:
            payload["revision"] = revision
        return self._request("POST", "/api/inspect", json=payload)

    def increment_counter(self, base_revision: int):
        """Increment counter."""
        # Create transaction to increment counter
        transaction = {
            "base_revision": base_revision,
            "operations": [
                {
                    "op": "update_entity",
                    "entity_id": "counter",
                    "updates": {
                        "value": {"op": "increment"}
                    }
                }
            ]
        }
        return self._request("POST", "/api/transact", json=transaction)

    def get_counter_value(self):
        """Get current counter value."""
        state = self.state()
        current_revision = state["result"]["head_revision"]

        try:
            counter_data = self.inspect_counter(current_revision)
            return counter_data["result"]["entity"]["value"]
        except (KeyError, TypeError):
            return None


def demo():
    """Run demo of counter service."""
    client = CounterClient()

    print("=== Counter Service Demo ===\n")

    # Check health
    print("1. Checking server health...")
    health = client.health()
    print(f"   Status: {health.get('status')}")
    print(f"   Initialized: {health.get('initialized')}\n")

    # Get current state
    print("2. Getting repository state...")
    state = client.state()
    current_revision = state["result"]["head_revision"]
    print(f"   Current revision: {current_revision}")
    print(f"   Graph hash: {state['result']['graph_hash']}\n")

    # Inspect counter
    print("3. Inspecting counter entity...")
    try:
        counter = client.inspect_counter(current_revision)
        print(f"   Entity: {json.dumps(counter['result']['entity'], indent=2)}\n")
        initial_value = counter['result']['entity'].get('value', 0)
    except Exception as e:
        print(f"   Counter not found (this is expected for first run)")
        print(f"   Please run: python examples/counter_service/init_counter.py\n")
        return

    # Try to get counter value
    print("4. Getting counter value...")
    value = client.get_counter_value()
    if value is not None:
        print(f"   Counter value: {value}\n")
    else:
        print(f"   Could not read counter value\n")

    print("✓ Demo completed successfully!")
    print("\nNote: Full counter operations (increment/decrement/reset) require")
    print("implementing the transaction operations in the il program.")


if __name__ == "__main__":
    try:
        demo()
    except KeyboardInterrupt:
        print("\n\n✗ Demo interrupted")
        sys.exit(1)
