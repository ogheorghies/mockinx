use clap::Parser;
use mockinx::server::{AppState, build_router};
use mockinx::stub::parse_stubs;
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(name = "mockinx", about = "Programmable HTTP test server")]
struct Cli {
    /// Port to listen on
    #[arg(default_value = "9999")]
    port: u16,

    /// Config file to load rules from at startup
    #[arg(short, long)]
    config: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let state = AppState::new();

    // Load config file if provided
    if let Some(ref config_path) = cli.config {
        match load_config(config_path, &state) {
            Ok(count) => eprintln!("loaded {count} rule(s) from {config_path}"),
            Err(e) => {
                eprintln!("error loading config {config_path}: {e}");
                std::process::exit(1);
            }
        }
    }

    let app = build_router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], cli.port));
    eprintln!("mockinx listening on {addr}");

    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

fn load_config(path: &str, state: &AppState) -> Result<usize, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;
    let val = yttp::parse(&content).map_err(|e| format!("parse error: {e}"))?;
    let stubs = parse_stubs(&val).map_err(|e| format!("rule error: {e}"))?;
    let count = stubs.len();
    state.register_stubs(stubs);
    Ok(count)
}
