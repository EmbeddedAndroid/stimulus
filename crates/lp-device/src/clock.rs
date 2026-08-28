use std::time::{Duration, Instant};

pub trait Clock {
    fn elapsed(&self) -> Duration;
    fn sleep(&mut self, duration: Duration);
    /// Account for one blocking transport poll in deterministic tests. Real
    /// USB reads are paced by the FTDI latency timer, so WallClock needs no
    /// synthetic delay.
    fn io_poll(&mut self) {}
}

pub struct WallClock {
    epoch: Instant,
}
impl Default for WallClock {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }
}
impl Clock for WallClock {
    fn elapsed(&self) -> Duration {
        self.epoch.elapsed()
    }
    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VirtualClock {
    elapsed: Duration,
}
impl Clock for VirtualClock {
    fn elapsed(&self) -> Duration {
        self.elapsed
    }
    fn sleep(&mut self, duration: Duration) {
        self.elapsed = self.elapsed.saturating_add(duration);
    }
    fn io_poll(&mut self) {
        self.elapsed = self.elapsed.saturating_add(Duration::from_millis(1));
    }
}
