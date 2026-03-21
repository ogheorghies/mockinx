# 0120 · [feature] CRUD store — in-memory REST resource

## Goal
In-memory key-value store per CRUD-enabled stub, supporting standard REST operations with auto-ID generation and seed data.

## Approach
Create `src/crud.rs` with:
- `CrudStore` — wraps `RwLock<CrudState>`.
- `CrudState`:
  - `items: IndexMap<Value, Value>` — ordered map from ID to item.
  - `next_id: u64` — auto-increment counter.
  - `id_field: String` — name of the ID field (default "id").
- `CrudStore::new(spec: &CrudSpec) -> Self`:
  - Initialize with seed data.
  - Set `next_id` to max existing ID + 1.
- Operations return `(u16, Value)` — (status code, response body):
  - `list() -> (200, array of all items)`.
  - `get(id) -> (200, item)` or `(404, not found message)`.
  - `create(body) -> (201, item with assigned ID)`.
  - `replace(id, body) -> (200, updated item)` or `(404, ...)`.
  - `patch(id, body) -> (200, merged item)` or `(404, ...)`.
  - `delete(id) -> (204, empty)` or `(404, ...)`.
- ID extraction: parse ID from URL path suffix (e.g., `/toys/3` → id=3).
- Auto-ID: for `new: auto`, assign `next_id` and increment.

## Deliverables
- `src/crud.rs` with `CrudStore` and all CRUD operations.

## Acceptance criteria
Unit tests covering:
- Seed data loaded correctly.
- List returns all items.
- Get by ID returns correct item.
- Get non-existent ID returns 404.
- Create assigns auto-ID, returns 201.
- Create increments ID counter.
- Replace updates existing item.
- Replace non-existent returns 404.
- Patch merges fields into existing item.
- Patch non-existent returns 404.
- Delete removes item, returns 204.
- Delete non-existent returns 404.
- Custom ID field name.
- Thread safety: concurrent operations don't corrupt state.
- Empty seed: store starts empty, create works.
- ID types: numeric IDs from seed data.
