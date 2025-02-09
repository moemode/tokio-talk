use clap::Parser;
use tokio_talk_st::{run_server, RunningServer, ServerOpts};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Maximum number of clients that can connect
    #[arg(short, long, default_value_t = 100)]
    max_clients: usize,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let opts = ServerOpts {
        max_clients: args.max_clients,
    };
    let server: RunningServer = run_server(opts).await?;
    let port = server.port;
    println!("Server listening on port {}", port);
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("Received Ctrl+C, shutting down...");
            let _ = server.tx.send(());
        }
        result = server.future => {
            if let Err(e) = result {
                eprintln!("Server error: {}", e);
            }
        }
    }
    println!("Server stopped");
    Ok(())
}
