use crate::stub::Stub;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// A stub with runtime state.
pub struct StubEntry {
    pub stub: Stub,
    /// Global index for runtime/CRUD store lookup.
    pub index: usize,
    /// Global call counter (used for per-rule sequences, legacy).
    pub call_count: AtomicU64,
    /// Per-connection call counters (keyed by peer address).
    conn_counters: Mutex<HashMap<SocketAddr, u64>>,
}

impl StubEntry {
    pub fn new(stub: Stub, index: usize) -> Self {
        StubEntry {
            stub,
            index,
            call_count: AtomicU64::new(0),
            conn_counters: Mutex::new(HashMap::new()),
        }
    }

    /// Increment and return the global call count (0-indexed).
    pub fn next_call(&self) -> u64 {
        self.call_count.fetch_add(1, Ordering::Relaxed)
    }

    /// Increment and return the per-connection call count (0-indexed).
    pub fn next_call_for(&self, addr: SocketAddr) -> u64 {
        let mut counters = self.conn_counters.lock().unwrap();
        let count = counters.entry(addr).or_insert(0);
        let current = *count;
        *count += 1;
        current
    }
}

/// Thread-safe, priority-ordered store for registered stubs.
///
/// Later stubs take precedence over earlier ones (matched in reverse order).
#[derive(Clone)]
pub struct StubStore {
    entries: Arc<RwLock<Vec<Arc<StubEntry>>>>,
}

impl StubStore {
    pub fn new() -> Self {
        StubStore {
            entries: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Add a single stub with the given index. It takes highest priority.
    pub fn add(&self, stub: Stub, index: usize) {
        let entry = Arc::new(StubEntry::new(stub, index));
        self.entries.write().unwrap().push(entry);
    }

    /// Add multiple stubs, maintaining their relative order.
    /// The last stub in the batch gets highest priority.
    /// Indices are assigned starting from `start_index`.
    pub fn add_batch(&self, stubs: Vec<Stub>, start_index: usize) {
        let mut entries = self.entries.write().unwrap();
        for (i, stub) in stubs.into_iter().enumerate() {
            entries.push(Arc::new(StubEntry::new(stub, start_index + i)));
        }
    }

    /// Find the best matching stub for the given method and path.
    ///
    /// Iterates in reverse order (last added = highest priority).
    pub fn match_request(&self, method: &str, path: &str) -> Option<Arc<StubEntry>> {
        let entries = self.entries.read().unwrap();
        for entry in entries.iter().rev() {
            if entry.stub.match_rule.matches(method, path) {
                return Some(Arc::clone(entry));
            }
        }
        None
    }

    /// Remove all stubs.
    pub fn clear(&self) {
        self.entries.write().unwrap().clear();
    }

    /// Number of registered stubs.
    pub fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.read().unwrap().is_empty()
    }
}

impl Default for StubStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::BehaviorSpec;
    use crate::delivery::DeliverySpec;
    use crate::match_rule::MatchRule;
    use crate::reply::{ReplySpec, ReplyStrategy};
    use serde_json::json;

    fn stub_with_path(method: Option<&str>, path: &str, status: u16) -> Stub {
        Stub {
            match_rule: MatchRule::MethodPath {
                method: method.map(|m| m.to_string()),
                path: path.to_string(),
            },
            reply: Some(ReplyStrategy::Static(ReplySpec {
                status,
                ..Default::default()
            })),
            delivery: DeliverySpec::default(),
            behavior: BehaviorSpec::default(),
            chaos: None,
        }
    }

    fn catch_all_stub(status: u16) -> Stub {
        Stub {
            match_rule: MatchRule::CatchAll,
            reply: Some(ReplyStrategy::Static(ReplySpec {
                status,
                ..Default::default()
            })),
            delivery: DeliverySpec::default(),
            behavior: BehaviorSpec::default(),
            chaos: None,
        }
    }

    fn get_status(entry: &Arc<StubEntry>) -> u16 {
        match entry.stub.reply.as_ref().unwrap() {
            ReplyStrategy::Static(r) => r.status,
            _ => panic!("expected Static"),
        }
    }

    #[test]
    fn add_and_match() {
        let store = StubStore::new();
        store.add(stub_with_path(Some("GET"), "/api/data", 200), 0);
        let entry = store.match_request("GET", "/api/data").unwrap();
        assert_eq!(get_status(&entry), 200);
    }

    #[test]
    fn no_match_returns_none() {
        let store = StubStore::new();
        store.add(stub_with_path(Some("GET"), "/api/data", 200), 0);
        assert!(store.match_request("GET", "/other").is_none());
    }

    #[test]
    fn later_stubs_have_priority() {
        let store = StubStore::new();
        store.add(stub_with_path(None, "/path", 200), 0);
        store.add(stub_with_path(None, "/path", 201), 1);
        let entry = store.match_request("GET", "/path").unwrap();
        assert_eq!(get_status(&entry), 201);
    }

    #[test]
    fn batch_preserves_order() {
        let store = StubStore::new();
        store.add_batch(vec![
            stub_with_path(None, "/a", 200),
            stub_with_path(None, "/b", 201),
            stub_with_path(None, "/a", 202), // should win for /a
        ], 0);
        let entry = store.match_request("GET", "/a").unwrap();
        assert_eq!(get_status(&entry), 202);
        let entry = store.match_request("GET", "/b").unwrap();
        assert_eq!(get_status(&entry), 201);
    }

    #[test]
    fn clear_removes_all() {
        let store = StubStore::new();
        store.add(stub_with_path(Some("GET"), "/path", 200), 0);
        assert_eq!(store.len(), 1);
        store.clear();
        assert!(store.is_empty());
        assert!(store.match_request("GET", "/path").is_none());
    }

    #[test]
    fn catch_all_lower_priority_than_specific() {
        let store = StubStore::new();
        store.add(catch_all_stub(404), 0);
        store.add(stub_with_path(Some("GET"), "/specific", 200), 1);
        // Specific wins for /specific
        let entry = store.match_request("GET", "/specific").unwrap();
        assert_eq!(get_status(&entry), 200);
        // Catch-all wins for anything else
        let entry = store.match_request("GET", "/other").unwrap();
        assert_eq!(get_status(&entry), 404);
    }

    #[test]
    fn sequence_counter_increments() {
        let store = StubStore::new();
        store.add(stub_with_path(Some("GET"), "/path", 200), 0);
        let entry = store.match_request("GET", "/path").unwrap();
        assert_eq!(entry.next_call(), 0);
        assert_eq!(entry.next_call(), 1);
        assert_eq!(entry.next_call(), 2);
    }

    #[tokio::test]
    async fn thread_safety_concurrent_access() {
        let store = StubStore::new();
        store.add(stub_with_path(Some("GET"), "/path", 200), 0);

        let mut handles = Vec::new();
        for _ in 0..10 {
            let store = store.clone();
            handles.push(tokio::spawn(async move {
                for _ in 0..100 {
                    let _ = store.match_request("GET", "/path");
                }
            }));
        }

        // Concurrent writes
        let store_w = store.clone();
        handles.push(tokio::spawn(async move {
            for i in 0..10 {
                store_w.add(stub_with_path(
                    Some("GET"),
                    &format!("/path{i}"),
                    200,
                ), 10 + i);
            }
        }));

        for h in handles {
            h.await.unwrap();
        }
        // Should not panic
        assert!(store.len() > 0);
    }
}
