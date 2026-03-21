# 0020 · [feature] Value units — sizes, durations, speeds, and ranges

## Goal
Parse size strings (`10kb`, `1mb`, `512b`), duration strings (`2s`, `500ms`), and speed strings (`10kb/s`, `100b/s`) into typed values. Support range/jitter syntax: explicit ranges (`4s..6s`, `1kb..4kb`) and percentage ranges (`1s..10%`, `10kb/s..20%`). Ranges must be samplable.

## Approach
Create `src/units.rs` with:
- `ByteSize` — parsed from `512b`, `10kb`, `1mb`, `10gb`. Stores bytes as u64.
- `Duration` — parsed from `100ms`, `2s`, `5m`. Stores as `std::time::Duration`.
- `Speed` — parsed from `10kb/s`, `100b/s`. Stores bytes-per-second as u64.
- `Range<T>` — wraps either a fixed value or a min..max pair. Constructed from:
  - Fixed: `"5s"` → Range with min=max=5s.
  - Explicit range: `"4s..6s"` → Range(4s, 6s).
  - Percentage range: `"1s..10%"` → Range(900ms, 1.1s).
- `Range::sample(&self, rng: &mut impl Rng) -> T` — uniform random within range.
- Parsing from `serde_json::Value` (strings) and from `&str`.

## Deliverables
- `src/units.rs` with `ByteSize`, `Duration`, `Speed`, `Range<T>`.
- Parsing functions for each type from string.
- `Range::sample()` method.

## Acceptance criteria
Unit tests covering:
- Parse valid sizes: `"512b"` → 512, `"10kb"` → 10240, `"1mb"` → 1048576.
- Parse valid durations: `"100ms"` → 100ms, `"2s"` → 2s.
- Parse valid speeds: `"10kb/s"` → 10240 bytes/sec.
- Parse ranges: `"4s..6s"` → Range(4s, 6s), `"1s..10%"` → Range(900ms, 1100ms).
- Sample from range stays within bounds (statistical test, 100 samples).
- Fixed value range always returns same value.
- Error on invalid input: `"10xx"`, `"abc"`, `""`, `"10kb..abc"`.
- Case handling: `"10KB"`, `"1Mb"`.
- Edge cases: `"0b"`, `"0s"`, `"0b/s"`.
