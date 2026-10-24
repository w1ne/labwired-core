// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `--log-timestamps` support: one process-global prefix for human-readable
//! stderr lines. `virtual` comes from the machine's committed cycle count and
//! declared `cpu_hz` (deterministic); `real` is wall-clock since process start
//! (explicitly NOT deterministic). Default `none` prints nothing, so every
//! existing artifact and log stream is byte-identical.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// `--log-timestamps` value. `None` is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum LogTimestamps {
    #[default]
    None,
    Virtual,
    Real,
    Both,
}

impl LogTimestamps {
    fn wants_virtual(self) -> bool {
        matches!(self, Self::Virtual | Self::Both)
    }

    fn wants_real(self) -> bool {
        matches!(self, Self::Real | Self::Both)
    }
}

/// The two numbers a prefix may need, already resolved.
pub(crate) struct PrefixNs {
    pub virtual_ns: Option<u64>,
    pub host_ns: Option<u64>,
}

/// Pure formatter: `""` when `None`, else a stable fixed-format bracket;
/// two deterministic runs compare equal byte-for-byte.
pub(crate) fn prefix_now(mode: LogTimestamps, ns: &PrefixNs) -> String {
    match mode {
        LogTimestamps::None => String::new(),
        LogTimestamps::Virtual => format!(
            "[t=virt={}.{:09}s] ",
            ns.virtual_ns.unwrap_or(0) / 1_000_000_000,
            ns.virtual_ns.unwrap_or(0) % 1_000_000_000
        ),
        LogTimestamps::Real => format!(
            "[t=real=+{}.{:09}s] ",
            ns.host_ns.unwrap_or(0) / 1_000_000_000,
            ns.host_ns.unwrap_or(0) % 1_000_000_000
        ),
        LogTimestamps::Both => format!(
            "[t=virt={}.{:09}s|real=+{}.{:09}s] ",
            ns.virtual_ns.unwrap_or(0) / 1_000_000_000,
            ns.virtual_ns.unwrap_or(0) % 1_000_000_000,
            ns.host_ns.unwrap_or(0) / 1_000_000_000,
            ns.host_ns.unwrap_or(0) % 1_000_000_000
        ),
    }
}

/// `cycles * 1e9 / cpu_hz`, integer, floor — the same derived clock the
/// simulator documents for `sim_time_us` (`cycles * 1_000_000 / cpu_hz`),
/// at ns resolution. `hz == 0` (no machine yet) reads as 0.
pub(crate) fn virtual_ns_from_cycles(cycles: u64, hz: u64) -> u64 {
    if hz == 0 {
        return 0;
    }
    u64::try_from((u128::from(cycles) * 1_000_000_000) / u128::from(hz)).unwrap_or(u64::MAX)
}

/// The `result.json` pair for this run, gated by the active mode: virtual for
/// `virtual|both`, host for `real|both`; `(None, None)` for `none`.
pub(crate) fn result_times(
    total_cycles: u64,
    cpu_hz: u64,
    run: std::time::Duration,
) -> (Option<u64>, Option<u64>) {
    let Some(ctx) = CONTEXT.get() else {
        return (None, None);
    };
    (
        ctx.mode
            .wants_virtual()
            .then(|| virtual_ns_from_cycles(total_cycles, cpu_hz)),
        ctx.mode
            .wants_real()
            .then(|| run.as_nanos().min(u128::from(u64::MAX)) as u64),
    )
}

struct Context {
    mode: LogTimestamps,
    start: Instant,
    cycles: AtomicU64,
    hz: AtomicU64,
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

/// Called once from `dispatch` with the mode parsed from the command line
/// (before the tracing subscriber is built). Later calls are ignored.
pub(crate) fn init(mode: LogTimestamps) {
    let _ = CONTEXT.set(Context {
        mode,
        start: Instant::now(),
        cycles: AtomicU64::new(0),
        hz: AtomicU64::new(0),
    });
}

/// The machine's declared core clock; call after the machine is built.
pub(crate) fn set_hz(hz: u64) {
    if let Some(ctx) = CONTEXT.get() {
        ctx.hz.store(hz, Ordering::Relaxed);
    }
}

/// Publish the machine's committed cycle count. One relaxed store; call it
/// once per loop iteration (not per printed line) — the probe pays nothing
/// when the context is absent or the mode is `None`.
#[inline]
pub(crate) fn refresh_cycles(cycles: u64) {
    if let Some(ctx) = CONTEXT.get() {
        if ctx.mode != LogTimestamps::None {
            ctx.cycles.store(cycles, Ordering::Relaxed);
        }
    }
}

/// The current prefix: `""` unless a timestamp mode is active.
pub fn prefix() -> String {
    let Some(ctx) = CONTEXT.get() else {
        return String::new();
    };
    if ctx.mode == LogTimestamps::None {
        return String::new();
    }
    let ns = PrefixNs {
        virtual_ns: ctx.mode.wants_virtual().then(|| {
            virtual_ns_from_cycles(
                ctx.cycles.load(Ordering::Relaxed),
                ctx.hz.load(Ordering::Relaxed),
            )
        }),
        host_ns: ctx
            .mode
            .wants_real()
            .then(|| ctx.start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64),
    };
    prefix_now(ctx.mode, &ns)
}

/// `tracing-subscriber` event formatter that prepends the `[t=…]` prefix,
/// then delegates to the subscriber's default format. Empty prefix (mode
/// `none`, or any non-run/test command) leaves existing output byte-identical.
///
/// With a prefix active the delegated format runs `without_time()`: the
/// `[t=…]` header IS the timestamp, and the default format's own RFC 3339
/// wall-clock stamp would otherwise sit inside the `virtual` stream and make
/// two runs of the same firmware differ byte-for-byte. `none` keeps the
/// default stamp, exactly as before the flag existed.
pub struct LogTsFormat;

impl<S, N> tracing_subscriber::fmt::FormatEvent<S, N> for LogTsFormat
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> tracing_subscriber::fmt::FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let prefix = prefix();
        if prefix.is_empty() {
            return tracing_subscriber::fmt::format::Format::default()
                .format_event(ctx, writer, event);
        }
        write!(writer, "{prefix}")?;
        tracing_subscriber::fmt::format::Format::default()
            .without_time()
            .format_event(ctx, writer, event)
    }
}

/// `eprintln!` with the timestamp prefix. Crate-internal: use for
/// human-readable event lines emitted while a run is active (progress,
/// bridges, verdicts). Startup and usage errors keep plain `eprintln!` —
/// there is no simulation clock yet.
macro_rules! eventln {
    ($($arg:tt)*) => {
        eprintln!("{}{}", $crate::log_ts::prefix(), format_args!($($arg)*))
    };
}
pub(crate) use eventln;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_now_formats_each_mode() {
        let ns = PrefixNs {
            virtual_ns: Some(1_500_000_000),
            host_ns: Some(123_456_789),
        };
        assert_eq!(prefix_now(LogTimestamps::None, &ns), "");
        assert_eq!(
            prefix_now(LogTimestamps::Virtual, &ns),
            "[t=virt=1.500000000s] "
        );
        assert_eq!(
            prefix_now(LogTimestamps::Real, &ns),
            "[t=real=+0.123456789s] "
        );
        assert_eq!(
            prefix_now(LogTimestamps::Both, &ns),
            "[t=virt=1.500000000s|real=+0.123456789s] "
        );

        let carry = PrefixNs {
            virtual_ns: Some(999_999_999),
            host_ns: Some(1),
        };
        assert_eq!(
            prefix_now(LogTimestamps::Virtual, &carry),
            "[t=virt=0.999999999s] "
        );
        assert_eq!(
            prefix_now(LogTimestamps::Both, &carry),
            "[t=virt=0.999999999s|real=+0.000000001s] "
        );
    }

    #[test]
    fn clap_parses_every_mode_and_rejects_typos() {
        use clap::Parser;
        for (s, want) in [
            ("none", LogTimestamps::None),
            ("virtual", LogTimestamps::Virtual),
            ("real", LogTimestamps::Real),
            ("both", LogTimestamps::Both),
        ] {
            let cli = crate::Cli::try_parse_from([
                "labwired",
                "test",
                "--script",
                "s.yaml",
                "--log-timestamps",
                s,
            ])
            .unwrap();
            match cli.command {
                Some(crate::dispatch::Commands::Test(a)) => {
                    assert_eq!(a.log_timestamps, want)
                }
                _ => panic!("expected test command"),
            }
        }
        let err = crate::Cli::try_parse_from([
            "labwired",
            "test",
            "--script",
            "s.yaml",
            "--log-timestamps",
            "faul",
        ]);
        assert!(err.is_err(), "typo must not parse");

        let cli = crate::Cli::try_parse_from(["labwired", "test", "--script", "s.yaml"]).unwrap();
        match cli.command {
            Some(crate::dispatch::Commands::Test(a)) => {
                assert_eq!(a.log_timestamps, LogTimestamps::None)
            }
            _ => panic!("expected test command"),
        }

        let cli = crate::Cli::try_parse_from([
            "labwired",
            "run",
            "--firmware",
            "f.elf",
            "--chip",
            "c.yaml",
            "--log-timestamps",
            "both",
        ])
        .unwrap();
        match cli.command {
            Some(crate::dispatch::Commands::Run(a)) => {
                assert_eq!(a.log_timestamps, LogTimestamps::Both)
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn virtual_ns_from_cycles_matches_derived_clock() {
        assert_eq!(virtual_ns_from_cycles(4000, 4_000_000), 1_000_000);
        assert_eq!(virtual_ns_from_cycles(1, 3_000_000), 333);
        assert_eq!(virtual_ns_from_cycles(0, 4_000_000), 0);
        assert_eq!(virtual_ns_from_cycles(10, 0), 0);
    }
}
