# Multi-threaded Async Chat Server

A Rust chat server implementation using Tokio's async/await with true multi-threading support.
The server leverages Tokio's thread pool to handle concurrent client connections and message processing.
Find at [server.rs](src/server.rs).

## Architecture

The chat server uses a thread-safe shared state architecture built on Tokio's multi-threaded runtime with the following key design elements:

### Multi-threaded Concurrency
- Runs on Tokio's multi-threaded runtime using the thread pool
- Uses `Arc<RwLock<>>` for thread-safe shared state management
- Each client's writer is protected by a `Mutex` for atomic writes
- Shared state stored in `Arc<RwLock<Map<ClientName, Mutex<Writer>>>>`

### Direct Message Writing
- Writes directly to client sockets for DMs and broadcasts
- No internal message channels between clients
- Socket writes are protected by per-client Mutex locks
- Improved throughput compared to channel-based messaging

### Client Handling
- Each client runs in its own task on the thread pool
- Uses async `RwLock` to safely access the shared client map
- Atomic message delivery through Mutex-protected writers
- Efficient concurrent message processing

### Features
- Client name choice via Join messages
- Direct messaging between clients
- Broadcast messages to all connected clients
- User listing
- Connection health checks via Ping/Pong
