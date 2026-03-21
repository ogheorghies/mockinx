# 0020 · [pack] Request log — _mx/log

## Goal
Design and implement `GET /_mx/log` and `DELETE /_mx/log` for request recording and inspection.

## Open questions
- **Storage**: unbounded vec, ring buffer, or capped with eviction?
- **Memory pressure**: high-frequency testing could accumulate large amounts of data. Options:
  - Max entries (e.g., last N requests).
  - Max memory budget.
  - Opt-in (disabled by default, enabled per-stub or globally).
  - Disk-backed (write to temp file, serve on demand).
- **Granularity**: what to capture? Method+path+headers+body+timestamp is the full picture, but body capture is the expensive part. Maybe body capture is opt-in.
- **Filtering**: by path, method — anything else? Time range?
- **Performance**: should recording be async/non-blocking relative to response delivery?
