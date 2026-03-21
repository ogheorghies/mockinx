use crate::behavior::CrudSpec;
use crate::units::{ByteSize, ParseError, parse_byte_size};
use serde_json::{Map, Value};

/// Specification for the body of a reply.
#[derive(Debug, Clone, PartialEq)]
pub enum BodySpec {
    /// No body.
    None,
    /// Literal value (string or JSON), serialized to bytes at response time.
    Literal(Value),
    /// Deterministic pseudo-random bytes.
    Rand { size: ByteSize, seed: u64 },
    /// Repeated pattern string, truncated to size.
    Pattern { repeat: String, size: ByteSize },
}

/// Specification for an HTTP reply: status, headers, body.
/// Uses yttp `{s: h: b:}` convention.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplySpec {
    /// HTTP status code (default 200).
    pub status: u16,
    /// Response headers (after shortcut expansion).
    pub headers: Map<String, Value>,
    /// Body specification.
    pub body: BodySpec,
}

impl Default for ReplySpec {
    fn default() -> Self {
        ReplySpec {
            status: 200,
            headers: Map::new(),
            body: BodySpec::None,
        }
    }
}

/// How to produce replies — static, sequence, or CRUD.
#[derive(Debug, Clone)]
pub enum ReplyStrategy {
    /// Single static reply.
    Static(ReplySpec),
    /// Sequence of replies, cycled in order (per-rule counter for now).
    Sequence(Vec<ReplySpec>),
    /// In-memory CRUD resource.
    Crud {
        spec: CrudSpec,
        /// Default headers from the reply context (e.g., Content-Type).
        headers: Map<String, Value>,
    },
}

/// Parse a `ReplyStrategy` from a `serde_json::Value`.
///
/// Polymorphic:
/// - Array → Sequence
/// - Object with `crud!` → Crud
/// - Object with s/h/b → Static
pub fn parse_reply_strategy(v: &Value) -> Result<ReplyStrategy, ParseError> {
    match v {
        Value::Array(arr) => {
            if arr.is_empty() {
                return Err(ParseError("reply sequence cannot be empty".into()));
            }
            let mut replies = Vec::with_capacity(arr.len());
            for item in arr {
                replies.push(parse_reply(item)?);
            }
            Ok(ReplyStrategy::Sequence(replies))
        }
        Value::Object(obj) => {
            if let Some(crud_val) = obj.get("crud!") {
                let crud_obj = crud_val
                    .as_object()
                    .ok_or_else(|| ParseError("crud! must be an object".into()))?;
                let spec = crate::behavior::parse_crud_spec(crud_obj)?;
                // Extract headers from h: field if present
                let mut headers = obj
                    .get("h")
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                yttp::expand_headers(&mut headers);
                Ok(ReplyStrategy::Crud { spec, headers })
            } else {
                Ok(ReplyStrategy::Static(parse_reply(v)?))
            }
        }
        _ => Err(ParseError(format!("reply must be an object or array, got {v}"))),
    }
}

/// Parse a `ReplySpec` from a `serde_json::Value`.
///
/// Expects an object with optional keys `s` (status), `h` (headers), `b` (body).
/// Missing fields use defaults: status=200, headers=empty, body=None.
pub fn parse_reply(v: &Value) -> Result<ReplySpec, ParseError> {
    let obj = v
        .as_object()
        .ok_or_else(|| ParseError("reply must be an object".into()))?;

    let status = parse_status(obj)?;
    let headers = parse_headers(obj)?;
    let body = parse_body(obj)?;

    Ok(ReplySpec {
        status,
        headers,
        body,
    })
}

fn parse_status(obj: &Map<String, Value>) -> Result<u16, ParseError> {
    match obj.get("s") {
        None => Ok(200),
        Some(Value::Number(n)) => {
            let code = n
                .as_u64()
                .ok_or_else(|| ParseError(format!("status must be a positive integer, got {n}")))?;
            if code > 999 {
                return Err(ParseError(format!("status code {code} out of range (0-999)")));
            }
            Ok(code as u16)
        }
        Some(v) => Err(ParseError(format!("status must be a number, got {v}"))),
    }
}

fn parse_headers(obj: &Map<String, Value>) -> Result<Map<String, Value>, ParseError> {
    match obj.get("h") {
        None => Ok(Map::new()),
        Some(Value::Object(h)) => {
            let mut headers = h.clone();
            yttp::expand_headers(&mut headers);
            Ok(headers)
        }
        Some(v) => Err(ParseError(format!("headers must be an object, got {v}"))),
    }
}

fn parse_body(obj: &Map<String, Value>) -> Result<BodySpec, ParseError> {
    match obj.get("b") {
        None => Ok(BodySpec::None),
        Some(Value::Object(b)) => {
            // Directives use ! suffix to distinguish from literal data
            if b.contains_key("rand!") {
                parse_rand_body(b, "rand!")
            } else if b.contains_key("pattern!") {
                parse_pattern_body(b, "pattern!")
            } else {
                // Regular JSON object literal (no ! = literal data)
                Ok(BodySpec::Literal(Value::Object(b.clone())))
            }
        }
        Some(v) => Ok(BodySpec::Literal(v.clone())),
    }
}

fn parse_rand_body(obj: &Map<String, Value>, key: &str) -> Result<BodySpec, ParseError> {
    let rand_obj = obj
        .get(key)
        .and_then(|v| v.as_object())
        .ok_or_else(|| ParseError("rand must be an object".into()))?;

    let size_val = rand_obj
        .get("size")
        .ok_or_else(|| ParseError("rand requires 'size' field".into()))?;
    let size_str = size_val
        .as_str()
        .ok_or_else(|| ParseError("rand size must be a string".into()))?;
    let size = parse_byte_size(size_str)?;

    let seed = rand_obj
        .get("seed")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ParseError("rand requires 'seed' as a positive integer".into()))?;

    Ok(BodySpec::Rand { size, seed })
}

fn parse_pattern_body(obj: &Map<String, Value>, key: &str) -> Result<BodySpec, ParseError> {
    let pattern_obj = obj
        .get(key)
        .and_then(|v| v.as_object())
        .ok_or_else(|| ParseError("pattern must be an object".into()))?;

    let repeat = pattern_obj
        .get("repeat")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ParseError("pattern requires 'repeat' string".into()))?;

    if repeat.is_empty() {
        return Err(ParseError("pattern repeat string cannot be empty".into()));
    }

    let size_val = pattern_obj
        .get("size")
        .ok_or_else(|| ParseError("pattern requires 'size' field".into()))?;
    let size_str = size_val
        .as_str()
        .ok_or_else(|| ParseError("pattern size must be a string".into()))?;
    let size = parse_byte_size(size_str)?;

    Ok(BodySpec::Pattern {
        repeat: repeat.to_string(),
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_full_reply() {
        let v = yttp::parse("{s: 200, h: {ct!: j!}, b: {name: Owl}}").unwrap();
        let reply = parse_reply(&v).unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.headers["Content-Type"], "application/json");
        match &reply.body {
            BodySpec::Literal(val) => assert_eq!(val["name"], "Owl"),
            other => panic!("expected Literal, got {other:?}"),
        }
    }

    #[test]
    fn parse_status_only() {
        let reply = parse_reply(&json!({"s": 204})).unwrap();
        assert_eq!(reply.status, 204);
        assert!(reply.headers.is_empty());
        assert_eq!(reply.body, BodySpec::None);
    }

    #[test]
    fn parse_default_status() {
        let reply = parse_reply(&json!({"b": "hello"})).unwrap();
        assert_eq!(reply.status, 200);
    }

    #[test]
    fn parse_string_body() {
        let reply = parse_reply(&json!({"s": 200, "b": "hello"})).unwrap();
        assert_eq!(reply.body, BodySpec::Literal(json!("hello")));
    }

    #[test]
    fn parse_rand_body() {
        let v = json!({"s": 200, "b": {"rand!": {"size": "10kb", "seed": 7}}});
        let reply = parse_reply(&v).unwrap();
        match &reply.body {
            BodySpec::Rand { size, seed } => {
                assert_eq!(size.bytes(), 10240);
                assert_eq!(*seed, 7);
            }
            other => panic!("expected Rand, got {other:?}"),
        }
    }

    #[test]
    fn parse_pattern_body() {
        let v = json!({"s": 200, "b": {"pattern!": {"repeat": "abc", "size": "1mb"}}});
        let reply = parse_reply(&v).unwrap();
        match &reply.body {
            BodySpec::Pattern { repeat, size } => {
                assert_eq!(repeat, "abc");
                assert_eq!(size.bytes(), 1048576);
            }
            other => panic!("expected Pattern, got {other:?}"),
        }
    }

    #[test]
    fn parse_header_shortcuts() {
        let v = yttp::parse("{s: 200, h: {ct!: t!}}").unwrap();
        let reply = parse_reply(&v).unwrap();
        assert_eq!(reply.headers["Content-Type"], "text/plain");
    }

    #[test]
    fn parse_malformed_content_type() {
        let v = yttp::parse("{s: 200, h: {ct!: h!}, b: '{\"valid\": \"json\"}'}").unwrap();
        let reply = parse_reply(&v).unwrap();
        assert_eq!(reply.headers["Content-Type"], "text/html");
        match &reply.body {
            BodySpec::Literal(val) => assert!(val.as_str().unwrap().contains("valid")),
            other => panic!("expected Literal, got {other:?}"),
        }
    }

    #[test]
    fn parse_minimal_overflow_reply() {
        let reply = parse_reply(&json!({"s": 429})).unwrap();
        assert_eq!(reply.status, 429);
        assert_eq!(reply.body, BodySpec::None);
    }

    #[test]
    fn parse_overflow_with_body() {
        let reply = parse_reply(&json!({"s": 429, "b": "too many"})).unwrap();
        assert_eq!(reply.status, 429);
        assert_eq!(reply.body, BodySpec::Literal(json!("too many")));
    }

    #[test]
    fn parse_error_status_too_large() {
        assert!(parse_reply(&json!({"s": 1000})).is_err());
    }

    #[test]
    fn parse_error_status_negative() {
        assert!(parse_reply(&json!({"s": -1})).is_err());
    }

    #[test]
    fn parse_error_status_not_number() {
        assert!(parse_reply(&json!({"s": "200"})).is_err());
    }

    #[test]
    fn parse_error_headers_not_object() {
        assert!(parse_reply(&json!({"h": "bad"})).is_err());
    }

    #[test]
    fn parse_error_not_object() {
        assert!(parse_reply(&json!("string")).is_err());
        assert!(parse_reply(&json!(42)).is_err());
    }

    #[test]
    fn parse_error_rand_missing_size() {
        let v = json!({"b": {"rand!": {"seed": 7}}});
        assert!(parse_reply(&v).is_err());
    }

    #[test]
    fn parse_error_rand_missing_seed() {
        let v = json!({"b": {"rand!": {"size": "10kb"}}});
        assert!(parse_reply(&v).is_err());
    }

    #[test]
    fn parse_error_pattern_empty_repeat() {
        let v = json!({"b": {"pattern!": {"repeat": "", "size": "1kb"}}});
        assert!(parse_reply(&v).is_err());
    }

    #[test]
    fn parse_error_pattern_missing_size() {
        let v = json!({"b": {"pattern!": {"repeat": "abc"}}});
        assert!(parse_reply(&v).is_err());
    }

    #[test]
    fn parse_json_object_body() {
        let reply = parse_reply(&json!({"s": 200, "b": {"items": [1, 2, 3]}})).unwrap();
        match &reply.body {
            BodySpec::Literal(val) => {
                assert_eq!(val["items"], json!([1, 2, 3]));
            }
            other => panic!("expected Literal, got {other:?}"),
        }
    }

    #[test]
    fn parse_number_body() {
        let reply = parse_reply(&json!({"b": 42})).unwrap();
        assert_eq!(reply.body, BodySpec::Literal(json!(42)));
    }

    #[test]
    fn parse_bool_body() {
        let reply = parse_reply(&json!({"b": true})).unwrap();
        assert_eq!(reply.body, BodySpec::Literal(json!(true)));
    }

    #[test]
    fn parse_array_body() {
        let reply = parse_reply(&json!({"b": [1, 2, 3]})).unwrap();
        assert_eq!(reply.body, BodySpec::Literal(json!([1, 2, 3])));
    }
}
