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

/// First byte delay configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct FirstByteSpec {
    pub delay: Range<Duration>,
}

/// Chunked delivery configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkSpec {
    pub size: Range<ByteSize>,
    pub delay: Range<Duration>,
}

/// A weighted delivery profile for probabilistic selection.
#[derive(Debug, Clone, PartialEq)]
pub struct PickEntry {
    /// Probability weight (0.0..1.0), all entries should sum to ~1.0.
    pub p: f64,
    /// Delivery spec for this profile.
    pub spec: DeliverySpec,
}

/// Specification for how a response is delivered on the wire.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeliverySpec {
    /// Spread body over this duration.
    pub duration: Option<Range<Duration>>,
    /// Bandwidth cap.
    pub speed: Option<Range<Speed>>,
    /// Kill connection after N bytes or N time.
    pub drop: Option<DropSpec>,
    /// Delay before first byte.
    pub first_byte: Option<FirstByteSpec>,
    /// Chunked streaming.
    pub chunk: Option<ChunkSpec>,
    /// Probabilistic delivery profile selection.
    pub pick: Option<Vec<PickEntry>>,
}

/// Parse a `DeliverySpec` from a `serde_json::Value`.
///
/// Expects an object with optional keys: `duration`, `speed`, `drop`,
/// `first_byte`, `chunk`, `pick`.
pub fn parse_delivery(v: &Value) -> Result<DeliverySpec, ParseError> {
    let obj = v
        .as_object()
        .ok_or_else(|| ParseError("delivery must be an object".into()))?;

    let duration = parse_optional_range(obj, "duration", parse_duration_range)?;
    let speed = parse_optional_range(obj, "speed", parse_speed_range)?;
    let drop = parse_drop(obj)?;
    let first_byte = parse_first_byte(obj)?;
    let chunk = parse_chunk(obj)?;
    let pick = parse_pick(obj)?;

    Ok(DeliverySpec {
        duration,
        speed,
        drop,
        first_byte,
        chunk,
        pick,
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

    let drop_obj = drop_val
        .as_object()
        .ok_or_else(|| ParseError("drop must be an object".into()))?;

    let after = drop_obj
        .get("after")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ParseError("drop requires 'after' string field".into()))?;

    // Try byte size first, then duration. The unit suffix disambiguates.
    if let Ok(range) = parse_byte_size_range(after) {
        Ok(Some(DropSpec::AfterBytes(range)))
    } else if let Ok(range) = parse_duration_range(after) {
        Ok(Some(DropSpec::AfterTime(range)))
    } else {
        Err(ParseError(format!(
            "drop.after '{after}' is neither a valid byte size nor duration"
        )))
    }
}

fn parse_first_byte(obj: &Map<String, Value>) -> Result<Option<FirstByteSpec>, ParseError> {
    let fb_val = match obj.get("first_byte") {
        None => return Ok(None),
        Some(v) => v,
    };

    let fb_obj = fb_val
        .as_object()
        .ok_or_else(|| ParseError("first_byte must be an object".into()))?;

    let delay_str = fb_obj
        .get("delay")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ParseError("first_byte requires 'delay' string field".into()))?;

    let delay = parse_duration_range(delay_str)?;
    Ok(Some(FirstByteSpec { delay }))
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

fn parse_pick(obj: &Map<String, Value>) -> Result<Option<Vec<PickEntry>>, ParseError> {
    let pick_val = match obj.get("pick") {
        None => return Ok(None),
        Some(v) => v,
    };

    let arr = pick_val
        .as_array()
        .ok_or_else(|| ParseError("pick must be an array".into()))?;

    if arr.is_empty() {
        return Err(ParseError("pick array cannot be empty".into()));
    }

    let mut entries = Vec::with_capacity(arr.len());
    let mut total_p = 0.0f64;

    for item in arr {
        let item_obj = item
            .as_object()
            .ok_or_else(|| ParseError("pick entry must be an object".into()))?;

        let p = item_obj
            .get("p")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ParseError("pick entry requires 'p' as a number".into()))?;

        if p < 0.0 || p > 1.0 {
            return Err(ParseError(format!("pick probability {p} out of range [0, 1]")));
        }

        total_p += p;

        // Parse remaining fields as a delivery spec (excluding 'p' and 'pick')
        let mut spec_obj = item_obj.clone();
        spec_obj.remove("p");
        let spec = if spec_obj.is_empty() {
            DeliverySpec::default()
        } else {
            parse_delivery(&Value::Object(spec_obj))?
        };

        entries.push(PickEntry { p, spec });
    }

    // Allow small floating-point tolerance
    if (total_p - 1.0).abs() > 0.01 {
        return Err(ParseError(format!(
            "pick probabilities sum to {total_p}, expected ~1.0"
        )));
    }

    Ok(Some(entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{ByteSize, Duration, Speed};
    use serde_json::json;

    #[test]
    fn parse_duration_delivery() {
        let spec = parse_delivery(&json!({"duration": "5s"})).unwrap();
        assert_eq!(
            spec.duration,
            Some(Range::Fixed(Duration(std::time::Duration::from_secs(5))))
        );
    }

    #[test]
    fn parse_speed_delivery() {
        let spec = parse_delivery(&json!({"speed": "10kb/s"})).unwrap();
        assert_eq!(spec.speed, Some(Range::Fixed(Speed(10240))));
    }

    #[test]
    fn parse_drop_after_bytes() {
        let spec = parse_delivery(&json!({"drop": {"after": "2kb"}})).unwrap();
        assert_eq!(spec.drop, Some(DropSpec::AfterBytes(Range::Fixed(ByteSize(2048)))));
    }

    #[test]
    fn parse_drop_after_time() {
        let spec = parse_delivery(&json!({"drop": {"after": "1s"}})).unwrap();
        assert_eq!(
            spec.drop,
            Some(DropSpec::AfterTime(Range::Fixed(Duration(
                std::time::Duration::from_secs(1)
            ))))
        );
    }

    #[test]
    fn parse_first_byte_delay() {
        let spec = parse_delivery(&json!({"first_byte": {"delay": "2s"}})).unwrap();
        let fb = spec.first_byte.unwrap();
        assert_eq!(
            fb.delay,
            Range::Fixed(Duration(std::time::Duration::from_secs(2)))
        );
    }

    #[test]
    fn parse_chunk_spec() {
        let spec = parse_delivery(&json!({"chunk": {"size": "1kb", "delay": "100ms"}})).unwrap();
        let chunk = spec.chunk.unwrap();
        assert_eq!(chunk.size, Range::Fixed(ByteSize(1024)));
        assert_eq!(
            chunk.delay,
            Range::Fixed(Duration(std::time::Duration::from_millis(100)))
        );
    }

    #[test]
    fn parse_range_duration() {
        let spec = parse_delivery(&json!({"duration": "4s..6s"})).unwrap();
        match spec.duration.unwrap() {
            Range::MinMax(min, max) => {
                assert_eq!(min.as_millis(), 4000);
                assert_eq!(max.as_millis(), 6000);
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn parse_range_speed_percentage() {
        let spec = parse_delivery(&json!({"speed": "10kb/s..20%"})).unwrap();
        match spec.speed.unwrap() {
            Range::MinMax(min, max) => {
                assert_eq!(min.bytes_per_sec(), 8192);
                assert_eq!(max.bytes_per_sec(), 12288);
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn parse_pick() {
        let spec = parse_delivery(&json!({
            "pick": [
                {"p": 0.9},
                {"p": 0.05, "drop": {"after": "2kb"}},
                {"p": 0.05, "speed": "100b/s"}
            ]
        }))
        .unwrap();

        let pick = spec.pick.unwrap();
        assert_eq!(pick.len(), 3);
        assert_eq!(pick[0].p, 0.9);
        assert_eq!(pick[0].spec, DeliverySpec::default());
        assert!(pick[1].spec.drop.is_some());
        assert!(pick[2].spec.speed.is_some());
    }

    #[test]
    fn parse_empty_default() {
        let spec = parse_delivery(&json!({})).unwrap();
        assert_eq!(spec, DeliverySpec::default());
    }

    #[test]
    fn parse_multiple_fields() {
        let spec = parse_delivery(&json!({
            "first_byte": {"delay": "2s"},
            "duration": "5s",
            "drop": {"after": "2kb"}
        }))
        .unwrap();
        assert!(spec.first_byte.is_some());
        assert!(spec.duration.is_some());
        assert!(spec.drop.is_some());
    }

    #[test]
    fn parse_drop_with_range() {
        let spec = parse_delivery(&json!({"drop": {"after": "1kb..4kb"}})).unwrap();
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
        let spec =
            parse_delivery(&json!({"first_byte": {"delay": "1s..10%"}})).unwrap();
        match spec.first_byte.unwrap().delay {
            Range::MinMax(min, max) => {
                assert_eq!(min.as_millis(), 900);
                assert_eq!(max.as_millis(), 1100);
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn parse_chunk_with_ranges() {
        let spec = parse_delivery(&json!({
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
        match chunk.delay {
            Range::MinMax(min, max) => {
                assert_eq!(min.as_millis(), 50);
                assert_eq!(max.as_millis(), 150);
            }
            _ => panic!("expected MinMax delay"),
        }
    }

    // --- Error cases ---

    #[test]
    fn parse_error_not_object() {
        assert!(parse_delivery(&json!("bad")).is_err());
    }

    #[test]
    fn parse_error_drop_missing_after() {
        assert!(parse_delivery(&json!({"drop": {}})).is_err());
    }

    #[test]
    fn parse_error_drop_invalid_after() {
        assert!(parse_delivery(&json!({"drop": {"after": "xyz"}})).is_err());
    }

    #[test]
    fn parse_error_first_byte_missing_delay() {
        assert!(parse_delivery(&json!({"first_byte": {}})).is_err());
    }

    #[test]
    fn parse_error_chunk_missing_size() {
        assert!(parse_delivery(&json!({"chunk": {"delay": "100ms"}})).is_err());
    }

    #[test]
    fn parse_error_chunk_missing_delay() {
        assert!(parse_delivery(&json!({"chunk": {"size": "1kb"}})).is_err());
    }

    #[test]
    fn parse_error_pick_empty() {
        assert!(parse_delivery(&json!({"pick": []})).is_err());
    }

    #[test]
    fn parse_error_pick_bad_probability_sum() {
        assert!(parse_delivery(&json!({
            "pick": [{"p": 0.5}, {"p": 0.3}]
        }))
        .is_err());
    }

    #[test]
    fn parse_error_pick_negative_probability() {
        assert!(parse_delivery(&json!({
            "pick": [{"p": -0.1}, {"p": 1.1}]
        }))
        .is_err());
    }

    #[test]
    fn parse_error_pick_probability_over_one() {
        assert!(parse_delivery(&json!({
            "pick": [{"p": 1.5}]
        }))
        .is_err());
    }
}
