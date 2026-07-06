//! The system-clock adapter implementing the [`Clock`] domain port.

use std::time::SystemTime;

use domain::Clock;

/// A [`Clock`] backed by the operating system's wall clock.
///
/// The production adapter; tests inject a fake clock instead so lockout timing
/// is deterministic without sleeping.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}
