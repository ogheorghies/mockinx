use crate::rule::Rule;
use crate::serve::PaceSpec;

/// A non-fatal warning about a rule's configuration.
#[derive(Debug, Clone)]
pub struct Warning {
    pub msg: String,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "⚠ {}", self.msg)
    }
}

/// Validate a rule for logical consistency. Returns warnings (not errors).
pub fn validate_rule(rule: &Rule, index: Option<usize>) -> Vec<Warning> {
    let mut warnings = Vec::new();
    let prefix = index.map(|i| format!("rule[{i}]")).unwrap_or_default();

    // Check: drop shorter than pace duration
    if let (Some(drop), Some(pace)) = (&rule.delivery.drop, &rule.delivery.pace) {
        if let crate::serve::DropSpec::AfterTime(drop_range) = drop {
            if let PaceSpec::Duration(pace_range) = pace {
                // Compare fixed values or min values
                let drop_ms = match drop_range {
                    crate::units::Range::Fixed(d) => d.as_millis(),
                    crate::units::Range::MinMax(min, _) => min.as_millis(),
                };
                let pace_ms = match pace_range {
                    crate::units::Range::Fixed(d) => d.as_millis(),
                    crate::units::Range::MinMax(_, max) => max.as_millis(),
                };
                if drop_ms < pace_ms {
                    warnings.push(Warning {
                        msg: format!(
                            "{prefix}{sep}serve: drop fires before pace completes — response will be truncated early",
                            sep = if prefix.is_empty() { "" } else { "." }
                        ),
                    });
                }
            }
        }
    }

    // Check: chaos percentages sum to 100%
    if let Some(ref chaos) = rule.chaos {
        let total: f64 = chaos.iter().map(|e| e.p).sum();
        if (total - 100.0).abs() < 0.01 {
            warnings.push(Warning {
                msg: format!(
                    "{prefix}{sep}chaos: percentages sum to 100% — no normal responses will ever be returned",
                    sep = if prefix.is_empty() { "" } else { "." }
                ),
            });
        }
    }

    warnings
}

/// Validate multiple rules, returning all warnings.
pub fn validate_rules(rules: &[Rule]) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for (i, rule) in rules.iter().enumerate() {
        let idx = if rules.len() > 1 { Some(i) } else { None };
        warnings.extend(validate_rule(rule, idx));
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::parse_rule;
    use serde_json::json;

    #[test]
    fn warn_drop_before_pace() {
        let rule = parse_rule(&json!({
            "match": {"g": "/test"},
            "reply": {"s": 200, "b": "ok"},
            "serve": {"pace": "5s", "drop": "500ms"}
        }))
        .unwrap();
        let warnings = validate_rule(&rule, None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].msg.contains("drop fires before pace"), "{}", warnings[0]);
    }

    #[test]
    fn no_warn_drop_after_pace() {
        let rule = parse_rule(&json!({
            "match": {"g": "/test"},
            "reply": {"s": 200, "b": "ok"},
            "serve": {"pace": "1s", "drop": "5s"}
        }))
        .unwrap();
        let warnings = validate_rule(&rule, None);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn warn_chaos_100_percent() {
        let rule = parse_rule(&json!({
            "match": {"g": "/test"},
            "reply": {"s": 200, "b": "ok"},
            "chaos": [
                {"p": "60%", "reply": {"s": 500}},
                {"p": "40%", "reply": {"s": 503}}
            ]
        }))
        .unwrap();
        let warnings = validate_rule(&rule, None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].msg.contains("100%"), "{}", warnings[0]);
    }

    #[test]
    fn no_warn_chaos_under_100() {
        let rule = parse_rule(&json!({
            "match": {"g": "/test"},
            "reply": {"s": 200, "b": "ok"},
            "chaos": [{"p": "50%", "reply": {"s": 500}}]
        }))
        .unwrap();
        let warnings = validate_rule(&rule, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn no_warn_clean_rule() {
        let rule = parse_rule(&json!({
            "match": {"g": "/test"},
            "reply": {"s": 200, "b": "ok"}
        }))
        .unwrap();
        let warnings = validate_rule(&rule, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn validate_multiple_with_indices() {
        let rules = vec![
            parse_rule(&json!({
                "match": {"g": "/ok"},
                "reply": {"s": 200}
            })).unwrap(),
            parse_rule(&json!({
                "match": {"g": "/bad"},
                "reply": {"s": 200},
                "chaos": [
                    {"p": "100%", "reply": {"s": 500}}
                ]
            })).unwrap(),
        ];
        let warnings = validate_rules(&rules);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].msg.contains("rule[1]"), "{}", warnings[0]);
    }

    #[test]
    fn warning_display_has_emoji() {
        let w = Warning { msg: "test warning".into() };
        assert_eq!(format!("{w}"), "⚠ test warning");
    }
}
