use clap::Parser;
use mockinx::server::{AppState, build_router};
use mockinx::rule::parse_rules;
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(
    name = "mockinx",
    about = "Mock server with codeless config of pacing, drops, throttling, and chaos",
    after_help = "Full docs: https://crates.io/crates/mockinx"
)]
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
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}

fn load_config(path: &str, state: &AppState) -> Result<usize, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;
    let val = yttp::parse(&content).map_err(|e| format!("parse error: {e}"))?;
    let rules = parse_rules(&val).map_err(|e| format!("rule error: {e}"))?;
    let count = rules.len();
    state.register_rules(rules);
    Ok(count)
}
