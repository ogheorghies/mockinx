# 0050 · [feature] Delivery spec — parse delivery configuration

## Goal
Parse the delivery block that controls how response bytes are shaped on the wire.

## Approach
Create `src/delivery.rs` with:
- `DeliverySpec` struct:
  - `first_byte: Option<FirstByteSpec>` — `{delay: Range<Duration>}`.
  - `duration: Option<Range<Duration>>`.
  - `speed: Option<Range<Speed>>`.
  - `chunk: Option<ChunkSpec>` — `{size: Range<ByteSize>, delay: Range<Duration>}`.
  - `drop: Option<DropSpec>`.
  - `pick: Option<Vec<PickEntry>>`.
- `DropSpec` enum — `AfterBytes(Range<ByteSize>)` or `AfterTime(Range<Duration>)`. Distinguished by unit suffix during parsing.
- `PickEntry` — `{ p: f64, spec: DeliverySpec }`. Probabilities should sum to 1.
- Parsing from `serde_json::Value`. All scalar fields use `Range` parsing from the units module.
- `DeliverySpec::default()` for no delivery shaping (passthrough).

## Deliverables
- `src/delivery.rs` with `DeliverySpec`, `DropSpec`, `PickEntry`, and parsing.

## Acceptance criteria
Unit tests covering:
- Parse `{duration: 5s}` — fixed duration.
- Parse `{speed: 10kb/s}` — fixed speed.
- Parse `{drop: {after: 2kb}}` — drop after bytes.
- Parse `{drop: {after: 1s}}` — drop after time.
- Parse `{first_byte: {delay: 2s}}` — first byte delay.
- Parse `{chunk: {size: 1kb, delay: 100ms}}` — chunk spec.
- Range values: `{duration: 4s..6s}`, `{speed: 10kb/s..20%}`.
- Parse pick array with probabilities.
- Empty/missing delivery → default (no shaping).
- Multiple fields together: `{first_byte: {delay: 2s}, duration: 5s, drop: {after: 2kb}}`.
- Error: pick probabilities don't sum to 1 (warn or error).
- Error: conflicting duration + speed + chunk (or allow — decide).
