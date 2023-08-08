use std::net::SocketAddr;
use std::sync::Arc;

use clap::Args;
use clap::Parser;
use clap::Subcommand;
use tokio::net::TcpListener;
use xrtc::service::run_http_service;
use xrtc::ConnectionId;
use xrtc::XrtcServer;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Run an xrtc server instance")]
    Server(ServerArgs),

    #[command(about = "Run an xrtc client instance")]
    Client(ClientArgs),
}

#[derive(Debug, Args)]
struct ServerArgs {
    #[arg(long = "ice-server", default_value = "stun:stun.l.google.com:19302")]
    ice_servers: Vec<String>,
    #[arg(long = "endpoint", default_value = "127.0.0.1:7777")]
    service_address: String,
}

#[derive(Debug, Args)]
struct ClientArgs {
    #[arg(long = "ice-server", default_value = "stun:stun.l.google.com:19302")]
    ice_servers: Vec<String>,
    #[arg(long = "endpoint", default_value = "127.0.0.1:6666")]
    service_address: String,
    #[arg(long = "server", default_value = "127.0.0.1:7777")]
    server_service_address: String,
    #[arg(long = "listen", default_value = "127.0.0.1:5555")]
    proxy_listen_address: String,
    #[arg(long = "target")]
    server_target_address: String,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    xrtc::logging::init();

    match cli.command {
        Command::Server(args) => run_server(args).await,
        Command::Client(args) => run_client(args).await,
    }
}

async fn run_server(args: ServerArgs) {
    let server = Arc::new(XrtcServer::new(args.ice_servers));
    futures::join!(
        server.run(),
        run_http_service(server.clone(), &args.service_address),
    );
}

async fn run_client(args: ClientArgs) {
    let server = Arc::new(XrtcServer::new(args.ice_servers));
    let cid = uuid::Uuid::new_v4().to_string();

    let proxy_listen_address = args
        .proxy_listen_address
        .parse()
        .expect("Invalid proxy listen address");
    let server_target_address = args
        .server_target_address
        .parse()
        .expect("Invalid server target address");

    let wait_then_connect = async {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        reqwest::Client::new()
            .post(format!("http://{}/connect", args.service_address))
            .json(&xrtc::service::Connect {
                cid: cid.clone(),
                endpoint: format!("http://{}", args.server_service_address),
            })
            .send()
            .await
            .expect("Failed to connect to server");
    };

    futures::join!(
        server.run(),
        run_http_service(server.clone(), &args.service_address),
        run_proxy_listener(
            server.clone(),
            cid.clone(),
            proxy_listen_address,
            server_target_address
        ),
        wait_then_connect,
    );
}

async fn run_proxy_listener(
    xrtc_server: Arc<XrtcServer>,
    cid: ConnectionId,
    proxy_listen_address: SocketAddr,
    server_target_address: SocketAddr,
) {
    let listener = TcpListener::bind(proxy_listen_address)
        .await
        .expect("Failed to bind proxy listener");

    loop {
        let (socket, _) = listener
            .accept()
            .await
            .expect("Failed to accept connection");

        xrtc_server
            .dial(cid.to_string(), server_target_address, socket)
            .await
            .expect("Failed to dial");
    }
}
