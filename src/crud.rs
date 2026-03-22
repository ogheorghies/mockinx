use crate::behavior::CrudSpec;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use std::sync::RwLock;

/// In-memory CRUD store for a single resource.
pub struct CrudStore {
    state: RwLock<CrudState>,
    id_field: String,
    id_strategy: String,
}

struct CrudState {
    items: IndexMap<String, Value>,
    next_id: u64,
}

/// Response from a CRUD operation: (status_code, body).
pub type CrudResponse = (u16, Value);

impl CrudStore {
    /// Create a new CRUD store from a spec, loading seed data.
    pub fn new(spec: &CrudSpec) -> Self {
        let id_field = spec.id.name.clone();
        let mut items = IndexMap::new();
        let mut max_id: u64 = 0;

        for item in &spec.seed {
            if let Some(id_val) = item.get(&id_field) {
                let id_key = value_to_id_key(id_val);
                if let Some(n) = id_val.as_u64() {
                    max_id = max_id.max(n);
                }
                items.insert(id_key, item.clone());
            }
        }

        CrudStore {
            state: RwLock::new(CrudState {
                items,
                next_id: max_id + 1,
            }),
            id_field,
            id_strategy: spec.id.new.clone(),
        }
    }

    /// List all items.
    pub fn list(&self) -> CrudResponse {
        let state = self.state.read().unwrap();
        let items: Vec<&Value> = state.items.values().collect();
        (200, json!(items))
    }

    /// Get a single item by ID.
    pub fn get(&self, id: &str) -> CrudResponse {
        let state = self.state.read().unwrap();
        match state.items.get(id) {
            Some(item) => (200, item.clone()),
            None => (404, json!({"error": "not found"})),
        }
    }

    /// Create a new item, assigning an ID based on the configured strategy.
    pub fn create(&self, body: Value) -> CrudResponse {
        let mut state = self.state.write().unwrap();

        // Generate ID based on strategy
        let (id_value, id_key) = match self.id_strategy.as_str() {
            "uuid" => {
                let u = uuid::Uuid::new_v4().to_string();
                (json!(u), u)
            }
            _ => {
                // "inc" (default)
                let id = state.next_id;
                state.next_id += 1;
                (json!(id), id.to_string())
            }
        };

        let mut item = match body {
            Value::Object(m) => Value::Object(m),
            other => {
                let mut m = Map::new();
                m.insert("value".to_string(), other);
                Value::Object(m)
            }
        };

        if let Value::Object(ref mut m) = item {
            m.insert(self.id_field.clone(), id_value);
        }

        state.items.insert(id_key, item.clone());
        (201, item)
    }

    /// Replace an item by ID (full update).
    pub fn replace(&self, id: &str, body: Value) -> CrudResponse {
        let mut state = self.state.write().unwrap();
        if !state.items.contains_key(id) {
            return (404, json!({"error": "not found"}));
        }

        let mut item = match body {
            Value::Object(m) => Value::Object(m),
            other => {
                let mut m = Map::new();
                m.insert("value".to_string(), other);
                Value::Object(m)
            }
        };

        // Ensure ID field is set
        if let Value::Object(ref mut m) = item {
            m.insert(self.id_field.clone(), json!(id_key_to_value(id)));
        }

        state.items.insert(id.to_string(), item.clone());
        (200, item)
    }

    /// Patch an item by ID (partial update, merge fields).
    pub fn patch(&self, id: &str, body: Value) -> CrudResponse {
        let mut state = self.state.write().unwrap();
        let existing = match state.items.get(id) {
            Some(item) => item.clone(),
            None => return (404, json!({"error": "not found"})),
        };

        let merged = merge_values(existing, body);
        state.items.insert(id.to_string(), merged.clone());
        (200, merged)
    }

    /// Delete an item by ID.
    pub fn delete(&self, id: &str) -> CrudResponse {
        let mut state = self.state.write().unwrap();
        match state.items.shift_remove(id) {
            Some(_) => (204, Value::Null),
            None => (404, json!({"error": "not found"})),
        }
    }
}

/// Convert a serde_json::Value to a string key for the map.
fn value_to_id_key(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Convert an ID key string back to a Value (try number first).
fn id_key_to_value(key: &str) -> Value {
    if let Ok(n) = key.parse::<u64>() {
        json!(n)
    } else if let Ok(n) = key.parse::<i64>() {
        json!(n)
    } else {
        json!(key)
    }
}

/// Merge two JSON values. If both are objects, merge fields (patch overwrites existing).
fn merge_values(base: Value, patch: Value) -> Value {
    match (base, patch) {
        (Value::Object(mut base_map), Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                base_map.insert(k, v);
            }
            Value::Object(base_map)
        }
        (_, patch) => patch,
    }
}

/// Extract the ID segment from a path relative to the resource base.
/// e.g., if base is "/toys" and path is "/toys/3", returns Some("3").
/// Returns None if path matches the base exactly (list operation).
pub fn extract_id(base_path: &str, request_path: &str) -> Option<String> {
    let base = base_path.trim_end_matches('/');
    let path = request_path.trim_end_matches('/');

    if path == base {
        return None;
    }

    let rest = path.strip_prefix(base)?;
    let rest = rest.strip_prefix('/')?;

    if rest.is_empty() || rest.contains('/') {
        return None; // No nested paths supported
    }

    Some(rest.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::{CrudIdSpec, CrudSpec};
    use std::sync::Arc;

    fn seeded_store() -> CrudStore {
        CrudStore::new(&CrudSpec {
            id: CrudIdSpec::default(),
            seed: vec![
                json!({"id": 1, "name": "Ball", "price": 2.99}),
                json!({"id": 3, "name": "Owl", "price": 5.99}),
            ],
        })
    }

    fn empty_store() -> CrudStore {
        CrudStore::new(&CrudSpec {
            id: CrudIdSpec::default(),
            seed: vec![],
        })
    }

    // --- Seed data ---

    #[test]
    fn seed_data_loaded() {
        let store = seeded_store();
        let (status, body) = store.list();
        assert_eq!(status, 200);
        let items = body.as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn list_returns_all() {
        let store = seeded_store();
        let (_, body) = store.list();
        let items = body.as_array().unwrap();
        assert_eq!(items[0]["name"], "Ball");
        assert_eq!(items[1]["name"], "Owl");
    }

    // --- Get ---

    #[test]
    fn get_by_id() {
        let store = seeded_store();
        let (status, body) = store.get("1");
        assert_eq!(status, 200);
        assert_eq!(body["name"], "Ball");
    }

    #[test]
    fn get_nonexistent() {
        let store = seeded_store();
        let (status, _) = store.get("99");
        assert_eq!(status, 404);
    }

    // --- Create ---

    #[test]
    fn create_assigns_inc_id() {
        let store = seeded_store();
        let (status, body) = store.create(json!({"name": "Car", "price": 1.50}));
        assert_eq!(status, 201);
        assert_eq!(body["id"], 4); // max seed id is 3, so next is 4
        assert_eq!(body["name"], "Car");
    }

    #[test]
    fn create_increments_id() {
        let store = seeded_store();
        let (_, first) = store.create(json!({"name": "A"}));
        let (_, second) = store.create(json!({"name": "B"}));
        assert_eq!(first["id"], 4);
        assert_eq!(second["id"], 5);
    }

    #[test]
    fn create_on_empty_store() {
        let store = empty_store();
        let (status, body) = store.create(json!({"name": "First"}));
        assert_eq!(status, 201);
        assert_eq!(body["id"], 1);
    }

    // --- Replace ---

    #[test]
    fn replace_existing() {
        let store = seeded_store();
        let (status, body) = store.replace("1", json!({"name": "Basketball", "price": 9.99}));
        assert_eq!(status, 200);
        assert_eq!(body["name"], "Basketball");
        // Verify persisted
        let (_, fetched) = store.get("1");
        assert_eq!(fetched["name"], "Basketball");
    }

    #[test]
    fn replace_nonexistent() {
        let store = seeded_store();
        let (status, _) = store.replace("99", json!({"name": "X"}));
        assert_eq!(status, 404);
    }

    // --- Patch ---

    #[test]
    fn patch_merges_fields() {
        let store = seeded_store();
        let (status, body) = store.patch("1", json!({"price": 3.99}));
        assert_eq!(status, 200);
        assert_eq!(body["name"], "Ball"); // unchanged
        assert_eq!(body["price"], 3.99); // updated
    }

    #[test]
    fn patch_nonexistent() {
        let store = seeded_store();
        let (status, _) = store.patch("99", json!({"price": 1.0}));
        assert_eq!(status, 404);
    }

    // --- Delete ---

    #[test]
    fn delete_existing() {
        let store = seeded_store();
        let (status, _) = store.delete("1");
        assert_eq!(status, 204);
        // Verify removed
        let (status, _) = store.get("1");
        assert_eq!(status, 404);
    }

    #[test]
    fn delete_nonexistent() {
        let store = seeded_store();
        let (status, _) = store.delete("99");
        assert_eq!(status, 404);
    }

    // --- Custom ID field ---

    #[test]
    fn custom_id_field() {
        let store = CrudStore::new(&CrudSpec {
            id: CrudIdSpec {
                name: "sku".into(),
                new: "inc".into(),
            },
            seed: vec![json!({"sku": 100, "name": "Widget"})],
        });
        let (status, body) = store.get("100");
        assert_eq!(status, 200);
        assert_eq!(body["name"], "Widget");

        let (_, created) = store.create(json!({"name": "Gadget"}));
        assert_eq!(created["sku"], 101);
    }

    // --- UUID strategy ---

    #[test]
    fn uuid_id_strategy() {
        let store = CrudStore::new(&CrudSpec {
            id: CrudIdSpec {
                name: "uid".into(),
                new: "uuid".into(),
            },
            seed: vec![],
        });
        let (status, body) = store.create(json!({"name": "Widget"}));
        assert_eq!(status, 201);
        let uid = body["uid"].as_str().unwrap();
        assert_eq!(uid.len(), 36, "UUID should be 36 chars: {uid}");
        assert!(uid.contains('-'), "UUID should contain dashes: {uid}");

        // Second create gets a different UUID
        let (_, body2) = store.create(json!({"name": "Gadget"}));
        let uid2 = body2["uid"].as_str().unwrap();
        assert_ne!(uid, uid2);

        // Can retrieve by UUID
        let (status, fetched) = store.get(uid);
        assert_eq!(status, 200);
        assert_eq!(fetched["name"], "Widget");
    }

    // --- Extract ID ---

    #[test]
    fn extract_id_from_path() {
        assert_eq!(extract_id("/toys", "/toys/3"), Some("3".into()));
        assert_eq!(extract_id("/toys", "/toys"), None);
        assert_eq!(extract_id("/toys", "/toys/"), None);
        assert_eq!(extract_id("/toys", "/other/3"), None);
    }

    // --- Thread safety ---

    #[tokio::test]
    async fn concurrent_operations() {
        let store = Arc::new(seeded_store());
        let mut handles = Vec::new();

        for i in 0..10 {
            let s = Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                s.create(json!({"name": format!("item-{i}")}));
                s.list();
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let (_, body) = store.list();
        let items = body.as_array().unwrap();
        assert_eq!(items.len(), 12); // 2 seed + 10 created
    }
}
