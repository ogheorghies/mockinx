use crate::reply::{ReplySpec, parse_reply};
use crate::units::{Duration, ParseError, Range, parse_duration_range};
use serde_json::{Map, Value};

/// How to handle requests that exceed concurrency limits.
#[derive(Debug, Clone, PartialEq)]
pub enum OverflowAction {
    /// Immediately reply with an error.
    Reply(ReplySpec),
    /// Block (queue) indefinitely until a slot opens.
    Block,
    /// Block up to a timeout, then reply with an error.
    BlockWithTimeout {
        timeout: Range<Duration>,
        then: ReplySpec,
    },
}

/// Concurrency limit configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ConcurrencySpec {
    pub max: u32,
    pub over: OverflowAction,
}

/// Rate limit configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitSpec {
    pub rps: u32,
    pub over: ReplySpec,
}

/// Failure injection configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct FailSpec {
    /// Fraction of requests that fail (0.0..1.0).
    pub rate: f64,
    /// Reply to send for failed requests.
    pub reply: ReplySpec,
}

/// Scope for sequence counter reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceScope {
    Connection,
    Stub,
}

/// Sequence configuration — different reply per call.
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceSpec {
    pub per: SequenceScope,
    pub replies: Vec<ReplySpec>,
}

/// CRUD ID configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrudIdSpec {
    /// Name of the ID field (default: "id").
    pub name: String,
    /// ID generation strategy (default: "auto").
    pub new: String,
}

impl Default for CrudIdSpec {
    fn default() -> Self {
        CrudIdSpec {
            name: "id".into(),
            new: "auto".into(),
        }
    }
}

/// CRUD behavior configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct CrudSpec {
    pub id: CrudIdSpec,
    pub seed: Vec<Value>,
}

/// Endpoint-level behavior policies.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BehaviorSpec {
    pub concurrency: Option<ConcurrencySpec>,
    pub rate_limit: Option<RateLimitSpec>,
    pub fail: Option<FailSpec>,
    pub timeout: Option<Range<Duration>>,
    pub sequence: Option<SequenceSpec>,
    pub crud: Option<CrudSpec>,
}

/// Parse a `BehaviorSpec` from a `serde_json::Value`.
pub fn parse_behavior(v: &Value) -> Result<BehaviorSpec, ParseError> {
    let obj = v
        .as_object()
        .ok_or_else(|| ParseError("behavior must be an object".into()))?;

    let concurrency = parse_concurrency(obj)?;
    let rate_limit = parse_rate_limit(obj)?;
    let fail = parse_fail(obj)?;
    let timeout = parse_timeout(obj)?;
    let sequence = parse_sequence(obj)?;
    let crud = parse_crud(obj)?;

    Ok(BehaviorSpec {
        concurrency,
        rate_limit,
        fail,
        timeout,
        sequence,
        crud,
    })
}

fn parse_concurrency(obj: &Map<String, Value>) -> Result<Option<ConcurrencySpec>, ParseError> {
    let val = match obj.get("concurrency") {
        None => return Ok(None),
        Some(v) => v,
    };

    let c_obj = val
        .as_object()
        .ok_or_else(|| ParseError("concurrency must be an object".into()))?;

    let max = c_obj
        .get("max")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ParseError("concurrency requires 'max' as a positive integer".into()))?;

    if max == 0 {
        return Err(ParseError("concurrency max must be > 0".into()));
    }

    let over = parse_overflow_action(c_obj)?;

    Ok(Some(ConcurrencySpec {
        max: max as u32,
        over,
    }))
}

fn parse_overflow_action(obj: &Map<String, Value>) -> Result<OverflowAction, ParseError> {
    let over_val = obj
        .get("over")
        .ok_or_else(|| ParseError("concurrency requires 'over' field".into()))?;

    // "block" string
    if let Some(s) = over_val.as_str() {
        if s == "block" {
            return Ok(OverflowAction::Block);
        }
        return Err(ParseError(format!(
            "concurrency.over string must be 'block', got '{s}'"
        )));
    }

    // Object: either a reply {s: ...} or {block: ..., then: ...}
    let over_obj = over_val
        .as_object()
        .ok_or_else(|| ParseError("concurrency.over must be a string or object".into()))?;

    if over_obj.contains_key("block") {
        // BlockWithTimeout
        let block_str = over_obj
            .get("block")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ParseError("block timeout must be a string".into()))?;
        let timeout = parse_duration_range(block_str)?;

        let then_val = over_obj
            .get("then")
            .ok_or_else(|| ParseError("block requires 'then' reply".into()))?;
        let then = parse_reply(then_val)?;

        Ok(OverflowAction::BlockWithTimeout { timeout, then })
    } else {
        // Reply
        Ok(OverflowAction::Reply(parse_reply(over_val)?))
    }
}

fn parse_rate_limit(obj: &Map<String, Value>) -> Result<Option<RateLimitSpec>, ParseError> {
    let val = match obj.get("rate_limit") {
        None => return Ok(None),
        Some(v) => v,
    };

    let rl_obj = val
        .as_object()
        .ok_or_else(|| ParseError("rate_limit must be an object".into()))?;

    let rps = rl_obj
        .get("rps")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ParseError("rate_limit requires 'rps' as a positive integer".into()))?;

    let over_val = rl_obj
        .get("over")
        .ok_or_else(|| ParseError("rate_limit requires 'over' reply".into()))?;
    let over = parse_reply(over_val)?;

    Ok(Some(RateLimitSpec {
        rps: rps as u32,
        over,
    }))
}

fn parse_fail(obj: &Map<String, Value>) -> Result<Option<FailSpec>, ParseError> {
    let val = match obj.get("fail") {
        None => return Ok(None),
        Some(v) => v,
    };

    let f_obj = val
        .as_object()
        .ok_or_else(|| ParseError("fail must be an object".into()))?;

    let rate = f_obj
        .get("rate")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| ParseError("fail requires 'rate' as a number".into()))?;

    if !(0.0..=1.0).contains(&rate) {
        return Err(ParseError(format!(
            "fail rate must be between 0.0 and 1.0, got {rate}"
        )));
    }

    let reply_val = f_obj
        .get("reply")
        .ok_or_else(|| ParseError("fail requires 'reply' field".into()))?;
    let reply = parse_reply(reply_val)?;

    Ok(Some(FailSpec { rate, reply }))
}

fn parse_timeout(obj: &Map<String, Value>) -> Result<Option<Range<Duration>>, ParseError> {
    match obj.get("timeout") {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(parse_duration_range(s)?)),
        Some(v) => Err(ParseError(format!("timeout must be a string, got {v}"))),
    }
}

fn parse_sequence(obj: &Map<String, Value>) -> Result<Option<SequenceSpec>, ParseError> {
    let val = match obj.get("sequence") {
        None => return Ok(None),
        Some(v) => v,
    };

    let seq_obj = val
        .as_object()
        .ok_or_else(|| ParseError("sequence must be an object".into()))?;

    let per_str = seq_obj
        .get("per")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ParseError("sequence requires 'per' field (connection or stub)".into()))?;

    let per = match per_str {
        "connection" => SequenceScope::Connection,
        "stub" => SequenceScope::Stub,
        other => {
            return Err(ParseError(format!(
                "sequence.per must be 'connection' or 'stub', got '{other}'"
            )))
        }
    };

    let replies_val = seq_obj
        .get("replies")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ParseError("sequence requires 'replies' array".into()))?;

    if replies_val.is_empty() {
        return Err(ParseError("sequence replies cannot be empty".into()));
    }

    let mut replies = Vec::with_capacity(replies_val.len());
    for r in replies_val {
        replies.push(parse_reply(r)?);
    }

    Ok(Some(SequenceSpec { per, replies }))
}

fn parse_crud(obj: &Map<String, Value>) -> Result<Option<CrudSpec>, ParseError> {
    let val = match obj.get("crud") {
        None => return Ok(None),
        Some(v) => v,
    };

    let crud_obj = val
        .as_object()
        .ok_or_else(|| ParseError("crud must be an object".into()))?;

    let id = match crud_obj.get("id") {
        None => CrudIdSpec::default(),
        Some(id_val) => {
            let id_obj = id_val
                .as_object()
                .ok_or_else(|| ParseError("crud.id must be an object".into()))?;

            let name = id_obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("id")
                .to_string();

            let new = id_obj
                .get("new")
                .and_then(|v| v.as_str())
                .unwrap_or("auto")
                .to_string();

            CrudIdSpec { name, new }
        }
    };

    let seed = match crud_obj.get("seed") {
        None => Vec::new(),
        Some(Value::Array(arr)) => arr.clone(),
        Some(v) => return Err(ParseError(format!("crud.seed must be an array, got {v}"))),
    };

    Ok(Some(CrudSpec { id, seed }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- Concurrency ---

    #[test]
    fn parse_concurrency_reject() {
        let spec = parse_behavior(&json!({
            "concurrency": {"max": 5, "over": {"s": 429}}
        }))
        .unwrap();
        let c = spec.concurrency.unwrap();
        assert_eq!(c.max, 5);
        match c.over {
            OverflowAction::Reply(r) => assert_eq!(r.status, 429),
            other => panic!("expected Reply, got {other:?}"),
        }
    }

    #[test]
    fn parse_concurrency_block() {
        let spec = parse_behavior(&json!({
            "concurrency": {"max": 5, "over": "block"}
        }))
        .unwrap();
        assert_eq!(spec.concurrency.unwrap().over, OverflowAction::Block);
    }

    #[test]
    fn parse_concurrency_block_timeout() {
        let spec = parse_behavior(&json!({
            "concurrency": {"max": 5, "over": {"block": "3s", "then": {"s": 429, "b": "timeout"}}}
        }))
        .unwrap();
        match spec.concurrency.unwrap().over {
            OverflowAction::BlockWithTimeout { timeout, then } => {
                assert_eq!(timeout, Range::Fixed(Duration(std::time::Duration::from_secs(3))));
                assert_eq!(then.status, 429);
            }
            other => panic!("expected BlockWithTimeout, got {other:?}"),
        }
    }

    #[test]
    fn parse_concurrency_max_zero_error() {
        assert!(parse_behavior(&json!({
            "concurrency": {"max": 0, "over": "block"}
        }))
        .is_err());
    }

    #[test]
    fn parse_concurrency_missing_over_error() {
        assert!(parse_behavior(&json!({
            "concurrency": {"max": 5}
        }))
        .is_err());
    }

    // --- Rate limit ---

    #[test]
    fn parse_rate_limit() {
        let spec = parse_behavior(&json!({
            "rate_limit": {"rps": 100, "over": {"s": 429}}
        }))
        .unwrap();
        let rl = spec.rate_limit.unwrap();
        assert_eq!(rl.rps, 100);
        assert_eq!(rl.over.status, 429);
    }

    #[test]
    fn parse_rate_limit_missing_rps_error() {
        assert!(parse_behavior(&json!({
            "rate_limit": {"over": {"s": 429}}
        }))
        .is_err());
    }

    // --- Fail ---

    #[test]
    fn parse_fail_injection() {
        let spec = parse_behavior(&json!({
            "fail": {"rate": 0.1, "reply": {"s": 500, "b": "internal error"}}
        }))
        .unwrap();
        let f = spec.fail.unwrap();
        assert_eq!(f.rate, 0.1);
        assert_eq!(f.reply.status, 500);
    }

    #[test]
    fn parse_fail_rate_too_high() {
        assert!(parse_behavior(&json!({
            "fail": {"rate": 1.5, "reply": {"s": 500}}
        }))
        .is_err());
    }

    #[test]
    fn parse_fail_rate_negative() {
        assert!(parse_behavior(&json!({
            "fail": {"rate": -0.1, "reply": {"s": 500}}
        }))
        .is_err());
    }

    #[test]
    fn parse_fail_boundary_rates() {
        // rate = 0.0 and 1.0 are valid
        parse_behavior(&json!({"fail": {"rate": 0.0, "reply": {"s": 500}}})).unwrap();
        parse_behavior(&json!({"fail": {"rate": 1.0, "reply": {"s": 500}}})).unwrap();
    }

    // --- Timeout ---

    #[test]
    fn parse_timeout() {
        let spec = parse_behavior(&json!({"timeout": "30s"})).unwrap();
        assert_eq!(
            spec.timeout,
            Some(Range::Fixed(Duration(std::time::Duration::from_secs(30))))
        );
    }

    #[test]
    fn parse_timeout_range() {
        let spec = parse_behavior(&json!({"timeout": "20s..40s"})).unwrap();
        match spec.timeout.unwrap() {
            Range::MinMax(min, max) => {
                assert_eq!(min.as_millis(), 20000);
                assert_eq!(max.as_millis(), 40000);
            }
            _ => panic!("expected MinMax"),
        }
    }

    // --- Sequence ---

    #[test]
    fn parse_sequence_per_connection() {
        let spec = parse_behavior(&json!({
            "sequence": {
                "per": "connection",
                "replies": [
                    {"s": 401, "b": "unauthorized"},
                    {"s": 200, "b": "ok"}
                ]
            }
        }))
        .unwrap();
        let seq = spec.sequence.unwrap();
        assert_eq!(seq.per, SequenceScope::Connection);
        assert_eq!(seq.replies.len(), 2);
        assert_eq!(seq.replies[0].status, 401);
        assert_eq!(seq.replies[1].status, 200);
    }

    #[test]
    fn parse_sequence_per_stub() {
        let spec = parse_behavior(&json!({
            "sequence": {
                "per": "stub",
                "replies": [{"s": 200}]
            }
        }))
        .unwrap();
        assert_eq!(spec.sequence.unwrap().per, SequenceScope::Stub);
    }

    #[test]
    fn parse_sequence_empty_replies_error() {
        assert!(parse_behavior(&json!({
            "sequence": {"per": "stub", "replies": []}
        }))
        .is_err());
    }

    #[test]
    fn parse_sequence_invalid_scope_error() {
        assert!(parse_behavior(&json!({
            "sequence": {"per": "invalid", "replies": [{"s": 200}]}
        }))
        .is_err());
    }

    // --- CRUD ---

    #[test]
    fn parse_crud_defaults() {
        let spec = parse_behavior(&json!({
            "crud": {
                "seed": [
                    {"id": 1, "name": "Ball", "price": 2.99}
                ]
            }
        }))
        .unwrap();
        let crud = spec.crud.unwrap();
        assert_eq!(crud.id.name, "id");
        assert_eq!(crud.id.new, "auto");
        assert_eq!(crud.seed.len(), 1);
    }

    #[test]
    fn parse_crud_custom_id() {
        let spec = parse_behavior(&json!({
            "crud": {
                "id": {"name": "sku", "new": "auto"},
                "seed": []
            }
        }))
        .unwrap();
        let crud = spec.crud.unwrap();
        assert_eq!(crud.id.name, "sku");
    }

    #[test]
    fn parse_crud_no_seed() {
        let spec = parse_behavior(&json!({"crud": {}})).unwrap();
        assert!(spec.crud.unwrap().seed.is_empty());
    }

    #[test]
    fn parse_crud_seed_not_array_error() {
        assert!(parse_behavior(&json!({"crud": {"seed": "bad"}})).is_err());
    }

    // --- General ---

    #[test]
    fn parse_empty_behavior() {
        let spec = parse_behavior(&json!({})).unwrap();
        assert_eq!(spec, BehaviorSpec::default());
    }

    #[test]
    fn parse_error_not_object() {
        assert!(parse_behavior(&json!("bad")).is_err());
    }

    #[test]
    fn parse_multiple_behaviors() {
        let spec = parse_behavior(&json!({
            "concurrency": {"max": 5, "over": "block"},
            "timeout": "30s",
            "fail": {"rate": 0.1, "reply": {"s": 500}}
        }))
        .unwrap();
        assert!(spec.concurrency.is_some());
        assert!(spec.timeout.is_some());
        assert!(spec.fail.is_some());
    }
}
