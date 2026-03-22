use crate::chaos::{ChaosEntry, parse_chaos};
use crate::match_rule::{MatchRule, parse_match_rule};
use crate::reply::{ReplySpec, ReplyStrategy, parse_reply, parse_reply_strategy};
use crate::serve::{BehaviorSpec, DeliverySpec, parse_behavior, parse_delivery_fields, parse_serve};
use crate::units::ParseError;
use serde_json::Value;

/// A complete rule definition: match + reply + serve + chaos.
#[derive(Debug, Clone)]
pub struct Rule {
    /// Which requests this rule matches.
    pub match_rule: MatchRule,
    /// How to produce replies (static, sequence, or CRUD).
    pub reply: Option<ReplyStrategy>,
    /// How to shape the response on the wire (delivery subset of serve).
    pub delivery: DeliverySpec,
    /// Endpoint-level policies (behavior subset of serve).
    pub behavior: BehaviorSpec,
    /// Probabilistic overrides.
    pub chaos: Option<Vec<ChaosEntry>>,
}

/// Parse a single rule from a `serde_json::Value` object.
///
/// Accepts `serve:` (new, merged delivery + behavior) or legacy
/// `delivery:` + `behavior:` (separate).
pub fn parse_rule(v: &Value) -> Result<Rule, ParseError> {
    let obj = v
        .as_object()
        .ok_or_else(|| ParseError("rule must be an object".into()))?;

    let match_val = obj
        .get("match")
        .ok_or_else(|| ParseError("rule requires 'match' field".into()))?;
    let match_rule = parse_match_rule(match_val)?;

    let reply = match obj.get("reply") {
        None => None,
        Some(r) => Some(parse_reply_strategy(r)?),
    };

    // Parse serve: (merged) or legacy delivery: + behavior:
    let (delivery, behavior) = if let Some(serve_val) = obj.get("serve") {
        parse_serve(serve_val)?
    } else {
        let delivery = match obj.get("delivery") {
            None => DeliverySpec::default(),
            Some(d) => {
                let d_obj = d.as_object()
                    .ok_or_else(|| ParseError("delivery must be an object".into()))?;
                parse_delivery_fields(d_obj)?
            }
        };
        let behavior = match obj.get("behavior") {
            None => BehaviorSpec::default(),
            Some(b) => parse_behavior(b)?,
        };
        (delivery, behavior)
    };

    // Validate: must have some way to produce a response
    let has_reply = reply.is_some();
    // Legacy: behavior.sequence and behavior.crud still work
    let has_legacy_sequence = behavior.sequence.is_some();
    let has_legacy_crud = behavior.crud.is_some();

    if !has_reply && !has_legacy_sequence && !has_legacy_crud {
        return Err(ParseError(
            "rule must have 'reply' (static, sequence, or crud!)".into(),
        ));
    }

    let chaos = match obj.get("chaos") {
        None => None,
        Some(v) => Some(parse_chaos(v)?),
    };

    Ok(Rule {
        match_rule,
        reply,
        delivery,
        behavior,
        chaos,
    })
}

/// Parse one or more rules from a `serde_json::Value`.
///
/// Accepts either a single object (returns vec of one) or an array of objects.
pub fn parse_rules(v: &Value) -> Result<Vec<Rule>, ParseError> {
    match v {
        Value::Array(arr) => {
            let mut rules = Vec::with_capacity(arr.len());
            for (i, item) in arr.iter().enumerate() {
                rules.push(
                    parse_rule(item)
                        .map_err(|e| ParseError(format!("rule[{i}]: {e}")))?,
                );
            }
            Ok(rules)
        }
        Value::Object(_) => Ok(vec![parse_rule(v)?]),
        _ => Err(ParseError("rules must be an object or array".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::SequenceScope;
    use crate::serve::DropSpec;

    fn unwrap_static(strategy: &ReplyStrategy) -> &ReplySpec {
        match strategy {
            ReplyStrategy::Static(r) => r,
            other => panic!("expected Static, got {other:?}"),
        }
    }
    use crate::match_rule::MatchRule;
    use crate::reply::BodySpec;
    use crate::units::{ByteSize, Range};
    use serde_json::json;

    #[test]
    fn parse_minimal_rule() {
        let rule = parse_rule(&json!({
            "match": {"g": "/path"},
            "reply": {"s": 200}
        }))
        .unwrap();
        assert_eq!(
            rule.match_rule,
            MatchRule::MethodPath {
                method: Some("GET".into()),
                path: "/path".into()
            }
        );
        assert_eq!(unwrap_static(rule.reply.as_ref().unwrap()).status, 200);
        assert_eq!(rule.delivery, DeliverySpec::default());
        assert_eq!(rule.behavior, BehaviorSpec::default());
    }

    #[test]
    fn parse_rule_with_serve() {
        let rule = parse_rule(&json!({
            "match": {"g": "/api/data"},
            "reply": {"s": 200, "b": {"items": [1, 2, 3]}},
            "serve": {"first_byte": "2s", "pace": "5s", "conn": {"max": 5, "over": {"block": "3s", "then": {"s": 429}}}}
        }))
        .unwrap();
        assert!(rule.reply.is_some());
        assert!(rule.delivery.first_byte.is_some());
        assert!(rule.delivery.pace.is_some());
        assert!(rule.behavior.concurrency.is_some());
    }

    #[test]
    fn parse_rule_with_serve_delivery_only() {
        let rule = parse_rule(&json!({
            "match": {"_": "/download"},
            "reply": {"s": 200, "b": {"rand!": {"size": "10mb", "seed": 42}}},
            "serve": {"speed": "10kb/s", "drop": "2kb"}
        }))
        .unwrap();
        let r = unwrap_static(rule.reply.as_ref().unwrap());
        match &r.body {
            BodySpec::Rand { size, seed } => {
                assert_eq!(size.bytes(), 10 * 1024 * 1024);
                assert_eq!(*seed, 42);
            }
            other => panic!("expected Rand, got {other:?}"),
        }
        match &rule.delivery.drop {
            Some(DropSpec::AfterBytes(Range::Fixed(bs))) => assert_eq!(bs.bytes(), 2048),
            other => panic!("expected AfterBytes, got {other:?}"),
        }
    }

    #[test]
    fn parse_rule_with_sequence_no_reply() {
        let rule = parse_rule(&json!({
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
        assert!(rule.reply.is_none());
        let seq = rule.behavior.sequence.unwrap();
        assert_eq!(seq.per, SequenceScope::Stub);
        assert_eq!(seq.replies.len(), 2);
    }

    #[test]
    fn parse_rule_with_crud_no_reply() {
        let rule = parse_rule(&json!({
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
        assert!(rule.reply.is_none());
        assert!(rule.behavior.crud.is_some());
    }

    #[test]
    fn parse_rule_error_no_response_source() {
        assert!(parse_rule(&json!({
            "match": {"g": "/path"}
        }))
        .is_err());
    }

    #[test]
    fn parse_rule_error_no_match() {
        assert!(parse_rule(&json!({
            "reply": {"s": 200}
        }))
        .is_err());
    }

    #[test]
    fn parse_rule_error_invalid_match() {
        assert!(parse_rule(&json!({
            "match": 42,
            "reply": {"s": 200}
        }))
        .is_err());
    }

    #[test]
    fn parse_rules_single() {
        let rules = parse_rules(&json!({
            "match": {"g": "/path"},
            "reply": {"s": 200}
        }))
        .unwrap();
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn parse_rules_array() {
        let rules = parse_rules(&json!([
            {"match": {"_": "/a"}, "reply": {"s": 200, "b": "a"}},
            {"match": {"_": "/b"}, "reply": {"s": 404}},
            {"match": {"_": "/c"}, "reply": {"s": 200, "b": "c"}, "serve": {"span": "5s"}}
        ]))
        .unwrap();
        assert_eq!(rules.len(), 3);
    }

    #[test]
    fn parse_rules_array_error_includes_index() {
        let result = parse_rules(&json!([
            {"match": {"g": "/ok"}, "reply": {"s": 200}},
            {"match": {"g": "/bad"}}
        ]));
        let err = result.unwrap_err();
        assert!(err.0.contains("rule[1]"), "error: {}", err.0);
    }

    #[test]
    fn parse_rules_not_object_or_array() {
        assert!(parse_rules(&json!("bad")).is_err());
    }

    #[test]
    fn parse_from_yaml_string() {
        let yaml = r#"
match: {g: /toys/3}
reply: {s: 200, h: {ct!: j!}, b: {name: Owl, price: 5.99}}
"#;
        let val = yttp::parse(yaml).unwrap();
        let rule = parse_rule(&val).unwrap();
        let r = unwrap_static(rule.reply.as_ref().unwrap());
        assert_eq!(r.status, 200);
        assert_eq!(r.headers["Content-Type"], "application/json");
    }

    #[test]
    fn parse_readme_crud_example_legacy() {
        // Legacy format still works
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
        let rule = parse_rule(&val).unwrap();
        assert!(rule.behavior.crud.is_some());
        assert!(rule.delivery.first_byte.is_some());
    }

    #[test]
    fn parse_serve_with_conn_and_rps() {
        let rule = parse_rule(&json!({
            "match": {"_": "/api"},
            "reply": {"s": 200},
            "serve": {
                "conn": {"max": 5, "over": {"s": 429}},
                "rps": {"max": 100, "over": {"s": 429}},
                "timeout": "30s"
            }
        }))
        .unwrap();
        assert!(rule.behavior.concurrency.is_some());
        assert!(rule.behavior.rate_limit.is_some());
        assert!(rule.behavior.timeout.is_some());
    }
}
