pub mod pace;
pub mod engine;
pub mod runtime;
mod behavior_types;

// Re-export types
pub use pace::{DeliverySpec, DropSpec, PaceSpec, parse_delivery_fields, parse_pace_str};
pub use engine::{DeliveryStream, deliver};
pub use runtime::BehaviorRuntime;
pub use behavior_types::{
    BehaviorSpec, ConcurrencySpec, CrudIdSpec, CrudSpec, FailSpec,
    OverflowAction, RateLimitSpec, SequenceScope, SequenceSpec,
    parse_behavior, parse_crud_spec,
};

use crate::units::ParseError;
use serde_json::Value;

/// Parse the merged `serve:` block — contains both delivery shaping and behavior fields.
pub fn parse_serve(v: &Value) -> Result<(DeliverySpec, BehaviorSpec), ParseError> {
    let obj = v
        .as_object()
        .ok_or_else(|| ParseError("serve must be an object".into()))?;

    let delivery = parse_delivery_fields(obj)?;
    let behavior = parse_behavior(v)?;

    Ok((delivery, behavior))
}
