# 0080 · [feature] Body generators — rand and pattern

## Goal
Implement body generators that produce byte streams from `BodySpec::Rand` and `BodySpec::Pattern`.

## Approach
Create `src/body.rs` with:
- `generate_body(spec: &BodySpec) -> Vec<u8>`:
  - `Literal(Value)` — serialize JSON values to bytes, pass strings as UTF-8.
  - `Rand { size, seed }` — deterministic pseudo-random bytes using `rand::rngs::StdRng::seed_from_u64(seed)`. Generate `size` bytes.
  - `Pattern { repeat, size }` — repeat the string cyclically to fill `size` bytes. Truncate last repetition if needed.
  - `None` — empty vec.
- For streaming (used later by delivery engine): `fn body_chunks(spec: &BodySpec, chunk_size: usize) -> impl Iterator<Item = Vec<u8>>`. Generates chunks lazily without materializing the full body in memory for large sizes.

## Deliverables
- `src/body.rs` with `generate_body` and `body_chunks`.

## Acceptance criteria
Unit tests covering:
- Literal string body: correct UTF-8 bytes.
- Literal JSON body: valid JSON serialization.
- Rand: correct size, deterministic (same seed → same bytes).
- Rand: different seeds → different bytes.
- Pattern: `{repeat: "abc", size: 7}` → `"abcabca"`.
- Pattern: size exactly divisible by repeat length.
- Pattern: size smaller than repeat string.
- Pattern: empty repeat string → error.
- Body chunks: verify chunks sum to total size.
- Body chunks: last chunk may be smaller.
- Large body (1MB+): doesn't OOM, produces correct size.
- None body: empty vec.
