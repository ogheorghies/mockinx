# 0010 · [feature] Project scaffold and yttp integration

## Goal
Set up the Rust project with Cargo, dependencies, and module structure. Ensure yttp is usable as a local dependency and that `resolve_method` is accessible.

## Approach
- `cargo init` with binary target.
- Add dependencies: axum, tokio, serde, serde_json, serde_yml, rand, yttp (path = "../yttp").
- Create module skeleton: `src/main.rs`, `src/lib.rs`, `src/units.rs`, `src/match_rule.rs`, `src/reply.rs`, `src/delivery.rs`, `src/behavior.rs`, `src/stub.rs`.
- Check if yttp's `resolve_method` is public. If not, make it public in yttp (add `pub` to the function in `../yttp/src/lib.rs`) so mockinx can reuse it for match parsing. Minimal change — just visibility.
- Initialize git repo, initial commit.

## Deliverables
- `Cargo.toml` with all dependencies.
- Empty module files with `mod` declarations.
- yttp's `resolve_method` exported (if not already).
- Compiling project with `cargo check`.

## Acceptance criteria
- `cargo check` passes.
- `use yttp::{parse, expand_headers, resolve_method};` compiles.
- Git repo initialized with initial commit.
