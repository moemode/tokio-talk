use clap::Parser;
use std::io::{self, BufRead};
use tokio_talk_mt::client::ClientSpawner;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Server address to connect to
    #[arg(short, long, default_value = "127.0.0.1")]
    address: String,

    /// Server port to connect to
    #[arg(short, long)]
    port: u16,

    /// Username to join with
    #[arg(short, long)]
    username: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let spawner = ClientSpawner::new(args.port);

    println!("Connecting to {}:{}", args.address, args.port);
    let mut client = spawner.client().await;

    println!("Joining as {}", args.username);
    client.join(&args.username).await;
    println!("Successfully joined chat!");
    print_help();

    let mut stdin = io::stdin().lock();
    let mut line = String::new();

    loop {
        line.clear();
        stdin.read_line(&mut line)?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        match handle_input(line, &mut client).await {
            Ok(should_quit) => {
                if should_quit {
                    break;
                }
            }
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    client.close().await;
    Ok(())
}

fn print_help() {
    println!("\nAvailable commands:");
    println!("  /help           - Show this help");
    println!("  /list           - List connected users");
    println!("  /dm <user> msg  - Send private message to user");
    println!("  /quit           - Exit the chat");
    println!("  message         - Send broadcast message to all users");
}

async fn handle_input(
    line: &str,
    client: &mut tokio_talk_mt::client::Client,
) -> anyhow::Result<bool> {
    if line.starts_with('/') {
        let parts: Vec<&str> = line[1..].splitn(2, ' ').collect();
        match parts[0] {
            "help" => print_help(),
            "quit" => return Ok(true),
            "list" => {
                let users = client.list_users().await;
                println!("Connected users:");
                for user in users {
                    println!("  {}", user);
                }
            }
            "dm" => {
                if parts.len() != 2 {
                    println!("Usage: /dm <username> message");
                    return Ok(false);
                }
                let dm_parts: Vec<&str> = parts[1].splitn(2, ' ').collect();
                if dm_parts.len() != 2 {
                    println!("Usage: /dm <username> message");
                    return Ok(false);
                }
                client.dm(dm_parts[0], dm_parts[1]).await;
            }
            _ => println!("Unknown command. Type /help for available commands."),
        }
    } else {
        use tokio_talk_mt::messages::ClientToServerMsg;
        client
            .send(ClientToServerMsg::Broadcast {
                message: line.to_string(),
            })
            .await;
    }

    // Start a background task to receive messages
    tokio::spawn({
        async move {
            match client.recv().await {
                tokio_talk_mt::messages::ServerToClientMsg::Message { from, message } => {
                    println!("{}: {}", from, message);
                }
                tokio_talk_mt::messages::ServerToClientMsg::Error(err) => {
                    eprintln!("Error: {}", err);
                }
                _ => {}
            }
        }
    });

    Ok(false)
}
