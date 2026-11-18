use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub backoff_factor: f64,
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            backoff_factor: 2.0,
            jitter: false,
        }
    }
}

impl RetryPolicy {
    pub fn new(max_attempts: u32) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
            ..Default::default()
        }
    }

    pub fn base_delay(mut self, delay: Duration) -> Self {
        self.base_delay = delay;
        self
    }

    pub fn max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    pub fn backoff_factor(mut self, factor: f64) -> Self {
        self.backoff_factor = factor;
        self
    }

    pub fn with_jitter(mut self, jitter: bool) -> Self {
        self.jitter = jitter;
        self
    }

    fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let factor = self.backoff_factor.max(1.0);
        let exponent = attempt.saturating_sub(1) as f64;
        let raw = self.base_delay.as_secs_f64() * factor.powf(exponent);
        let capped = raw.min(self.max_delay.as_secs_f64());
        let mut delay = Duration::from_secs_f64(capped);
        if self.jitter {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as f64)
                .unwrap_or(0.0);
            let factor = 0.8 + (nanos / u32::MAX as f64) * 0.4;
            delay = Duration::from_secs_f64(delay.as_secs_f64() * factor);
        }
        delay
    }

    pub fn retry<T, E, F>(self, mut operation: F) -> Result<T, E>
    where
        F: FnMut(u32) -> Result<T, E>,
    {
        let max = self.max_attempts;
        let mut last_error = None;
        for attempt in 1..=max {
            match operation(attempt) {
                Ok(value) => return Ok(value),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < max {
                        std::thread::sleep(self.delay_for_attempt(attempt));
                    }
                }
            }
        }
        Err(last_error.expect("at least one attempt was made"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug)]
pub enum CircuitError<E> {
    Open,
    Operation(E),
}

impl<E: std::fmt::Debug> CircuitError<E> {
    pub fn into_operation_error(self) -> Option<E> {
        match self {
            CircuitError::Open => None,
            CircuitError::Operation(e) => Some(e),
        }
    }
}

pub struct CircuitBreaker {
    failure_threshold: u32,
    cooldown: Duration,
    state: Mutex<CircuitStateInternal>,
}

#[derive(Debug)]
struct CircuitStateInternal {
    state: CircuitState,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, cooldown: Duration) -> Self {
        Self {
            failure_threshold: failure_threshold.max(1),
            cooldown,
            state: Mutex::new(CircuitStateInternal {
                state: CircuitState::Closed,
                consecutive_failures: 0,
                opened_at: None,
            }),
        }
    }

    pub fn state(&self) -> CircuitState {
        self.state.lock().expect("circuit lock poisoned").state
    }

    pub fn reset(&self) {
        let mut guard = self.state.lock().expect("circuit lock poisoned");
        *guard = CircuitStateInternal {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            opened_at: None,
        };
    }

    pub fn call<T, E, F>(&self, operation: F) -> Result<T, CircuitError<E>>
    where
        F: FnOnce() -> Result<T, E>,
    {
        let mut guard = self.state.lock().expect("circuit lock poisoned");
        if guard.state == CircuitState::Open {
            let opened_at = guard.opened_at.unwrap_or_else(Instant::now);
            if opened_at.elapsed() >= self.cooldown {
                guard.state = CircuitState::HalfOpen;
            } else {
                return Err(CircuitError::Open);
            }
        }
        drop(guard);

        let result = operation();

        let mut guard = self.state.lock().expect("circuit lock poisoned");
        match result {
            Ok(value) => {
                guard.state = CircuitState::Closed;
                guard.consecutive_failures = 0;
                guard.opened_at = None;
                Ok(value)
            }
            Err(e) => {
                guard.consecutive_failures = guard.consecutive_failures.saturating_add(1);
                if guard.state == CircuitState::HalfOpen
                    || guard.consecutive_failures >= self.failure_threshold
                {
                    guard.state = CircuitState::Open;
                    guard.opened_at = Some(Instant::now());
                }
                Err(CircuitError::Operation(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_succeeds_immediately() {
        let mut calls = 0;
        let result = RetryPolicy::new(3).retry(|_| {
            calls += 1;
            Ok::<i32, String>(7)
        });
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls, 1);
    }

    #[test]
    fn retry_recovers_on_third_attempt() {
        let mut calls = 0;
        let result = RetryPolicy::new(5)
            .base_delay(Duration::from_millis(1))
            .retry(|attempt| {
                calls += 1;
                if attempt < 3 {
                    Err(format!("attempt {attempt} failed"))
                } else {
                    Ok(attempt)
                }
            });
        assert_eq!(result.unwrap(), 3);
        assert_eq!(calls, 3);
    }

    #[test]
    fn retry_exhausts_attempts_and_returns_last_error() {
        let mut calls = 0;
        let result: Result<i32, String> = RetryPolicy::new(2)
            .base_delay(Duration::from_millis(1))
            .retry(|_| {
                calls += 1;
                Err("always fails".to_string())
            });
        assert_eq!(result.unwrap_err(), "always fails");
        assert_eq!(calls, 2);
    }

    #[test]
    fn retry_respects_single_attempt() {
        let mut calls = 0;
        let outcome: Result<i32, String> = RetryPolicy::new(1).retry(|_| {
            calls += 1;
            Err("no retries".to_string())
        });
        assert!(outcome.is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn circuit_opens_after_threshold_and_recovers() {
        let breaker = CircuitBreaker::new(2, Duration::from_millis(50));
        let fail: fn() -> Result<(), String> = || Err("boom".into());

        let _ = breaker.call(fail);
        assert_eq!(breaker.state(), CircuitState::Closed);

        let _ = breaker.call(fail);
        assert_eq!(breaker.state(), CircuitState::Open);

        let rejected = breaker.call(|| Ok::<i32, String>(1));
        assert!(matches!(rejected, Err(CircuitError::Open)));

        std::thread::sleep(Duration::from_millis(60));
        let recovered = breaker.call(|| Ok::<i32, String>(1));
        assert!(recovered.is_ok());
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn reset_returns_breaker_to_closed() {
        let breaker = CircuitBreaker::new(1, Duration::from_secs(60));
        let _ = breaker.call(|| Err::<i32, String>("x".into()));
        assert_eq!(breaker.state(), CircuitState::Open);
        breaker.reset();
        assert_eq!(breaker.state(), CircuitState::Closed);
    }
}
