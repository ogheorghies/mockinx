use crate::units::ParseError;
use serde_json::Value;

/// A rule for matching incoming HTTP requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchRule {
    /// Matches any request.
    CatchAll,
    /// Matches by method (optional) and path (exact).
    MethodPath {
        /// HTTP method (None = any method).
        method: Option<String>,
        /// Path to match (normalized with leading `/`).
        path: String,
    },
}

impl MatchRule {
    /// Test whether this rule matches the given method and path.
    pub fn matches(&self, method: &str, path: &str) -> bool {
        match self {
            MatchRule::CatchAll => true,
            MatchRule::MethodPath {
                method: rule_method,
                path: rule_path,
            } => {
                if let Some(m) = rule_method {
                    if !m.eq_ignore_ascii_case(method) {
                        return false;
                    }
                }
                let normalized_path = normalize_path(path);
                let normalized_rule = normalize_path(rule_path);
                normalized_path == normalized_rule
            }
        }
    }
}

/// Normalize a path to always have a leading `/` and no trailing `/` (except root).
fn normalize_path(path: &str) -> &str {
    // For comparison, just ensure consistent leading slash handling
    path.strip_suffix('/').unwrap_or(path)
}

/// Parse a `MatchRule` from a `serde_json::Value`.
///
/// Accepts:
/// - String `"_"` → `CatchAll`
/// - Object with single key: method shortcut + path, or `_` + path
pub fn parse_match_rule(v: &Value) -> Result<MatchRule, ParseError> {
    match v {
        Value::String(s) if s == "_" => Ok(MatchRule::CatchAll),
        Value::String(s) => Err(ParseError(format!(
            "invalid match string '{s}', only '_' (catch-all) is allowed"
        ))),
        Value::Object(obj) => {
            if obj.is_empty() {
                return Err(ParseError("match object cannot be empty".into()));
            }
            if obj.len() > 1 {
                return Err(ParseError(
                    "match object must have exactly one key (method: path)".into(),
                ));
            }
            let (key, val) = obj.iter().next().unwrap();
            let path = val
                .as_str()
                .ok_or_else(|| ParseError(format!("match path must be a string, got {val}")))?;

            // Ensure path has leading /
            let path = if path.starts_with('/') {
                path.to_string()
            } else {
                format!("/{path}")
            };

            if key == "_" {
                // Any method
                Ok(MatchRule::MethodPath { method: None, path })
            } else if let Some(method) = yttp::resolve_method(key) {
                Ok(MatchRule::MethodPath {
                    method: Some(method.to_string()),
                    path,
                })
            } else {
                Err(ParseError(format!("unknown method shortcut '{key}'")))
            }
        }
        _ => Err(ParseError(format!(
            "match must be a string or object, got {v}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- Parsing ---

    #[test]
    fn parse_catch_all() {
        let rule = parse_match_rule(&json!("_")).unwrap();
        assert_eq!(rule, MatchRule::CatchAll);
    }

    #[test]
    fn parse_get() {
        let rule = parse_match_rule(&json!({"g": "/api/data"})).unwrap();
        assert_eq!(
            rule,
            MatchRule::MethodPath {
                method: Some("GET".into()),
                path: "/api/data".into(),
            }
        );
    }

    #[test]
    fn parse_post() {
        let rule = parse_match_rule(&json!({"p": "/api/data"})).unwrap();
        assert_eq!(
            rule,
            MatchRule::MethodPath {
                method: Some("POST".into()),
                path: "/api/data".into(),
            }
        );
    }

    #[test]
    fn parse_any_method() {
        let rule = parse_match_rule(&json!({"_": "/api/data"})).unwrap();
        assert_eq!(
            rule,
            MatchRule::MethodPath {
                method: None,
                path: "/api/data".into(),
            }
        );
    }

    #[test]
    fn parse_all_method_shortcuts() {
        let cases = vec![
            ("g", "GET"),
            ("p", "POST"),
            ("d", "DELETE"),
            ("put", "PUT"),
            ("patch", "PATCH"),
            ("head", "HEAD"),
            ("options", "OPTIONS"),
            ("trace", "TRACE"),
        ];
        for (short, full) in cases {
            let rule = parse_match_rule(&json!({short: "/path"})).unwrap();
            assert_eq!(
                rule,
                MatchRule::MethodPath {
                    method: Some(full.into()),
                    path: "/path".into(),
                },
                "shortcut '{short}' should map to '{full}'"
            );
        }
    }

    #[test]
    fn parse_path_without_leading_slash() {
        let rule = parse_match_rule(&json!({"g": "api/data"})).unwrap();
        assert_eq!(
            rule,
            MatchRule::MethodPath {
                method: Some("GET".into()),
                path: "/api/data".into(),
            }
        );
    }

    #[test]
    fn parse_error_invalid_string() {
        assert!(parse_match_rule(&json!("foo")).is_err());
    }

    #[test]
    fn parse_error_empty_object() {
        assert!(parse_match_rule(&json!({})).is_err());
    }

    #[test]
    fn parse_error_multiple_keys() {
        assert!(parse_match_rule(&json!({"g": "/a", "p": "/b"})).is_err());
    }

    #[test]
    fn parse_error_unknown_method() {
        assert!(parse_match_rule(&json!({"xyz": "/path"})).is_err());
    }

    #[test]
    fn parse_error_number() {
        assert!(parse_match_rule(&json!(42)).is_err());
    }

    #[test]
    fn parse_error_path_not_string() {
        assert!(parse_match_rule(&json!({"g": 42})).is_err());
    }

    // --- Matching ---

    #[test]
    fn catch_all_matches_everything() {
        let rule = MatchRule::CatchAll;
        assert!(rule.matches("GET", "/anything"));
        assert!(rule.matches("POST", "/other"));
        assert!(rule.matches("DELETE", "/"));
    }

    #[test]
    fn method_path_matches_exact() {
        let rule = parse_match_rule(&json!({"g": "/api/data"})).unwrap();
        assert!(rule.matches("GET", "/api/data"));
    }

    #[test]
    fn method_path_rejects_wrong_method() {
        let rule = parse_match_rule(&json!({"g": "/api/data"})).unwrap();
        assert!(!rule.matches("POST", "/api/data"));
    }

    #[test]
    fn method_path_rejects_wrong_path() {
        let rule = parse_match_rule(&json!({"g": "/api/data"})).unwrap();
        assert!(!rule.matches("GET", "/api/other"));
    }

    #[test]
    fn any_method_matches_all_methods() {
        let rule = parse_match_rule(&json!({"_": "/api/data"})).unwrap();
        assert!(rule.matches("GET", "/api/data"));
        assert!(rule.matches("POST", "/api/data"));
        assert!(rule.matches("DELETE", "/api/data"));
        assert!(rule.matches("PATCH", "/api/data"));
    }

    #[test]
    fn any_method_rejects_wrong_path() {
        let rule = parse_match_rule(&json!({"_": "/api/data"})).unwrap();
        assert!(!rule.matches("GET", "/other"));
    }

    #[test]
    fn method_matching_is_case_insensitive() {
        let rule = parse_match_rule(&json!({"g": "/path"})).unwrap();
        assert!(rule.matches("GET", "/path"));
        assert!(rule.matches("get", "/path"));
        assert!(rule.matches("Get", "/path"));
    }

    #[test]
    fn path_trailing_slash_normalized() {
        let rule = parse_match_rule(&json!({"g": "/api/data"})).unwrap();
        assert!(rule.matches("GET", "/api/data/"));
    }
}
