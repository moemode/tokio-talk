use clap::Parser;
use colored::*;
use std::io::{self, BufRead, Write};
use tokio_talk_st::client::ClientSpawner;
use tokio_talk_st::messages::ServerToClientMsg;

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

struct TerminalUi;

impl TerminalUi {
    fn print_prompt() {
        print!("{}", ">>> ".green());
        io::stdout().flush().unwrap();
    }

    fn clear_line() {
        print!("\r\x1b[K");
        io::stdout().flush().unwrap();
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
}

struct CommandHandler<'a> {
    writer: &'a mut tokio_talk_st::client::ClientWriter,
}

impl<'a> CommandHandler<'a> {
    fn new(writer: &'a mut tokio_talk_st::client::ClientWriter) -> Self {
        Self { writer }
    }

    async fn handle_command(&mut self, command: &str, args: Option<&str>) -> anyhow::Result<bool> {
        match command {
            "h" => {
                TerminalUi::print_help();
                Ok(false)
            }
            "q" => Ok(true),
            "l" => {
                self.writer.list_users().await;
                Ok(false)
            }
            "b" => {
                if let Some(message) = args {
                    self.writer.broadcast(message).await;
                    Ok(false)
                } else {
                    println!("Usage: /b <message>");
                    Ok(false)
                }
            }
            "d" => {
                if let Some(args) = args {
                    let dm_parts: Vec<&str> = args.splitn(2, ' ').collect();
                    if dm_parts.len() == 2 {
                        self.writer.dm(dm_parts[0], dm_parts[1]).await;
                        Ok(false)
                    } else {
                        println!("Usage: /d <username> message");
                        Ok(false)
                    }
                } else {
                    println!("Usage: /d <username> message");
                    Ok(false)
                }
            }
            _ => {
                println!("Unknown command. Type /h for available commands.");
                Ok(false)
            }
        }
    }
}

async fn handle_server_messages(mut reader: tokio_talk_st::client::ClientReader) {
    loop {
        match reader.recv().await {
            ServerToClientMsg::Message { from, message } => {
                TerminalUi::clear_line();
                println!("{}: {}", from.cyan().bold(), message.cyan());
            }
            ServerToClientMsg::Error(err) => {
                TerminalUi::clear_line();
                eprintln!("{}", format!("Error: {}", err).red());
            }
            ServerToClientMsg::UserList { users } => {
                TerminalUi::clear_line();
                println!("{}", "Connected users:".yellow());
                for user in users {
                    println!("  {}", user.yellow());
                }
            }
            ServerToClientMsg::Welcome => {
                TerminalUi::clear_line();
                println!("{}", "Server acknowledged connection".yellow());
            }
            ServerToClientMsg::Pong => {
                TerminalUi::clear_line();
                println!("{}", "Server responded with pong".yellow());
            }
        }
        TerminalUi::print_prompt();
    }
}

async fn run_command_loop(
    stdin: &mut impl BufRead,
    line: &mut String,
    command_handler: &mut CommandHandler<'_>,
) -> anyhow::Result<()> {
    loop {
        line.clear();
        stdin.read_line(line)?;
        let line = line.trim();

        if line.starts_with('/') {
            let parts: Vec<&str> = line[1..].splitn(2, ' ').collect();
            let command = parts[0];
            let args = parts.get(1).copied();
            match command_handler.handle_command(command, args).await {
                Ok(should_quit) => {
                    if should_quit {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("{}", format!("Error: {}", e).red());
                }
            }
        }
        TerminalUi::print_prompt();
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let spawner = ClientSpawner::new(args.port);

    println!("Connecting to {}:{}", args.address, args.port);
    let (mut reader, mut writer) = spawner.client().await;

    println!("Joining as {}", args.username);
    writer.join(&args.username).await;

    match reader.recv().await {
        ServerToClientMsg::Welcome => println!("Successfully joined chat!"),
        ServerToClientMsg::Error(e) if e == "Username already taken" => {
            eprintln!(
                "{}",
                "Error: Username already taken. Please try a different username.".red()
            );
            return Ok(());
        }
        msg => panic!("Unexpected response from server: {:?}", msg),
    }

    TerminalUi::print_help();

    let reader_handle = tokio::spawn(handle_server_messages(reader));

    let mut stdin = io::stdin().lock();
    let mut line = String::new();
    let mut command_handler = CommandHandler::new(&mut writer);

    TerminalUi::print_prompt();

    run_command_loop(&mut stdin, &mut line, &mut command_handler).await?;

    writer.close().await;
    reader_handle.abort();
    Ok(())
}
