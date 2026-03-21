use mockinx::server::{AppState, build_router};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Test server helper: starts mockinx on a random port.
struct TestServer {
    base_url: String,
    _handle: JoinHandle<()>,
    state: AppState,
}

impl TestServer {
    async fn start() -> Self {
        let state = AppState::new();
        let app = build_router(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        TestServer {
            base_url: format!("http://127.0.0.1:{}", addr.port()),
            _handle: handle,
            state,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Register a rule via POST /_mx (YAML body).
    async fn register(&self, yaml: &str) -> reqwest::Response {
        reqwest::Client::new()
            .post(&self.url("/_mx"))
            .body(yaml.to_string())
            .send()
            .await
            .unwrap()
    }

    /// Register a rule via POST /_mx (JSON body — needed for ! keys).
    async fn register_json(&self, json: &serde_json::Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(&self.url("/_mx"))
            .header("content-type", "application/json")
            .body(json.to_string())
            .send()
            .await
            .unwrap()
    }
}

// =========================================================================
// Basic matching and replies
// =========================================================================

#[tokio::test]
async fn basic_rule_match_and_reply() {
    let srv = TestServer::start().await;
    let resp = srv.register("{match: {g: /hello}, reply: {s: 200, b: world}}").await;
    assert_eq!(resp.status(), 201);

    let resp = reqwest::get(&srv.url("/hello")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "world");
}

#[tokio::test]
async fn header_shortcut_expansion() {
    let srv = TestServer::start().await;
    srv.register("{match: {g: /json}, reply: {s: 200, h: {ct!: j!}, b: {ok: true}}}").await;

    let resp = reqwest::get(&srv.url("/json")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert_eq!(ct, "application/json");
}

#[tokio::test]
async fn no_match_returns_404() {
    let srv = TestServer::start().await;
    let resp = reqwest::get(&srv.url("/nothing")).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn later_rule_has_priority() {
    let srv = TestServer::start().await;
    srv.register("{match: {_: /path}, reply: {s: 200, b: first}}").await;
    srv.register("{match: {_: /path}, reply: {s: 201, b: second}}").await;

    let resp = reqwest::get(&srv.url("/path")).await.unwrap();
    assert_eq!(resp.status(), 201);
    assert_eq!(resp.text().await.unwrap(), "second");
}

#[tokio::test]
async fn catch_all_match() {
    let srv = TestServer::start().await;
    srv.register("{match: _, reply: {s: 200, b: fallback}}").await;

    let resp = reqwest::get(&srv.url("/anything")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "fallback");
}

#[tokio::test]
async fn batch_rule_registration() {
    let srv = TestServer::start().await;
    let resp = srv.register(r#"[
        {match: {_: /a}, reply: {s: 200, b: a}},
        {match: {_: /b}, reply: {s: 201, b: b}}
    ]"#).await;
    assert_eq!(resp.status(), 201);

    let resp_a = reqwest::get(&srv.url("/a")).await.unwrap();
    assert_eq!(resp_a.status(), 200);
    let resp_b = reqwest::get(&srv.url("/b")).await.unwrap();
    assert_eq!(resp_b.status(), 201);
}

// =========================================================================
// Body generators (use JSON for ! keys)
// =========================================================================

#[tokio::test]
async fn rand_body_correct_size_and_deterministic() {
    let srv = TestServer::start().await;
    srv.register_json(&serde_json::json!({
        "match": {"g": "/rand"},
        "reply": {"s": 200, "b": {"rand!": {"size": "1kb", "seed": 42}}}
    })).await;

    let body1 = reqwest::get(&srv.url("/rand")).await.unwrap().bytes().await.unwrap();
    let body2 = reqwest::get(&srv.url("/rand")).await.unwrap().bytes().await.unwrap();
    assert_eq!(body1.len(), 1024);
    assert_eq!(body1, body2, "same seed should produce same bytes");
}

#[tokio::test]
async fn pattern_body() {
    let srv = TestServer::start().await;
    srv.register_json(&serde_json::json!({
        "match": {"g": "/pat"},
        "reply": {"s": 200, "b": {"pattern!": {"repeat": "abc", "size": "7b"}}}
    })).await;

    let body = reqwest::get(&srv.url("/pat")).await.unwrap().text().await.unwrap();
    assert_eq!(body, "abcabca");
}

#[tokio::test]
async fn rand_without_bang_is_literal() {
    let srv = TestServer::start().await;
    srv.register_json(&serde_json::json!({
        "match": {"g": "/literal"},
        "reply": {"s": 200, "b": {"rand": {"size": "1kb", "seed": 42}}}
    })).await;

    let resp = reqwest::get(&srv.url("/literal")).await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    // Should be returned as-is, not generated
    assert_eq!(body["rand"]["size"], "1kb");
}

// =========================================================================
// Serve (delivery shaping) — using serve: key
// =========================================================================

#[tokio::test]
async fn first_byte_delay_via_serve() {
    let srv = TestServer::start().await;
    srv.register_json(&serde_json::json!({
        "match": {"g": "/slow"},
        "reply": {"s": 200, "b": "ok"},
        "serve": {"first_byte": "300ms"}
    })).await;

    let start = std::time::Instant::now();
    let resp = reqwest::get(&srv.url("/slow")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let _body = resp.text().await.unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_millis(250),
        "first byte delay too short: {elapsed:?}"
    );
}

#[tokio::test]
async fn drop_after_bytes_via_serve() {
    let srv = TestServer::start().await;
    srv.register_json(&serde_json::json!({
        "match": {"g": "/drop"},
        "reply": {"s": 200, "b": {"rand!": {"size": "10kb", "seed": 1}}},
        "serve": {"drop": "1kb"}
    })).await;

    let resp = reqwest::get(&srv.url("/drop")).await.unwrap();
    let body = resp.bytes().await.unwrap();
    assert!(body.len() <= 2048, "got too many bytes: {}", body.len());
    assert!(body.len() >= 512, "got too few bytes: {}", body.len());
}

// =========================================================================
// Behavior via serve:
// =========================================================================

#[tokio::test]
async fn concurrency_reject_via_serve() {
    let srv = TestServer::start().await;
    srv.register_json(&serde_json::json!({
        "match": {"_": "/limited"},
        "reply": {"s": 200, "b": {"rand!": {"size": "10kb", "seed": 1}}},
        "serve": {"span": "1s", "conn": {"max": 1, "over": {"s": 429, "b": "too many"}}}
    })).await;

    let client = reqwest::Client::new();
    let url = srv.url("/limited");

    let first = tokio::spawn({
        let client = client.clone();
        let url = url.clone();
        async move {
            let resp = client.get(&url).send().await.unwrap();
            let status = resp.status().as_u16();
            let _ = resp.bytes().await;
            status
        }
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let second = client.get(&url).send().await.unwrap();
    assert_eq!(second.status(), 429);

    let first_status = first.await.unwrap();
    assert_eq!(first_status, 200);
}

#[tokio::test]
async fn fail_injection() {
    let srv = TestServer::start().await;
    // fail still works via legacy behavior: path
    srv.register(r#"{
        match: {_: /flaky},
        reply: {s: 200, b: ok},
        behavior: {fail: {rate: 0.5, reply: {s: 500, b: error}}}
    }"#).await;

    let client = reqwest::Client::new();
    let mut ok_count = 0u32;
    let mut err_count = 0u32;

    for _ in 0..50 {
        let resp = client.get(&srv.url("/flaky")).send().await.unwrap();
        if resp.status() == 200 {
            ok_count += 1;
        } else {
            err_count += 1;
        }
    }

    assert!(ok_count > 10, "too few successes: {ok_count}");
    assert!(err_count > 10, "too few failures: {err_count}");
}

#[tokio::test]
async fn sequence_per_stub() {
    let srv = TestServer::start().await;
    srv.register(r#"{
        match: {_: /seq},
        behavior: {sequence: {per: stub, replies: [
            {s: 401, b: unauthorized},
            {s: 200, b: ok}
        ]}}
    }"#).await;

    let resp1 = reqwest::get(&srv.url("/seq")).await.unwrap();
    assert_eq!(resp1.status(), 401);

    let resp2 = reqwest::get(&srv.url("/seq")).await.unwrap();
    assert_eq!(resp2.status(), 200);

    let resp3 = reqwest::get(&srv.url("/seq")).await.unwrap();
    assert_eq!(resp3.status(), 401);
}

// =========================================================================
// CRUD
// =========================================================================

#[tokio::test]
async fn crud_operations() {
    let srv = TestServer::start().await;
    srv.register(r#"{
        match: {_: /toys},
        reply: {h: {ct!: j!}},
        behavior: {crud: {seed: [
            {id: 1, name: Ball, price: 2.99},
            {id: 3, name: Owl, price: 5.99}
        ]}}
    }"#).await;

    let client = reqwest::Client::new();

    let resp = client.get(&srv.url("/toys")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);

    let resp = client.get(&srv.url("/toys/1")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "Ball");

    let resp = client.get(&srv.url("/toys/99")).send().await.unwrap();
    assert_eq!(resp.status(), 404);

    let resp = client
        .post(&srv.url("/toys"))
        .json(&serde_json::json!({"name": "Car", "price": 1.50}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "Car");
    assert_eq!(body["id"], 4);

    let resp = client
        .patch(&srv.url("/toys/1"))
        .json(&serde_json::json!({"price": 3.99}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "Ball");
    assert_eq!(body["price"], 3.99);

    let resp = client.delete(&srv.url("/toys/1")).send().await.unwrap();
    assert_eq!(resp.status(), 204);

    let resp = client.get(&srv.url("/toys/1")).send().await.unwrap();
    assert_eq!(resp.status(), 404);
}

// =========================================================================
// Config file
// =========================================================================

#[tokio::test]
async fn config_file_loading() {
    let dir = std::env::temp_dir().join("mockinx-test");
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("test-rules.yaml");
    std::fs::write(
        &config_path,
        "[{match: {g: /from-config}, reply: {s: 200, b: loaded}}]",
    )
    .unwrap();

    let state = AppState::new();
    let content = std::fs::read_to_string(&config_path).unwrap();
    let val = yttp::parse(&content).unwrap();
    let stubs = mockinx::stub::parse_stubs(&val).unwrap();
    state.register_stubs(stubs);

    let app = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let resp = reqwest::get(&format!("http://127.0.0.1:{}/from-config", addr.port()))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "loaded");

    let _ = std::fs::remove_file(&config_path);
}

// =========================================================================
// Malformed input
// =========================================================================

#[tokio::test]
async fn malformed_rule_returns_400() {
    let srv = TestServer::start().await;
    let resp = srv.register("not valid yaml {{{").await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn rule_missing_match_returns_400() {
    let srv = TestServer::start().await;
    let resp = srv.register("{reply: {s: 200}}").await;
    assert_eq!(resp.status(), 400);
}
