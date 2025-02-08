# Tokio Chat Server Implementations

This repository contains two async chat server implementations in Rust using Tokio:

| Implementation  | Source |
|---------------|---------|
| Single-threaded | [server.rs](tokio-talk-st/src/server.rs) |
| Multi-threaded | [server.rs](tokio-talk-mt/src/server.rs) |

## Approaches

### Single-threaded Version
Uses non-`Send` futures and internal channels for message routing.
The internal channels are stored like this:
```rust
type ClientChannel = mpsc::Sender<ServerToClientMsg>;
type ClientMap = Rc<RefCell<HashMap<String, ClientChannel>>>;
```
Each client handler owns both reader and writer halves of the TCP stream. Messages are routed through 
channels before being written to the socket, ensuring sequential message processing on a single thread.

### Multi-threaded Version
Uses Tokio's thread pool with locks and avoid internal channels.
Per client there is one writer over the TCP stream stored in a map:
```rust
type SharedWriter = Mutex<MessageWriter<ServerToClientMsg, OwnedWriteHalf>>;
type ClientMap = Arc<RwLock<HashMap<String, SharedWriter>>>;
```
The writer half of the TCP stream is shared via the ClientMap, while each client handler future owns its reader. 


### Features
- Client name choice via Join messages
- Direct messaging between clients
- Broadcast messages to all connected clients
- User listing
- Connection health checks via Ping/Pong