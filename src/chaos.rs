use crate::delivery::{DeliverySpec, parse_delivery_fields};
use crate::reply::{ReplySpec, parse_reply};
use crate::units::ParseError;
use rand::Rng;
use serde_json::Value;

/// A chaos entry: probabilistic override for reply and/or delivery.
#[derive(Debug, Clone)]
pub struct ChaosEntry {
    /// Weight as a percentage (0.1 = 0.1% of requests).
    pub p: f64,
    /// Optional reply override.
    pub reply: Option<ReplySpec>,
    /// Optional delivery override (serve-level shaping only).
    pub serve: Option<DeliverySpec>,
}

/// Result of chaos resolution for a single request.
pub enum ChaosResult {
    /// No chaos entry selected — use rule defaults.
    Normal,
    /// A chaos entry was selected.
    Override {
        reply: Option<ReplySpec>,
        serve: Option<DeliverySpec>,
    },
}

/// Parse chaos entries from a `serde_json::Value` (expects an array).
pub fn parse_chaos(v: &Value) -> Result<Vec<ChaosEntry>, ParseError> {
    let arr = v
        .as_array()
        .ok_or_else(|| ParseError("chaos must be an array".into()))?;

    if arr.is_empty() {
        return Err(ParseError("chaos array cannot be empty".into()));
    }

    let mut entries = Vec::with_capacity(arr.len());
    let mut total_p = 0.0f64;

    for item in arr {
        let obj = item
            .as_object()
            .ok_or_else(|| ParseError("chaos entry must be an object".into()))?;

        let p = obj
            .get("p")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ParseError("chaos entry requires 'p' as a number".into()))?;

        if p < 0.0 {
            return Err(ParseError(format!("chaos probability {p} cannot be negative")));
        }

        total_p += p;

        let reply = match obj.get("reply") {
            Some(v) => Some(parse_reply(v)?),
            None => None,
        };

        let serve = match obj.get("serve") {
            Some(v) => {
                let serve_obj = v
                    .as_object()
                    .ok_or_else(|| ParseError("chaos serve must be an object".into()))?;
                let spec = parse_delivery_fields(serve_obj)?;
                if spec == DeliverySpec::default() {
                    None
                } else {
                    Some(spec)
                }
            }
            None => None,
        };

        if reply.is_none() && serve.is_none() {
            return Err(ParseError(
                "chaos entry must have 'reply' and/or 'serve'".into(),
            ));
        }

        entries.push(ChaosEntry { p, reply, serve });
    }

    if total_p > 100.0 {
        return Err(ParseError(format!(
            "chaos weights sum to {total_p}, cannot exceed 100"
        )));
    }

    Ok(entries)
}

/// Resolve chaos for a single request.
///
/// Rolls a random number [0, 100) and selects an entry based on cumulative weights.
/// If no entry matches (weights sum < 100), returns Normal.
pub fn resolve_chaos(entries: &[ChaosEntry], rng: &mut impl Rng) -> ChaosResult {
    let roll: f64 = rng.r#gen::<f64>() * 100.0;
    let mut cumulative = 0.0;

    for entry in entries {
        cumulative += entry.p;
        if roll < cumulative {
            return ChaosResult::Override {
                reply: entry.reply.clone(),
                serve: entry.serve.clone(),
            };
        }
    }

    ChaosResult::Normal
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use serde_json::json;

    #[test]
    fn parse_chaos_reply_override() {
        let entries = parse_chaos(&json!([
            {"p": 10, "reply": {"s": 500, "b": "error"}}
        ]))
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].p, 10.0);
        assert!(entries[0].reply.is_some());
        assert!(entries[0].serve.is_none());
    }

    #[test]
    fn parse_chaos_serve_override() {
        let entries = parse_chaos(&json!([
            {"p": 5, "serve": {"drop": "1kb"}}
        ]))
        .unwrap();
        assert!(entries[0].serve.is_some());
        assert!(entries[0].reply.is_none());
    }

    #[test]
    fn parse_chaos_both_overrides() {
        let entries = parse_chaos(&json!([
            {"p": 3, "reply": {"s": 500}, "serve": {"pace": "100b/s"}}
        ]))
        .unwrap();
        assert!(entries[0].reply.is_some());
        assert!(entries[0].serve.is_some());
    }

    #[test]
    fn parse_chaos_error_empty() {
        assert!(parse_chaos(&json!([])).is_err());
    }

    #[test]
    fn parse_chaos_error_over_100() {
        assert!(parse_chaos(&json!([
            {"p": 60, "reply": {"s": 500}},
            {"p": 50, "reply": {"s": 503}}
        ]))
        .is_err());
    }

    #[test]
    fn parse_chaos_error_no_override() {
        assert!(parse_chaos(&json!([{"p": 10}])).is_err());
    }

    #[test]
    fn parse_chaos_error_negative_p() {
        assert!(parse_chaos(&json!([{"p": -5, "reply": {"s": 500}}])).is_err());
    }

    #[test]
    fn resolve_chaos_statistical() {
        let entries = parse_chaos(&json!([
            {"p": 50, "reply": {"s": 500, "b": "error"}}
        ]))
        .unwrap();

        let mut override_count = 0;
        let mut normal_count = 0;
        for seed in 0..200 {
            let mut rng = StdRng::seed_from_u64(seed);
            match resolve_chaos(&entries, &mut rng) {
                ChaosResult::Normal => normal_count += 1,
                ChaosResult::Override { .. } => override_count += 1,
            }
        }

        assert!(override_count > 70, "too few overrides: {override_count}");
        assert!(normal_count > 70, "too few normals: {normal_count}");
    }

    #[test]
    fn resolve_chaos_remainder_is_normal() {
        let entries = parse_chaos(&json!([
            {"p": 1, "reply": {"s": 500}}
        ]))
        .unwrap();

        // 1% chance — over 100 tries, most should be Normal
        let mut normal_count = 0;
        for seed in 0..100 {
            let mut rng = StdRng::seed_from_u64(seed);
            if matches!(resolve_chaos(&entries, &mut rng), ChaosResult::Normal) {
                normal_count += 1;
            }
        }
        assert!(normal_count > 90, "too few normals: {normal_count}");
    }
}
