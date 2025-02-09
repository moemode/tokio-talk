# Tokio Chat Server Implementations

This repository contains two async chat server implementations in Rust using Tokio:

| Implementation  | Source |
|---------------|---------|
| Single-threaded | [server.rs](tokio-talk-st/src/server.rs) |
| Multi-threaded | [server.rs](tokio-talk-mt/src/server.rs) |

Originally developed when I self-studied the [Programming in Rust course](https://github.com/Kobzol/rust-course-fei) at FEI VŠB-TUO.
No solutions were available to me, I completely wrote the server.rs in both cases.
I also added a simple terminal client.

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


## How to use

### Starting the server
From the project directory, run:
```bash
# Start the server (it will print the port number)
cargo run --bin server
# Example output: Server listening on port 46521
```

### Connecting with a client
In another terminal, connect using:
```bash
cargo run --bin client -- --username <your_name> --port <server_port>

# Example:
cargo run --bin client -- --username alice --port 46521
```

### Available chat commands
Once connected, you can use these commands:
- `/h` - Show help menu
- `/l` - List connected users
- `/d <user> <message>` - Send private message
- `/b <message>` - Broadcast message to all users
- `/q` - Quit the chat

### Features
- Client name choice via Join messages
- Direct messaging between clients
- Broadcast messages to all connected clients
- User listing