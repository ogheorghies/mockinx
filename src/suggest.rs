/// Suggestion result for an unknown key.
#[derive(Debug)]
pub enum Suggestion {
    /// Use this key instead (direct alias or close typo).
    UseKey(String),
    /// A hint message (e.g. "use chaos: instead").
    Hint(String),
}

/// Alias tables: common synonyms → correct mockinx key or hint.
const RULE_ALIASES: &[(&str, &str)] = &[
    ("response", "reply"),
    ("resp", "reply"),
    ("answer", "reply"),
    ("behavior", "serve"),
    ("behaviour", "serve"),
    ("delivery", "serve"),
];

const SERVE_ALIASES: &[(&str, &str)] = &[
    ("speed", "pace"),
    ("bandwidth", "pace"),
    ("rate", "pace"),
    ("duration", "pace"),
    ("span", "pace"),
    ("time", "pace"),
    ("delay", "first_byte"),
    ("latency", "first_byte"),
    ("wait", "first_byte"),
    ("concurrency", "conn"),
    ("connections", "conn"),
    ("max_connections", "conn"),
    ("rate_limit", "rps"),
    ("throttle", "rps"),
];

const SERVE_HINTS: &[(&str, &str)] = &[
    ("fail", "use chaos: instead"),
    ("error", "use chaos: instead"),
    ("fault", "use chaos: instead"),
    ("chunk", "use pace: with @ syntax, e.g. pace: 1kb@100ms"),
];

const REPLY_ALIASES: &[(&str, &str)] = &[
    ("status", "s"),
    ("headers", "h"),
    ("body", "b"),
];

const REPLY_HINTS: &[(&str, &str)] = &[
    ("body_size", "use rand! or pattern! in b:"),
];

/// Valid keys for each block.
const RULE_KEYS: &[&str] = &["match", "reply", "serve", "chaos"];
const SERVE_KEYS: &[&str] = &["pace", "drop", "first_byte", "conn", "rps", "timeout"];
const REPLY_KEYS: &[&str] = &["s", "h", "b"];

/// Levenshtein edit distance between two strings.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];

    for i in 0..=a.len() {
        dp[i][0] = i;
    }
    for j in 0..=b.len() {
        dp[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[a.len()][b.len()]
}

/// Find a suggestion for an unknown key.
/// Checks aliases first (exact match on synonyms), then edit distance for typos.
fn suggest_from(
    input: &str,
    aliases: &[(&str, &str)],
    hints: &[(&str, &str)],
    valid_keys: &[&str],
) -> Option<Suggestion> {
    let input_lower = input.to_lowercase();

    // 1. Check aliases (exact match on synonym)
    for (alias, target) in aliases {
        if input_lower == *alias {
            return Some(Suggestion::UseKey(target.to_string()));
        }
    }

    // 2. Check hints (exact match)
    for (alias, hint) in hints {
        if input_lower == *alias {
            return Some(Suggestion::Hint(hint.to_string()));
        }
    }

    // 3. Edit distance against valid keys (max distance 2)
    let mut best: Option<(&str, usize)> = None;
    for key in valid_keys {
        let dist = edit_distance(&input_lower, key);
        if dist <= 2 && dist > 0 {
            if best.is_none() || dist < best.unwrap().1 {
                best = Some((key, dist));
            }
        }
    }
    best.map(|(key, _)| Suggestion::UseKey(key.to_string()))
}

/// Suggest for an unknown top-level rule key.
pub fn suggest_rule_key(input: &str) -> Option<Suggestion> {
    suggest_from(input, RULE_ALIASES, &[], RULE_KEYS)
}

/// Suggest for an unknown serve: block key.
pub fn suggest_serve_key(input: &str) -> Option<Suggestion> {
    suggest_from(input, SERVE_ALIASES, SERVE_HINTS, SERVE_KEYS)
}

/// Suggest for an unknown reply key.
pub fn suggest_reply_key(input: &str) -> Option<Suggestion> {
    suggest_from(input, REPLY_ALIASES, REPLY_HINTS, REPLY_KEYS)
}

/// Format a suggestion into an error message suffix.
pub fn format_suggestion(key: &str, block: &str, suggestion: &Suggestion) -> String {
    match suggestion {
        Suggestion::UseKey(target) => {
            format!("unknown key '{key}' in {block} → use '{target}'")
        }
        Suggestion::Hint(hint) => {
            format!("unknown key '{key}' in {block} → {hint}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Aliases ---

    #[test]
    fn alias_response_to_reply() {
        match suggest_rule_key("response") {
            Some(Suggestion::UseKey(k)) => assert_eq!(k, "reply"),
            other => panic!("expected UseKey(reply), got {other:?}"),
        }
    }

    #[test]
    fn alias_resp_to_reply() {
        match suggest_rule_key("resp") {
            Some(Suggestion::UseKey(k)) => assert_eq!(k, "reply"),
            other => panic!("expected UseKey(reply), got {other:?}"),
        }
    }

    #[test]
    fn alias_speed_to_pace() {
        match suggest_serve_key("speed") {
            Some(Suggestion::UseKey(k)) => assert_eq!(k, "pace"),
            other => panic!("expected UseKey(pace), got {other:?}"),
        }
    }

    #[test]
    fn alias_behavior_to_serve() {
        match suggest_rule_key("behavior") {
            Some(Suggestion::UseKey(k)) => assert_eq!(k, "serve"),
            other => panic!("expected UseKey(serve), got {other:?}"),
        }
    }

    #[test]
    fn alias_concurrency_to_conn() {
        match suggest_serve_key("concurrency") {
            Some(Suggestion::UseKey(k)) => assert_eq!(k, "conn"),
            other => panic!("expected UseKey(conn), got {other:?}"),
        }
    }

    #[test]
    fn alias_status_to_s() {
        match suggest_reply_key("status") {
            Some(Suggestion::UseKey(k)) => assert_eq!(k, "s"),
            other => panic!("expected UseKey(s), got {other:?}"),
        }
    }

    // --- Hints ---

    #[test]
    fn hint_fail_in_serve() {
        match suggest_serve_key("fail") {
            Some(Suggestion::Hint(h)) => assert!(h.contains("chaos"), "hint: {h}"),
            other => panic!("expected Hint, got {other:?}"),
        }
    }

    #[test]
    fn hint_chunk_in_serve() {
        match suggest_serve_key("chunk") {
            Some(Suggestion::Hint(h)) => assert!(h.contains("pace"), "hint: {h}"),
            other => panic!("expected Hint, got {other:?}"),
        }
    }

    #[test]
    fn hint_body_size_in_reply() {
        match suggest_reply_key("body_size") {
            Some(Suggestion::Hint(h)) => assert!(h.contains("rand!"), "hint: {h}"),
            other => panic!("expected Hint, got {other:?}"),
        }
    }

    // --- Edit distance typos ---

    #[test]
    fn typo_fist_byte_to_first_byte() {
        match suggest_serve_key("fist_byte") {
            Some(Suggestion::UseKey(k)) => assert_eq!(k, "first_byte"),
            other => panic!("expected UseKey(first_byte), got {other:?}"),
        }
    }

    #[test]
    fn typo_mach_to_match() {
        match suggest_rule_key("mach") {
            Some(Suggestion::UseKey(k)) => assert_eq!(k, "match"),
            other => panic!("expected UseKey(match), got {other:?}"),
        }
    }

    #[test]
    fn typo_drp_to_drop() {
        match suggest_serve_key("drp") {
            Some(Suggestion::UseKey(k)) => assert_eq!(k, "drop"),
            other => panic!("expected UseKey(drop), got {other:?}"),
        }
    }

    // --- No suggestion ---

    #[test]
    fn no_suggestion_for_unrelated() {
        assert!(suggest_rule_key("foobar").is_none());
        assert!(suggest_serve_key("zzzzz").is_none());
    }

    // --- Format ---

    #[test]
    fn format_use_key() {
        let s = format_suggestion("speed", "serve", &Suggestion::UseKey("pace".into()));
        assert_eq!(s, "unknown key 'speed' in serve → use 'pace'");
    }

    #[test]
    fn format_hint() {
        let s = format_suggestion("fail", "serve", &Suggestion::Hint("use chaos: instead".into()));
        assert_eq!(s, "unknown key 'fail' in serve → use chaos: instead");
    }

    // --- Edit distance ---

    #[test]
    fn edit_distance_identical() {
        assert_eq!(edit_distance("abc", "abc"), 0);
    }

    #[test]
    fn edit_distance_one() {
        assert_eq!(edit_distance("abc", "abc1"), 1);
        assert_eq!(edit_distance("abc", "ab"), 1);
    }

    #[test]
    fn edit_distance_two() {
        assert_eq!(edit_distance("pace", "pase"), 1);
        assert_eq!(edit_distance("mach", "match"), 1);
    }

    // --- Case insensitive ---

    #[test]
    fn alias_case_insensitive() {
        match suggest_rule_key("Response") {
            Some(Suggestion::UseKey(k)) => assert_eq!(k, "reply"),
            other => panic!("expected UseKey(reply), got {other:?}"),
        }
    }
}
