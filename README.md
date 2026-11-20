> **DEPRECATED:** this crate has been renamed to [`polly-guru`](https://crates.io/crates/polly-guru). New code should depend on `polly-guru`.

# pollyguru

**Polly-style resilience for Rust** — retry with exponential backoff and jitter, plus a thread-safe circuit breaker, zero dependencies.

For developers coming from C#'s [Polly](https://github.com/App-vNext/Polly) or Python's [tenacity](https://github.com/jd/tenacity) who want familiar resilience primitives without a framework.

## Retry

```rust
use pollyguru::RetryPolicy;
use std::time::Duration;

fn main() {
    let result = RetryPolicy::new(5)
        .base_delay(Duration::from_millis(200))
        .backoff_factor(2.0)
        .max_delay(Duration::from_secs(10))
        .with_jitter(true)
        .retry(|attempt| {
            // attempt is 1-based
            flaky_api_call(attempt)
        });

    match result {
        Ok(data) => println!("got {data:?}"),
        Err(e) => println!("all attempts failed: {e}"),
    }
}
```

## Circuit breaker

```rust
use pollyguru::{CircuitBreaker, CircuitError};
use std::time::Duration;

let breaker = CircuitBreaker::new(3, Duration::from_secs(30));

let outcome = breaker.call(|| call_remote_service());
match outcome {
    Ok(value) => println!("{value:?}"),
    Err(CircuitError::Open) => println!("circuit is open, failing fast"),
    Err(CircuitError::Operation(e)) => println!("call failed: {e:?}"),
}
```

After `failure_threshold` consecutive failures the breaker opens and fails fast for the cooldown duration, then half-opens: the next success closes it, the next failure re-opens it.

## API

- `RetryPolicy::new(max_attempts)` — defaults: 100ms base delay, x2 exponential backoff, 30s cap
- `.base_delay` / `.backoff_factor` / `.max_delay` / `.with_jitter` — tune the schedule
- `.retry(operation)` — `operation(attempt_number)` is called until it succeeds or attempts run out; the last error is returned
- `CircuitBreaker::new(failure_threshold, cooldown)` — thread-safe (`Mutex`-based), `.call()`, `.state()`, `.reset()`

## License

Licensed under either of MIT or Apache-2.0, at your option.

Name is a tribute to C#'s [Polly](https://github.com/App-vNext/Polly); this crate is an independent implementation for Rust with zero dependencies.
