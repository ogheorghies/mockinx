use super::pace::{DeliverySpec, DropSpec, PaceSpec};
use bytes::Bytes;
use rand::Rng;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::time::{self, Instant};
use tokio_stream::Stream;

/// An async stream of byte chunks shaped by a delivery spec.
pub struct DeliveryStream {
    /// Body data (full).
    body: Vec<u8>,
    /// Current offset into body.
    offset: usize,
    /// Resolved chunk size in bytes.
    chunk_size: usize,
    /// Delay between chunks.
    chunk_delay: Option<std::time::Duration>,
    /// Drop after this many bytes (truncation).
    drop_after_bytes: Option<usize>,
    /// Drop after this deadline.
    drop_deadline: Option<Instant>,
    /// Whether we've applied the first-byte delay.
    first_byte_done: bool,
    /// First-byte delay.
    first_byte_delay: Option<std::time::Duration>,
    /// Sleep future for inter-chunk delay.
    sleep: Option<Pin<Box<time::Sleep>>>,
    /// Whether we've yielded anything yet.
    started: bool,
}

const DEFAULT_CHUNK_SIZE: usize = 8192;
const MIN_PACE_CHUNKS: usize = 10;

/// Resolve a delivery spec into a stream, sampling ranges from the provided RNG.
pub fn deliver(body: Vec<u8>, spec: &DeliverySpec, rng: &mut impl Rng) -> DeliveryStream {
    let first_byte_delay = spec
        .first_byte
        .as_ref()
        .map(|fb| fb.sample(rng).as_std());

    let drop_after_bytes = match &spec.drop {
        Some(DropSpec::AfterBytes(r)) => Some(r.sample(rng).bytes() as usize),
        _ => None,
    };

    let drop_deadline = match &spec.drop {
        Some(DropSpec::AfterTime(r)) => {
            let dur = r.sample(rng).as_std();
            Some(Instant::now() + dur)
        }
        _ => None,
    };

    // Determine chunk size and delay from PaceSpec
    let (chunk_size, chunk_delay) = match &spec.pace {
        Some(PaceSpec::Chunk { size, interval }) => {
            let s = size.sample(rng).bytes() as usize;
            let d = interval.sample(rng).as_std();
            (s.max(1), Some(d))
        }
        Some(PaceSpec::Duration(range)) => {
            let duration = range.sample(rng).as_std();
            // Target at least MIN_PACE_CHUNKS for smooth progressive delivery
            let chunk_size = (body.len() / MIN_PACE_CHUNKS).max(1).min(DEFAULT_CHUNK_SIZE);
            let num_chunks = (body.len() + chunk_size - 1) / chunk_size.max(1);
            let delay = if num_chunks > 1 {
                duration / (num_chunks - 1) as u32
            } else {
                duration
            };
            (chunk_size, Some(delay))
        }
        Some(PaceSpec::Speed(range)) => {
            let bps = range.sample(rng).bytes_per_sec();
            if bps == 0 {
                (body.len().max(1), None)
            } else {
                let chunk_size = (bps as usize / 10).max(1); // ~100ms chunks
                let delay_secs = chunk_size as f64 / bps as f64;
                (
                    chunk_size,
                    Some(std::time::Duration::from_secs_f64(delay_secs)),
                )
            }
        }
        None => {
            // No pacing: deliver all at once
            (body.len().max(1), None)
        }
    };

    DeliveryStream {
        body,
        offset: 0,
        chunk_size,
        chunk_delay,
        drop_after_bytes,
        drop_deadline,
        first_byte_done: false,
        first_byte_delay,
        sleep: None,
        started: false,
    }
}

impl Stream for DeliveryStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Check time-based drop
        if let Some(deadline) = self.drop_deadline {
            if Instant::now() >= deadline {
                return Poll::Ready(None);
            }
        }

        // Handle first-byte delay
        if !self.first_byte_done {
            if let Some(delay) = self.first_byte_delay {
                if self.sleep.is_none() {
                    self.sleep = Some(Box::pin(time::sleep(delay)));
                }
                if let Some(ref mut sleep) = self.sleep {
                    match sleep.as_mut().poll(cx) {
                        Poll::Ready(()) => {
                            self.first_byte_done = true;
                            self.sleep = None;
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                }
            } else {
                self.first_byte_done = true;
            }
        }

        // Handle inter-chunk delay (after first chunk)
        if self.started {
            if let Some(delay) = self.chunk_delay {
                if self.sleep.is_none() {
                    self.sleep = Some(Box::pin(time::sleep(delay)));
                }
                if let Some(ref mut sleep) = self.sleep {
                    match sleep.as_mut().poll(cx) {
                        Poll::Ready(()) => {
                            self.sleep = None;
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                }
            }
        }

        // Check if we're done
        let effective_len = if let Some(drop_bytes) = self.drop_after_bytes {
            self.body.len().min(drop_bytes)
        } else {
            self.body.len()
        };

        if self.offset >= effective_len {
            return Poll::Ready(None);
        }

        // Check time-based drop again (after delays)
        if let Some(deadline) = self.drop_deadline {
            if Instant::now() >= deadline {
                return Poll::Ready(None);
            }
        }

        // Yield next chunk
        let remaining = effective_len - self.offset;
        let len = remaining.min(self.chunk_size);
        let chunk = Bytes::copy_from_slice(&self.body[self.offset..self.offset + len]);
        self.offset += len;
        self.started = true;

        Poll::Ready(Some(Ok(chunk)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::pace::*;
    use crate::units::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use tokio_stream::StreamExt;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(42)
    }

    #[tokio::test]
    async fn no_shaping_yields_full_body() {
        let body = b"hello world".to_vec();
        let spec = DeliverySpec::default();
        let stream = deliver(body.clone(), &spec, &mut rng());
        let chunks: Vec<Bytes> = stream.map(|r| r.unwrap()).collect().await;
        let total: Vec<u8> = chunks.into_iter().flat_map(|b| b.to_vec()).collect();
        assert_eq!(total, body);
    }

    #[tokio::test]
    async fn first_byte_delay() {
        let body = b"hello".to_vec();
        let spec = DeliverySpec {
            first_byte: Some(Range::Fixed(Duration(std::time::Duration::from_millis(200)))),
            ..Default::default()
        };
        let start = Instant::now();
        let stream = deliver(body, &spec, &mut rng());
        let _: Vec<_> = stream.collect().await;
        let elapsed = start.elapsed();
        assert!(
            elapsed >= std::time::Duration::from_millis(180),
            "elapsed {elapsed:?} < 180ms"
        );
    }

    #[tokio::test]
    async fn pace_chunk() {
        let body = vec![0u8; 1000];
        let spec = DeliverySpec {
            pace: Some(PaceSpec::Chunk {
                size: Range::Fixed(ByteSize(300)),
                interval: Range::Fixed(Duration(std::time::Duration::from_millis(1))),
            }),
            ..Default::default()
        };
        let stream = deliver(body, &spec, &mut rng());
        let chunks: Vec<Bytes> = stream.map(|r| r.unwrap()).collect().await;
        assert_eq!(chunks.len(), 4); // 300 + 300 + 300 + 100
        assert_eq!(chunks[0].len(), 300);
        assert_eq!(chunks[3].len(), 100);
    }

    #[tokio::test]
    async fn pace_duration() {
        let body = vec![0u8; 10000];
        let spec = DeliverySpec {
            pace: Some(PaceSpec::Duration(Range::Fixed(Duration(
                std::time::Duration::from_millis(500),
            )))),
            ..Default::default()
        };
        let start = Instant::now();
        let stream = deliver(body, &spec, &mut rng());
        let chunks: Vec<Bytes> = stream.map(|r| r.unwrap()).collect().await;
        let elapsed = start.elapsed();
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, 10000);
        assert!(
            elapsed >= std::time::Duration::from_millis(350),
            "elapsed {elapsed:?} too short"
        );
        assert!(
            elapsed <= std::time::Duration::from_millis(800),
            "elapsed {elapsed:?} too long"
        );
    }

    #[tokio::test]
    async fn pace_speed() {
        // 10KB at 20KB/s should take ~500ms
        let body = vec![0u8; 10240];
        let spec = DeliverySpec {
            pace: Some(PaceSpec::Speed(Range::Fixed(Speed(20480)))),
            ..Default::default()
        };
        let start = Instant::now();
        let stream = deliver(body, &spec, &mut rng());
        let chunks: Vec<Bytes> = stream.map(|r| r.unwrap()).collect().await;
        let elapsed = start.elapsed();
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, 10240);
        assert!(
            elapsed >= std::time::Duration::from_millis(350),
            "elapsed {elapsed:?} too short for speed throttle"
        );
        assert!(
            elapsed <= std::time::Duration::from_millis(1000),
            "elapsed {elapsed:?} too long for speed throttle"
        );
    }

    #[tokio::test]
    async fn drop_after_bytes() {
        let body = vec![0u8; 10000];
        let spec = DeliverySpec {
            drop: Some(DropSpec::AfterBytes(Range::Fixed(ByteSize(2048)))),
            ..Default::default()
        };
        let stream = deliver(body, &spec, &mut rng());
        let chunks: Vec<Bytes> = stream.map(|r| r.unwrap()).collect().await;
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, 2048);
    }

    #[tokio::test]
    async fn drop_after_time() {
        let body = vec![0u8; 100000];
        let spec = DeliverySpec {
            pace: Some(PaceSpec::Chunk {
                size: Range::Fixed(ByteSize(100)),
                interval: Range::Fixed(Duration(std::time::Duration::from_millis(50))),
            }),
            drop: Some(DropSpec::AfterTime(Range::Fixed(Duration(
                std::time::Duration::from_millis(200),
            )))),
            ..Default::default()
        };
        let start = Instant::now();
        let stream = deliver(body, &spec, &mut rng());
        let chunks: Vec<Bytes> = stream.map(|r| r.unwrap()).collect().await;
        let elapsed = start.elapsed();
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert!(total < 100000, "should have dropped before full body");
        assert!(total > 0, "should have delivered something");
        assert!(
            elapsed <= std::time::Duration::from_millis(500),
            "elapsed {elapsed:?} too long"
        );
    }

    #[tokio::test]
    async fn first_byte_plus_pace_chunk() {
        let body = vec![0u8; 300];
        let spec = DeliverySpec {
            first_byte: Some(Range::Fixed(Duration(std::time::Duration::from_millis(100)))),
            pace: Some(PaceSpec::Chunk {
                size: Range::Fixed(ByteSize(100)),
                interval: Range::Fixed(Duration(std::time::Duration::from_millis(50))),
            }),
            ..Default::default()
        };
        let start = Instant::now();
        let stream = deliver(body, &spec, &mut rng());
        let chunks: Vec<Bytes> = stream.map(|r| r.unwrap()).collect().await;
        let elapsed = start.elapsed();
        assert_eq!(chunks.len(), 3);
        assert!(
            elapsed >= std::time::Duration::from_millis(180),
            "elapsed {elapsed:?} too short"
        );
    }
}
