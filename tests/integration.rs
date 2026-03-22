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
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
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
async fn span_delivers_progressively() {
    let srv = TestServer::start().await;
    srv.register_json(&serde_json::json!({
        "match": {"g": "/progressive"},
        "reply": {"s": 200, "b": {"rand!": {"size": "10kb", "seed": 99}}},
        "serve": {"pace": "1s"}
    })).await;

    let resp = reqwest::get(&srv.url("/progressive")).await.unwrap();
    assert_eq!(resp.status(), 200);

    // Stream the body, recording how much data arrives by each checkpoint
    let start = std::time::Instant::now();
    let mut total_bytes = 0usize;
    let mut bytes_at_300ms = None;
    let mut bytes_at_600ms = None;
    let mut stream = resp.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.unwrap();
        total_bytes += chunk.len();
        let elapsed = start.elapsed();
        if bytes_at_300ms.is_none() && elapsed >= std::time::Duration::from_millis(300) {
            bytes_at_300ms = Some(total_bytes);
        }
        if bytes_at_600ms.is_none() && elapsed >= std::time::Duration::from_millis(600) {
            bytes_at_600ms = Some(total_bytes);
        }
    }

    assert_eq!(total_bytes, 10240, "should receive full 10kb");

    // At 300ms (~30% of 1s span), should have received some but not all data
    let at_300 = bytes_at_300ms.unwrap_or(total_bytes);
    assert!(at_300 > 0, "should have received some data by 300ms");
    assert!(at_300 < 8000, "should NOT have received most data by 300ms, got {at_300}");

    // At 600ms (~60% of 1s span), should have more than at 300ms
    let at_600 = bytes_at_600ms.unwrap_or(total_bytes);
    assert!(at_600 > at_300, "should have more data at 600ms ({at_600}) than 300ms ({at_300})");
    assert!(at_600 < total_bytes, "should NOT have all data by 600ms, got {at_600}");
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
        "serve": {"pace": "1s", "conn": {"max": 1, "over": {"s": 429, "b": "too many"}}}
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
async fn fail_injection_via_chaos() {
    let srv = TestServer::start().await;
    srv.register_json(&serde_json::json!({
        "match": {"_": "/flaky"},
        "reply": {"s": 200, "b": "ok"},
        "chaos": [{"p": "50%", "reply": {"s": 500, "b": "error"}}]
    })).await;

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
async fn sequence_via_reply_array() {
    let srv = TestServer::start().await;
    srv.register(r#"{
        match: {_: /seq},
        reply: [
            {s: 401, b: unauthorized},
            {s: 200, b: ok}
        ]
    }"#).await;

    // Use a single client to ensure same connection (per-connection counter)
    let client = reqwest::Client::new();

    let resp1 = client.get(&srv.url("/seq")).send().await.unwrap();
    assert_eq!(resp1.status(), 401);

    let resp2 = client.get(&srv.url("/seq")).send().await.unwrap();
    assert_eq!(resp2.status(), 200);

    let resp3 = client.get(&srv.url("/seq")).send().await.unwrap();
    assert_eq!(resp3.status(), 401);
}

// =========================================================================
// CRUD
// =========================================================================

#[tokio::test]
async fn crud_operations() {
    let srv = TestServer::start().await;
    srv.register_json(&serde_json::json!({
        "match": {"_": "/toys"},
        "reply": {"crud!": {"data": [
            {"id": 1, "name": "Ball", "price": 2.99},
            {"id": 3, "name": "Owl", "price": 5.99}
        ]}, "h": {"ct!": "j!"}}
    })).await;

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
    let rules = mockinx::rule::parse_rules(&val).unwrap();
    state.register_rules(rules);

    let app = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let resp = reqwest::get(&format!("http://127.0.0.1:{}/from-config", addr.port()))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "loaded");

    let _ = std::fs::remove_file(&config_path);
}

// =========================================================================
// Chaos
// =========================================================================

#[tokio::test]
async fn chaos_reply_override() {
    let srv = TestServer::start().await;
    srv.register_json(&serde_json::json!({
        "match": {"_": "/chaos"},
        "reply": {"s": 200, "b": "ok"},
        "chaos": [
            {"p": "50%", "reply": {"s": 500, "b": "error"}}
        ]
    })).await;

    let client = reqwest::Client::new();
    let mut ok_count = 0u32;
    let mut err_count = 0u32;

    for _ in 0..100 {
        let resp = client.get(&srv.url("/chaos")).send().await.unwrap();
        if resp.status() == 200 {
            ok_count += 1;
        } else {
            err_count += 1;
        }
    }

    assert!(ok_count > 30, "too few successes: {ok_count}");
    assert!(err_count > 30, "too few failures: {err_count}");
}

// =========================================================================
// Content-type inference
// =========================================================================

#[tokio::test]
async fn json_body_infers_content_type() {
    let srv = TestServer::start().await;
    srv.register("{match: {g: /infer}, reply: {s: 200, b: {items: [1, 2]}}}").await;

    let resp = reqwest::get(&srv.url("/infer")).await.unwrap();
    let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert_eq!(ct, "application/json");
}

#[tokio::test]
async fn string_body_infers_text_plain() {
    let srv = TestServer::start().await;
    srv.register("{match: {g: /text}, reply: {s: 200, b: hello}}").await;

    let resp = reqwest::get(&srv.url("/text")).await.unwrap();
    let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert_eq!(ct, "text/plain");
}

#[tokio::test]
async fn explicit_content_type_overrides_inference() {
    let srv = TestServer::start().await;
    srv.register("{match: {g: /override}, reply: {s: 200, h: {ct!: h!}, b: {items: []}}}").await;

    let resp = reqwest::get(&srv.url("/override")).await.unwrap();
    let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert_eq!(ct, "text/html");
}

// =========================================================================
// Config file (README example): tests/fixtures/rules.yaml
// =========================================================================

/// Start a server with the README example config file loaded.
async fn start_with_fixture() -> TestServer {
    let state = AppState::new();

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rules.yaml");
    let content = std::fs::read_to_string(&fixture).unwrap();
    let val = yttp::parse(&content).unwrap();
    let rules = mockinx::rule::parse_rules(&val).unwrap();
    state.register_rules(rules);

    let app = build_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    TestServer {
        base_url: format!("http://127.0.0.1:{}", addr.port()),
        _handle: handle,
        state,
    }
}

#[tokio::test]
async fn fixture_health_check() {
    let srv = start_with_fixture().await;
    let resp = reqwest::get(&srv.url("/health")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn fixture_crud_list() {
    let srv = start_with_fixture().await;
    let resp = reqwest::get(&srv.url("/toys")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let items = body.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "Ball");
    assert_eq!(items[1]["name"], "Owl");
}

#[tokio::test]
async fn fixture_crud_get_by_id() {
    let srv = start_with_fixture().await;
    let resp = reqwest::get(&srv.url("/toys/1")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "Ball");
    assert_eq!(body["price"], 2.99);
}

#[tokio::test]
async fn fixture_crud_create() {
    let srv = start_with_fixture().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(&srv.url("/toys"))
        .json(&serde_json::json!({"name": "Car", "price": 1.50}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "Car");
    assert_eq!(body["id"], 4); // next after max id 3
}

#[tokio::test]
async fn fixture_slow_endpoint_paced() {
    let srv = start_with_fixture().await;
    let start = std::time::Instant::now();
    let resp = reqwest::get(&srv.url("/api/data")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let _body = resp.text().await.unwrap();
    let elapsed = start.elapsed();
    // pace: 500ms — body delivery should take at least ~400ms
    assert!(
        elapsed >= std::time::Duration::from_millis(350),
        "pace too fast: {elapsed:?}"
    );
}

#[tokio::test]
async fn fixture_concurrency_limit() {
    let srv = start_with_fixture().await;
    let client = reqwest::Client::new();
    let url = srv.url("/api/data");

    // Start 2 requests (at the conn limit of 2)
    let h1 = tokio::spawn({
        let c = client.clone();
        let u = url.clone();
        async move {
            let r = c.get(&u).send().await.unwrap();
            let s = r.status().as_u16();
            let _ = r.bytes().await;
            s
        }
    });
    let h2 = tokio::spawn({
        let c = client.clone();
        let u = url.clone();
        async move {
            let r = c.get(&u).send().await.unwrap();
            let s = r.status().as_u16();
            let _ = r.bytes().await;
            s
        }
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // 3rd should be rejected (max: 2)
    let resp3 = client.get(&url).send().await.unwrap();
    assert_eq!(resp3.status(), 429);

    assert_eq!(h1.await.unwrap(), 200);
    assert_eq!(h2.await.unwrap(), 200);
}

#[tokio::test]
async fn fixture_toys_6_overrides_crud() {
    let srv = start_with_fixture().await;

    // /toys/6 is served by the specific flaky rule, not CRUD
    let resp = reqwest::get(&srv.url("/toys/6")).await.unwrap();
    // Could be 200 or 500 (30% chaos), but the body on success is Dice
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap();
    if status == 200 {
        let val: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(val["name"], "Dice");
    }
    // Other toy IDs still served by CRUD
    let resp = reqwest::get(&srv.url("/toys/1")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "Ball");
}

#[tokio::test]
async fn fixture_toys_6_chaos_distribution() {
    let srv = start_with_fixture().await;
    let client = reqwest::Client::new();

    let mut ok_count = 0u32;
    let mut err_count = 0u32;

    for _ in 0..100 {
        let resp = client.get(&srv.url("/toys/6")).send().await.unwrap();
        if resp.status() == 200 {
            ok_count += 1;
        } else {
            err_count += 1;
        }
    }

    // 30% error + 10% drop ≈ ~40% failures, ~60% success
    assert!(ok_count > 30, "too few successes: {ok_count}");
    assert!(err_count > 15, "too few failures: {err_count}");
}

#[tokio::test]
async fn fixture_unmatched_returns_404() {
    let srv = start_with_fixture().await;
    let resp = reqwest::get(&srv.url("/nonexistent")).await.unwrap();
    assert_eq!(resp.status(), 404);
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
