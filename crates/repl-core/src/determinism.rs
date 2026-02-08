use chrono::{DateTime, Duration, Utc};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

/// A random number generator that can be seeded for deterministic output.
pub struct DeterministicRng {
    rng: ChaCha8Rng,
}

impl DeterministicRng {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed),
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }
}

/// A clock that can be frozen and advanced manually for deterministic time.
pub struct Clock {
    frozen_time: Option<DateTime<Utc>>,
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    pub fn new() -> Self {
        Self { frozen_time: None }
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.frozen_time.unwrap_or_else(Utc::now)
    }

    pub fn freeze(&mut self, time: DateTime<Utc>) {
        self.frozen_time = Some(time);
    }

    pub fn advance(&mut self, duration: Duration) {
        if let Some(time) = &mut self.frozen_time {
            *time += duration;
        }
    }
}
