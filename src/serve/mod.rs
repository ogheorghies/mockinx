pub mod pace;
pub mod engine;
pub mod runtime;
mod behavior_types;

// Re-export types
pub use pace::{DeliverySpec, DropSpec, HangSpec, PaceSpec, parse_delivery_fields, parse_pace_str};
pub use engine::{DeliveryStream, deliver};
pub use runtime::BehaviorRuntime;
pub use behavior_types::{
    BehaviorSpec, ConcurrencySpec, CrudIdSpec, CrudSpec,
    OverflowAction, RateLimitSpec,
    parse_behavior, parse_crud_spec,
};

use crate::suggest::{suggest_serve_key, format_suggestion};
use crate::units::ParseError;
use serde_json::Value;

/// All valid keys in a serve: block.
const SERVE_KNOWN_KEYS: &[&str] = &["pace", "drop", "hang", "first_byte", "conn", "rps", "timeout"];

/// Parse the merged `serve:` block — contains both delivery shaping and behavior fields.
pub fn parse_serve(v: &Value) -> Result<(DeliverySpec, BehaviorSpec), ParseError> {
    let obj = v
        .as_object()
        .ok_or_else(|| ParseError::new("serve must be an object"))?;

    // Check for unknown keys
    for key in obj.keys() {
        if !SERVE_KNOWN_KEYS.contains(&key.as_str()) {
            if let Some(suggestion) = suggest_serve_key(key) {
                return Err(ParseError::new(format_suggestion(key, "serve", &suggestion)));
            }
            return Err(ParseError::new(format!("unknown key '{key}' in serve")));
        }
    }

    let delivery = parse_delivery_fields(obj)?;
    let behavior = parse_behavior(v)?;

    Ok((delivery, behavior))
}
