# 0070 · [feature] Stub — top-level struct combining all specs

## Goal
Parse complete stub definitions from YAML/JSON, combining match + reply + delivery + behavior. Support single object and array (batch) input.

## Approach
Create `src/stub.rs` with:
- `Stub` struct:
  - `match_rule: MatchRule`.
  - `reply: Option<ReplySpec>` — optional because behavior.crud or behavior.sequence can provide replies.
  - `delivery: DeliverySpec` — defaults to passthrough if absent.
  - `behavior: BehaviorSpec` — defaults to empty if absent.
- Parsing from `serde_json::Value`:
  - Use `yttp::parse` to parse raw YAML/JSON string into Value.
  - Extract `match`, `reply`, `delivery`, `behavior` keys.
  - Delegate to respective parsers.
- `parse_stubs(val: &Value) -> Result<Vec<Stub>>`:
  - If Value is an array → parse each element as a Stub.
  - If Value is an object → parse as single Stub, return vec of one.
- Validation: a stub must have either `reply`, `behavior.sequence`, or `behavior.crud` (otherwise there's nothing to respond with).

## Deliverables
- `src/stub.rs` with `Stub` and parsing functions.

## Acceptance criteria
Unit tests covering:
- Parse minimal stub: `{match: {g: /path}, reply: {s: 200}}`.
- Parse full stub with all four sections.
- Parse stub with reply + delivery, no behavior.
- Parse stub with behavior.sequence, no reply.
- Parse stub with behavior.crud, no reply.
- Parse array of stubs (batch).
- Parse single stub returns vec of one.
- Error: stub with no reply and no behavior that provides replies.
- Error: invalid match value.
- Round-trip: parse from YAML string via `yttp::parse` then `parse_stubs`.
- Full examples from README parse successfully.
