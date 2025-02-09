//! Multi-threaded async chat server
//!
//! Unlike the single-threaded version, this implementation:
//! - Directly writes to client sockets for DMs and broadcasts
//! - Uses no internal message channels between clients
//! - Relies on Tokio's thread pool for concurrent message handling
//! - Uses async locks (RwLock/Mutex) for safe concurrent access
//! - Uses `SharedWriter = Mutex<MessageWriter<ServerToClientMsg, OwnedWriteHalf>>`
//!   and `ClientMap = Arc<RwLock<HashMap<String, SharedWriter>>>` to improve throughput.
//!   Further improvements could be made using a lock-free map.
//!
//! The server is multi-threaded and uses `Arc<RwLock<>>` for shared state.
//! Each client's writer is protected by a `Mutex` to ensure atomic writes.
//! The shared state is stored in `Arc<RwLock<Map<ClientName, Mutex<Writer>>>>`.
//!
//! # Protocol
//! The server implements the following connection protocol:
//!
//! ## Client Connection
//! 1. When a client connects, it must send a `Join` message within 2 seconds
//! 2. If the client:
//!    - Does not send a `Join` message in time -> "Timed out waiting for Join" error
//!    - Sends any other message -> "Unexpected message received" error
//!    - Sends `Join` with taken username -> "Username already taken" error
//!    - Sends valid `Join` -> Receives `Welcome` message
//!
//! ## Connected State
//! Once connected, clients can:
//! - Send direct messages (DM) to other users
//! - Broadcast messages to all users
//! - List connected users
//! - Send ping messages
//! - Receive messages from other users
//!
//! ## Error Conditions
//! - Server is full (>= max_clients) -> "Server is full" error
//! - Sending DM to self -> "Cannot send a DM to yourself" error
//! - Sending DM to nonexistent user -> "User <name> does not exist" error
//! - Sending `Join` after already joined -> "Unexpected message received" error
use crate::{
    messages::{ClientToServerMsg, ServerToClientMsg},
    reader::MessageReader,
    writer::MessageWriter,
};
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};
use tokio::task::JoinSet;
use tokio::{
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::{Mutex, RwLock},
};

type SharedWriter = Mutex<MessageWriter<ServerToClientMsg, OwnedWriteHalf>>;
type ClientMap = Arc<RwLock<HashMap<String, SharedWriter>>>;

/// A running chat server instance
pub struct RunningServer {
    /// Maximum number of clients that can be connected to the server
    max_clients: usize,
    /// Port on which the server is running
    pub port: u16,
    /// Main future of the server
    pub future: Pin<Box<dyn Future<Output = anyhow::Result<()>>>>,
    /// Channel that can be used to tell the server to stop
    pub tx: tokio::sync::oneshot::Sender<()>,
}

/// Main server loop that accepts client connections and manages client tasks.
///
/// # Arguments
/// * `rx` - Receiver for shutdown signal
/// * `listener` - TCP listener for accepting new connections
/// * `max_clients` - Maximum number of concurrent client connections allowed
///
/// # Operation
/// The server loop:
/// 1. Accepts new client connections
/// 2. Rejects connections when at max capacity
/// 3. Spawns a task for each connected client on the Tokio thread pool
/// 4. Monitors client tasks for completion
/// 5. Gracefully shuts down when signaled
///
/// The loop continues until a shutdown signal is received through `rx`.
/// During shutdown, it stops accepting new connections and waits for existing
/// client tasks to complete.
async fn run_server(
    mut rx: tokio::sync::oneshot::Receiver<()>,
    listener: tokio::net::TcpListener,
    max_clients: usize,
) -> anyhow::Result<()> {
    let clients: ClientMap = Arc::new(RwLock::new(HashMap::new()));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut rx => break,
            Ok((client, _)) = listener.accept() => {
                let (rx, tx) = client.into_split();
                let reader = MessageReader::<ClientToServerMsg, _>::new(rx);
                let mut writer = MessageWriter::<ServerToClientMsg, _>::new(tx);
                if tasks.len() >= max_clients {
                    writer.send(ServerToClientMsg::Error("Server is full".to_owned())).await?;
                    continue;
                }
                tasks.spawn(handle_client(reader, Mutex::new(writer), clients.clone()));
            }
            task_res = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Err(e)) = task_res {
                    println!("Error in client task: {e}");
                }
            }
        }
    }
    Ok(())
}

/// Handles sending an error message and returns an error with the same message
async fn handle_join_error(writer: &SharedWriter, error_msg: &str) -> anyhow::Result<String> {
    let mut locked_writer = writer.lock().await;
    locked_writer
        .send(ServerToClientMsg::Error(error_msg.to_owned()))
        .await?;
    Err(anyhow::anyhow!(error_msg.to_owned()))
}

/// Handles successful join by sending welcome message
async fn handle_join_success(writer: &SharedWriter, name: String) -> anyhow::Result<String> {
    let mut locked_writer = writer.lock().await;
    locked_writer.send(ServerToClientMsg::Welcome).await?;
    Ok(name)
}

/// Handles the initial connection handshake with a client.
///
/// # Protocol
/// The client must send a `Join` message as its first message. Upon receiving this message,
/// the server will respond with a `Welcome` message if the join is successful.
///
/// # Errors
/// This function will return an error if:
/// - The client does not send a `Join` message within 2 seconds
/// - The client sends any message other than `Join` as its first message
/// - The connection is closed before receiving a message
///
/// # Returns
/// - `Ok(String)` containing the client's username if join is successful
/// - `Err` if the join fails for any reason listed above
async fn join_client(
    reader: &mut MessageReader<ClientToServerMsg, OwnedReadHalf>,
    writer: &SharedWriter,
    clients: &ClientMap,
) -> anyhow::Result<String> {
    tokio::select! {
        msg = reader.recv() => match msg {
            Some(Ok(ClientToServerMsg::Join { name })) => {
                if clients.read().await.contains_key(&name) {
                    handle_join_error(writer, "Username already taken").await
                } else {
                    handle_join_success(writer, name).await
                }
            }
            Some(Ok(_)) => handle_join_error(writer, "Unexpected message received").await,
            Some(Err(e)) => Err(e.into()),
            None => handle_join_error(writer, "Connection closed").await,
        },
        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
            handle_join_error(writer, "Timed out waiting for Join").await
        }
    }
}

/// Handles a connected client's message flow after successful connection.
///
/// # Protocol Flow
/// 1. Performs initial client join handshake
/// 2. Adds client's writer to shared client map
/// 3. Processes incoming messages until client disconnects
/// 4. Removes client from active clients when connection ends
///
/// Unlike the single-threaded version, this implementation:
/// - Directly writes to client sockets
/// - Uses no message channels between clients
/// - Client writer is protected by a Mutex for thread-safe access
///
/// # Arguments
/// * `reader` - Stream for receiving messages from the client
/// * `writer` - Mutex-protected writer for sending messages to the client
/// * `clients` - Thread-safe shared map of all connected clients
///
/// # Returns
/// * `Ok(())` when client disconnects normally
/// * `Err` if there are communication errors
async fn handle_client(
    mut reader: MessageReader<ClientToServerMsg, OwnedReadHalf>,
    writer: SharedWriter,
    clients: ClientMap,
) -> anyhow::Result<()> {
    let name = join_client(&mut reader, &writer, &clients).await?;
    clients.write().await.insert(name.clone(), writer);
    while let Some(Ok(msg)) = reader.recv().await {
        if react_client_msg(msg, &name, &clients).await.is_err() {
            break;
        }
    }
    clients.write().await.remove(&name);
    Ok(())
}

/// Handles sending an error message to a client
async fn send_error(writer: &SharedWriter, error_msg: String) -> anyhow::Result<()> {
    let mut locked_writer = writer.lock().await;
    locked_writer
        .send(ServerToClientMsg::Error(error_msg))
        .await
}

/// Broadcast a message to all clients except the sender
async fn broadcast(
    name: &str,
    message: String,
    clients_guard: &HashMap<String, SharedWriter>,
) -> anyhow::Result<()> {
    for (client_name, writer) in clients_guard.iter() {
        if client_name != name {
            let mut writer = writer.lock().await;
            writer
                .send(ServerToClientMsg::Message {
                    from: name.into(),
                    message: message.clone(),
                })
                .await?;
        }
    }
    Ok(())
}

/// Send a direct message to a specific client
async fn dm(
    from: &str,
    to: String,
    message: String,
    sender: &SharedWriter,
    clients_guard: &HashMap<String, SharedWriter>,
) -> anyhow::Result<()> {
    if to == from {
        return send_error(sender, "Cannot send a DM to yourself".to_owned()).await;
    }
    let target = match clients_guard.get(&to) {
        Some(writer) => writer,
        None => {
            return send_error(sender, format!("User {to} does not exist")).await;
        }
    };
    let mut writer = target.lock().await;
    writer
        .send(ServerToClientMsg::Message {
            from: from.into(),
            message,
        })
        .await?;
    Ok(())
}

/// Processes an individual client message and sends appropriate responses.
///
/// # Message Handling
/// * `Ping` - Responds with `Pong`
/// * `ListUsers` - Responds with list of connected usernames
/// * `Broadcast` - Sends message to all other connected clients
/// * `SendDM` - Delivers direct message to specific client
/// * Other messages - Responds with error and disconnects client
///
/// # Arguments
/// * `msg` - The client message to process
/// * `name` - Username of the sending client
/// * `writer` - Stream for sending responses back to client
/// * `clients` - Shared map of all connected clients
///
/// # Returns
/// * `Ok(())` if message was handled successfully
/// * `Err` for protocol violations or communication errors
async fn react_client_msg(
    msg: ClientToServerMsg,
    name: &str,
    clients: &ClientMap,
) -> anyhow::Result<()> {
    let clients_guard = clients.read().await;
    let sender = match clients_guard.get(name) {
        Some(writer) => writer,
        None => return Ok(()),
    };
    match msg {
        ClientToServerMsg::Ping => {
            let mut writer = sender.lock().await;
            writer.send(ServerToClientMsg::Pong).await?;
        }
        ClientToServerMsg::ListUsers => {
            let users = clients_guard.keys().cloned().collect();
            let mut writer = sender.lock().await;
            println!(
                "Sending user list to {name}: {users:?}",
                name = name,
                users = users
            );
            writer.send(ServerToClientMsg::UserList { users }).await?;
        }
        ClientToServerMsg::Broadcast { message } => {
            broadcast(name, message, &clients_guard).await?;
        }
        ClientToServerMsg::SendDM { to, message } => {
            dm(name, to, message, sender, &clients_guard).await?;
        }
        _ => {
            send_error(sender, "Unexpected message received".to_owned()).await?;
            anyhow::bail!("Unexpected message received");
        }
    }
    Ok(())
}

impl RunningServer {
    pub async fn new(max_clients: usize) -> anyhow::Result<Self> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let server_future = run_server(rx, listener, max_clients);
        Ok(RunningServer {
            max_clients,
            port,
            future: Box::pin(server_future),
            tx,
        })
    }
}
