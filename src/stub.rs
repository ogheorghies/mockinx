use crate::behavior::{BehaviorSpec, parse_behavior};
use crate::delivery::{DeliverySpec, parse_delivery};
use crate::match_rule::{MatchRule, parse_match_rule};
use crate::reply::{ReplySpec, parse_reply};
use crate::units::ParseError;
use serde_json::Value;

/// A complete stub definition: match + reply + delivery + behavior.
#[derive(Debug, Clone)]
pub struct Stub {
    /// Which requests this stub matches.
    pub match_rule: MatchRule,
    /// Response to send (optional if behavior provides replies).
    pub reply: Option<ReplySpec>,
    /// How to shape the response on the wire.
    pub delivery: DeliverySpec,
    /// Endpoint-level policies.
    pub behavior: BehaviorSpec,
}

/// Parse a single `Stub` from a `serde_json::Value` object.
pub fn parse_stub(v: &Value) -> Result<Stub, ParseError> {
    let obj = v
        .as_object()
        .ok_or_else(|| ParseError("stub must be an object".into()))?;

    let match_val = obj
        .get("match")
        .ok_or_else(|| ParseError("stub requires 'match' field".into()))?;
    let match_rule = parse_match_rule(match_val)?;

    let reply = match obj.get("reply") {
        None => None,
        Some(r) => Some(parse_reply(r)?),
    };

    let delivery = match obj.get("delivery") {
        None => DeliverySpec::default(),
        Some(d) => parse_delivery(d)?,
    };

    let behavior = match obj.get("behavior") {
        None => BehaviorSpec::default(),
        Some(b) => parse_behavior(b)?,
    };

    // Validate: must have some way to produce a response
    let has_reply = reply.is_some();
    let has_sequence = behavior.sequence.is_some();
    let has_crud = behavior.crud.is_some();

    if !has_reply && !has_sequence && !has_crud {
        return Err(ParseError(
            "stub must have 'reply', behavior.sequence, or behavior.crud".into(),
        ));
    }

    Ok(Stub {
        match_rule,
        reply,
        delivery,
        behavior,
    })
}

/// Parse one or more stubs from a `serde_json::Value`.
///
/// Accepts either a single object (returns vec of one) or an array of objects.
pub fn parse_stubs(v: &Value) -> Result<Vec<Stub>, ParseError> {
    match v {
        Value::Array(arr) => {
            let mut stubs = Vec::with_capacity(arr.len());
            for (i, item) in arr.iter().enumerate() {
                stubs.push(
                    parse_stub(item)
                        .map_err(|e| ParseError(format!("stub[{i}]: {e}")))?,
                );
            }
            Ok(stubs)
        }
        Value::Object(_) => Ok(vec![parse_stub(v)?]),
        _ => Err(ParseError("stubs must be an object or array".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::SequenceScope;
    use crate::delivery::DropSpec;
    use crate::match_rule::MatchRule;
    use crate::reply::BodySpec;
    use crate::units::{ByteSize, Range};
    use serde_json::json;

    #[test]
    fn parse_minimal_stub() {
        let stub = parse_stub(&json!({
            "match": {"g": "/path"},
            "reply": {"s": 200}
        }))
        .unwrap();
        assert_eq!(
            stub.match_rule,
            MatchRule::MethodPath {
                method: Some("GET".into()),
                path: "/path".into()
            }
        );
        assert_eq!(stub.reply.unwrap().status, 200);
        assert_eq!(stub.delivery, DeliverySpec::default());
        assert_eq!(stub.behavior, BehaviorSpec::default());
    }

    #[test]
    fn parse_full_stub() {
        let stub = parse_stub(&json!({
            "match": {"g": "/api/data"},
            "reply": {"s": 200, "h": {"ct!": "j!"}, "b": {"items": [1, 2, 3]}},
            "delivery": {"first_byte": {"delay": "2s"}, "duration": "5s"},
            "behavior": {"concurrency": {"max": 5, "over": {"block": "3s", "then": {"s": 429}}}}
        }))
        .unwrap();
        assert!(stub.reply.is_some());
        assert!(stub.delivery.first_byte.is_some());
        assert!(stub.delivery.duration.is_some());
        assert!(stub.behavior.concurrency.is_some());
    }

    #[test]
    fn parse_stub_with_delivery_no_behavior() {
        let stub = parse_stub(&json!({
            "match": {"_": "/download"},
            "reply": {"s": 200, "b": {"rand": {"size": "10mb", "seed": 42}}},
            "delivery": {"speed": "10kb/s", "drop": {"after": "2kb"}}
        }))
        .unwrap();
        match &stub.reply.unwrap().body {
            BodySpec::Rand { size, seed } => {
                assert_eq!(size.bytes(), 10 * 1024 * 1024);
                assert_eq!(*seed, 42);
            }
            other => panic!("expected Rand, got {other:?}"),
        }
        match &stub.delivery.drop {
            Some(DropSpec::AfterBytes(Range::Fixed(bs))) => assert_eq!(bs.bytes(), 2048),
            other => panic!("expected AfterBytes, got {other:?}"),
        }
    }

    #[test]
    fn parse_stub_with_sequence_no_reply() {
        let stub = parse_stub(&json!({
            "match": {"_": "/auth"},
            "behavior": {
                "sequence": {
                    "per": "stub",
                    "replies": [
                        {"s": 401, "b": "unauthorized"},
                        {"s": 200, "b": "ok"}
                    ]
                }
            }
        }))
        .unwrap();
        assert!(stub.reply.is_none());
        let seq = stub.behavior.sequence.unwrap();
        assert_eq!(seq.per, SequenceScope::Stub);
        assert_eq!(seq.replies.len(), 2);
    }

    #[test]
    fn parse_stub_with_crud_no_reply() {
        let stub = parse_stub(&json!({
            "match": {"_": "/toys"},
            "behavior": {
                "crud": {
                    "seed": [
                        {"id": 1, "name": "Ball"},
                        {"id": 3, "name": "Owl"}
                    ]
                }
            }
        }))
        .unwrap();
        assert!(stub.reply.is_none());
        assert!(stub.behavior.crud.is_some());
    }

    #[test]
    fn parse_stub_error_no_response_source() {
        assert!(parse_stub(&json!({
            "match": {"g": "/path"}
        }))
        .is_err());
    }

    #[test]
    fn parse_stub_error_no_match() {
        assert!(parse_stub(&json!({
            "reply": {"s": 200}
        }))
        .is_err());
    }

    #[test]
    fn parse_stub_error_invalid_match() {
        assert!(parse_stub(&json!({
            "match": 42,
            "reply": {"s": 200}
        }))
        .is_err());
    }

    #[test]
    fn parse_stubs_single() {
        let stubs = parse_stubs(&json!({
            "match": {"g": "/path"},
            "reply": {"s": 200}
        }))
        .unwrap();
        assert_eq!(stubs.len(), 1);
    }

    #[test]
    fn parse_stubs_array() {
        let stubs = parse_stubs(&json!([
            {"match": {"_": "/a"}, "reply": {"s": 200, "b": "a"}},
            {"match": {"_": "/b"}, "reply": {"s": 404}},
            {"match": {"_": "/c"}, "reply": {"s": 200, "b": "c"}, "delivery": {"duration": "5s"}}
        ]))
        .unwrap();
        assert_eq!(stubs.len(), 3);
    }

    #[test]
    fn parse_stubs_array_error_includes_index() {
        let result = parse_stubs(&json!([
            {"match": {"g": "/ok"}, "reply": {"s": 200}},
            {"match": {"g": "/bad"}}
        ]));
        let err = result.unwrap_err();
        assert!(err.0.contains("stub[1]"), "error: {}", err.0);
    }

    #[test]
    fn parse_stubs_not_object_or_array() {
        assert!(parse_stubs(&json!("bad")).is_err());
    }

    #[test]
    fn parse_from_yaml_string() {
        let yaml = r#"
match: {g: /toys/3}
reply: {s: 200, h: {ct!: j!}, b: {name: Owl, price: 5.99}}
"#;
        let val = yttp::parse(yaml).unwrap();
        let stub = parse_stub(&val).unwrap();
        assert_eq!(stub.reply.as_ref().unwrap().status, 200);
        assert_eq!(
            stub.reply.as_ref().unwrap().headers["Content-Type"],
            "application/json"
        );
    }

    #[test]
    fn parse_readme_crud_example() {
        let yaml = r#"
match: {_: /toys}
reply: {h: {ct!: j!}}
delivery: {first_byte: {delay: 200ms}}
behavior:
  crud:
    seed:
      - {id: 1, name: Ball, price: 2.99}
      - {id: 3, name: Owl, price: 5.99}
"#;
        let val = yttp::parse(yaml).unwrap();
        let stub = parse_stub(&val).unwrap();
        assert!(stub.behavior.crud.is_some());
        assert!(stub.delivery.first_byte.is_some());
    }
}
