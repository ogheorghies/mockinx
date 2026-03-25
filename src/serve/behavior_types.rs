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

/// CRUD ID configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrudIdSpec {
    /// Name of the ID field (default: "id").
    pub name: String,
    /// ID generation strategy (default: "inc").
    pub new: String,
}

impl Default for CrudIdSpec {
    fn default() -> Self {
        CrudIdSpec {
            name: "id".into(),
            new: "inc".into(),
        }
    }
}

/// CRUD behavior configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct CrudSpec {
    pub id: CrudIdSpec,
    pub data: Vec<Value>,
}

/// Endpoint-level behavior policies (parsed from `serve:` block).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BehaviorSpec {
    pub concurrency: Option<ConcurrencySpec>,
    pub rate_limit: Option<RateLimitSpec>,
    pub timeout: Option<Range<Duration>>,
}

/// Parse a `BehaviorSpec` from a `serde_json::Value`.
pub fn parse_behavior(v: &Value) -> Result<BehaviorSpec, ParseError> {
    let obj = v
        .as_object()
        .ok_or_else(|| ParseError::new("behavior must be an object"))?;

    let concurrency = parse_conn(obj)?;
    let rate_limit = parse_rps(obj)?;
    let timeout = parse_timeout(obj)?;

    Ok(BehaviorSpec {
        concurrency,
        rate_limit,
        timeout,
    })
}

fn parse_conn(obj: &Map<String, Value>) -> Result<Option<ConcurrencySpec>, ParseError> {
    let val = match obj.get("conn") {
        None => return Ok(None),
        Some(v) => v,
    };

    let c_obj = val
        .as_object()
        .ok_or_else(|| ParseError::new("conn must be an object"))?;

    let max = c_obj
        .get("max")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ParseError::new("conn requires 'max' as a positive integer"))?;

    if max == 0 {
        return Err(ParseError::new("conn max must be > 0"));
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
        .ok_or_else(|| ParseError::new("concurrency requires 'over' field"))?;

    // "block" string
    if let Some(s) = over_val.as_str() {
        if s == "block" {
            return Ok(OverflowAction::Block);
        }
        return Err(ParseError::new(format!(
            "concurrency.over string must be 'block', got '{s}'"
        )));
    }

    // Object: either a reply {s: ...} or {block: ..., then: ...}
    let over_obj = over_val
        .as_object()
        .ok_or_else(|| ParseError::new("concurrency.over must be a string or object"))?;

    if over_obj.contains_key("block") {
        // BlockWithTimeout
        let block_str = over_obj
            .get("block")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ParseError::new("block timeout must be a string"))?;
        let timeout = parse_duration_range(block_str)?;

        let then_val = over_obj
            .get("then")
            .ok_or_else(|| ParseError::new("block requires 'then' reply"))?;
        let then = parse_reply(then_val, None)?;

        Ok(OverflowAction::BlockWithTimeout { timeout, then })
    } else {
        // Reply
        Ok(OverflowAction::Reply(parse_reply(over_val, None)?))
    }
}

fn parse_rps(obj: &Map<String, Value>) -> Result<Option<RateLimitSpec>, ParseError> {
    let val = match obj.get("rps") {
        None => return Ok(None),
        Some(v) => v,
    };

    let rl_obj = val
        .as_object()
        .ok_or_else(|| ParseError::new("rps must be an object"))?;

    let max = rl_obj
        .get("max")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ParseError::new("rps requires 'max' as a positive integer"))?;

    let over_val = rl_obj
        .get("over")
        .ok_or_else(|| ParseError::new("rps requires 'over' reply"))?;
    let over = parse_reply(over_val, None)?;

    Ok(Some(RateLimitSpec {
        rps: max as u32,
        over,
    }))
}

fn parse_timeout(obj: &Map<String, Value>) -> Result<Option<Range<Duration>>, ParseError> {
    match obj.get("timeout") {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(parse_duration_range(s)?)),
        Some(v) => Err(ParseError::new(format!("timeout must be a string, got {v}"))),
    }
}

/// Parse a `CrudSpec` from an object (the value inside `crud!:` or `crud:`).
pub fn parse_crud_spec(crud_obj: &Map<String, Value>) -> Result<CrudSpec, ParseError> {
    let id = match crud_obj.get("id") {
        None => CrudIdSpec::default(),
        Some(id_val) => {
            let id_obj = id_val
                .as_object()
                .ok_or_else(|| ParseError::new("crud.id must be an object"))?;

            let name = id_obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("id")
                .to_string();

            let new = id_obj
                .get("new")
                .and_then(|v| v.as_str())
                .unwrap_or("inc")
                .to_string();

            CrudIdSpec { name, new }
        }
    };

    let data = match crud_obj.get("data") {
        None => Vec::new(),
        Some(Value::Array(arr)) => arr.clone(),
        Some(v) => return Err(ParseError::new(format!("crud.data must be an array, got {v}"))),
    };

    Ok(CrudSpec { id, data })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- Concurrency ---

    #[test]
    fn parse_concurrency_reject() {
        let spec = parse_behavior(&json!({
            "conn": {"max": 5, "over": {"s": 429}}
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
            "conn": {"max": 5, "over": "block"}
        }))
        .unwrap();
        assert_eq!(spec.concurrency.unwrap().over, OverflowAction::Block);
    }

    #[test]
    fn parse_concurrency_block_timeout() {
        let spec = parse_behavior(&json!({
            "conn": {"max": 5, "over": {"block": "3s", "then": {"s": 429, "b": "timeout"}}}
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
            "conn": {"max": 0, "over": "block"}
        }))
        .is_err());
    }

    #[test]
    fn parse_concurrency_missing_over_error() {
        assert!(parse_behavior(&json!({
            "conn": {"max": 5}
        }))
        .is_err());
    }

    // --- Rate limit ---

    #[test]
    fn parse_rate_limit() {
        let spec = parse_behavior(&json!({
            "rps": {"max": 100, "over": {"s": 429}}
        }))
        .unwrap();
        let rl = spec.rate_limit.unwrap();
        assert_eq!(rl.rps, 100);
        assert_eq!(rl.over.status, 429);
    }

    #[test]
    fn parse_rps_missing_max_error() {
        assert!(parse_behavior(&json!({
            "rps": {"over": {"s": 429}}
        }))
        .is_err());
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
            "conn": {"max": 5, "over": "block"},
            "timeout": "30s"
        }))
        .unwrap();
        assert!(spec.concurrency.is_some());
        assert!(spec.timeout.is_some());
    }
}
