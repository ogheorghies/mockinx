# 0100 · [feature] Stub store — thread-safe stub registry with matching

## Goal
A thread-safe, priority-ordered store for registered stubs. Supports matching incoming requests to the best stub and managing sequence state.

## Approach
Create `src/store.rs` with:
- `StubStore` — wraps `Arc<RwLock<Vec<StubEntry>>>`.
- `StubEntry` — a `Stub` plus runtime state (sequence counter, concurrency semaphore, etc.).
- `StubStore::add(stub: Stub)` — appends to the list. Later stubs take precedence.
- `StubStore::add_batch(stubs: Vec<Stub>)` — appends all, maintaining order.
- `StubStore::match_request(&self, method: &str, path: &str) -> Option<Arc<StubEntry>>`:
  - Iterate in reverse order (last added = highest priority).
  - Return first matching stub.
- `StubStore::clear()` — remove all stubs.
- Sequence state: `StubEntry` tracks a call counter (atomic). For per-stub sequences, the counter persists across requests. Per-connection sequences need connection-level state (tracked separately, perhaps by connection ID).

## Deliverables
- `src/store.rs` with `StubStore` and `StubEntry`.

## Acceptance criteria
Unit tests covering:
- Add and match a single stub.
- Priority: later stubs matched first when multiple match.
- No match returns None.
- Batch add preserves order within batch.
- Clear removes all stubs.
- Thread safety: concurrent reads and writes don't panic (spawn multiple tokio tasks).
- Sequence counter increments on each match.
- Catch-all stub matches everything but is lower priority than specific stubs.
