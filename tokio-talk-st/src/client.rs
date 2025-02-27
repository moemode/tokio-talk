use crate::messages::{ClientToServerMsg, ServerToClientMsg};
use crate::reader::MessageReader;
use crate::writer::MessageWriter;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

pub struct ClientReader {
    pub(crate) reader: MessageReader<ServerToClientMsg, OwnedReadHalf>,
}

pub struct ClientWriter {
    pub(crate) writer: MessageWriter<ClientToServerMsg, OwnedWriteHalf>,
}

impl ClientReader {
    pub async fn expect_message(&mut self, expected_from: &str, expected_message: &str) {
        let msg = self.recv().await;
        match msg {
            ServerToClientMsg::Message { from, message } => {
                assert_eq!(from, expected_from);
                assert_eq!(message, expected_message);
            }
            msg => panic!("Unexpected message {msg:?}"),
        }
    }

    pub async fn expect_error(&mut self, expected_error: &str) {
        let msg = self.recv().await;
        match msg {
            ServerToClientMsg::Error(error) => {
                assert_eq!(error, expected_error);
            }
            msg => {
                panic!("Unexpected response {msg:?}");
            }
        }
    }

    pub async fn recv(&mut self) -> ServerToClientMsg {
        self.reader
            .recv()
            .await
            .expect("connection was closed")
            .expect("did not receive welcome message")
    }

    pub async fn check_closed(mut self) {
        assert!(matches!(self.reader.recv().await, None | Some(Err(_))));
    }
}

impl ClientWriter {
    pub async fn join(&mut self, name: &str) {
        self.send(ClientToServerMsg::Join {
            name: name.to_string(),
        })
        .await;
    }

    pub async fn ping(&mut self) {
        self.send(ClientToServerMsg::Ping).await;
    }

    pub async fn list_users(&mut self) {
        self.send(ClientToServerMsg::ListUsers).await;
    }

    pub async fn dm(&mut self, to: &str, message: &str) {
        self.send(ClientToServerMsg::SendDM {
            to: to.to_string(),
            message: message.to_string(),
        })
        .await;
    }

    pub async fn broadcast(&mut self, message: &str) {
        self.send(ClientToServerMsg::Broadcast {
            message: message.to_string(),
        })
        .await;
    }

    pub async fn send(&mut self, msg: ClientToServerMsg) {
        self.writer.send(msg).await.expect("cannot send message");
    }

    pub async fn try_send(&mut self, msg: ClientToServerMsg) -> anyhow::Result<()> {
        self.writer.send(msg).await
    }

    pub async fn close(self) {
        self.writer.into_inner().shutdown().await.unwrap();
    }
}

pub struct Client {
    reader: ClientReader,
    writer: ClientWriter,
}

impl Client {
    pub async fn join(&mut self, name: &str) {
        self.writer.join(name).await;
        let msg = self.reader.recv().await;
        assert!(matches!(msg, ServerToClientMsg::Welcome));
    }

    pub async fn ping(&mut self) {
        self.writer.ping().await;
        let msg = self.reader.recv().await;
        assert!(matches!(msg, ServerToClientMsg::Pong));
    }

    pub async fn list_users(&mut self) -> Vec<String> {
        self.writer.list_users().await;
        let msg = self.reader.recv().await;
        match msg {
            ServerToClientMsg::UserList { mut users } => {
                users.sort();
                users
            }
            msg => {
                panic!("Unexpected response {msg:?}");
            }
        }
    }

    pub async fn dm(&mut self, to: &str, message: &str) {
        self.writer.dm(to, message).await;
    }

    pub async fn expect_message(&mut self, expected_from: &str, expected_message: &str) {
        self.reader
            .expect_message(expected_from, expected_message)
            .await;
    }

    pub async fn expect_error(&mut self, expected_error: &str) {
        self.reader.expect_error(expected_error).await;
    }

    pub async fn close(self) {
        self.writer.close().await;
    }

    pub async fn check_closed(self) {
        self.reader.check_closed().await;
    }
}

#[derive(Copy, Clone)]
pub struct ClientSpawner {
    port: u16,
}

impl ClientSpawner {
    pub fn new(port: u16) -> Self {
        Self { port }
    }

    pub async fn client(&self) -> (ClientReader, ClientWriter) {
        let client = TcpStream::connect(("127.0.0.1", self.port))
            .await
            .expect("cannot connect to server");

        let (rx, tx) = client.into_split();

        let reader = MessageReader::<ServerToClientMsg, _>::new(rx);
        let writer = MessageWriter::<ClientToServerMsg, _>::new(tx);
        (ClientReader { reader }, ClientWriter { writer })
    }

    pub async fn combined_client(&self) -> Client {
        let (reader, writer) = self.client().await;
        Client { reader, writer }
    }
}
