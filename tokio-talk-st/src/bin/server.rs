use clap::Parser;
use tokio::runtime::Builder;
use tokio::task::LocalSet;
use tokio_talk_st::{run_server, RunningServer, ServerOpts};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Maximum number of clients that can connect
    #[arg(short, long, default_value_t = 100)]
    max_clients: usize,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let opts = ServerOpts {
        max_clients: args.max_clients,
    };

    // Build a current-thread runtime
    let rt = Builder::new_current_thread().enable_all().build()?;
    // Create a LocalSet for non-Send tasks.
    let local = LocalSet::new();

    rt.block_on(local.run_until(async move {
        let server: RunningServer = run_server(opts).await?;
        let port = server.port;
        println!("Server listening on port {}", port);

        let server_fut = tokio::task::spawn_local(server.future);

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("Received Ctrl+C, shutting down...");
                let _ = server.tx.send(());
            }
            result = server_fut => {
                let join_res = result; // capture without moving multiple times
                if let Err(ref e) = join_res {
                    eprintln!("Server error: {}", e);
                }
                join_res??;
            }
        }
        println!("Server stopped");
        Ok(())
    }))
}
