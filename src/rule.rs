use crate::chaos::{ChaosEntry, parse_chaos};
use crate::match_rule::{MatchRule, parse_match_rule};
use crate::reply::{ReplyStrategy, parse_reply_strategy};
use crate::serve::{BehaviorSpec, DeliverySpec, parse_serve};
use crate::suggest::{suggest_rule_key, format_suggestion};
use crate::units::ParseError;
use serde_json::Value;

/// A complete rule definition: match + reply + serve + chaos.
#[derive(Debug, Clone)]
pub struct Rule {
    /// Which requests this rule matches.
    pub match_rule: MatchRule,
    /// How to produce replies (static, sequence, or CRUD).
    pub reply: ReplyStrategy,
    /// How to shape the response on the wire (delivery subset of serve).
    pub delivery: DeliverySpec,
    /// Endpoint-level policies (behavior subset of serve).
    pub behavior: BehaviorSpec,
    /// Probabilistic overrides.
    pub chaos: Option<Vec<ChaosEntry>>,
    /// Original parsed value (for GET /_mx serialization).
    pub source: Value,
}

/// Parse a single rule from a `serde_json::Value` object.
pub fn parse_rule(v: &Value) -> Result<Rule, ParseError> {
    let obj = v
        .as_object()
        .ok_or_else(|| ParseError::new("rule must be an object"))?;

    // Check for unknown keys
    let known = ["match", "reply", "serve", "chaos"];
    for key in obj.keys() {
        if !known.contains(&key.as_str()) {
            if let Some(suggestion) = suggest_rule_key(key) {
                return Err(ParseError::new(format_suggestion(key, "rule", &suggestion)));
            }
            return Err(ParseError::new(format!("unknown key '{key}' in rule")));
        }
    }

    let match_val = obj
        .get("match")
        .ok_or_else(|| ParseError::new("rule requires 'match' field"))?;
    let match_rule = parse_match_rule(match_val).map_err(|e| e.in_field("match"))?;

    let reply_val = obj
        .get("reply")
        .ok_or_else(|| ParseError::new("missing 'reply' — every rule needs a reply (static, sequence, or crud!)"))?;
    let reply = parse_reply_strategy(reply_val).map_err(|e| e.in_field("reply"))?;

    let (delivery, behavior) = match obj.get("serve") {
        Some(serve_val) => parse_serve(serve_val).map_err(|e| e.in_field("serve"))?,
        None => (DeliverySpec::default(), BehaviorSpec::default()),
    };

    let chaos = match obj.get("chaos") {
        None => None,
        Some(v) => Some(parse_chaos(v).map_err(|e| e.in_field("chaos"))?),
    };

    Ok(Rule {
        match_rule,
        reply,
        delivery,
        behavior,
        chaos,
        source: v.clone(),
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
                        .map_err(|e| e.in_index("rule", i))?,
                );
            }
            Ok(rules)
        }
        Value::Object(_) => Ok(vec![parse_rule(v)?]),
        _ => Err(ParseError::new("rules must be an object or array")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reply::{BodySpec, ReplySpec};
    use crate::serve::DropSpec;
    use crate::match_rule::MatchRule;
    use crate::units::{ByteSize, Range};
    use serde_json::json;

    fn unwrap_static(strategy: &ReplyStrategy) -> &ReplySpec {
        match strategy {
            ReplyStrategy::Static(r) => r,
            other => panic!("expected Static, got {other:?}"),
        }
    }

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
        assert_eq!(unwrap_static(&rule.reply).status, 200);
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
        assert!(rule.delivery.first_byte.is_some());
        assert!(rule.delivery.pace.is_some());
        assert!(rule.behavior.concurrency.is_some());
    }

    #[test]
    fn parse_rule_with_serve_delivery_only() {
        let rule = parse_rule(&json!({
            "match": {"_": "/download"},
            "reply": {"s": 200, "b": {"rand!": {"size": "10mb", "seed": 42}}},
            "serve": {"pace": "10kb/s", "drop": "2kb"}
        }))
        .unwrap();
        let r = unwrap_static(&rule.reply);
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
    fn parse_rule_with_sequence_reply() {
        let rule = parse_rule(&json!({
            "match": {"_": "/auth"},
            "reply": [
                {"s": 401, "b": "unauthorized"},
                {"s": 200, "b": "ok"}
            ]
        }))
        .unwrap();
        match &rule.reply {
            ReplyStrategy::Sequence(replies) => {
                assert_eq!(replies.len(), 2);
                assert_eq!(replies[0].status, 401);
                assert_eq!(replies[1].status, 200);
            }
            other => panic!("expected Sequence, got {other:?}"),
        }
    }

    #[test]
    fn parse_rule_with_crud_true() {
        let rule = parse_rule(&json!({
            "match": {"_": "/items"},
            "reply": {"crud!": true}
        }))
        .unwrap();
        match &rule.reply {
            ReplyStrategy::Crud { spec, .. } => {
                assert!(spec.data.is_empty());
                assert_eq!(spec.id.name, "id");
                assert_eq!(spec.id.new, "inc");
            }
            other => panic!("expected Crud, got {other:?}"),
        }
    }

    #[test]
    fn parse_rule_with_crud_reply() {
        let rule = parse_rule(&json!({
            "match": {"_": "/toys"},
            "reply": {"crud!": {"data": [
                {"id": 1, "name": "Ball"},
                {"id": 3, "name": "Owl"}
            ]}}
        }))
        .unwrap();
        match &rule.reply {
            ReplyStrategy::Crud { spec, .. } => {
                assert_eq!(spec.data.len(), 2);
            }
            other => panic!("expected Crud, got {other:?}"),
        }
    }

    #[test]
    fn parse_rule_error_no_reply() {
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
            {"match": {"_": "/c"}, "reply": {"s": 200, "b": "c"}, "serve": {"pace": "5s"}}
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
        let msg = format!("{err}");
        assert!(msg.contains("rule[1]"), "error: {msg}");
    }

    #[test]
    fn parse_from_yaml_string() {
        let yaml = r#"
match: {g: /toys/3}
reply: {s: 200, h: {ct!: j!}, b: {name: Owl, price: 5.99}}
"#;
        let val = yttp::parse(yaml).unwrap();
        let rule = parse_rule(&val).unwrap();
        let r = unwrap_static(&rule.reply);
        assert_eq!(r.status, 200);
        assert_eq!(r.headers["Content-Type"], "application/json");
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

    #[test]
    fn parse_rule_error_legacy_behavior_rejected() {
        // behavior: key is no longer accepted
        assert!(parse_rule(&json!({
            "match": {"_": "/path"},
            "behavior": {"fail": {"rate": 0.5, "reply": {"s": 500}}}
        }))
        .is_err());
    }
}
