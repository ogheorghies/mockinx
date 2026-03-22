use crate::units::{
    ByteSize, Duration, ParseError, Range, Speed,
    parse_byte_size_range, parse_duration_range, parse_speed_range,
};
use serde_json::{Map, Value};

/// How to drop a connection.
#[derive(Debug, Clone, PartialEq)]
pub enum DropSpec {
    /// Drop after sending N bytes.
    AfterBytes(Range<ByteSize>),
    /// Drop after N time has elapsed.
    AfterTime(Range<Duration>),
}

/// How to pace delivery.
#[derive(Debug, Clone, PartialEq)]
pub enum PaceSpec {
    /// Spread body over a target duration: `pace: 5s`
    Duration(Range<Duration>),
    /// Bandwidth cap: `pace: 10kb/s`
    Speed(Range<Speed>),
    /// Explicit chunking: `pace: 1kb@100ms`
    Chunk {
        size: Range<ByteSize>,
        interval: Range<Duration>,
    },
}

/// Delivery shaping specification (the delivery subset of `serve:`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeliverySpec {
    /// Pacing: duration, speed, or explicit chunks (`pace:`).
    pub pace: Option<PaceSpec>,
    /// Kill connection after N bytes or N time (`drop:`).
    pub drop: Option<DropSpec>,
    /// Delay before first byte (`first_byte:`).
    pub first_byte: Option<Range<Duration>>,
}

/// Parse delivery-shaping fields from an object (subset of `serve:` keys).
pub fn parse_delivery_fields(obj: &Map<String, Value>) -> Result<DeliverySpec, ParseError> {
    let pace = parse_pace(obj)?;
    let drop = parse_drop(obj)?;
    let first_byte = parse_first_byte(obj)?;

    Ok(DeliverySpec {
        pace,
        drop,
        first_byte,
    })
}

/// Parse the polymorphic `pace:` field.
///
/// Three string forms distinguished by sigils:
/// - Contains `@` → chunk mode: `1kb@100ms`, `512b..2kb@50ms..150ms`
/// - Contains `/s` → speed mode: `10kb/s`, `10kb/s..20%`
/// - Otherwise → duration mode: `5s`, `4s..6s`
fn parse_pace(obj: &Map<String, Value>) -> Result<Option<PaceSpec>, ParseError> {
    let pace_val = match obj.get("pace") {
        None => return Ok(None),
        Some(v) => v,
    };

    let s = pace_val
        .as_str()
        .ok_or_else(|| ParseError::new("pace must be a string"))?;

    Ok(Some(parse_pace_str(s)?))
}

/// Parse a pace string into a PaceSpec.
pub fn parse_pace_str(s: &str) -> Result<PaceSpec, ParseError> {
    let s = s.trim();

    if let Some(at_pos) = s.find('@') {
        // Chunk mode: "1kb@100ms" or "512b..2kb@50ms..150ms"
        let size_str = &s[..at_pos];
        let interval_str = &s[at_pos + 1..];
        let size = parse_byte_size_range(size_str)?;
        let interval = parse_duration_range(interval_str)?;
        Ok(PaceSpec::Chunk { size, interval })
    } else if s.contains("/s") {
        // Speed mode: "10kb/s" or "10kb/s..20%"
        let speed = parse_speed_range(s)?;
        Ok(PaceSpec::Speed(speed))
    } else {
        // Duration mode: "5s" or "4s..6s"
        let duration = parse_duration_range(s)?;
        Ok(PaceSpec::Duration(duration))
    }
}

fn parse_drop(obj: &Map<String, Value>) -> Result<Option<DropSpec>, ParseError> {
    let drop_val = match obj.get("drop") {
        None => return Ok(None),
        Some(v) => v,
    };

    // Flat scalar: drop: "2kb" or drop: "1s"
    if let Some(s) = drop_val.as_str() {
        return parse_drop_value(s);
    }

    // Legacy object: drop: {after: "2kb"}
    if let Some(drop_obj) = drop_val.as_object() {
        let after = drop_obj
            .get("after")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ParseError::new("drop object requires 'after' string field"))?;
        return parse_drop_value(after);
    }

    Err(ParseError::new("drop must be a string or object"))
}

fn parse_drop_value(s: &str) -> Result<Option<DropSpec>, ParseError> {
    if let Ok(range) = parse_byte_size_range(s) {
        Ok(Some(DropSpec::AfterBytes(range)))
    } else if let Ok(range) = parse_duration_range(s) {
        Ok(Some(DropSpec::AfterTime(range)))
    } else {
        Err(ParseError::new(format!(
            "drop '{s}' is neither a valid byte size nor duration"
        )))
    }
}

fn parse_first_byte(obj: &Map<String, Value>) -> Result<Option<Range<Duration>>, ParseError> {
    let fb_val = match obj.get("first_byte") {
        None => return Ok(None),
        Some(v) => v,
    };

    // Flat scalar: first_byte: "2s"
    if let Some(s) = fb_val.as_str() {
        return Ok(Some(parse_duration_range(s)?));
    }

    // Legacy object: first_byte: {delay: "2s"}
    if let Some(fb_obj) = fb_val.as_object() {
        let delay_str = fb_obj
            .get("delay")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ParseError::new("first_byte object requires 'delay' string field"))?;
        return Ok(Some(parse_duration_range(delay_str)?));
    }

    Err(ParseError::new("first_byte must be a string or object"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{ByteSize, Duration, Speed};
    use serde_json::json;

    fn parse(v: serde_json::Value) -> Result<DeliverySpec, ParseError> {
        parse_delivery_fields(v.as_object().unwrap())
    }

    // --- PaceSpec parsing ---

    #[test]
    fn pace_duration() {
        let spec = parse(json!({"pace": "5s"})).unwrap();
        match spec.pace.unwrap() {
            PaceSpec::Duration(Range::Fixed(d)) => {
                assert_eq!(d.as_std(), std::time::Duration::from_secs(5));
            }
            other => panic!("expected Duration, got {other:?}"),
        }
    }

    #[test]
    fn pace_speed() {
        let spec = parse(json!({"pace": "10kb/s"})).unwrap();
        match spec.pace.unwrap() {
            PaceSpec::Speed(Range::Fixed(s)) => {
                assert_eq!(s.bytes_per_sec(), 10240);
            }
            other => panic!("expected Speed, got {other:?}"),
        }
    }

    #[test]
    fn pace_chunk() {
        let spec = parse(json!({"pace": "1kb@100ms"})).unwrap();
        match spec.pace.unwrap() {
            PaceSpec::Chunk { size, interval } => {
                assert_eq!(size, Range::Fixed(ByteSize(1024)));
                assert_eq!(
                    interval,
                    Range::Fixed(Duration(std::time::Duration::from_millis(100)))
                );
            }
            other => panic!("expected Chunk, got {other:?}"),
        }
    }

    #[test]
    fn pace_duration_range() {
        let spec = parse(json!({"pace": "4s..6s"})).unwrap();
        match spec.pace.unwrap() {
            PaceSpec::Duration(Range::MinMax(min, max)) => {
                assert_eq!(min.as_millis(), 4000);
                assert_eq!(max.as_millis(), 6000);
            }
            other => panic!("expected Duration MinMax, got {other:?}"),
        }
    }

    #[test]
    fn pace_speed_percentage() {
        let spec = parse(json!({"pace": "10kb/s..20%"})).unwrap();
        match spec.pace.unwrap() {
            PaceSpec::Speed(Range::MinMax(min, max)) => {
                assert_eq!(min.bytes_per_sec(), 8192);
                assert_eq!(max.bytes_per_sec(), 12288);
            }
            other => panic!("expected Speed MinMax, got {other:?}"),
        }
    }

    #[test]
    fn pace_chunk_ranges() {
        let spec = parse(json!({"pace": "512b..2kb@50ms..150ms"})).unwrap();
        match spec.pace.unwrap() {
            PaceSpec::Chunk { size, interval } => {
                match size {
                    Range::MinMax(min, max) => {
                        assert_eq!(min.bytes(), 512);
                        assert_eq!(max.bytes(), 2048);
                    }
                    _ => panic!("expected MinMax size"),
                }
                match interval {
                    Range::MinMax(min, max) => {
                        assert_eq!(min.as_millis(), 50);
                        assert_eq!(max.as_millis(), 150);
                    }
                    _ => panic!("expected MinMax interval"),
                }
            }
            other => panic!("expected Chunk, got {other:?}"),
        }
    }

    // --- Drop ---

    #[test]
    fn drop_flat_bytes() {
        let spec = parse(json!({"drop": "2kb"})).unwrap();
        assert_eq!(spec.drop, Some(DropSpec::AfterBytes(Range::Fixed(ByteSize(2048)))));
    }

    #[test]
    fn drop_flat_time() {
        let spec = parse(json!({"drop": "1s"})).unwrap();
        assert_eq!(
            spec.drop,
            Some(DropSpec::AfterTime(Range::Fixed(Duration(
                std::time::Duration::from_secs(1)
            ))))
        );
    }

    #[test]
    fn drop_range() {
        let spec = parse(json!({"drop": "1kb..4kb"})).unwrap();
        match spec.drop.unwrap() {
            DropSpec::AfterBytes(Range::MinMax(min, max)) => {
                assert_eq!(min.bytes(), 1024);
                assert_eq!(max.bytes(), 4096);
            }
            other => panic!("expected AfterBytes MinMax, got {other:?}"),
        }
    }

    // --- First byte ---

    #[test]
    fn first_byte_flat() {
        let spec = parse(json!({"first_byte": "2s"})).unwrap();
        assert_eq!(
            spec.first_byte,
            Some(Range::Fixed(Duration(std::time::Duration::from_secs(2))))
        );
    }

    #[test]
    fn first_byte_range() {
        let spec = parse(json!({"first_byte": "1s..10%"})).unwrap();
        match spec.first_byte.unwrap() {
            Range::MinMax(min, max) => {
                assert_eq!(min.as_millis(), 900);
                assert_eq!(max.as_millis(), 1100);
            }
            _ => panic!("expected MinMax"),
        }
    }

    // --- Empty / multiple ---

    #[test]
    fn empty_default() {
        let spec = parse(json!({})).unwrap();
        assert_eq!(spec, DeliverySpec::default());
    }

    #[test]
    fn multiple_fields() {
        let spec = parse(json!({
            "first_byte": "2s",
            "pace": "5s",
            "drop": "2kb"
        }))
        .unwrap();
        assert!(spec.first_byte.is_some());
        assert!(spec.pace.is_some());
        assert!(spec.drop.is_some());
    }

    // --- Error cases ---

    #[test]
    fn pace_invalid() {
        assert!(parse(json!({"pace": "xyz"})).is_err());
    }

    #[test]
    fn drop_invalid() {
        assert!(parse(json!({"drop": "xyz"})).is_err());
    }
}
