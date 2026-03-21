# 0130 · [feature] Server skeleton — axum app with stub registration and request handling

## Goal
Build the HTTP server with `POST /_mx` for stub registration and a catch-all handler that matches requests to stubs and produces responses.

## Approach
Create/update `src/main.rs` and `src/server.rs`:
- CLI: parse `-p <port>` (default 9999) and `-c <config_file>` using clap or manual arg parsing.
- Axum app with shared state (`StubStore`):
  - `POST /_mx` — parse body as YAML/JSON (via yttp::parse), call `parse_stubs`, add to store. Return 201 on success.
  - Catch-all `ANY /*path` — the main handler:
    1. Match against stub store.
    3. If no match → 404.
    4. Check behavior policies via behavior engine.
    5. If rejected → send reject reply.
    6. Resolve reply (from stub reply, sequence, or CRUD).
    7. Generate body.
    8. Apply delivery shaping.
    9. Send response.
- Config file (`-c`): read file, parse as YAML (may contain array of stubs or `---` separated docs), register all stubs at startup.
## Deliverables
- `src/server.rs` with axum router setup and handlers.
- `src/main.rs` with CLI parsing and server startup.

## Acceptance criteria
- Server starts and listens on configured port.
- `POST /_mx` registers stubs (single and array).
- Catch-all returns 404 when no stubs match.
- Catch-all returns configured reply when stub matches.
- Config file loaded at startup.
- Graceful error responses for malformed stub input.
