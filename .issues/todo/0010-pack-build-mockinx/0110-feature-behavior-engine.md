# 0110 · [feature] Behavior engine — runtime policy enforcement

## Goal
Implement runtime enforcement of behavior policies: concurrency limits, rate limiting, failure injection, and timeout.

## Approach
Create `src/behavior_engine.rs` with:
- `BehaviorRuntime` — per-stub runtime state:
  - `concurrency: Option<Semaphore>` — tokio semaphore with max permits.
  - `rate_limiter: Option<RateLimiter>` — token bucket or sliding window, `rps` requests per second.
  - `fail_rng: Option<Mutex<StdRng>>` — for fail injection randomness.
- `BehaviorRuntime::new(spec: &BehaviorSpec) -> Self`.
- `async fn check_behavior(runtime: &BehaviorRuntime, spec: &BehaviorSpec) -> BehaviorResult`:
  - Check rate limit → if exceeded, return `Reject(reply)`.
  - Check fail injection → with probability `rate`, return `Reject(reply)`.
  - Check concurrency → try acquire semaphore:
    - If `over` is Reply: try_acquire, if fails return `Reject(reply)`.
    - If `over` is Block: acquire (wait indefinitely).
    - If `over` is BlockWithTimeout: acquire with timeout, if times out return `Reject(reply)`.
  - Return `Proceed(permit)` — caller must hold permit until response is complete.
- `BehaviorResult` enum: `Proceed(SemaphorePermit)` or `Reject(ReplySpec)`.
- Timeout: wrap the entire response in `tokio::time::timeout`.

## Deliverables
- `src/behavior_engine.rs` with `BehaviorRuntime` and `check_behavior`.

## Acceptance criteria
Unit tests (tokio::test):
- Concurrency reject: 5 concurrent tasks, 6th gets reject reply.
- Concurrency block: 6th task waits until one finishes.
- Concurrency block+timeout: 6th task waits up to timeout, then gets reject.
- Rate limit: burst of requests, some get rejected.
- Fail injection: over many calls, approximately `rate` fraction are failures.
- No behavior: all requests proceed.
- Timeout: long-running handler gets cancelled after timeout.
