# 0090 · [feature] Delivery engine — shape response byte streams

## Goal
Given a body (as bytes or stream) and a `DeliverySpec`, produce a shaped output that applies timing, chunking, throttling, and fault injection.

## Approach
Create `src/delivery_engine.rs` with:
- `async fn deliver(body: Vec<u8>, spec: &DeliverySpec, rng: &mut impl Rng) -> DeliveryStream`:
  - Resolve ranges by sampling from rng.
  - `first_byte` — sleep for the sampled delay before yielding anything.
  - `chunk` — split body into chunks of configured size, sleep between each.
  - `duration` — compute chunk delay from `duration / num_chunks`. If no chunk size specified, use a reasonable default (e.g., 8KB).
  - `speed` — compute delay per chunk as `chunk_size / speed` seconds.
  - `drop.after_bytes` — truncate the body stream after N bytes.
  - `drop.after_time` — set a timer; abort the stream when it fires.
  - `pick` — select one delivery profile by weighted random, then apply it.
- `DeliveryStream` — an async stream of `Result<Bytes, Error>` suitable for use as an axum response body.
- If no delivery spec (default), yield the entire body at once.

## Deliverables
- `src/delivery_engine.rs` with `deliver` function and `DeliveryStream`.

## Acceptance criteria
Unit tests (using tokio::test):
- No shaping: body yielded in full, immediately.
- First byte delay: measure elapsed time, verify ≥ delay (with tolerance).
- Chunking: verify correct number of chunks and sizes.
- Duration: body delivered over approximately the target duration (±20% tolerance).
- Speed throttle: time to deliver matches expected `size/speed` (±20%).
- Drop after bytes: stream yields exactly N bytes then stops.
- Drop after time: stream stops after approximately N time.
- Pick: verify that over many invocations, different profiles are selected.
- Combined: first_byte delay + chunking works together.
