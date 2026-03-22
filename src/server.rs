use crate::chaos::{ChaosResult, resolve_chaos};
use crate::reply::{BodySpec, ReplySpec};
use crate::reply::body::generate_body;
use crate::reply::crud::CrudStore;
use crate::rule::parse_rules;
use crate::serve::{BehaviorRuntime, DeliveryStream, deliver};
use crate::serve::runtime::BehaviorResult;
use crate::store::{RuleEntry, RuleStore};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use tokio::sync::OwnedSemaphorePermit;
use tokio_stream::Stream;

/// A stream wrapper that holds a semaphore permit for the duration of streaming.
/// The permit is released when this stream is dropped (body fully sent or client disconnects).
struct PermitStream {
    inner: DeliveryStream,
    _permit: OwnedSemaphorePermit,
}

impl Stream for PermitStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Safety: DeliveryStream is Unpin (all fields are Unpin)
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub store: RuleStore,
    /// Per-rule behavior runtimes, keyed by rule index.
    pub runtimes: Arc<RwLock<Vec<Arc<BehaviorRuntime>>>>,
    /// Per-rule CRUD stores.
    pub crud_stores: Arc<RwLock<HashMap<usize, Arc<CrudStore>>>>,
    /// Global rule counter for runtime/crud indexing.
    pub stub_counter: Arc<std::sync::atomic::AtomicUsize>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            store: RuleStore::new(),
            runtimes: Arc::new(RwLock::new(Vec::new())),
            crud_stores: Arc::new(RwLock::new(HashMap::new())),
            stub_counter: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Register rules and create their runtimes.
    pub fn register_rules(&self, rules: Vec<crate::rule::Rule>) {
        let mut runtimes = self.runtimes.write().unwrap();
        let mut crud_stores = self.crud_stores.write().unwrap();
        let start_idx = self.stub_counter.fetch_add(rules.len(), std::sync::atomic::Ordering::Relaxed);

        for (i, rule) in rules.into_iter().enumerate() {
            let idx = start_idx + i;
            let runtime = Arc::new(BehaviorRuntime::new(&rule.behavior));

            // Ensure runtimes vec is large enough
            while runtimes.len() <= idx {
                runtimes.push(Arc::new(BehaviorRuntime::new(&crate::serve::BehaviorSpec::default())));
            }
            runtimes[idx] = runtime;

            // Register CRUD store if reply is CRUD
            if let crate::reply::ReplyStrategy::Crud { ref spec, .. } = rule.reply {
                crud_stores.insert(idx, Arc::new(CrudStore::new(spec)));
            }

            self.store.add(rule, idx);
        }
    }

    /// Clear all rules, runtimes, and CRUD stores.
    pub fn clear_all(&self) {
        self.store.clear();
        self.runtimes.write().unwrap().clear();
        self.crud_stores.write().unwrap().clear();
        self.stub_counter.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the axum router.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/_mx", get(handle_list_rules).post(handle_append_rules).put(handle_replace_rules))
        .fallback(handle_request)
        .with_state(state)
}

/// GET /_mx — list active rules.
async fn handle_list_rules(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let sources = state.store.list_sources();
    axum::Json(sources).into_response()
}

/// POST /_mx — append rules.
async fn handle_append_rules(
    State(state): State<AppState>,
    body: Bytes,
) -> impl IntoResponse {
    match parse_body_as_rules(&body) {
        Ok(rules) => {
            let count = rules.len();
            let warnings = crate::validate::validate_rules(&rules);
            state.register_rules(rules);
            let mut msg = format!("{count} rule(s) added");
            for w in &warnings {
                msg.push_str(&format!("\n{w}"));
            }
            (StatusCode::CREATED, msg).into_response()
        }
        Err(resp) => resp,
    }
}

/// PUT /_mx — replace all rules.
async fn handle_replace_rules(
    State(state): State<AppState>,
    body: Bytes,
) -> impl IntoResponse {
    match parse_body_as_rules(&body) {
        Ok(rules) => {
            state.clear_all();
            let count = rules.len();
            let warnings = crate::validate::validate_rules(&rules);
            state.register_rules(rules);
            let mut msg = format!("{count} rule(s) loaded");
            for w in &warnings {
                msg.push_str(&format!("\n{w}"));
            }
            (StatusCode::OK, msg).into_response()
        }
        Err(resp) => resp,
    }
}

fn plain_error(status: StatusCode, msg: impl Into<String>) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(Body::from(msg.into()))
        .unwrap()
}

fn parse_body_as_rules(body: &Bytes) -> Result<Vec<crate::rule::Rule>, Response> {
    let body_str = std::str::from_utf8(body)
        .map_err(|_| plain_error(StatusCode::BAD_REQUEST, "invalid UTF-8"))?;

    let val = yttp::parse(body_str)
        .map_err(|e| plain_error(StatusCode::BAD_REQUEST, format!("parse error: {e}")))?;

    parse_rules(&val)
        .map_err(|e| plain_error(StatusCode::BAD_REQUEST, format!("{e}")))
}

/// Catch-all handler for matched requests.
async fn handle_request(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(peer_addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    req: Request,
) -> Response {
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let peer_addr = Some(peer_addr);

    // Read request body for CRUD operations
    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_REQUEST, "failed to read body").into_response(),
    };

    // Match against rule store
    let entry = match state.store.match_request(&method, &path) {
        Some(e) => e,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    // Find the rule's runtime index
    let stub_idx = entry.index;

    // Check behavior policies
    let mut rng = StdRng::from_entropy();
    let permit = if let Some(runtime) = get_runtime(&state, stub_idx) {
        match runtime.check(&entry.rule.behavior, &mut rng).await {
            BehaviorResult::Reject(reply) => return build_reply_response(&reply),
            BehaviorResult::Proceed(permit) => permit,
        }
    } else {
        None
    };

    resolve_and_deliver(&state, &entry, &method, &path, &body_bytes, stub_idx, &mut rng, permit, peer_addr).await
}

async fn resolve_and_deliver(
    state: &AppState,
    entry: &Arc<RuleEntry>,
    method: &str,
    path: &str,
    body_bytes: &Bytes,
    stub_idx: usize,
    rng: &mut StdRng,
    permit: Option<OwnedSemaphorePermit>,
    peer_addr: Option<std::net::SocketAddr>,
) -> Response {
    // Check chaos first — may override reply and/or delivery
    let (chaos_reply, chaos_delivery) = if let Some(ref chaos_entries) = entry.rule.chaos {
        match resolve_chaos(chaos_entries, rng) {
            ChaosResult::Normal => (None, None),
            ChaosResult::Override { reply, serve } => (reply, serve),
        }
    } else {
        (None, None)
    };

    // If chaos provided a reply, use it directly
    if let Some(ref chaos_reply) = chaos_reply {
        let delivery = chaos_delivery.as_ref().unwrap_or(&entry.rule.delivery);
        if let Some(ref timeout_range) = entry.rule.behavior.timeout {
            let timeout_dur = timeout_range.sample(rng).as_std();
            return match tokio::time::timeout(
                timeout_dur,
                build_delivery_response(chaos_reply, delivery, rng, permit),
            ).await {
                Ok(response) => response,
                Err(_) => StatusCode::GATEWAY_TIMEOUT.into_response(),
            };
        }
        return build_delivery_response(chaos_reply, delivery, rng, permit).await;
    }

    // Resolve reply from ReplyStrategy
    let reply = match &entry.rule.reply {
        crate::reply::ReplyStrategy::Static(r) => r.clone(),
        crate::reply::ReplyStrategy::Sequence(replies) => {
            let call_idx = if let Some(addr) = peer_addr {
                entry.next_call_for(addr) as usize
            } else {
                entry.next_call() as usize
            };
            replies[call_idx % replies.len()].clone()
        }
        crate::reply::ReplyStrategy::Crud { spec: _, headers } => {
            if let Some(crud_store) = get_crud_store(state, stub_idx) {
                resolve_crud_reply(headers, &crud_store, method, path, body_bytes)
            } else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    };

    // Use chaos delivery override if present, otherwise rule default
    let delivery = chaos_delivery.as_ref().unwrap_or(&entry.rule.delivery);

    // Apply timeout if configured
    if let Some(ref timeout_range) = entry.rule.behavior.timeout {
        let timeout_dur = timeout_range.sample(rng).as_std();
        match tokio::time::timeout(
            timeout_dur,
            build_delivery_response(&reply, delivery, rng, permit),
        )
        .await
        {
            Ok(response) => response,
            Err(_) => StatusCode::GATEWAY_TIMEOUT.into_response(),
        }
    } else {
        build_delivery_response(&reply, delivery, rng, permit).await
    }
}

fn resolve_crud_reply(
    default_headers: &serde_json::Map<String, Value>,
    crud_store: &CrudStore,
    method: &str,
    path: &str,
    body_bytes: &Bytes,
) -> ReplySpec {
    // Extract ID from path — for CRUD, we need the base path from the match rule.
    // The crud_store handles the actual routing, so we extract from the request path.
    // Find the last path segment as the potential ID.
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    let (id, _is_collection) = if segments.len() >= 2 {
        (Some(segments.last().unwrap().to_string()), false)
    } else {
        (None, true)
    };

    let request_body: Value = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(body_bytes).unwrap_or(Value::Null)
    };

    let (status, body) = match (method.to_uppercase().as_str(), id) {
        ("GET", None) => crud_store.list(),
        ("GET", Some(id)) => crud_store.get(&id),
        ("POST", None) => crud_store.create(request_body),
        ("PUT", Some(id)) => crud_store.replace(&id, request_body),
        ("PATCH", Some(id)) => crud_store.patch(&id, request_body),
        ("DELETE", Some(id)) => crud_store.delete(&id),
        _ => (405, serde_json::json!({"error": "method not allowed"})),
    };

    let headers = default_headers.clone();

    ReplySpec {
        status,
        headers,
        body: if body.is_null() && status == 204 {
            BodySpec::None
        } else {
            BodySpec::Literal(body)
        },
    }
}

/// Infer Content-Type from body if not explicitly set.
fn infer_content_type(reply: &ReplySpec) -> Option<&'static str> {
    // If headers already have Content-Type, don't infer
    if reply.headers.keys().any(|k| k.eq_ignore_ascii_case("content-type")) {
        return None;
    }
    match &reply.body {
        BodySpec::Literal(Value::Object(_)) | BodySpec::Literal(Value::Array(_)) => {
            Some("application/json")
        }
        BodySpec::Literal(Value::String(_)) => Some("text/plain"),
        _ => None,
    }
}

/// Build a response with no delivery shaping (full body at once).
fn build_reply_response(reply: &ReplySpec) -> Response {
    let body_bytes = generate_body(&reply.body);
    let mut response = Response::builder()
        .status(reply.status);

    // Infer content-type if not set
    if let Some(ct) = infer_content_type(reply) {
        response = response.header("content-type", ct);
    }

    for (key, val) in &reply.headers {
        if let Some(v) = val.as_str() {
            if let (Ok(name), Ok(value)) = (
                HeaderName::try_from(key.as_str()),
                HeaderValue::try_from(v),
            ) {
                response = response.header(name, value);
            }
        }
    }

    response
        .body(Body::from(body_bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Build a response with delivery shaping, optionally holding a concurrency permit.
async fn build_delivery_response(
    reply: &ReplySpec,
    delivery: &crate::serve::DeliverySpec,
    rng: &mut StdRng,
    permit: Option<OwnedSemaphorePermit>,
) -> Response {
    let body_bytes = generate_body(&reply.body);

    // If default delivery (no shaping), return immediately.
    // Permit is dropped here — fine because the full body is buffered.
    if *delivery == crate::serve::DeliverySpec::default() {
        return build_reply_response(reply);
    }

    // Use delivery engine for shaped streaming
    let stream = deliver(body_bytes, delivery, rng);

    // If we have a concurrency permit, wrap the stream so the permit
    // lives as long as the stream (released when body is fully sent
    // or client disconnects).
    let body = if let Some(permit) = permit {
        Body::from_stream(PermitStream {
            inner: stream,
            _permit: permit,
        })
    } else {
        Body::from_stream(stream)
    };

    let mut response = Response::builder()
        .status(reply.status);

    if let Some(ct) = infer_content_type(reply) {
        response = response.header("content-type", ct);
    }

    for (key, val) in &reply.headers {
        if let Some(v) = val.as_str() {
            if let (Ok(name), Ok(value)) = (
                HeaderName::try_from(key.as_str()),
                HeaderValue::try_from(v),
            ) {
                response = response.header(name, value);
            }
        }
    }

    response
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn get_runtime(state: &AppState, idx: usize) -> Option<Arc<BehaviorRuntime>> {
    state.runtimes.read().unwrap().get(idx).cloned()
}

fn get_crud_store(state: &AppState, idx: usize) -> Option<Arc<CrudStore>> {
    state.crud_stores.read().unwrap().get(&idx).cloned()
}
