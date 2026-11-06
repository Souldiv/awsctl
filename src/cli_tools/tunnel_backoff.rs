use std::time::Duration;

const FIRST_DELAY: Duration = Duration::from_secs(1);
const MAX_DELAY: Duration = Duration::from_secs(5 * 60);
pub const HEALTHY_SESSION: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    pub fn delay(self) -> Duration {
        let factor = 2u32.saturating_pow(self.failures);
        FIRST_DELAY.saturating_mul(factor).min(MAX_DELAY)
    }

    pub fn failed(self) -> Self {
        Backoff {
            failures: self.failures.saturating_add(1),
        }
    }

    pub fn after_session(self, lasted: Duration) -> Self {
        if lasted >= HEALTHY_SESSION {
            Backoff::default()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Backoff, HEALTHY_SESSION, MAX_DELAY};
    use std::time::Duration;

    #[test]
    fn doubles_the_delay_after_each_failure() {
        let delays: Vec<u64> =
            std::iter::successors(Some(Backoff::default()), |b| Some(b.failed()))
                .take(5)
                .map(|backoff| backoff.delay().as_secs())
                .collect();

        assert_eq!(delays, [1, 2, 4, 8, 16]);
    }

    #[test]
    fn caps_the_delay() {
        let backoff = (0..40).fold(Backoff::default(), |backoff, _| backoff.failed());

        assert_eq!(backoff.delay(), MAX_DELAY);
    }

    #[test]
    fn starts_over_after_a_healthy_session() {
        let backoff = Backoff::default().failed().failed().failed();

        assert_eq!(backoff.after_session(HEALTHY_SESSION), Backoff::default());
    }

    #[test]
    fn keeps_backing_off_after_a_short_session() {
        let backoff = Backoff::default().failed().failed();

        assert_eq!(backoff.after_session(Duration::from_secs(5)), backoff);
    }
}
