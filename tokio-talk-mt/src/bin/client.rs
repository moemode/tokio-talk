use clap::Parser;
use std::io::{self, BufRead};
use tokio_talk_mt::client::ClientSpawner;
use tokio_talk_mt::messages::ServerToClientMsg;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "127.0.0.1")]
    address: String,

    #[arg(short, long)]
    port: u16,

    #[arg(short, long)]
    username: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let spawner = ClientSpawner::new(args.port);

    println!("Connecting to {}:{}", args.address, args.port);
    let (mut reader, mut writer) = spawner.client().await;

    println!("Joining as {}", args.username);
    writer.join(&args.username).await;

    // Wait for welcome message
    match reader.recv().await {
        ServerToClientMsg::Welcome => println!("Successfully joined chat!"),
        _ => panic!("Did not receive welcome message"),
    }

    print_help();

    // Spawn message receiver task
    let reader_handle = tokio::spawn(async move {
        loop {
            match reader.recv().await {
                ServerToClientMsg::Message { from, message } => {
                    println!("{}: {}", from, message);
                }
                ServerToClientMsg::Error(err) => {
                    eprintln!("Error: {}", err);
                }
                ServerToClientMsg::UserList { users } => {
                    println!("Connected users:");
                    for user in users {
                        println!("  {}", user);
                    }
                }
                ServerToClientMsg::Welcome => {
                    println!("Server acknowledged connection");
                }
                ServerToClientMsg::Pong => {
                    println!("Server responded with pong");
                }
            }
        }
    });

    let mut stdin = io::stdin().lock();
    let mut line = String::new();

    loop {
        line.clear();
        stdin.read_line(&mut line)?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        match handle_input(line, &mut writer).await {
            Ok(should_quit) => {
                if should_quit {
                    break;
                }
            }
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    writer.close().await;
    reader_handle.abort();
    Ok(())
}

fn print_help() {
    println!("\nAvailable commands:");
    println!("  /h             - Show this help");
    println!("  /l             - List connected users");
    println!("  /d <user> msg  - Send private message to user");
    println!("  /b <message>   - Broadcast message to all users");
    println!("  /q             - Exit the chat");
    println!("  message        - Send broadcast message to all users");
}

async fn handle_input(
    line: &str,
    writer: &mut tokio_talk_mt::client::ClientWriter,
) -> anyhow::Result<bool> {
    if line.starts_with('/') {
        let parts: Vec<&str> = line[1..].splitn(2, ' ').collect();
        match parts[0] {
            "h" => print_help(),
            "q" => return Ok(true),
            "l" => {
                writer.list_users().await;
            }
            "b" => {
                if parts.len() != 2 {
                    println!("Usage: /b <message>");
                    return Ok(false);
                }
                writer.broadcast(parts[1]).await;
            }
            "d" => {
                if parts.len() != 2 {
                    println!("Usage: /d <username> message");
                    return Ok(false);
                }
                let dm_parts: Vec<&str> = parts[1].splitn(2, ' ').collect();
                if dm_parts.len() != 2 {
                    println!("Usage: /d <username> message");
                    return Ok(false);
                }
                writer.dm(dm_parts[0], dm_parts[1]).await;
            }
            _ => println!("Unknown command. Type /h for available commands."),
        }
    }

    Ok(false)
}
