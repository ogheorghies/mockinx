# 0030 · [feature] Match rules — parse and evaluate request matching

## Goal
Parse the match syntax from stub config and evaluate it against incoming (method, path) pairs.

## Approach
Create `src/match_rule.rs` with:
- `MatchRule` enum:
  - `CatchAll` — from `match: _`, matches everything.
  - `MethodPath { method: Option<String>, path: String }` — from `{g: /path}`, `{_: /path}`.
- Parsing from `serde_json::Value`:
  - String `"_"` → `CatchAll`.
  - Object with single key: use yttp's `resolve_method` for method shortcuts (`g` → GET, `p` → POST, etc.). `_` key means any method.
- `MatchRule::matches(&self, method: &str, path: &str) -> bool`.
- Path matching: exact match for now. The path from the match rule should match the request path (with or without leading `/` normalization).

## Deliverables
- `src/match_rule.rs` with `MatchRule` and parsing/matching logic.

## Acceptance criteria
Unit tests covering:
- `{g: /api/data}` matches GET /api/data, not POST /api/data.
- `{p: /api/data}` matches POST /api/data.
- `{_: /api/data}` matches any method on /api/data.
- `_` matches everything.
- All yttp method shortcuts: g, p, d, put, patch, head, options, trace.
- Non-matching path returns false.
- Invalid match value (e.g., number, empty object) returns error.
- Path with and without leading `/`.
