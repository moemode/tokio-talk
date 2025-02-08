# Tokio Chat Server Implementations

This repository contains two async chat server implementations in Rust using Tokio:

| Implementation  | Source |
|---------------|---------|
| Single-threaded | [server.rs](tokio-talk-st/src/server.rs) |
| Multi-threaded | [server.rs](tokio-talk-mt/src/server.rs) |

## Approaches

In addition to the The single-threaded version uses internal channels for dispatching dms and broadcasts.
It uses !Send futures with `Rc<RefCell<>>` for shared state.

The multi-threaded version writes directly to client sockets and leverages Tokio's thread pool with `Arc<RwLock<>>` for thread-safe state sharing.

### Features
- Client name choice via Join messages
- Direct messaging between clients
- Broadcast messages to all connected clients
- User listing
- Connection health checks via Ping/Pong