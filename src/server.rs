use crate::behavior_engine::{BehaviorResult, BehaviorRuntime};
use crate::body::generate_body;
use crate::chaos::{ChaosResult, resolve_chaos};
use crate::crud::{CrudStore, extract_id};
use crate::delivery_engine::{DeliveryStream, deliver};
use crate::reply::{BodySpec, ReplySpec};
use crate::store::{StubEntry, StubStore};
use crate::stub::parse_stubs;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
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
    pub store: StubStore,
    /// Per-stub behavior runtimes, keyed by stub index.
    pub runtimes: Arc<RwLock<Vec<Arc<BehaviorRuntime>>>>,
    /// Per-stub CRUD stores.
    pub crud_stores: Arc<RwLock<HashMap<usize, Arc<CrudStore>>>>,
    /// Global stub counter for runtime/crud indexing.
    pub stub_counter: Arc<std::sync::atomic::AtomicUsize>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            store: StubStore::new(),
            runtimes: Arc::new(RwLock::new(Vec::new())),
            crud_stores: Arc::new(RwLock::new(HashMap::new())),
            stub_counter: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Register stubs and create their runtimes.
    pub fn register_stubs(&self, stubs: Vec<crate::stub::Stub>) {
        let mut runtimes = self.runtimes.write().unwrap();
        let mut crud_stores = self.crud_stores.write().unwrap();
        let start_idx = self.stub_counter.fetch_add(stubs.len(), std::sync::atomic::Ordering::Relaxed);

        for (i, stub) in stubs.into_iter().enumerate() {
            let idx = start_idx + i;
            let runtime = Arc::new(BehaviorRuntime::new(&stub.behavior));

            // Ensure runtimes vec is large enough
            while runtimes.len() <= idx {
                runtimes.push(Arc::new(BehaviorRuntime::new(&crate::behavior::BehaviorSpec::default())));
            }
            runtimes[idx] = runtime;

            // Check for CRUD in ReplyStrategy or legacy behavior.crud
            if let Some(crate::reply::ReplyStrategy::Crud { ref spec, .. }) = stub.reply {
                crud_stores.insert(idx, Arc::new(CrudStore::new(spec)));
            } else if let Some(ref crud_spec) = stub.behavior.crud {
                crud_stores.insert(idx, Arc::new(CrudStore::new(crud_spec)));
            }

            self.store.add(stub, idx);
        }
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
        .route("/_mx", post(handle_stub_registration))
        .fallback(handle_request)
        .with_state(state)
}

/// POST /_mx — register stubs.
async fn handle_stub_registration(
    State(state): State<AppState>,
    body: Bytes,
) -> impl IntoResponse {
    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid UTF-8").into_response(),
    };

    let val = match yttp::parse(body_str) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("parse error: {e}")).into_response(),
    };

    let stubs = match parse_stubs(&val) {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("stub error: {e}")).into_response(),
    };

    let count = stubs.len();
    state.register_stubs(stubs);

    (StatusCode::CREATED, format!("{count} stub(s) registered")).into_response()
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

    // Match against stub store
    let entry = match state.store.match_request(&method, &path) {
        Some(e) => e,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    // Find the stub's runtime index
    let stub_idx = entry.index;

    // Check behavior policies
    let mut rng = StdRng::from_entropy();
    let permit = if let Some(runtime) = get_runtime(&state, stub_idx) {
        match runtime.check(&entry.stub.behavior, &mut rng).await {
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
    entry: &Arc<StubEntry>,
    method: &str,
    path: &str,
    body_bytes: &Bytes,
    stub_idx: usize,
    rng: &mut StdRng,
    permit: Option<OwnedSemaphorePermit>,
    peer_addr: Option<std::net::SocketAddr>,
) -> Response {
    // Check chaos first — may override reply and/or delivery
    let (chaos_reply, chaos_delivery) = if let Some(ref chaos_entries) = entry.stub.chaos {
        match resolve_chaos(chaos_entries, rng) {
            ChaosResult::Normal => (None, None),
            ChaosResult::Override { reply, serve } => (reply, serve),
        }
    } else {
        (None, None)
    };

    // If chaos provided a reply, use it directly
    if let Some(ref chaos_reply) = chaos_reply {
        let delivery = chaos_delivery.as_ref().unwrap_or(&entry.stub.delivery);
        if let Some(ref timeout_range) = entry.stub.behavior.timeout {
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
    let reply = match &entry.stub.reply {
        Some(crate::reply::ReplyStrategy::Static(r)) => r.clone(),
        Some(crate::reply::ReplyStrategy::Sequence(replies)) => {
            // Per-connection cycling (falls back to global if no peer addr)
            let call_idx = if let Some(addr) = peer_addr {
                entry.next_call_for(addr) as usize
            } else {
                entry.next_call() as usize
            };
            replies[call_idx % replies.len()].clone()
        }
        Some(crate::reply::ReplyStrategy::Crud { spec: _, headers }) => {
            if let Some(crud_store) = get_crud_store(state, stub_idx) {
                resolve_crud_reply(headers, &crud_store, method, path, body_bytes)
            } else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
        None => {
            // Legacy: check behavior.sequence and behavior.crud
            if let Some(ref seq) = entry.stub.behavior.sequence {
                let call_idx = entry.next_call() as usize;
                seq.replies[call_idx % seq.replies.len()].clone()
            } else if let Some(crud_store) = get_crud_store(state, stub_idx) {
                let headers = serde_json::Map::new();
                resolve_crud_reply(&headers, &crud_store, method, path, body_bytes)
            } else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    };

    // Use chaos delivery override if present, otherwise rule default
    let delivery = chaos_delivery.as_ref().unwrap_or(&entry.stub.delivery);

    // Apply timeout if configured
    if let Some(ref timeout_range) = entry.stub.behavior.timeout {
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
    delivery: &crate::delivery::DeliverySpec,
    rng: &mut StdRng,
    permit: Option<OwnedSemaphorePermit>,
) -> Response {
    let body_bytes = generate_body(&reply.body);

    // If default delivery (no shaping), return immediately.
    // Permit is dropped here — fine because the full body is buffered.
    if *delivery == crate::delivery::DeliverySpec::default() {
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
