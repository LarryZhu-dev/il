# Counter Service Example

A simple counter service built with il backend.

## What it does

This service provides a counter that can be incremented, decremented, and reset through HTTP API.

## il Program

The counter is implemented as an il program with:
- State: A counter value stored in the graph
- Operations: increment, decrement, reset, get

## API Endpoints

- `POST /counter/increment` - Increment counter by 1
- `POST /counter/decrement` - Decrement counter by 1
- `POST /counter/reset` - Reset counter to 0
- `GET /counter/value` - Get current counter value

## Running

From the il project root:

```bash
# Start the server
python quick_start.py

# In another terminal, initialize counter
python examples/counter_service/init_counter.py

# Use the API
python examples/counter_service/client.py
```

## Architecture

```
counter_service/
├── counter.il          # il program source
├── init_counter.py     # Initialize counter in repository
├── client.py           # Example client
└── README.md           # This file
```

## il Program Structure

```il
entity counter {
    field value: Int64;
    
    operation increment() {
        this.value = this.value + 1;
    }
    
    operation decrement() {
        this.value = this.value - 1;
    }
    
    operation reset() {
        this.value = 0;
    }
    
    operation get() -> Int64 {
        return this.value;
    }
}
```

## How it works

1. The `init_counter.py` script parses the il source and creates a transaction to add the counter entity to the repository
2. The HTTP server accepts API requests and translates them to il transactions
3. Each counter operation becomes a transaction that modifies the graph
4. The state persists in the `.il/project` store between requests
