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

/// Chunked delivery configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkSpec {
    pub size: Range<ByteSize>,
    pub delay: Range<Duration>,
}

/// Delivery shaping specification (the delivery subset of `serve:`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeliverySpec {
    /// Spread body over this timespan (`span:`).
    pub span: Option<Range<Duration>>,
    /// Bandwidth cap (`speed:`).
    pub speed: Option<Range<Speed>>,
    /// Kill connection after N bytes or N time (`drop:`).
    pub drop: Option<DropSpec>,
    /// Delay before first byte (`first_byte:`).
    pub first_byte: Option<Range<Duration>>,
    /// Chunked streaming (`chunk:`).
    pub chunk: Option<ChunkSpec>,
}

/// Parse delivery-shaping fields from an object (subset of `serve:` keys).
pub fn parse_delivery_fields(obj: &Map<String, Value>) -> Result<DeliverySpec, ParseError> {
    let span = parse_optional_range(obj, "span", parse_duration_range)?;
    let speed = parse_optional_range(obj, "speed", parse_speed_range)?;
    let drop = parse_drop(obj)?;
    let first_byte = parse_first_byte(obj)?;
    let chunk = parse_chunk(obj)?;

    Ok(DeliverySpec {
        span,
        speed,
        drop,
        first_byte,
        chunk,
    })
}

fn parse_optional_range<T, F>(
    obj: &Map<String, Value>,
    key: &str,
    parser: F,
) -> Result<Option<Range<T>>, ParseError>
where
    F: Fn(&str) -> Result<Range<T>, ParseError>,
{
    match obj.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(parser(s)?)),
        Some(v) => Err(ParseError(format!("{key} must be a string, got {v}"))),
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
            .ok_or_else(|| ParseError("drop object requires 'after' string field".into()))?;
        return parse_drop_value(after);
    }

    Err(ParseError("drop must be a string or object".into()))
}

fn parse_drop_value(s: &str) -> Result<Option<DropSpec>, ParseError> {
    // Try byte size first, then duration. The unit suffix disambiguates.
    if let Ok(range) = parse_byte_size_range(s) {
        Ok(Some(DropSpec::AfterBytes(range)))
    } else if let Ok(range) = parse_duration_range(s) {
        Ok(Some(DropSpec::AfterTime(range)))
    } else {
        Err(ParseError(format!(
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
            .ok_or_else(|| ParseError("first_byte object requires 'delay' string field".into()))?;
        return Ok(Some(parse_duration_range(delay_str)?));
    }

    Err(ParseError("first_byte must be a string or object".into()))
}

fn parse_chunk(obj: &Map<String, Value>) -> Result<Option<ChunkSpec>, ParseError> {
    let chunk_val = match obj.get("chunk") {
        None => return Ok(None),
        Some(v) => v,
    };

    let chunk_obj = chunk_val
        .as_object()
        .ok_or_else(|| ParseError("chunk must be an object".into()))?;

    let size_str = chunk_obj
        .get("size")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ParseError("chunk requires 'size' string field".into()))?;
    let size = parse_byte_size_range(size_str)?;

    let delay_str = chunk_obj
        .get("delay")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ParseError("chunk requires 'delay' string field".into()))?;
    let delay = parse_duration_range(delay_str)?;

    Ok(Some(ChunkSpec { size, delay }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{ByteSize, Duration, Speed};
    use serde_json::json;

    fn parse(v: serde_json::Value) -> Result<DeliverySpec, ParseError> {
        parse_delivery_fields(v.as_object().unwrap())
    }

    #[test]
    fn parse_span() {
        let spec = parse(json!({"span": "5s"})).unwrap();
        assert_eq!(
            spec.span,
            Some(Range::Fixed(Duration(std::time::Duration::from_secs(5))))
        );
    }

    #[test]
    fn parse_speed() {
        let spec = parse(json!({"speed": "10kb/s"})).unwrap();
        assert_eq!(spec.speed, Some(Range::Fixed(Speed(10240))));
    }

    #[test]
    fn parse_drop_flat_bytes() {
        let spec = parse(json!({"drop": "2kb"})).unwrap();
        assert_eq!(spec.drop, Some(DropSpec::AfterBytes(Range::Fixed(ByteSize(2048)))));
    }

    #[test]
    fn parse_drop_flat_time() {
        let spec = parse(json!({"drop": "1s"})).unwrap();
        assert_eq!(
            spec.drop,
            Some(DropSpec::AfterTime(Range::Fixed(Duration(
                std::time::Duration::from_secs(1)
            ))))
        );
    }

    #[test]
    fn parse_drop_legacy_object() {
        let spec = parse(json!({"drop": {"after": "2kb"}})).unwrap();
        assert_eq!(spec.drop, Some(DropSpec::AfterBytes(Range::Fixed(ByteSize(2048)))));
    }

    #[test]
    fn parse_first_byte_flat() {
        let spec = parse(json!({"first_byte": "2s"})).unwrap();
        assert_eq!(
            spec.first_byte,
            Some(Range::Fixed(Duration(std::time::Duration::from_secs(2))))
        );
    }

    #[test]
    fn parse_first_byte_legacy_object() {
        let spec = parse(json!({"first_byte": {"delay": "2s"}})).unwrap();
        assert_eq!(
            spec.first_byte,
            Some(Range::Fixed(Duration(std::time::Duration::from_secs(2))))
        );
    }

    #[test]
    fn parse_chunk_spec() {
        let spec = parse(json!({"chunk": {"size": "1kb", "delay": "100ms"}})).unwrap();
        let chunk = spec.chunk.unwrap();
        assert_eq!(chunk.size, Range::Fixed(ByteSize(1024)));
        assert_eq!(
            chunk.delay,
            Range::Fixed(Duration(std::time::Duration::from_millis(100)))
        );
    }

    #[test]
    fn parse_range_span() {
        let spec = parse(json!({"span": "4s..6s"})).unwrap();
        match spec.span.unwrap() {
            Range::MinMax(min, max) => {
                assert_eq!(min.as_millis(), 4000);
                assert_eq!(max.as_millis(), 6000);
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn parse_range_speed_percentage() {
        let spec = parse(json!({"speed": "10kb/s..20%"})).unwrap();
        match spec.speed.unwrap() {
            Range::MinMax(min, max) => {
                assert_eq!(min.bytes_per_sec(), 8192);
                assert_eq!(max.bytes_per_sec(), 12288);
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn parse_empty_default() {
        let spec = parse(json!({})).unwrap();
        assert_eq!(spec, DeliverySpec::default());
    }

    #[test]
    fn parse_multiple_fields() {
        let spec = parse(json!({
            "first_byte": "2s",
            "span": "5s",
            "drop": "2kb"
        }))
        .unwrap();
        assert!(spec.first_byte.is_some());
        assert!(spec.span.is_some());
        assert!(spec.drop.is_some());
    }

    #[test]
    fn parse_drop_with_range() {
        let spec = parse(json!({"drop": "1kb..4kb"})).unwrap();
        match spec.drop.unwrap() {
            DropSpec::AfterBytes(Range::MinMax(min, max)) => {
                assert_eq!(min.bytes(), 1024);
                assert_eq!(max.bytes(), 4096);
            }
            other => panic!("expected AfterBytes MinMax, got {other:?}"),
        }
    }

    #[test]
    fn parse_first_byte_with_range() {
        let spec = parse(json!({"first_byte": "1s..10%"})).unwrap();
        match spec.first_byte.unwrap() {
            Range::MinMax(min, max) => {
                assert_eq!(min.as_millis(), 900);
                assert_eq!(max.as_millis(), 1100);
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn parse_chunk_with_ranges() {
        let spec = parse(json!({
            "chunk": {"size": "512b..2kb", "delay": "50ms..150ms"}
        }))
        .unwrap();
        let chunk = spec.chunk.unwrap();
        match chunk.size {
            Range::MinMax(min, max) => {
                assert_eq!(min.bytes(), 512);
                assert_eq!(max.bytes(), 2048);
            }
            _ => panic!("expected MinMax size"),
        }
    }

    // --- Error cases ---

    #[test]
    fn parse_error_drop_invalid() {
        assert!(parse(json!({"drop": "xyz"})).is_err());
    }

    #[test]
    fn parse_error_chunk_missing_size() {
        assert!(parse(json!({"chunk": {"delay": "100ms"}})).is_err());
    }

    #[test]
    fn parse_error_chunk_missing_delay() {
        assert!(parse(json!({"chunk": {"size": "1kb"}})).is_err());
    }
}
