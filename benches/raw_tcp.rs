/// Raw TCP server — reads until \r\n\r\n, writes hardcoded HTTP response.
/// No HTTP parsing, no framework. The absolute performance ceiling.
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 27\r\n\r\n{\"name\":\"Owl\",\"price\":5.99}";

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(9997);

    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await.unwrap();
    eprintln!("raw_tcp listening on 0.0.0.0:{port}");

    loop {
        let (mut stream, _) = listener.accept().await.unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            loop {
                // Read request (just enough to find end of headers)
                let n = match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                // Check if we got the end of headers
                if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    let _ = stream.write_all(RESPONSE).await;
                } else {
                    // Partial read — just respond anyway (good enough for benchmarks)
                    let _ = stream.write_all(RESPONSE).await;
                }
            }
        });
    }
}
