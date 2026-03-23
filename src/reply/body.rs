use super::BodySpec;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;

/// Generate a complete body from a `BodySpec`.
pub fn generate_body(spec: &BodySpec) -> Vec<u8> {
    match spec {
        BodySpec::None => Vec::new(),
        BodySpec::Literal(val) => match val {
            serde_json::Value::String(s) => s.as_bytes().to_vec(),
            other => serde_json::to_vec(other).unwrap_or_default(),
        },
        BodySpec::Rand { size, seed } => generate_rand(size.bytes() as usize, *seed),
        BodySpec::Pattern { repeat, size } => generate_pattern(repeat, size.bytes() as usize),
        BodySpec::Reflect(_) => unreachable!("reflect! should be resolved before body generation"),
    }
}

/// Generate deterministic pseudo-random bytes.
fn generate_rand(size: usize, seed: u64) -> Vec<u8> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut buf = vec![0u8; size];
    rng.fill(&mut buf[..]);
    buf
}

/// Generate a pattern body by repeating a string cyclically.
fn generate_pattern(repeat: &str, size: usize) -> Vec<u8> {
    if size == 0 {
        return Vec::new();
    }
    let pattern = repeat.as_bytes();
    let mut buf = Vec::with_capacity(size);
    let mut cycle = pattern.iter().cycle();
    for _ in 0..size {
        buf.push(*cycle.next().unwrap());
    }
    buf
}

/// Iterator that yields body chunks lazily.
pub struct BodyChunks {
    spec: BodySpec,
    chunk_size: usize,
    offset: usize,
    total_size: usize,
    // For Rand: seeded RNG state
    rng: Option<StdRng>,
    // For Pattern: precomputed pattern bytes
    pattern: Option<Vec<u8>>,
    // For Literal: materialized bytes
    literal: Option<Vec<u8>>,
}

impl BodyChunks {
    pub fn new(spec: &BodySpec, chunk_size: usize) -> Self {
        let chunk_size = chunk_size.max(1);
        let total_size = body_size(spec);

        let (rng, pattern, literal) = match spec {
            BodySpec::Rand { seed, .. } => (Some(StdRng::seed_from_u64(*seed)), None, None),
            BodySpec::Pattern { repeat, .. } => (None, Some(repeat.as_bytes().to_vec()), None),
            BodySpec::Literal(_) => (None, None, Some(generate_body(spec))),
            BodySpec::None => (None, None, None),
            BodySpec::Reflect(_) => unreachable!("reflect! should be resolved before chunking"),
        };

        BodyChunks {
            spec: spec.clone(),
            chunk_size,
            offset: 0,
            total_size,
            rng,
            pattern,
            literal,
        }
    }
}

impl Iterator for BodyChunks {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        if self.offset >= self.total_size {
            return None;
        }

        let remaining = self.total_size - self.offset;
        let len = remaining.min(self.chunk_size);

        let chunk = match &self.spec {
            BodySpec::None => return None,
            BodySpec::Rand { .. } => {
                let rng = self.rng.as_mut().unwrap();
                let mut buf = vec![0u8; len];
                rng.fill(&mut buf[..]);
                buf
            }
            BodySpec::Pattern { .. } => {
                let pat = self.pattern.as_ref().unwrap();
                let mut buf = Vec::with_capacity(len);
                for i in 0..len {
                    buf.push(pat[(self.offset + i) % pat.len()]);
                }
                buf
            }
            BodySpec::Literal(_) => {
                let data = self.literal.as_ref().unwrap();
                data[self.offset..self.offset + len].to_vec()
            }
            BodySpec::Reflect(_) => unreachable!("reflect! should be resolved before chunking"),
        };

        self.offset += len;
        Some(chunk)
    }
}

/// Compute the total body size for a spec.
fn body_size(spec: &BodySpec) -> usize {
    match spec {
        BodySpec::None => 0,
        BodySpec::Literal(val) => match val {
            serde_json::Value::String(s) => s.len(),
            other => serde_json::to_vec(other).map(|v| v.len()).unwrap_or(0),
        },
        BodySpec::Rand { size, .. } => size.bytes() as usize,
        BodySpec::Pattern { size, .. } => size.bytes() as usize,
        BodySpec::Reflect(_) => unreachable!("reflect! should be resolved before size calculation"),
    }
}

/// Create a lazy chunk iterator for a body spec.
pub fn body_chunks(spec: &BodySpec, chunk_size: usize) -> BodyChunks {
    BodyChunks::new(spec, chunk_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::ByteSize;
    use serde_json::json;

    #[test]
    fn generate_none_body() {
        assert!(generate_body(&BodySpec::None).is_empty());
    }

    #[test]
    fn generate_string_literal() {
        let body = generate_body(&BodySpec::Literal(json!("hello")));
        assert_eq!(body, b"hello");
    }

    #[test]
    fn generate_json_literal() {
        let body = generate_body(&BodySpec::Literal(json!({"key": "val"})));
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["key"], "val");
    }

    #[test]
    fn generate_array_literal() {
        let body = generate_body(&BodySpec::Literal(json!([1, 2, 3])));
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed, json!([1, 2, 3]));
    }

    #[test]
    fn generate_number_literal() {
        let body = generate_body(&BodySpec::Literal(json!(42)));
        assert_eq!(body, b"42");
    }

    #[test]
    fn generate_rand_correct_size() {
        let body = generate_body(&BodySpec::Rand {
            size: ByteSize(1024),
            seed: 7,
        });
        assert_eq!(body.len(), 1024);
    }

    #[test]
    fn generate_rand_deterministic() {
        let body1 = generate_body(&BodySpec::Rand {
            size: ByteSize(100),
            seed: 42,
        });
        let body2 = generate_body(&BodySpec::Rand {
            size: ByteSize(100),
            seed: 42,
        });
        assert_eq!(body1, body2);
    }

    #[test]
    fn generate_rand_different_seeds() {
        let body1 = generate_body(&BodySpec::Rand {
            size: ByteSize(100),
            seed: 1,
        });
        let body2 = generate_body(&BodySpec::Rand {
            size: ByteSize(100),
            seed: 2,
        });
        assert_ne!(body1, body2);
    }

    #[test]
    fn generate_pattern_exact() {
        let body = generate_body(&BodySpec::Pattern {
            repeat: "abc".into(),
            size: ByteSize(6),
        });
        assert_eq!(body, b"abcabc");
    }

    #[test]
    fn generate_pattern_truncated() {
        let body = generate_body(&BodySpec::Pattern {
            repeat: "abc".into(),
            size: ByteSize(7),
        });
        assert_eq!(body, b"abcabca");
    }

    #[test]
    fn generate_pattern_smaller_than_repeat() {
        let body = generate_body(&BodySpec::Pattern {
            repeat: "abcdef".into(),
            size: ByteSize(3),
        });
        assert_eq!(body, b"abc");
    }

    #[test]
    fn generate_pattern_zero_size() {
        let body = generate_body(&BodySpec::Pattern {
            repeat: "abc".into(),
            size: ByteSize(0),
        });
        assert!(body.is_empty());
    }

    // --- Chunked iteration ---

    #[test]
    fn chunks_sum_to_total_size() {
        let spec = BodySpec::Rand {
            size: ByteSize(1000),
            seed: 7,
        };
        let chunks: Vec<Vec<u8>> = body_chunks(&spec, 300).collect();
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, 1000);
    }

    #[test]
    fn chunks_last_may_be_smaller() {
        let spec = BodySpec::Rand {
            size: ByteSize(1000),
            seed: 7,
        };
        let chunks: Vec<Vec<u8>> = body_chunks(&spec, 300).collect();
        assert_eq!(chunks.len(), 4); // 300 + 300 + 300 + 100
        assert_eq!(chunks[0].len(), 300);
        assert_eq!(chunks[3].len(), 100);
    }

    #[test]
    fn chunks_pattern_matches_full_body() {
        let spec = BodySpec::Pattern {
            repeat: "abc".into(),
            size: ByteSize(10),
        };
        let full = generate_body(&spec);
        let chunked: Vec<u8> = body_chunks(&spec, 3).flat_map(|c| c).collect();
        assert_eq!(full, chunked);
    }

    #[test]
    fn chunks_rand_deterministic() {
        // Same seed + same chunk_size → same bytes each time
        let spec = BodySpec::Rand {
            size: ByteSize(100),
            seed: 42,
        };
        let run1: Vec<u8> = body_chunks(&spec, 30).flat_map(|c| c).collect();
        let run2: Vec<u8> = body_chunks(&spec, 30).flat_map(|c| c).collect();
        assert_eq!(run1, run2);
        assert_eq!(run1.len(), 100);
    }

    #[test]
    fn chunks_literal() {
        let spec = BodySpec::Literal(json!("hello world"));
        let chunks: Vec<Vec<u8>> = body_chunks(&spec, 5).collect();
        let total: Vec<u8> = chunks.into_iter().flat_map(|c| c).collect();
        assert_eq!(total, b"hello world");
    }

    #[test]
    fn chunks_none_body() {
        let spec = BodySpec::None;
        let chunks: Vec<Vec<u8>> = body_chunks(&spec, 100).collect();
        assert!(chunks.is_empty());
    }

    #[test]
    fn large_rand_body() {
        let spec = BodySpec::Rand {
            size: ByteSize(1024 * 1024), // 1MB
            seed: 99,
        };
        let body = generate_body(&spec);
        assert_eq!(body.len(), 1024 * 1024);
    }

    #[test]
    fn large_pattern_body() {
        let spec = BodySpec::Pattern {
            repeat: "abcdefghij".into(),
            size: ByteSize(1024 * 1024), // 1MB
        };
        let body = generate_body(&spec);
        assert_eq!(body.len(), 1024 * 1024);
        // Verify pattern correctness at boundaries
        assert_eq!(body[0], b'a');
        assert_eq!(body[9], b'j');
        assert_eq!(body[10], b'a');
    }
}
