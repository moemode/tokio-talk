# Single-threaded Async Chat Server

A Rust chat server implementation using Tokio's async/await with non-blocking I/O.  
The server is single thread but supports concurrent client connections through asynchronous task management.

## Core Components

| Component | Description | Source |
|-----------|-------------|--------|
| Server Implementation | Core server logic and client handling | [server.rs](tokio-talk-st/src/server.rs) |

## Architecture

The chat server uses a message-passing architecture built on Tokio's async runtime with the following key design elements:

### Single-threaded Concurrency
- Runs entirely on a single thread using `tokio::task::LocalSet`
- Uses `Rc<RefCell<>>` for shared state management instead of `Arc<Mutex<>>`
- Carefully manages `RefCell` borrows to prevent deadlocks across `await` points

### Message Routing
- Each connected client has a dedicated channel stored in a shared `Rc<RefCell<Map<ClientName, Channel>>>`
- Direct messages and broadcasts are sent through these channels rather than writing directly to sockets
- Prevents message interleaving by sequentially processing messages through the channels

### Client Handling
- Each client task runs a `select!` loop that multiplexes:
  - Incoming messages from the client's socket
  - Messages from other clients via the channel
- This design ensures atomic message delivery and maintains protocol integrity

### Benefits
- Simple concurrency model without multi-threading complexity
- No need for inter-thread synchronization
- Guaranteed message ordering through sequential processing
- Clean separation between message routing and socket I/O

### Features
- Client authentication via Join messages
- Direct messaging between clients
- Broadcast messages to all connected clients
- User listing
- Connection health checks via Ping/Pong