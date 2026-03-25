pub mod body;
pub mod crud;

use crate::serve::CrudSpec;
use crate::units::{ByteSize, ParseError, parse_byte_size};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Fields that can be reflected back in a reflect! response.
#[derive(Debug, Clone, PartialEq)]
pub enum ReflectField {
    Method,
    Headers,
    Url,
    Query,
    Body,
}

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
    /// Reflect request fields back as JSON.
    Reflect(Vec<ReflectField>),
    /// Read body from a file at response time.
    File(PathBuf),
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
pub fn parse_reply_strategy(v: &Value, base_dir: Option<&Path>) -> Result<ReplyStrategy, ParseError> {
    match v {
        Value::Array(arr) => {
            if arr.is_empty() {
                return Err(ParseError::new("reply sequence cannot be empty"));
            }
            let mut replies = Vec::with_capacity(arr.len());
            for item in arr {
                replies.push(parse_reply(item, base_dir)?);
            }
            Ok(ReplyStrategy::Sequence(replies))
        }
        Value::Object(obj) => {
            if let Some(crud_val) = obj.get("crud!") {
                // crud!: true → empty CRUD with defaults
                // crud!: {data: [...], id: {...}} → configured CRUD
                let spec = if crud_val.as_bool() == Some(true) {
                    crate::serve::CrudSpec {
                        id: crate::serve::CrudIdSpec::default(),
                        data: Vec::new(),
                    }
                } else {
                    let crud_obj = crud_val
                        .as_object()
                        .ok_or_else(|| ParseError::new("crud! must be true or an object"))?;
                    crate::serve::parse_crud_spec(crud_obj)?
                };
                // Extract headers from h: field if present
                let mut headers = obj
                    .get("h")
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                yttp::expand_headers(&mut headers);
                Ok(ReplyStrategy::Crud { spec, headers })
            } else {
                Ok(ReplyStrategy::Static(parse_reply(v, base_dir)?))
            }
        }
        _ => Err(ParseError::new(format!("reply must be an object or array, got {v}"))),
    }
}

/// Parse a `ReplySpec` from a `serde_json::Value`.
///
/// Expects an object with optional keys `s` (status), `h` (headers), `b` (body).
/// Missing fields use defaults: status=200, headers=empty, body=None.
pub fn parse_reply(v: &Value, base_dir: Option<&Path>) -> Result<ReplySpec, ParseError> {
    let obj = v
        .as_object()
        .ok_or_else(|| ParseError::new("reply must be an object"))?;

    let status = parse_status(obj)?;
    let headers = parse_headers(obj)?;
    let body = parse_body(obj, base_dir)?;

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
                .ok_or_else(|| ParseError::new(format!("status must be a positive integer, got {n}")))?;
            if code > 999 {
                return Err(ParseError::new(format!("status code {code} out of range (0-999)")));
            }
            Ok(code as u16)
        }
        Some(v) => Err(ParseError::new(format!("status must be a number, got {v}"))),
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
        Some(v) => Err(ParseError::new(format!("headers must be an object, got {v}"))),
    }
}

fn parse_body(obj: &Map<String, Value>, base_dir: Option<&Path>) -> Result<BodySpec, ParseError> {
    match obj.get("b") {
        None => Ok(BodySpec::None),
        Some(Value::Object(b)) => {
            // Directives use ! suffix to distinguish from literal data
            if b.contains_key("rand!") {
                parse_rand_body(b, "rand!")
            } else if b.contains_key("pattern!") {
                parse_pattern_body(b, "pattern!")
            } else if b.contains_key("reflect!") {
                parse_reflect_body(b)
            } else if b.contains_key("file!") {
                let path = b.get("file!").and_then(|v| v.as_str())
                    .ok_or_else(|| ParseError::new("file! must be a string path"))?;
                let path = PathBuf::from(path);
                let path = if path.is_relative() {
                    if let Some(base) = base_dir {
                        base.join(&path)
                    } else {
                        path
                    }
                } else {
                    path
                };
                Ok(BodySpec::File(path))
            } else {
                // Regular JSON object literal (no ! = literal data)
                Ok(BodySpec::Literal(Value::Object(b.clone())))
            }
        }
        Some(v) => Ok(BodySpec::Literal(v.clone())),
    }
}

fn parse_reflect_field(s: &str) -> Result<ReflectField, ParseError> {
    match s {
        "i.m" => Ok(ReflectField::Method),
        "i.h" => Ok(ReflectField::Headers),
        "i.u" => Ok(ReflectField::Url),
        "i.q" => Ok(ReflectField::Query),
        "i.b" => Ok(ReflectField::Body),
        _ => Err(ParseError::new(format!("unknown reflect field: {s} (expected i.m, i.h, i.u, i.q, i.b)"))),
    }
}

fn parse_reflect_body(obj: &Map<String, Value>) -> Result<BodySpec, ParseError> {
    let val = obj.get("reflect!").unwrap();
    match val {
        // reflect!: true → all except body
        Value::Bool(true) => Ok(BodySpec::Reflect(vec![
            ReflectField::Method,
            ReflectField::Headers,
            ReflectField::Url,
            ReflectField::Query,
        ])),
        // reflect!: [i.m, i.h, ...]
        Value::Array(arr) => {
            let fields: Result<Vec<_>, _> = arr.iter().map(|v| {
                let s = v.as_str().ok_or_else(|| ParseError::new("reflect! array elements must be strings"))?;
                parse_reflect_field(s)
            }).collect();
            let fields = fields?;
            if fields.is_empty() {
                return Err(ParseError::new("reflect! array cannot be empty"));
            }
            Ok(BodySpec::Reflect(fields))
        }
        _ => Err(ParseError::new("reflect! must be true or an array of field names")),
    }
}

fn parse_rand_body(obj: &Map<String, Value>, key: &str) -> Result<BodySpec, ParseError> {
    let rand_obj = obj
        .get(key)
        .and_then(|v| v.as_object())
        .ok_or_else(|| ParseError::new("rand must be an object"))?;

    let size_val = rand_obj
        .get("size")
        .ok_or_else(|| ParseError::new("rand requires 'size' field"))?;
    let size_str = size_val
        .as_str()
        .ok_or_else(|| ParseError::new("rand size must be a string"))?;
    let size = parse_byte_size(size_str)?;

    let seed = rand_obj
        .get("seed")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ParseError::new("rand requires 'seed' as a positive integer"))?;

    Ok(BodySpec::Rand { size, seed })
}

fn parse_pattern_body(obj: &Map<String, Value>, key: &str) -> Result<BodySpec, ParseError> {
    let pattern_obj = obj
        .get(key)
        .and_then(|v| v.as_object())
        .ok_or_else(|| ParseError::new("pattern must be an object"))?;

    let repeat = pattern_obj
        .get("repeat")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ParseError::new("pattern requires 'repeat' string"))?;

    if repeat.is_empty() {
        return Err(ParseError::new("pattern repeat string cannot be empty"));
    }

    let size_val = pattern_obj
        .get("size")
        .ok_or_else(|| ParseError::new("pattern requires 'size' field"))?;
    let size_str = size_val
        .as_str()
        .ok_or_else(|| ParseError::new("pattern size must be a string"))?;
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
        let reply = parse_reply(&v, None).unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.headers["Content-Type"], "application/json");
        match &reply.body {
            BodySpec::Literal(val) => assert_eq!(val["name"], "Owl"),
            other => panic!("expected Literal, got {other:?}"),
        }
    }

    #[test]
    fn parse_status_only() {
        let reply = parse_reply(&json!({"s": 204}), None).unwrap();
        assert_eq!(reply.status, 204);
        assert!(reply.headers.is_empty());
        assert_eq!(reply.body, BodySpec::None);
    }

    #[test]
    fn parse_default_status() {
        let reply = parse_reply(&json!({"b": "hello"}), None).unwrap();
        assert_eq!(reply.status, 200);
    }

    #[test]
    fn parse_string_body() {
        let reply = parse_reply(&json!({"s": 200, "b": "hello"}), None).unwrap();
        assert_eq!(reply.body, BodySpec::Literal(json!("hello")));
    }

    #[test]
    fn parse_rand_body() {
        let v = json!({"s": 200, "b": {"rand!": {"size": "10kb", "seed": 7}}});
        let reply = parse_reply(&v, None).unwrap();
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
        let reply = parse_reply(&v, None).unwrap();
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
        let reply = parse_reply(&v, None).unwrap();
        assert_eq!(reply.headers["Content-Type"], "text/plain");
    }

    #[test]
    fn parse_malformed_content_type() {
        let v = yttp::parse("{s: 200, h: {ct!: h!}, b: '{\"valid\": \"json\"}'}").unwrap();
        let reply = parse_reply(&v, None).unwrap();
        assert_eq!(reply.headers["Content-Type"], "text/html");
        match &reply.body {
            BodySpec::Literal(val) => assert!(val.as_str().unwrap().contains("valid")),
            other => panic!("expected Literal, got {other:?}"),
        }
    }

    #[test]
    fn parse_minimal_overflow_reply() {
        let reply = parse_reply(&json!({"s": 429}), None).unwrap();
        assert_eq!(reply.status, 429);
        assert_eq!(reply.body, BodySpec::None);
    }

    #[test]
    fn parse_overflow_with_body() {
        let reply = parse_reply(&json!({"s": 429, "b": "too many"}), None).unwrap();
        assert_eq!(reply.status, 429);
        assert_eq!(reply.body, BodySpec::Literal(json!("too many")));
    }

    #[test]
    fn parse_error_status_too_large() {
        assert!(parse_reply(&json!({"s": 1000}), None).is_err());
    }

    #[test]
    fn parse_error_status_negative() {
        assert!(parse_reply(&json!({"s": -1}), None).is_err());
    }

    #[test]
    fn parse_error_status_not_number() {
        assert!(parse_reply(&json!({"s": "200"}), None).is_err());
    }

    #[test]
    fn parse_error_headers_not_object() {
        assert!(parse_reply(&json!({"h": "bad"}), None).is_err());
    }

    #[test]
    fn parse_error_not_object() {
        assert!(parse_reply(&json!("string"), None).is_err());
        assert!(parse_reply(&json!(42), None).is_err());
    }

    #[test]
    fn parse_error_rand_missing_size() {
        let v = json!({"b": {"rand!": {"seed": 7}}});
        assert!(parse_reply(&v, None).is_err());
    }

    #[test]
    fn parse_error_rand_missing_seed() {
        let v = json!({"b": {"rand!": {"size": "10kb"}}});
        assert!(parse_reply(&v, None).is_err());
    }

    #[test]
    fn parse_error_pattern_empty_repeat() {
        let v = json!({"b": {"pattern!": {"repeat": "", "size": "1kb"}}});
        assert!(parse_reply(&v, None).is_err());
    }

    #[test]
    fn parse_error_pattern_missing_size() {
        let v = json!({"b": {"pattern!": {"repeat": "abc"}}});
        assert!(parse_reply(&v, None).is_err());
    }

    #[test]
    fn parse_json_object_body() {
        let reply = parse_reply(&json!({"s": 200, "b": {"items": [1, 2, 3]}}), None).unwrap();
        match &reply.body {
            BodySpec::Literal(val) => {
                assert_eq!(val["items"], json!([1, 2, 3]));
            }
            other => panic!("expected Literal, got {other:?}"),
        }
    }

    #[test]
    fn parse_number_body() {
        let reply = parse_reply(&json!({"b": 42}), None).unwrap();
        assert_eq!(reply.body, BodySpec::Literal(json!(42)));
    }

    #[test]
    fn parse_bool_body() {
        let reply = parse_reply(&json!({"b": true}), None).unwrap();
        assert_eq!(reply.body, BodySpec::Literal(json!(true)));
    }

    #[test]
    fn parse_array_body() {
        let reply = parse_reply(&json!({"b": [1, 2, 3]}), None).unwrap();
        assert_eq!(reply.body, BodySpec::Literal(json!([1, 2, 3])));
    }

    #[test]
    fn file_path_resolved_relative_to_base_dir() {
        let base = Path::new("/srv/configs");
        let reply = parse_reply(&json!({"b": {"file!": "data.json"}}), Some(base)).unwrap();
        assert_eq!(reply.body, BodySpec::File(PathBuf::from("/srv/configs/data.json")));
    }

    #[test]
    fn file_absolute_path_unchanged_with_base_dir() {
        let base = Path::new("/srv/configs");
        let reply = parse_reply(&json!({"b": {"file!": "/tmp/data.json"}}), Some(base)).unwrap();
        assert_eq!(reply.body, BodySpec::File(PathBuf::from("/tmp/data.json")));
    }

    #[test]
    fn file_relative_path_without_base_dir() {
        let reply = parse_reply(&json!({"b": {"file!": "data.json"}}), None).unwrap();
        assert_eq!(reply.body, BodySpec::File(PathBuf::from("data.json")));
    }
}
