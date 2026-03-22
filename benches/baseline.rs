/// Minimal axum server returning static JSON — the performance ceiling.
/// Used as a baseline for comparing mockinx overhead.
use axum::{Router, routing::get, response::IntoResponse, http::HeaderValue};

async fn handler() -> impl IntoResponse {
    let mut resp = axum::response::Response::new(
        axum::body::Body::from(r#"{"name":"Owl","price":5.99}"#),
    );
    resp.headers_mut().insert(
        "content-type",
        HeaderValue::from_static("application/json"),
    );
    resp
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(9998);

    let app = Router::new().route("/api/test", get(handler));
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    eprintln!("baseline listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
