# 0040 · [feature] Reply spec — parse response definitions

## Goal
Parse reply specifications using the yttp `{s: h: b:}` convention, with header shortcut expansion and body generator support.

## Approach
Create `src/reply.rs` with:
- `ReplySpec` struct:
  - `status: u16` (default 200).
  - `headers: Map<String, Value>` — after shortcut expansion via `yttp::expand_headers`.
  - `body: BodySpec`.
- `BodySpec` enum:
  - `None` — no body.
  - `Literal(Value)` — string or JSON object, serialized to bytes at response time.
  - `Rand { size: ByteSize, seed: u64 }` — from `{rand: {size: 10kb, seed: 7}}`.
  - `Pattern { repeat: String, size: ByteSize }` — from `{pattern: {repeat: "abc", size: 1mb}}`.
- Parsing from `serde_json::Value`:
  - `s` → status code (number).
  - `h` → headers map, expanded via `yttp::expand_headers`.
  - `b` → body: if object with `rand` key → Rand, if object with `pattern` key → Pattern, otherwise Literal.
- `ReplySpec` also used for overflow/error responses in behavior (e.g., `{s: 429, b: "too many"}`), so parsing must handle minimal specs (just `s`, or `s` + `b`).

## Deliverables
- `src/reply.rs` with `ReplySpec`, `BodySpec`, and parsing.

## Acceptance criteria
Unit tests covering:
- Parse `{s: 200, h: {ct!: j!}, b: {name: Owl}}` — status 200, Content-Type expanded, body literal.
- Parse `{s: 204}` — status only, no body.
- Parse `{s: 200, b: {rand: {size: 10kb, seed: 7}}}` — Rand body spec.
- Parse `{s: 200, b: {pattern: {repeat: "abc", size: 1mb}}}` — Pattern body spec.
- Parse `{s: 200, b: "hello"}` — string literal body.
- Header shortcuts expanded: `ct!: t!` → `Content-Type: text/plain`.
- Missing `s` defaults to 200.
- Malformed response: `{s: 200, h: {ct!: h!}, b: '{"valid": "json"}'}`.
- Error on invalid status (negative, > 999, non-number).
- Minimal spec: `{s: 429}` for behavior overflow responses.
