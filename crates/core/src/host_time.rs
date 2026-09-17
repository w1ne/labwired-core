// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Host wall-clock policy for a simulation run.
//!
//! Guest time stays `1 insn = 1 cycle` at `bus.cpu_hz`. This module only
//! decides whether the *host* sleeps so virtual time does not run ahead of
//! wall time.

use std::time::Duration;

/// Host policy: run as fast as the host can, or sleep to track wall time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostTimeMode {
    /// Current behavior: never sleep.
    #[default]
    MaxSpeed,
    /// After committed work, sleep when virtual time is ≥ 1 ms ahead of wall.
    Realtime,
}

impl HostTimeMode {
    /// CLI / config spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MaxSpeed => "max-speed",
            Self::Realtime => "realtime",
        }
    }
}

impl std::fmt::Display for HostTimeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for HostTimeMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "max-speed" => Ok(Self::MaxSpeed),
            "realtime" => Ok(Self::Realtime),
            other => Err(format!(
                "unknown time mode '{other}', expected max-speed or realtime"
            )),
        }
    }
}

/// Injectable host clock. Tests use a fake; production uses [`StdHostClock`].
pub trait HostClock: Send {
    /// Monotonic time from this clock's origin.
    fn now(&self) -> Duration;
    /// Block until `duration` has elapsed on this clock.
    fn sleep(&self, duration: Duration);
}

/// `std::time::Instant` clock. Origin is construction of this value.
///
/// wasm32-unknown-unknown has NO monotonic time: `Instant::now()` panics with
/// "time not implemented on this platform". The browser builds every machine
/// with this clock, so the `now()`/`sleep()` pair must be wasm-safe — sleeping
/// was already cfg-gated, and the origin/elapsed side is gated the same way.
/// On wasm `now()` is always zero, which makes Realtime pacing a no-op instead
/// of a trap (there is no wall clock to pace against in the browser anyway).
#[derive(Debug, Clone, Copy)]
pub struct StdHostClock {
    #[cfg(not(target_arch = "wasm32"))]
    origin: std::time::Instant,
}

impl StdHostClock {
    pub fn new() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            origin: std::time::Instant::now(),
        }
    }
}

impl Default for StdHostClock {
    fn default() -> Self {
        Self::new()
    }
}

impl HostClock for StdHostClock {
    fn now(&self) -> Duration {
        #[cfg(target_arch = "wasm32")]
        {
            Duration::ZERO
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.origin.elapsed()
        }
    }

    fn sleep(&self, duration: Duration) {
        if duration.is_zero() {
            return;
        }
        // wasm32 has `std::thread::sleep` but it panics at runtime.
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::sleep(duration);
    }
}

/// Sleep slack: only catch up when virtual time is at least this far ahead.
const REALTIME_SLACK: Duration = Duration::from_millis(1);

/// Convert committed guest cycles at `cpu_hz` into a host duration.
fn virtual_duration(cycles: u64, cpu_hz: u64) -> Duration {
    debug_assert!(cpu_hz != 0);
    let nanos = (u128::from(cycles).saturating_mul(1_000_000_000)) / u128::from(cpu_hz);
    Duration::from_nanos(nanos.min(u128::from(u64::MAX)) as u64)
}

/// If `mode` is [`HostTimeMode::Realtime`] and virtual time (`cycles/cpu_hz`)
/// is ahead of wall time by ≥ 1 ms, sleep the difference.
///
/// `cpu_hz == 0` is a no-op (cannot convert cycles to time).
pub fn pace(
    mode: HostTimeMode,
    cpu_hz: u64,
    start_cycles: u64,
    now_cycles: u64,
    start_wall: Duration,
    now_wall: Duration,
    clock: &dyn HostClock,
) {
    if mode != HostTimeMode::Realtime || cpu_hz == 0 {
        return;
    }
    let virt = virtual_duration(now_cycles.saturating_sub(start_cycles), cpu_hz);
    let wall = now_wall.saturating_sub(start_wall);
    if virt >= wall.saturating_add(REALTIME_SLACK) {
        clock.sleep(virt.saturating_sub(wall));
    }
}

/// Test clock: records sleeps and advances `now` by the slept amount.
///
/// `Clone` shares the inner state so a handle can inspect sleeps after the
/// clock is moved into [`crate::Machine::with_host_clock`].
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct FakeClock {
    inner: std::sync::Arc<std::sync::Mutex<FakeClockInner>>,
}

#[cfg(test)]
struct FakeClockInner {
    now: Duration,
    sleeps: Vec<Duration>,
}

#[cfg(test)]
impl FakeClock {
    pub(crate) fn new() -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(FakeClockInner {
                now: Duration::ZERO,
                sleeps: Vec::new(),
            })),
        }
    }

    pub(crate) fn sleeps(&self) -> Vec<Duration> {
        self.inner.lock().expect("fake clock").sleeps.clone()
    }
}

#[cfg(test)]
impl HostClock for FakeClock {
    fn now(&self) -> Duration {
        self.inner.lock().expect("fake clock").now
    }

    fn sleep(&self, duration: Duration) {
        let mut inner = self.inner.lock().expect("fake clock");
        inner.sleeps.push(duration);
        inner.now = inner.now.saturating_add(duration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SimulationConfig;

    fn pace_with(mode: HostTimeMode, cpu_hz: u64, cycles: u64, wall: Duration, clock: &FakeClock) {
        pace(mode, cpu_hz, 0, cycles, Duration::ZERO, wall, clock);
    }

    #[test]
    fn realtime_million_cycles_at_1mhz_sleeps_about_one_second() {
        let clock = FakeClock::new();
        pace_with(
            HostTimeMode::Realtime,
            1_000_000,
            1_000_000,
            Duration::ZERO,
            &clock,
        );
        let sleeps = clock.sleeps();
        assert_eq!(
            sleeps.len(),
            1,
            "expected one catch-up sleep, got {sleeps:?}"
        );
        let slept = sleeps[0];
        let one_sec = Duration::from_secs(1);
        assert!(
            slept <= one_sec,
            "sleep {slept:?} exceeded virtual time {one_sec:?}"
        );
        assert!(
            slept >= one_sec.saturating_sub(REALTIME_SLACK),
            "sleep {slept:?} dropped more than 1ms slack from {one_sec:?}"
        );
    }

    #[test]
    fn max_speed_never_sleeps() {
        let clock = FakeClock::new();
        pace_with(
            HostTimeMode::MaxSpeed,
            1_000_000,
            1_000_000,
            Duration::ZERO,
            &clock,
        );
        assert!(clock.sleeps().is_empty());
    }

    #[test]
    fn realtime_cpu_hz_zero_is_noop() {
        let clock = FakeClock::new();
        pace_with(HostTimeMode::Realtime, 0, 1_000_000, Duration::ZERO, &clock);
        assert!(clock.sleeps().is_empty());
    }

    #[test]
    fn host_time_mode_default_is_max_speed() {
        assert_eq!(HostTimeMode::default(), HostTimeMode::MaxSpeed);
        assert_eq!(
            SimulationConfig::default().host_time_mode,
            HostTimeMode::MaxSpeed
        );
        // Other fields are required; omitting only `host_time_mode` must default.
        let cfg: SimulationConfig = serde_json::from_str(
            r#"{
                "decode_cache_enabled": true,
                "peripheral_tick_interval": 1,
                "optimized_bus_access": true,
                "batch_mode_enabled": true,
                "idle_fast_forward_enabled": false
            }"#,
        )
        .unwrap();
        assert_eq!(cfg.host_time_mode, HostTimeMode::MaxSpeed);
    }
}
