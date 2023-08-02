use std::sync::Arc;

use clap::Args;
use clap::Parser;
use clap::Subcommand;
use xrtc::service::run_http_service;
use xrtc::XrtcServer;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Run a xrtc server instance")]
    Run(RunArgs),
}

#[derive(Debug, Args)]
struct RunArgs {
    #[arg(long = "ice-server", default_value = "stun:stun.l.google.com:19302")]
    ice_servers: Vec<String>,
    #[arg(long = "bind", default_value = "127.0.0.1:6666")]
    service_address: String,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    xrtc::logging::init();

    match cli.command {
        Command::Run(args) => run(args).await,
    }
}

async fn run(args: RunArgs) {
    let server = Arc::new(XrtcServer::new(args.ice_servers));
    futures::join!(
        server.run(),
        run_http_service(&args.service_address, server.clone()),
    );
}
