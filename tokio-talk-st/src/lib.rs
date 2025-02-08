#![warn(clippy::await_holding_refcell_ref)]

//! A chat server implementation using tokio with non-blocking I/O.
//!
//! The server is designed to be single-threaded and supports multiple concurrent client connections
//! using async/await. It provides basic chat functionality including direct messages and broadcasts.
//!
//! # Threading
//! The server uses spawn_local to spawn the threads, which are !Send because they use Rc<RefCell<>>.
//! As it is single-threaded, it uses `Rc<RefCell<>>` for shared state management.

pub use crate::server::RunningServer;

mod messages;
mod reader;
mod writer;

mod server;

#[derive(Copy, Clone)]
pub struct ServerOpts {
    /// Maximum number of clients that can be connected to the server at once.
    pub max_clients: usize,
}

/// Prepare to start a chat server on a dynamically assigned TCP/IP port.
///
/// Returns a [`RunningServer`] which contains the server's port, control channel for shutdown,
/// and the main server future.
///
/// # Arguments
/// * `opts` - Server configuration
pub async fn run_server(opts: ServerOpts) -> anyhow::Result<RunningServer> {
    RunningServer::new(opts.max_clients).await
}

#[cfg(test)]
mod tests {
    use crate::messages::{ClientToServerMsg, ServerToClientMsg};
    use crate::reader::MessageReader;
    use crate::writer::MessageWriter;
    use crate::{run_server, ServerOpts};
    use std::cell::{Cell, RefCell};
    use std::future::Future;
    use std::rc::Rc;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;
    use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
    use tokio::net::TcpStream;
    use tokio::task::LocalSet;

    #[tokio::test]
    async fn empty_server_shuts_down() {
        run_test(opts(2), |_| async move { Ok(()) }).await;
    }

    #[tokio::test]
    async fn max_clients() {
        run_test(opts(2), |server| async move {
            let _client = server.client().await;
            let _client2 = server.client().await;
            let mut client3 = server.client().await;
            client3.expect_error("Server is full").await;
            client3.check_closed().await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn max_clients_after_client_leaves() {
        run_test(opts(2), |spawner| async move {
            let _client = spawner.client().await;
            let client2 = spawner.client().await;
            client2.close().await;

            sleep(500).await;

            let mut client3 = spawner.client().await;
            client3.join("Foo").await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn max_clients_herd() {
        let max_clients = 5;
        run_test(opts(max_clients), |spawner| async move {
            let client_count = 50;

            let errors = Rc::new(Cell::new(0));
            let successes = Rc::new(Cell::new(0));

            let joined_clients = Rc::new(RefCell::new(vec![]));

            let futs = (0..client_count).map(|client_id| {
                let errors = errors.clone();
                let successes = successes.clone();
                let joined_clients = joined_clients.clone();

                async move {
                    let mut client = spawner.client().await;
                    let _ = client
                        .try_send(ClientToServerMsg::Join {
                            name: format!("Client {client_id}"),
                        })
                        .await;
                    match client.recv().await {
                        ServerToClientMsg::Error(_) => {
                            errors.set(errors.get() + 1);
                        }
                        ServerToClientMsg::Welcome => {
                            successes.set(successes.get() + 1);
                            // Make sure that the client doesn't disconnect
                            joined_clients.borrow_mut().push(client);
                        }
                        msg => {
                            panic!("Unexpected message {msg:?}");
                        }
                    }
                }
            });
            futures_util::future::join_all(futs).await;

            assert_eq!(errors.get(), client_count - max_clients);
            assert_eq!(successes.get(), max_clients);

            drop(joined_clients);

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn list_users_before_join() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            client.send(ClientToServerMsg::ListUsers).await;
            client.expect_error("Unexpected message received").await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn join_after_half_sec() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            sleep(500).await;
            client.join("Foo").await;
            assert_eq!(client.list_users().await, vec!["Foo".to_string()]);

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn join_timeout() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            sleep(3000).await;
            match client
                .try_send(ClientToServerMsg::Join {
                    name: "Bilbo".to_string(),
                })
                .await
            {
                Ok(_) => {
                    client.expect_error("Timed out waiting for Join").await;
                }
                Err(_) => {}
            }

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn duplicated_join() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            client.join("Foo").await;
            client
                .send(ClientToServerMsg::Join {
                    name: "Bar".to_string(),
                })
                .await;
            client.expect_error("Unexpected message received").await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn error_then_disconnect() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            client.join("Foo").await;
            client
                .send(ClientToServerMsg::Join {
                    name: "Bar".to_string(),
                })
                .await;
            client.close().await;

            let mut client2 = spawner.client().await;
            client2.join("Bar").await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn duplicated_username() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            client.join("Foo").await;

            let mut client2 = spawner.client().await;
            client2
                .send(ClientToServerMsg::Join {
                    name: "Foo".to_string(),
                })
                .await;
            client2.expect_error("Username already taken").await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn ping() {
        run_test(opts(2), |spawner| async move {
            let mut luca = spawner.client().await;
            luca.join("Luca").await;
            luca.ping().await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn ping_before_join() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            client.send(ClientToServerMsg::Ping).await;
            client.expect_error("Unexpected message received").await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn list_users_reconnect() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            client.join("Foo").await;
            client.close().await;

            let mut client = spawner.client().await;
            client.join("Foo").await;
            assert_eq!(client.list_users().await, vec!["Foo".to_string()]);

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn list_users_self() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            client.join("Martin").await;
            assert_eq!(client.list_users().await, vec!["Martin".to_string()]);

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn list_users_ignore_not_joined_users() {
        run_test(opts(2), |spawner| async move {
            let _client = spawner.client().await;
            let mut client2 = spawner.client().await;
            client2.join("Joe").await;
            assert_eq!(client2.list_users().await, vec!["Joe".to_string()]);

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn list_users_after_error() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            client.join("Terrence").await;

            let mut client2 = spawner.client().await;
            client2.join("Joe").await;

            client
                .send(ClientToServerMsg::Join {
                    name: "Barbara".to_string(),
                })
                .await;

            sleep(1000).await;

            assert_eq!(client2.list_users().await, vec!["Joe".to_string()]);

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn list_users() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            client.join("Terrence").await;

            let mut client2 = spawner.client().await;
            client2.join("Joe").await;
            assert_eq!(
                client2.list_users().await,
                vec!["Joe".to_string(), "Terrence".to_string()]
            );
            client2.close().await;

            sleep(1000).await;

            assert_eq!(client.list_users().await, vec!["Terrence".to_string()]);

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn dm_nonexistent_user() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            client.join("Mark").await;
            client.dm("Fiona", "Hi").await;
            client.expect_error("User Fiona does not exist").await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn dm_self() {
        run_test(opts(2), |spawner| async move {
            let mut client = spawner.client().await;
            client.join("Xal'atath").await;
            client.dm("Xal'atath", "I'm so lonely :(").await;
            client.expect_error("Cannot send a DM to yourself").await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn dm_other() {
        run_test(opts(2), |spawner| async move {
            let mut terrence = spawner.client().await;
            terrence.join("Terrence").await;

            let mut joe = spawner.client().await;
            joe.join("Joe").await;

            terrence.dm("Joe", "How you doin'").await;
            joe.expect_message("Terrence", "How you doin'").await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn dm_spam() {
        run_test(opts(2), |spawner| async move {
            let mut diana = spawner.client().await;
            diana.join("Diana").await;

            let mut francesca = spawner.client().await;
            francesca.join("Francesca").await;

            let count = 10000;

            // Let's say that someone is spamming you...
            let t1 = async move {
                for _ in 0..count {
                    diana
                        .dm("Francesca", "Can I borrow your brush? Pleeeeeease :(((")
                        .await;
                }
            };

            // ...so you get angry, and start spamming them back.
            // But you make a critical *error*, because you're sending the message
            // to the wrong account.
            // Can your chat server handle that?
            let t2 = async move {
                for _ in 0..count {
                    francesca.dm("Daina", "NO! Get your own!").await;
                    match francesca.recv().await {
                        ServerToClientMsg::Message { from, message } => {
                            assert_eq!(from, "Diana");
                            assert_eq!(message, "Can I borrow your brush? Pleeeeeease :(((");
                        }
                        ServerToClientMsg::Error(error) => {
                            assert_eq!(error, "User Daina does not exist");
                        }
                        msg => panic!("Unexpected message {msg:?}"),
                    }
                }
                // Francesca should receive count * 2 messages, `count` from Diana and `count`
                // error messages
                for _ in 0..count {
                    match francesca.recv().await {
                        ServerToClientMsg::Message { from, message } => {
                            assert_eq!(from, "Diana");
                            assert_eq!(message, "Can I borrow your brush? Pleeeeeease :(((");
                        }
                        ServerToClientMsg::Error(error) => {
                            assert_eq!(error, "User Daina does not exist");
                        }
                        msg => panic!("Unexpected message {msg:?}"),
                    }
                }
            };

            // Wait until both processes complete
            let t1 = tokio::task::spawn_local(t1);
            let t2 = tokio::task::spawn_local(t2);
            let (ret1, ret2) = tokio::join!(t1, t2);
            Ok(ret1.and(ret2)?)
        })
        .await;
    }

    #[tokio::test]
    async fn dm_spam_2() {
        // Meanwhile, in a parallel universe...
        run_test(opts(2), |spawner| async move {
            let mut diana = spawner.client().await;
            diana.join("Diana").await;

            let mut francesca = spawner.client().await;
            francesca.join("Francesca").await;

            let count = 10000;

            // Let's say that someone is spamming you...
            let t1 = async move {
                for _ in 0..count {
                    diana
                        .dm("Francesca", "Can I borrow your brush? Pleeeeeease :(((")
                        .await;
                }
            };

            // ...so you get angry, and start spamming them back.
            // But you make a critical *error*, because you push the wrong button and start
            // sending pings to the server instead.
            // Can your chat server handle that?
            let t2 = async move {
                for _ in 0..count {
                    francesca.send(ClientToServerMsg::Ping).await;
                    match francesca.recv().await {
                        ServerToClientMsg::Message { from, message } => {
                            assert_eq!(from, "Diana");
                            assert_eq!(message, "Can I borrow your brush? Pleeeeeease :(((");
                        }
                        ServerToClientMsg::Pong => {}
                        msg => panic!("Unexpected message {msg:?}"),
                    }
                }
                // Francesca should receive count * 2 messages, `count` from Diana and `count`
                // pong messages
                for _ in 0..count {
                    match francesca.recv().await {
                        ServerToClientMsg::Message { from, message } => {
                            assert_eq!(from, "Diana");
                            assert_eq!(message, "Can I borrow your brush? Pleeeeeease :(((");
                        }
                        ServerToClientMsg::Pong => {}
                        msg => panic!("Unexpected message {msg:?}"),
                    }
                }
            };

            let t1 = tokio::task::spawn_local(t1);
            let t2 = tokio::task::spawn_local(t2);
            let (ret1, ret2) = tokio::join!(t1, t2);
            Ok(ret1.and(ret2)?)
        })
        .await;
    }

    #[tokio::test]
    async fn broadcast_empty() {
        run_test(opts(2), |spawner| async move {
            let mut ji = spawner.client().await;
            ji.join("Ji").await;
            ji.send(ClientToServerMsg::Broadcast {
                message: "Haaaaaai!".to_string(),
            })
            .await;
            ji.ping().await;

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn broadcast() {
        run_test(opts(10), |spawner| async move {
            let mut niko = spawner.client().await;
            niko.join("Niko").await;

            let users: Vec<_> = (0..5)
                .map(|i| async move {
                    let mut client = spawner.client().await;
                    client.join(&format!("NPC {i}")).await;
                    client
                })
                .collect();
            let users: Vec<Client> = futures_util::future::join_all(users).await;

            niko.send(ClientToServerMsg::Broadcast {
                message: "Borrow this!".to_string(),
            })
            .await;
            niko.ping().await;

            for mut user in users {
                user.expect_message("Niko", "Borrow this!").await;
            }

            Ok(())
        })
        .await;
    }

    /*
    #[tokio::test]
    async fn message_timeout() {
        run_test(opts(2), |spawner| async move {
            let mut niko = spawner.client().await;
            niko.join("Niko").await;

            sleep(1000).await;
            niko.ping().await;
            sleep(1000).await;
            niko.list_users().await;
            sleep(4000).await;

            niko.expect_error("Timeouted").await;
            niko.check_closed().await;

            Ok(())
        })
        .await;
    }
    */

    // This test runs for ~10s
    #[tokio::test]
    async fn message_timeout_receiving_dms() {
        run_test(opts(10), |spawner| async move {
            // Do not timeout user if he is receiving DMs
            let mut niko = spawner.client().await;
            niko.join("Niko").await;

            let mut kobzol = spawner.client().await;
            kobzol.join("Kobzol").await;

            sleep(1500).await;
            kobzol.dm("Niko", "Hi there!").await;
            niko.recv().await;
            sleep(1500).await;
            kobzol.dm("Niko", "So, what you're up to?").await;
            niko.recv().await;
            sleep(1500).await;
            kobzol.dm("Niko", "See you at RustWeek?").await;
            niko.recv().await;
            sleep(1500).await;
            kobzol
                .send(ClientToServerMsg::Broadcast {
                    message: "Rust is really cool, you know".to_string(),
                })
                .await;
            niko.recv().await;
            sleep(2000).await;
            kobzol
                .send(ClientToServerMsg::Broadcast {
                    message: "...anyone here?".to_string(),
                })
                .await;
            niko.recv().await;
            sleep(2000).await;

            niko.ping().await;

            Ok(())
        })
        .await;
    }

    // The server should correctly close client sockets when it shuts down,
    // to avoid a situation where the clients would be stuck waiting for a message
    // for some indeterminate amount of time.
    #[tokio::test]
    async fn drop_clients_on_shutdown() {
        let (mut client, mut client2) = run_test(opts(10), |spawner| async move {
            let mut client = spawner.client().await;
            client.join("Bar").await;
            let mut client2 = spawner.client().await;
            client2.join("Foo").await;
            Ok((client, client2))
        })
        .await;

        assert!(client.reader.recv().await.is_none());
        assert!(client2.reader.recv().await.is_none());
    }

    async fn run_test<C, F, R>(opts: ServerOpts, func: C) -> R
    where
        C: FnOnce(ClientSpawner) -> F,
        F: Future<Output = anyhow::Result<R>>,
    {
        let localset = LocalSet::new();
        let (port, ret) = localset
            .run_until(async {
                // Start the server
                let server = run_server(opts).await.expect("creating server failed");
                let port = server.port;

                let spawner = ClientSpawner { port };

                // Spawn the server future
                let server_fut = tokio::task::spawn_local(server.future);

                // Run the test
                let ret = func(spawner).await.expect("test failed");

                // Tell the server to shut down
                server.tx.send(()).unwrap();

                // Wait until it shuts down
                server_fut.await.unwrap().unwrap();

                (port, ret)
            })
            .await;

        TcpStream::connect(("127.0.0.1", port))
            .await
            .expect_err("server is still alive");
        ret
    }

    struct Client {
        writer: MessageWriter<ClientToServerMsg, OwnedWriteHalf>,
        reader: MessageReader<ServerToClientMsg, OwnedReadHalf>,
    }

    impl Client {
        async fn join(&mut self, name: &str) {
            self.send(ClientToServerMsg::Join {
                name: name.to_string(),
            })
            .await;
            let msg = self.recv().await;
            assert!(matches!(msg, ServerToClientMsg::Welcome));
        }

        async fn ping(&mut self) {
            self.send(ClientToServerMsg::Ping).await;
            let msg = self.recv().await;
            assert!(matches!(msg, ServerToClientMsg::Pong));
        }

        async fn list_users(&mut self) -> Vec<String> {
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

        async fn dm(&mut self, to: &str, message: &str) {
            self.send(ClientToServerMsg::SendDM {
                to: to.to_string(),
                message: message.to_string(),
            })
            .await;
        }

        async fn expect_message(&mut self, expected_from: &str, expected_message: &str) {
            let msg = self.recv().await;
            match msg {
                ServerToClientMsg::Message { from, message } => {
                    assert_eq!(from, expected_from);
                    assert_eq!(message, expected_message);
                }
                msg => panic!("Unexpected message {msg:?}"),
            }
        }

        async fn send(&mut self, msg: ClientToServerMsg) {
            self.writer.send(msg).await.expect("cannot send message");
        }

        async fn try_send(&mut self, msg: ClientToServerMsg) -> anyhow::Result<()> {
            self.writer.send(msg).await
        }

        async fn expect_error(&mut self, expected_error: &str) {
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

        async fn recv(&mut self) -> ServerToClientMsg {
            self.reader
                .recv()
                .await
                .expect("connection was closed")
                .expect("did not receive welcome message")
        }

        async fn close(self) {
            self.writer.into_inner().shutdown().await.unwrap();
        }

        async fn check_closed(mut self) {
            assert!(matches!(self.reader.recv().await, None | Some(Err(_))));
        }
    }

    #[derive(Copy, Clone)]
    struct ClientSpawner {
        port: u16,
    }

    impl ClientSpawner {
        async fn client(&self) -> Client {
            let client = TcpStream::connect(("127.0.0.1", self.port))
                .await
                .expect("cannot connect to server");

            let (rx, tx) = client.into_split();

            let reader = MessageReader::<ServerToClientMsg, _>::new(rx);
            let writer = MessageWriter::<ClientToServerMsg, _>::new(tx);
            Client { reader, writer }
        }
    }

    async fn sleep(duration_ms: u64) {
        tokio::time::sleep(Duration::from_millis(duration_ms)).await;
    }

    fn opts(max_clients: usize) -> ServerOpts {
        ServerOpts { max_clients }
    }
}
