use super::behavior_types::{BehaviorSpec, OverflowAction};
use crate::reply::ReplySpec;
use rand::Rng;
use std::sync::Mutex;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use std::sync::Arc;
use std::time::Instant;

/// Result of checking behavior policies.
pub enum BehaviorResult {
    /// Request may proceed. Hold the permit until response is complete.
    Proceed(Option<OwnedSemaphorePermit>),
    /// Request is rejected with this reply.
    Reject(ReplySpec),
}

/// Per-stub runtime state for behavior enforcement.
pub struct BehaviorRuntime {
    /// Concurrency semaphore.
    semaphore: Option<Arc<Semaphore>>,
    /// Simple rate limiter: sliding window of request timestamps.
    rate_limiter: Option<RateLimiter>,
}

struct RateLimiter {
    max_rps: u32,
    /// Timestamps of recent requests (within the last second).
    timestamps: Mutex<Vec<Instant>>,
}

impl RateLimiter {
    fn new(rps: u32) -> Self {
        RateLimiter {
            max_rps: rps,
            timestamps: Mutex::new(Vec::new()),
        }
    }

    /// Try to acquire a rate limit slot. Returns true if allowed.
    fn try_acquire(&self) -> bool {
        let now = Instant::now();
        let one_sec_ago = now - std::time::Duration::from_secs(1);
        let mut timestamps = self.timestamps.lock().unwrap();

        // Remove timestamps older than 1 second
        timestamps.retain(|&t| t > one_sec_ago);

        if timestamps.len() < self.max_rps as usize {
            timestamps.push(now);
            true
        } else {
            false
        }
    }
}

impl BehaviorRuntime {
    /// Create runtime state from a behavior spec.
    pub fn new(spec: &BehaviorSpec) -> Self {
        let semaphore = spec
            .concurrency
            .as_ref()
            .map(|c| Arc::new(Semaphore::new(c.max as usize)));

        let rate_limiter = spec
            .rate_limit
            .as_ref()
            .map(|rl| RateLimiter::new(rl.rps));

        BehaviorRuntime {
            semaphore,
            rate_limiter,
        }
    }

    /// Check all behavior policies. Returns Proceed or Reject.
    pub async fn check(&self, spec: &BehaviorSpec, rng: &mut impl Rng) -> BehaviorResult {
        // 1. Rate limit
        if let (Some(limiter), Some(rl_spec)) = (&self.rate_limiter, &spec.rate_limit) {
            if !limiter.try_acquire() {
                return BehaviorResult::Reject(rl_spec.over.clone());
            }
        }

        // 2. Fail injection
        if let Some(fail) = &spec.fail {
            let r: f64 = rng.r#gen();
            if r < fail.rate {
                return BehaviorResult::Reject(fail.reply.clone());
            }
        }

        // 3. Concurrency
        if let (Some(sem), Some(conc)) = (&self.semaphore, &spec.concurrency) {
            match &conc.over {
                OverflowAction::Reply(reply) => {
                    match Arc::clone(sem).try_acquire_owned() {
                        Ok(permit) => return BehaviorResult::Proceed(Some(permit)),
                        Err(_) => return BehaviorResult::Reject(reply.clone()),
                    }
                }
                OverflowAction::Block => {
                    let permit = Arc::clone(sem).acquire_owned().await.unwrap();
                    return BehaviorResult::Proceed(Some(permit));
                }
                OverflowAction::BlockWithTimeout { timeout, then } => {
                    let dur = timeout.sample(rng).as_std();
                    match tokio::time::timeout(dur, Arc::clone(sem).acquire_owned()).await {
                        Ok(Ok(permit)) => return BehaviorResult::Proceed(Some(permit)),
                        _ => return BehaviorResult::Reject(then.clone()),
                    }
                }
            }
        }

        BehaviorResult::Proceed(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::*;
    use crate::reply::ReplySpec;
    use crate::units::{Duration, Range};
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use serde_json::Map;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(42)
    }

    fn reject_reply(status: u16) -> ReplySpec {
        ReplySpec {
            status,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn no_behavior_proceeds() {
        let spec = BehaviorSpec::default();
        let runtime = BehaviorRuntime::new(&spec);
        match runtime.check(&spec, &mut rng()).await {
            BehaviorResult::Proceed(_) => {}
            BehaviorResult::Reject(_) => panic!("should proceed"),
        }
    }

    #[tokio::test]
    async fn concurrency_reject() {
        let spec = BehaviorSpec {
            concurrency: Some(ConcurrencySpec {
                max: 2,
                over: OverflowAction::Reply(reject_reply(429)),
            }),
            ..Default::default()
        };
        let runtime = BehaviorRuntime::new(&spec);

        // Acquire 2 permits
        let p1 = match runtime.check(&spec, &mut rng()).await {
            BehaviorResult::Proceed(p) => p,
            _ => panic!("should proceed"),
        };
        let p2 = match runtime.check(&spec, &mut rng()).await {
            BehaviorResult::Proceed(p) => p,
            _ => panic!("should proceed"),
        };

        // 3rd should be rejected
        match runtime.check(&spec, &mut rng()).await {
            BehaviorResult::Reject(r) => assert_eq!(r.status, 429),
            _ => panic!("should reject"),
        }

        // Drop a permit, next should proceed
        drop(p1);
        match runtime.check(&spec, &mut rng()).await {
            BehaviorResult::Proceed(_) => {}
            _ => panic!("should proceed after permit released"),
        }

        drop(p2);
    }

    #[tokio::test]
    async fn concurrency_block() {
        let spec = BehaviorSpec {
            concurrency: Some(ConcurrencySpec {
                max: 1,
                over: OverflowAction::Block,
            }),
            ..Default::default()
        };
        let runtime = Arc::new(BehaviorRuntime::new(&spec));
        let spec = Arc::new(spec);

        // Acquire the one permit
        let permit = match runtime.check(&spec, &mut rng()).await {
            BehaviorResult::Proceed(p) => p,
            _ => panic!("should proceed"),
        };

        // Spawn a task that will block
        let rt = Arc::clone(&runtime);
        let sp = Arc::clone(&spec);
        let handle = tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            match rt.check(&sp, &mut StdRng::seed_from_u64(1)).await {
                BehaviorResult::Proceed(_) => start.elapsed(),
                _ => panic!("should eventually proceed"),
            }
        });

        // Release permit after 100ms
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        drop(permit);

        let elapsed = handle.await.unwrap();
        assert!(
            elapsed >= std::time::Duration::from_millis(80),
            "blocked task should wait: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn concurrency_block_timeout() {
        let spec = BehaviorSpec {
            concurrency: Some(ConcurrencySpec {
                max: 1,
                over: OverflowAction::BlockWithTimeout {
                    timeout: Range::Fixed(Duration(std::time::Duration::from_millis(100))),
                    then: reject_reply(429),
                },
            }),
            ..Default::default()
        };
        let runtime = BehaviorRuntime::new(&spec);

        // Acquire the permit
        let _permit = match runtime.check(&spec, &mut rng()).await {
            BehaviorResult::Proceed(p) => p,
            _ => panic!("should proceed"),
        };

        // Next should timeout and reject
        let start = tokio::time::Instant::now();
        match runtime.check(&spec, &mut rng()).await {
            BehaviorResult::Reject(r) => {
                assert_eq!(r.status, 429);
                assert!(start.elapsed() >= std::time::Duration::from_millis(80));
            }
            _ => panic!("should reject after timeout"),
        }
    }

    #[tokio::test]
    async fn rate_limit() {
        let spec = BehaviorSpec {
            rate_limit: Some(RateLimitSpec {
                rps: 3,
                over: reject_reply(429),
            }),
            ..Default::default()
        };
        let runtime = BehaviorRuntime::new(&spec);

        // First 3 should proceed
        for _ in 0..3 {
            match runtime.check(&spec, &mut rng()).await {
                BehaviorResult::Proceed(_) => {}
                _ => panic!("should proceed"),
            }
        }

        // 4th should be rejected
        match runtime.check(&spec, &mut rng()).await {
            BehaviorResult::Reject(r) => assert_eq!(r.status, 429),
            _ => panic!("should reject"),
        }
    }

    #[tokio::test]
    async fn fail_injection() {
        let spec = BehaviorSpec {
            fail: Some(FailSpec {
                rate: 0.5,
                reply: reject_reply(500),
            }),
            ..Default::default()
        };
        let runtime = BehaviorRuntime::new(&spec);

        let mut proceed_count = 0;
        let mut reject_count = 0;
        for seed in 0..100 {
            let mut rng = StdRng::seed_from_u64(seed);
            match runtime.check(&spec, &mut rng).await {
                BehaviorResult::Proceed(_) => proceed_count += 1,
                BehaviorResult::Reject(_) => reject_count += 1,
            }
        }

        // With rate=0.5, expect roughly 50/50 (±15)
        assert!(
            proceed_count > 30,
            "too few proceeds: {proceed_count}"
        );
        assert!(
            reject_count > 30,
            "too few rejects: {reject_count}"
        );
    }
}
