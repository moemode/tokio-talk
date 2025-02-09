use clap::Parser;
use colored::*;
use std::io::{self, BufRead, Write};
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

fn print_prompt() {
    print!("{}", ">>> ".green());
    io::stdout().flush().unwrap();
}

fn clear_line() {
    print!("\r\x1b[K");
    io::stdout().flush().unwrap();
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
                    clear_line();
                    println!("{}: {}", from.cyan().bold(), message.cyan());
                    print_prompt();
                }
                ServerToClientMsg::Error(err) => {
                    clear_line();
                    eprintln!("{}", format!("Error: {}", err).red());
                    print_prompt();
                }
                ServerToClientMsg::UserList { users } => {
                    clear_line();
                    println!("{}", "Connected users:".yellow());
                    for user in users {
                        println!("  {}", user.yellow());
                    }
                    print_prompt();
                }
                ServerToClientMsg::Welcome => {
                    clear_line();
                    println!("{}", "Server acknowledged connection".yellow());
                    print_prompt();
                }
                ServerToClientMsg::Pong => {
                    clear_line();
                    println!("{}", "Server responded with pong".yellow());
                    print_prompt();
                }
            }
        }
    });

    let mut stdin = io::stdin().lock();
    let mut line = String::new();

    print_prompt();

    loop {
        line.clear();
        stdin.read_line(&mut line)?;
        let line = line.trim();

        if line.is_empty() {
            print_prompt();
            continue;
        }

        match handle_input(line, &mut writer).await {
            Ok(should_quit) => {
                if should_quit {
                    break;
                }
                print_prompt();
            }
            Err(e) => {
                eprintln!("{}", format!("Error: {}", e).red());
                print_prompt();
            }
        }
    }

    writer.close().await;
    reader_handle.abort();
    Ok(())
}

fn print_help() {
    println!("\n{}", "Available commands:".yellow());
    println!("  {}  - Show this help", "/h".green());
    println!("  {}  - List connected users", "/l".green());
    println!(
        "  {} {} - Send private message to user",
        "/d".green(),
        "<user> msg".cyan()
    );
    println!(
        "  {} {} - Broadcast message to all users",
        "/b".green(),
        "<message>".cyan()
    );
    println!("  {}  - Exit the chat", "/q".green());
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
