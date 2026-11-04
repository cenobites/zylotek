use std::time::{SystemTime, UNIX_EPOCH};

use zylotek_domain::asks::value_objects::Timestamp;

/// Where the use cases get the time from, so tests can fix it. The domain never reads a clock.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());

        Timestamp(u64::try_from(millis).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_is_after_2020() {
        assert!(SystemClock.now() > Timestamp(1_577_836_800_000));
    }
}
