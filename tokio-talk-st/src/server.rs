use crate::{
    messages::{ClientToServerMsg, ServerToClientMsg},
    reader::MessageReader,
    writer::MessageWriter,
};
use std::{cell::RefCell, collections::HashMap, future::Future, pin::Pin, rc::Rc};
use tokio::{
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    task::JoinSet,
};

type ClientChannel = tokio::sync::mpsc::UnboundedSender<ServerToClientMsg>;
type ClientMap = Rc<RefCell<HashMap<String, ClientChannel>>>;

/// A running chat server instance.
///
/// # Protocol
/// The server implements the following connection protocol:
///
/// ## Client Connection
/// 1. When a client connects, it must send a `Join` message within 2 seconds
/// 2. If the client:
///    - Does not send a `Join` message in time -> "Timed out waiting for Join" error
///    - Sends any other message -> "Unexpected message received" error
///    - Sends `Join` with taken username -> "Username already taken" error
///    - Sends valid `Join` -> Receives `Welcome` message
///
/// ## Connected State
/// Once connected, clients can:
/// - Send direct messages (DM) to other users
/// - Broadcast messages to all users
/// - List connected users
/// - Send ping messages
/// - Receive messages from other users
///
/// ## Error Conditions
/// - Server is full (>= max_clients) -> "Server is full" error
/// - Sending DM to self -> "Cannot send a DM to yourself" error
/// - Sending DM to nonexistent user -> "User <name> does not exist" error
/// - Sending `Join` after already joined -> "Unexpected message received" error
///
/// ## Implementation Notes
/// The server is single-threaded and uses `Rc<RefCell<>>` for shared state.
/// Care is taken not to hold `RefCell` borrows across `await` points to prevent
/// deadlocks and maintain the aliasing XOR mutability invariant.
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

async fn run_server(
    mut rx: tokio::sync::oneshot::Receiver<()>,
    listener: tokio::net::TcpListener,
    max_clients: usize,
) -> anyhow::Result<()> {
    let clients: ClientMap = Rc::new(RefCell::new(HashMap::new()));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut rx => {
                break;
            }
            Ok((client, _)) = listener.accept() => {
                let (rx, tx) = client.into_split();
                let reader = MessageReader::<ClientToServerMsg, _>::new(rx);
                let mut writer = MessageWriter::<ServerToClientMsg, _>::new(tx);
                if tasks.len() >= max_clients {
                    writer.send(ServerToClientMsg::Error("Server is full".to_owned())).await?;
                    continue;
                }
                tasks.spawn_local(handle_client(reader, writer, clients.clone()));
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
    writer: &mut MessageWriter<ServerToClientMsg, OwnedWriteHalf>,
    clients: &ClientMap,
) -> anyhow::Result<String> {
    tokio::select! {
        msg = reader.recv() => match msg {
            Some(Ok(ClientToServerMsg::Join { name })) => {
                if clients.borrow().contains_key(&name) {
                    writer.send(ServerToClientMsg::Error("Username already taken".to_owned())).await?;
                    return Err(anyhow::anyhow!("Username already taken"));
                }
                writer.send(ServerToClientMsg::Welcome).await?;
                Ok(name)
            }
            Some(Ok(_)) => {
                writer.send(ServerToClientMsg::Error("Unexpected message received".to_owned())).await?;
                Err(anyhow::anyhow!("Unexpected message received"))
            }
            Some(Err(e)) => Err(e.into()),
            None => {
                writer.send(ServerToClientMsg::Error("Connection closed".to_owned())).await?;
                Err(anyhow::anyhow!("Connection closed"))
            }
        },
        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
            writer.send(ServerToClientMsg::Error("Timed out waiting for Join".to_owned())).await?;
            Err(anyhow::anyhow!("Timed out waiting for Join"))
        }
    }
}

/// Then it should start receiving requests from the client.
/// - If the client ever sends the `Join` message again, the server should respond with an error
/// "Unexpected message received" and disconnect the client immediately.
async fn handle_client(
    mut reader: MessageReader<ClientToServerMsg, OwnedReadHalf>,
    mut writer: MessageWriter<ServerToClientMsg, OwnedWriteHalf>,
    clients: ClientMap,
) -> anyhow::Result<()> {
    let name = join_client(&mut reader, &mut writer, &clients).await?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    clients.borrow_mut().insert(name.clone(), tx);
    loop {
        tokio::select! {
            msg = reader.recv() => match msg {
                Some(Ok(msg)) => {
                    if (react_client_msg(msg, &name, &mut writer, &clients).await).is_err() {
                        break;
                    }
                }
                _ => break,
            },
            msg = rx.recv() => match msg {
                Some(msg) => writer.send(msg).await?,
                None => break,
            }
        }
    }
    clients.borrow_mut().remove(&name);
    Ok(())
}

/// pub enum ClientToServerMsg {
/// This is the first message in the communication, which should be sent by the client.
/// When some other client with the same name already exists, the server should respond
/// with an error "Username already taken" and disconnect the new client.
/// Join { name: String },
/// This message checks that the connection is OK.
/// The server should respond with [ServerToClientMsg::Pong].
/// Ping,
/// Send a request to list the usernames of users currently connected to the server.
/// The order of the usernames is not important.
/// ListUsers,
/// Sends a direct message to the user with the given name (`to`).
/// If the user does not exist, the server responds with an error "User <to> does not exist".
/// If the client tries to send a message to themselves, the server responds with an error
/// "Cannot send a DM to yourself".
/// SendDM { to: String, message: String },
/// Sends a message to all currently connected users (except for the sender of the broadcast).
/// Broadcast { message: String },
/// }
async fn react_client_msg(
    msg: ClientToServerMsg,
    name: &str,
    writer: &mut MessageWriter<ServerToClientMsg, OwnedWriteHalf>,
    clients: &ClientMap,
) -> anyhow::Result<()> {
    match msg {
        ClientToServerMsg::Ping => {
            writer.send(ServerToClientMsg::Pong).await?;
        }
        ClientToServerMsg::ListUsers => {
            let users = clients.borrow().keys().cloned().collect();
            writer.send(ServerToClientMsg::UserList { users }).await?;
        }
        ClientToServerMsg::Broadcast { message } => {
            for (client_name, channel) in clients.borrow().iter() {
                if client_name != name {
                    channel.send(ServerToClientMsg::Message {
                        from: name.into(),
                        message: message.clone(),
                    })?;
                }
            }
        }
        ClientToServerMsg::SendDM { to, message } => {
            if to == *name {
                writer
                    .send(ServerToClientMsg::Error(
                        "Cannot send a DM to yourself".to_owned(),
                    ))
                    .await?;
                return Ok(());
            }
            if let Some(channel) = clients.borrow().get(&to) {
                channel.send(ServerToClientMsg::Message {
                    from: name.into(),
                    message,
                })?;
                return Ok(());
            }
            writer
                .send(ServerToClientMsg::Error(format!(
                    "User {to} does not exist"
                )))
                .await?;
            return Ok(());
        }
        _ => {
            writer
                .send(ServerToClientMsg::Error(
                    "Unexpected message received".to_owned(),
                ))
                .await?;
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
