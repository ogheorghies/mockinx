# 0140 · [feature] Integration tests — end-to-end verification with running server

## Goal
Comprehensive integration tests that spin up a real mockinx server and verify all behaviors via HTTP requests.

## Approach
Create `tests/integration.rs`:
- Test harness: helper that starts mockinx on a random available port, returns the base URL, and shuts down on drop.
- Use `reqwest` as the HTTP client for tests.
- Test groups:

**Basic matching and replies:**
- Register stub, hit endpoint, verify status/headers/body.
- Header shortcut expansion in replies (ct!: j! → Content-Type: application/json).
- Multiple stubs, priority ordering (later wins).
- Catch-all match.
- No match → 404.
- Batch stub registration (array).

**Body generators:**
- `rand` body: correct size, deterministic (same seed → same bytes across requests).
- `pattern` body: correct content and size.

**Delivery:**
- `first_byte: {delay: 500ms}` — measure TTFB, verify ≥ 500ms.
- `duration: 2s` — measure total download time, verify ≈ 2s (±500ms).
- `speed: 10kb/s` — measure throughput, verify ≈ 10KB/s (±30%).
- `chunk` — verify Transfer-Encoding chunked behavior.
- `drop: {after: 1kb}` — verify connection drops after ≈ 1KB received.
- `drop: {after: 500ms}` — verify connection drops after ≈ 500ms.
- `pick` — statistical: over 100 requests, verify distribution roughly matches probabilities (±15%).
- Range values: verify responses fall within configured bounds over multiple requests.

**Behavior:**
- Concurrency reject: spawn 6 concurrent requests with max 5, verify one gets 429.
- Concurrency block: spawn 6, verify all eventually succeed (with measured delay).
- Concurrency block+timeout: verify timeout produces reject reply.
- Rate limit: burst requests, verify some rejected.
- Fail injection: over 100 requests with rate 0.5, verify roughly half fail (±15%).
- Sequence per-stub: successive requests get different replies in order.
- Timeout: stub with timeout shorter than delivery duration → connection terminated.

**CRUD:**
- GET /resource → list (empty and with seed data).
- GET /resource/id → single item.
- GET /resource/missing → 404.
- POST /resource → create with auto-id, returns 201.
- PUT /resource/id → replace.
- PATCH /resource/id → partial update.
- DELETE /resource/id → 204.
- DELETE /resource/missing → 404.
- CRUD + delivery: verify latency applied to CRUD responses.

**Config file:**
- Start with config file, verify stubs loaded.

## Deliverables
- `tests/integration.rs` with all test groups.
- Test helper for server lifecycle management.

## Acceptance criteria
- All tests pass with `cargo test`.
- Timing-sensitive tests use reasonable tolerances (±20-30%).
- Tests are independent (each starts a fresh server).
- No flaky tests under normal conditions.
