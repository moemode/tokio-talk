use crate::messages::{ClientToServerMsg, ServerToClientMsg};
use crate::reader::MessageReader;
use crate::writer::MessageWriter;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

pub struct Client {
    pub(crate) writer: MessageWriter<ClientToServerMsg, OwnedWriteHalf>,
    pub(crate) reader: MessageReader<ServerToClientMsg, OwnedReadHalf>,
}

impl Client {
    pub async fn join(&mut self, name: &str) {
        self.send(ClientToServerMsg::Join {
            name: name.to_string(),
        })
        .await;
        let msg = self.recv().await;
        assert!(matches!(msg, ServerToClientMsg::Welcome));
    }

    pub async fn ping(&mut self) {
        self.send(ClientToServerMsg::Ping).await;
        let msg = self.recv().await;
        assert!(matches!(msg, ServerToClientMsg::Pong));
    }

    pub async fn list_users(&mut self) -> Vec<String> {
        self.send(ClientToServerMsg::ListUsers).await;
        let msg = self.recv().await;
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
        self.send(ClientToServerMsg::SendDM {
            to: to.to_string(),
            message: message.to_string(),
        })
        .await;
    }

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

    pub async fn send(&mut self, msg: ClientToServerMsg) {
        self.writer.send(msg).await.expect("cannot send message");
    }

    pub async fn try_send(&mut self, msg: ClientToServerMsg) -> anyhow::Result<()> {
        self.writer.send(msg).await
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

    pub async fn close(self) {
        self.writer.into_inner().shutdown().await.unwrap();
    }

    pub async fn check_closed(mut self) {
        assert!(matches!(self.reader.recv().await, None | Some(Err(_))));
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

    pub async fn client(&self) -> Client {
        let client = TcpStream::connect(("127.0.0.1", self.port))
            .await
            .expect("cannot connect to server");

        let (rx, tx) = client.into_split();

        let reader = MessageReader::<ServerToClientMsg, _>::new(rx);
        let writer = MessageWriter::<ClientToServerMsg, _>::new(tx);
        Client { reader, writer }
    }
}
