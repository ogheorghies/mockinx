use rand::Rng;
use std::fmt;

/// Parse error for unit values.
#[derive(Debug, Clone)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Trait for types that support linear interpolation and scaling.
/// Required by Range for sampling and percentage-based ranges.
pub trait Scalable: Copy {
    fn lerp(self, other: Self, t: f64) -> Self;
    fn scale(self, factor: f64) -> Self;
}

/// Byte size value (stores bytes as u64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteSize(pub u64);

impl ByteSize {
    pub fn bytes(self) -> u64 {
        self.0
    }
}

impl Scalable for ByteSize {
    fn lerp(self, other: Self, t: f64) -> Self {
        ByteSize((self.0 as f64 + (other.0 as f64 - self.0 as f64) * t) as u64)
    }

    fn scale(self, factor: f64) -> Self {
        ByteSize((self.0 as f64 * factor) as u64)
    }
}

/// Parse a byte size from a string like "512b", "10kb", "1mb", "10gb".
/// Case-insensitive.
pub fn parse_byte_size(s: &str) -> Result<ByteSize, ParseError> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return Err(ParseError("empty byte size string".into()));
    }

    let (num_str, multiplier) = if let Some(n) = s.strip_suffix("gb") {
        (n, 1024u64 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix("mb") {
        (n, 1024u64 * 1024)
    } else if let Some(n) = s.strip_suffix("kb") {
        (n, 1024u64)
    } else if let Some(n) = s.strip_suffix("b") {
        (n, 1u64)
    } else {
        return Err(ParseError(format!("invalid byte size unit in '{s}', expected b/kb/mb/gb")));
    };

    let num: f64 = num_str
        .parse()
        .map_err(|_| ParseError(format!("invalid number in byte size '{s}'")))?;

    if num < 0.0 {
        return Err(ParseError(format!("byte size cannot be negative: '{s}'")));
    }

    Ok(ByteSize((num * multiplier as f64) as u64))
}

/// Duration value (wraps std::time::Duration).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Duration(pub std::time::Duration);

impl Duration {
    pub fn as_std(self) -> std::time::Duration {
        self.0
    }

    pub fn as_millis(self) -> u128 {
        self.0.as_millis()
    }
}

impl Scalable for Duration {
    fn lerp(self, other: Self, t: f64) -> Self {
        let min = self.0.as_secs_f64();
        let max = other.0.as_secs_f64();
        Duration(std::time::Duration::from_secs_f64(min + (max - min) * t))
    }

    fn scale(self, factor: f64) -> Self {
        Duration(std::time::Duration::from_secs_f64(self.0.as_secs_f64() * factor))
    }
}

/// Parse a duration from a string like "100ms", "2s", "5m".
/// Case-insensitive.
pub fn parse_duration(s: &str) -> Result<Duration, ParseError> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return Err(ParseError("empty duration string".into()));
    }

    let (num_str, factor_secs) = if let Some(n) = s.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = s.strip_suffix("s") {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix("m") {
        (n, 60.0)
    } else {
        return Err(ParseError(format!("invalid duration unit in '{s}', expected ms/s/m")));
    };

    let num: f64 = num_str
        .parse()
        .map_err(|_| ParseError(format!("invalid number in duration '{s}'")))?;

    if num < 0.0 {
        return Err(ParseError(format!("duration cannot be negative: '{s}'")));
    }

    Ok(Duration(std::time::Duration::from_secs_f64(num * factor_secs)))
}

/// Speed value (bytes per second).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Speed(pub u64);

impl Speed {
    pub fn bytes_per_sec(self) -> u64 {
        self.0
    }
}

impl Scalable for Speed {
    fn lerp(self, other: Self, t: f64) -> Self {
        Speed((self.0 as f64 + (other.0 as f64 - self.0 as f64) * t) as u64)
    }

    fn scale(self, factor: f64) -> Self {
        Speed((self.0 as f64 * factor) as u64)
    }
}

/// Parse a speed from a string like "10kb/s", "100b/s".
/// Case-insensitive.
pub fn parse_speed(s: &str) -> Result<Speed, ParseError> {
    let s_trimmed = s.trim().to_lowercase();
    if s_trimmed.is_empty() {
        return Err(ParseError("empty speed string".into()));
    }

    let size_str = s_trimmed
        .strip_suffix("/s")
        .ok_or_else(|| ParseError(format!("invalid speed format '{s_trimmed}', expected .../s")))?;

    let byte_size = parse_byte_size(size_str)?;
    Ok(Speed(byte_size.bytes()))
}

/// A value that is either fixed or a range (min..max), samplable uniformly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range<T> {
    Fixed(T),
    MinMax(T, T),
}

impl<T: Scalable> Range<T> {
    /// Sample a value from this range using uniform distribution.
    pub fn sample(&self, rng: &mut impl Rng) -> T {
        match self {
            Range::Fixed(v) => *v,
            Range::MinMax(min, max) => {
                let t: f64 = rng.r#gen();
                min.lerp(*max, t)
            }
        }
    }
}

/// Parse a range string. Supports:
/// - Fixed: `"5s"` → `Range::Fixed`
/// - Explicit range: `"4s..6s"` → `Range::MinMax`
/// - Percentage range: `"1s..10%"` → `Range::MinMax(900ms, 1100ms)`
fn parse_range_str<T, F>(s: &str, parse_fn: F) -> Result<Range<T>, ParseError>
where
    T: Scalable,
    F: Fn(&str) -> Result<T, ParseError>,
{
    let s = s.trim();

    if let Some(dot_pos) = s.find("..") {
        let base_str = &s[..dot_pos];
        let rest = &s[dot_pos + 2..];

        if let Some(pct_str) = rest.strip_suffix('%') {
            // Percentage range: "1s..10%"
            let base = parse_fn(base_str)?;
            let pct: f64 = pct_str
                .parse()
                .map_err(|_| ParseError(format!("invalid percentage in '{s}'")))?;
            if pct < 0.0 {
                return Err(ParseError(format!("percentage cannot be negative: '{s}'")));
            }
            let fraction = pct / 100.0;
            let lo = base.scale(1.0 - fraction);
            let hi = base.scale(1.0 + fraction);
            Ok(Range::MinMax(lo, hi))
        } else {
            // Explicit range: "4s..6s"
            let min = parse_fn(base_str)?;
            let max = parse_fn(rest)?;
            Ok(Range::MinMax(min, max))
        }
    } else {
        let val = parse_fn(s)?;
        Ok(Range::Fixed(val))
    }
}

/// Parse a `Range<ByteSize>` from a string.
pub fn parse_byte_size_range(s: &str) -> Result<Range<ByteSize>, ParseError> {
    parse_range_str(s, parse_byte_size)
}

/// Parse a `Range<Duration>` from a string.
pub fn parse_duration_range(s: &str) -> Result<Range<Duration>, ParseError> {
    parse_range_str(s, parse_duration)
}

/// Parse a `Range<Speed>` from a string.
pub fn parse_speed_range(s: &str) -> Result<Range<Speed>, ParseError> {
    parse_range_str(s, parse_speed)
}

/// Parse a `Range<ByteSize>` from a `serde_json::Value` (expects a string).
pub fn parse_byte_size_range_value(v: &serde_json::Value) -> Result<Range<ByteSize>, ParseError> {
    let s = v
        .as_str()
        .ok_or_else(|| ParseError("byte size range must be a string".into()))?;
    parse_byte_size_range(s)
}

/// Parse a `Range<Duration>` from a `serde_json::Value` (expects a string).
pub fn parse_duration_range_value(v: &serde_json::Value) -> Result<Range<Duration>, ParseError> {
    let s = v
        .as_str()
        .ok_or_else(|| ParseError("duration range must be a string".into()))?;
    parse_duration_range(s)
}

/// Parse a `Range<Speed>` from a `serde_json::Value` (expects a string).
pub fn parse_speed_range_value(v: &serde_json::Value) -> Result<Range<Speed>, ParseError> {
    let s = v
        .as_str()
        .ok_or_else(|| ParseError("speed range must be a string".into()))?;
    parse_speed_range(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    // --- ByteSize parsing ---

    #[test]
    fn parse_bytes() {
        assert_eq!(parse_byte_size("512b").unwrap().bytes(), 512);
    }

    #[test]
    fn parse_kilobytes() {
        assert_eq!(parse_byte_size("10kb").unwrap().bytes(), 10240);
    }

    #[test]
    fn parse_megabytes() {
        assert_eq!(parse_byte_size("1mb").unwrap().bytes(), 1048576);
    }

    #[test]
    fn parse_gigabytes() {
        assert_eq!(parse_byte_size("1gb").unwrap().bytes(), 1073741824);
    }

    #[test]
    fn parse_fractional_kb() {
        assert_eq!(parse_byte_size("1.5kb").unwrap().bytes(), 1536);
    }

    #[test]
    fn parse_zero_bytes() {
        assert_eq!(parse_byte_size("0b").unwrap().bytes(), 0);
    }

    #[test]
    fn parse_bytes_case_insensitive() {
        assert_eq!(parse_byte_size("10KB").unwrap().bytes(), 10240);
        assert_eq!(parse_byte_size("1Mb").unwrap().bytes(), 1048576);
        assert_eq!(parse_byte_size("1GB").unwrap().bytes(), 1073741824);
    }

    #[test]
    fn parse_bytes_with_whitespace() {
        assert_eq!(parse_byte_size("  10kb  ").unwrap().bytes(), 10240);
    }

    #[test]
    fn parse_bytes_invalid_unit() {
        assert!(parse_byte_size("10xx").is_err());
    }

    #[test]
    fn parse_bytes_invalid_number() {
        assert!(parse_byte_size("abckb").is_err());
    }

    #[test]
    fn parse_bytes_empty() {
        assert!(parse_byte_size("").is_err());
    }

    #[test]
    fn parse_bytes_negative() {
        assert!(parse_byte_size("-1kb").is_err());
    }

    // --- Duration parsing ---

    #[test]
    fn parse_milliseconds() {
        assert_eq!(
            parse_duration("100ms").unwrap().as_std(),
            std::time::Duration::from_millis(100)
        );
    }

    #[test]
    fn parse_seconds() {
        assert_eq!(
            parse_duration("2s").unwrap().as_std(),
            std::time::Duration::from_secs(2)
        );
    }

    #[test]
    fn parse_minutes() {
        assert_eq!(
            parse_duration("5m").unwrap().as_std(),
            std::time::Duration::from_secs(300)
        );
    }

    #[test]
    fn parse_fractional_seconds() {
        let d = parse_duration("1.5s").unwrap().as_std();
        assert_eq!(d, std::time::Duration::from_millis(1500));
    }

    #[test]
    fn parse_zero_duration() {
        assert_eq!(
            parse_duration("0s").unwrap().as_std(),
            std::time::Duration::ZERO
        );
    }

    #[test]
    fn parse_duration_case_insensitive() {
        assert_eq!(
            parse_duration("100MS").unwrap().as_std(),
            std::time::Duration::from_millis(100)
        );
        assert_eq!(
            parse_duration("2S").unwrap().as_std(),
            std::time::Duration::from_secs(2)
        );
    }

    #[test]
    fn parse_duration_invalid_unit() {
        assert!(parse_duration("10xx").is_err());
    }

    #[test]
    fn parse_duration_invalid_number() {
        assert!(parse_duration("abcs").is_err());
    }

    #[test]
    fn parse_duration_empty() {
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn parse_duration_negative() {
        assert!(parse_duration("-1s").is_err());
    }

    // --- Speed parsing ---

    #[test]
    fn parse_speed_bytes_per_sec() {
        assert_eq!(parse_speed("100b/s").unwrap().bytes_per_sec(), 100);
    }

    #[test]
    fn parse_speed_kb_per_sec() {
        assert_eq!(parse_speed("10kb/s").unwrap().bytes_per_sec(), 10240);
    }

    #[test]
    fn parse_speed_mb_per_sec() {
        assert_eq!(parse_speed("1mb/s").unwrap().bytes_per_sec(), 1048576);
    }

    #[test]
    fn parse_speed_zero() {
        assert_eq!(parse_speed("0b/s").unwrap().bytes_per_sec(), 0);
    }

    #[test]
    fn parse_speed_case_insensitive() {
        assert_eq!(parse_speed("10KB/S").unwrap().bytes_per_sec(), 10240);
    }

    #[test]
    fn parse_speed_missing_per_s() {
        assert!(parse_speed("10kb").is_err());
    }

    #[test]
    fn parse_speed_invalid() {
        assert!(parse_speed("abc/s").is_err());
    }

    #[test]
    fn parse_speed_empty() {
        assert!(parse_speed("").is_err());
    }

    // --- Range parsing ---

    #[test]
    fn range_fixed_duration() {
        let r = parse_duration_range("5s").unwrap();
        assert_eq!(r, Range::Fixed(Duration(std::time::Duration::from_secs(5))));
    }

    #[test]
    fn range_explicit_duration() {
        let r = parse_duration_range("4s..6s").unwrap();
        match r {
            Range::MinMax(min, max) => {
                assert_eq!(min.as_std(), std::time::Duration::from_secs(4));
                assert_eq!(max.as_std(), std::time::Duration::from_secs(6));
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn range_percentage_duration() {
        let r = parse_duration_range("1s..10%").unwrap();
        match r {
            Range::MinMax(min, max) => {
                assert_eq!(min.as_millis(), 900);
                assert_eq!(max.as_millis(), 1100);
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn range_explicit_byte_size() {
        let r = parse_byte_size_range("1kb..4kb").unwrap();
        match r {
            Range::MinMax(min, max) => {
                assert_eq!(min.bytes(), 1024);
                assert_eq!(max.bytes(), 4096);
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn range_percentage_speed() {
        let r = parse_speed_range("10kb/s..20%").unwrap();
        match r {
            Range::MinMax(min, max) => {
                assert_eq!(min.bytes_per_sec(), 8192);
                assert_eq!(max.bytes_per_sec(), 12288);
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn range_percentage_byte_size() {
        let r = parse_byte_size_range("1kb..5%").unwrap();
        match r {
            Range::MinMax(min, max) => {
                // 1024 * 0.95 = 972.8 → 972
                // 1024 * 1.05 = 1075.2 → 1075
                assert_eq!(min.bytes(), 972);
                assert_eq!(max.bytes(), 1075);
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn range_sample_fixed_always_same() {
        let r = Range::Fixed(Duration(std::time::Duration::from_secs(5)));
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..100 {
            assert_eq!(r.sample(&mut rng).as_std(), std::time::Duration::from_secs(5));
        }
    }

    #[test]
    fn range_sample_duration_within_bounds() {
        let r = parse_duration_range("4s..6s").unwrap();
        let mut rng = StdRng::seed_from_u64(42);
        let min = std::time::Duration::from_secs(4);
        let max = std::time::Duration::from_secs(6);
        for _ in 0..100 {
            let sample = r.sample(&mut rng).as_std();
            assert!(sample >= min, "sample {sample:?} < min {min:?}");
            assert!(sample <= max, "sample {sample:?} > max {max:?}");
        }
    }

    #[test]
    fn range_sample_byte_size_within_bounds() {
        let r = parse_byte_size_range("1kb..4kb").unwrap();
        let mut rng = StdRng::seed_from_u64(99);
        for _ in 0..100 {
            let sample = r.sample(&mut rng).bytes();
            assert!(sample >= 1024, "sample {sample} < 1024");
            assert!(sample <= 4096, "sample {sample} > 4096");
        }
    }

    #[test]
    fn range_sample_speed_within_bounds() {
        let r = parse_speed_range("10kb/s..20%").unwrap();
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..100 {
            let sample = r.sample(&mut rng).bytes_per_sec();
            assert!(sample >= 8192, "sample {sample} < 8192");
            assert!(sample <= 12288, "sample {sample} > 12288");
        }
    }

    #[test]
    fn range_sample_varies() {
        // Verify that sampling a range produces different values (not all the same)
        let r = parse_duration_range("1s..10s").unwrap();
        let mut rng = StdRng::seed_from_u64(42);
        let samples: Vec<u128> = (0..10).map(|_| r.sample(&mut rng).as_millis()).collect();
        let first = samples[0];
        assert!(samples.iter().any(|&s| s != first), "all samples identical: {samples:?}");
    }

    #[test]
    fn range_invalid_percentage() {
        assert!(parse_duration_range("1s..-5%").is_err());
    }

    #[test]
    fn range_invalid_right_side() {
        assert!(parse_duration_range("4s..abc").is_err());
    }

    #[test]
    fn range_invalid_left_side() {
        assert!(parse_duration_range("abc..4s").is_err());
    }

    // --- Value parsing ---

    #[test]
    fn parse_byte_size_range_from_value() {
        let v = serde_json::Value::String("10kb".into());
        let r = parse_byte_size_range_value(&v).unwrap();
        assert_eq!(r, Range::Fixed(ByteSize(10240)));
    }

    #[test]
    fn parse_duration_range_from_value() {
        let v = serde_json::Value::String("2s..4s".into());
        let r = parse_duration_range_value(&v).unwrap();
        match r {
            Range::MinMax(min, max) => {
                assert_eq!(min.as_millis(), 2000);
                assert_eq!(max.as_millis(), 4000);
            }
            _ => panic!("expected MinMax"),
        }
    }

    #[test]
    fn parse_range_value_not_string() {
        let v = serde_json::json!(42);
        assert!(parse_byte_size_range_value(&v).is_err());
        assert!(parse_duration_range_value(&v).is_err());
        assert!(parse_speed_range_value(&v).is_err());
    }
}
