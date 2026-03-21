# 0060 · [feature] Behavior spec — parse endpoint-level policies

## Goal
Parse the behavior block that defines endpoint-level policies: concurrency, rate limiting, failure injection, timeout, sequences, and CRUD.

## Approach
Create `src/behavior.rs` with:
- `BehaviorSpec` struct containing optional fields:
  - `concurrency: Option<ConcurrencySpec>`.
  - `rate_limit: Option<RateLimitSpec>`.
  - `fail: Option<FailSpec>`.
  - `timeout: Option<Range<Duration>>`.
  - `sequence: Option<SequenceSpec>`.
  - `crud: Option<CrudSpec>`.
- `ConcurrencySpec` — `{ max: u32, over: OverflowAction }`.
- `OverflowAction` enum:
  - `Reply(ReplySpec)` — e.g., `{s: 429, b: "too many"}`.
  - `Block` — queue indefinitely.
  - `BlockWithTimeout { timeout: Range<Duration>, then: ReplySpec }` — e.g., `{block: 3s, then: {s: 429}}`.
- `RateLimitSpec` — `{ rps: u32, over: ReplySpec }`.
- `FailSpec` — `{ rate: f64, reply: ReplySpec }`. Rate is 0.0..1.0.
- `SequenceSpec` — `{ per: SequenceScope, replies: Vec<ReplySpec> }`.
- `SequenceScope` enum — `Connection`, `Stub`.
- `CrudSpec` — `{ id: CrudIdSpec, seed: Vec<Value> }`.
- `CrudIdSpec` — `{ name: String, new: String }`. Defaults: name="id", new="auto".
- Parsing from `serde_json::Value`. Reuses `ReplySpec` parsing for embedded replies.

## Deliverables
- `src/behavior.rs` with all spec types and parsing.

## Acceptance criteria
Unit tests covering:
- Parse concurrency with reject: `{concurrency: {max: 5, over: {s: 429}}}`.
- Parse concurrency with block: `{concurrency: {max: 5, over: block}}`.
- Parse concurrency with block+timeout: `{concurrency: {max: 5, over: {block: 3s, then: {s: 429}}}}`.
- Parse rate limit: `{rate_limit: {rps: 100, over: {s: 429}}}`.
- Parse fail: `{fail: {rate: 0.1, reply: {s: 500}}}`.
- Parse timeout: `{timeout: 30s}`.
- Parse sequence with per-connection: `{sequence: {per: connection, replies: [...]}}`.
- Parse sequence with per-stub scope.
- Parse crud with defaults: `{crud: {seed: [...]}}` → id name="id", new="auto".
- Parse crud with custom id: `{crud: {id: {name: sku, new: auto}, seed: [...]}}`.
- Empty/missing behavior → all None.
- Error: fail rate outside 0..1.
- Error: concurrency max is 0 or negative.
- Error: sequence with empty replies list.
