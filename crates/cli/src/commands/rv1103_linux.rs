//! Explicit reduced RV1103 Linux gate; ordinary run modes are untouched.
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
pub struct Rv1103LinuxArgs {
    /// Directory produced by scripts/prepare-rv1103-linux.py.
    #[arg(long)]
    pub artifacts: PathBuf,
    /// Full bounded console, artifact/backend hashes and acceptance evidence.
    #[arg(long)]
    pub receipt: PathBuf,
    /// Functional instruction tick calibration; default is 1.2 GHz, not physical timing.
    #[arg(long, default_value_t = 1_200_000_000)]
    pub cpu_hz: u64,
    #[arg(long, default_value_t = 24_000_000_000)]
    pub max_cycles: u64,
    #[arg(long, default_value_t = 2_000_000_000)]
    pub max_retirements: u64,
    /// Machine fuel also counts cycles skipped by idle fast-forward.
    #[arg(long, default_value_t = 24_000_000_000)]
    pub max_fuel: u64,
    #[arg(long, default_value_t = 1800)]
    pub wall_seconds: u64,
    /// Hard limit on accumulated UART2 bytes, at most 64 KiB.
    #[arg(long, default_value_t = 65536)]
    pub console_bytes: usize,
}

#[cfg(any(feature = "qemu-arm-system", test))]
#[derive(Debug, PartialEq)]
pub(super) struct GuestEvidence {
    pub(super) uptime_before: f64,
    pub(super) uptime_after: f64,
}
#[cfg(any(feature = "qemu-arm-system", test))]
pub(super) fn acceptance(console: &str) -> Option<GuestEvidence> {
    if console.contains("Kernel panic") || console.contains("LW_LINUX_BOOT_FAILURE") {
        return None;
    }
    for required in [
        "Linux version 6.18.56-0-lts ",
        "Run /init as init process",
        "LW_LINUX_MOUNTS proc=ok sysfs=ok devtmpfs=ok",
        "LW_LINUX_PID1 pid=1 kernel=6.18.56-0-lts machine=armv7l",
        "processor",
        "CPU architecture",
        "MemTotal:",
    ] {
        if !console.contains(required) {
            return None;
        }
    }
    fn stamp(console: &str, prefix: &str) -> Option<f64> {
        let mut found = console
            .lines()
            .filter_map(|line| line.trim_end_matches('\r').strip_prefix(prefix));
        let value: f64 = found.next()?.parse().ok()?;
        if found.next().is_some() || !value.is_finite() || value < 0.0 {
            return None;
        }
        Some(value)
    }
    if !console
        .lines()
        .any(|line| line.trim_end_matches('\r') == "LW_LINUX_BOOT_ACCEPTED")
    {
        return None;
    }
    let before = stamp(console, "LW_LINUX_UPTIME_BEFORE=")?;
    let after = stamp(console, "LW_LINUX_UPTIME_AFTER=")?;
    if after - before < 0.8 {
        return None;
    }
    Some(GuestEvidence {
        uptime_before: before,
        uptime_after: after,
    })
}

pub fn run(args: Rv1103LinuxArgs) -> ExitCode {
    #[cfg(not(feature = "qemu-arm-system"))]
    {
        let _ = args;
        eprintln!("RV1103 Linux requires a native build with --features qemu-arm-system (and event-scheduler)");
        ExitCode::FAILURE
    }
    #[cfg(feature = "qemu-arm-system")]
    {
        run_native(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CONSOLE: &str = "Linux version 6.18.56-0-lts (builder)\nRun /init as init process\nLW_LINUX_MOUNTS proc=ok sysfs=ok devtmpfs=ok\nLW_LINUX_PID1 pid=1 kernel=6.18.56-0-lts machine=armv7l\nprocessor : 0\nCPU architecture: 7\nMemTotal: 40000 kB\nLW_LINUX_UPTIME_BEFORE=1.00\nLW_LINUX_UPTIME_AFTER=2.01\nLW_LINUX_BOOT_ACCEPTED\n";
    #[test]
    fn acceptance_requires_complete_real_console_contract() {
        assert_eq!(
            acceptance(CONSOLE),
            Some(GuestEvidence {
                uptime_before: 1.00,
                uptime_after: 2.01
            })
        );
        for token in [
            "Linux version",
            "Run /init",
            "LW_LINUX_MOUNTS",
            "LW_LINUX_PID1",
            "processor",
            "CPU architecture",
            "MemTotal",
            "LW_LINUX_UPTIME_BEFORE",
            "LW_LINUX_UPTIME_AFTER",
            "LW_LINUX_BOOT_ACCEPTED",
        ] {
            assert!(
                acceptance(&CONSOLE.replace(token, "REMOVED")).is_none(),
                "missing {token}"
            );
        }
    }
    #[test]
    fn panic_failure_short_sleep_and_wrong_identity_refuse_marker() {
        for bad in [
            CONSOLE.replace("2.01", "1.10"),
            CONSOLE.replace("6.18.56-0-lts", "6.1-other"),
            CONSOLE.replace("pid=1", "pid=2"),
            CONSOLE.replace("machine=armv7l", "machine=x86_64"),
            format!("{CONSOLE}Kernel panic - not syncing"),
            format!("{CONSOLE}LW_LINUX_BOOT_FAILURE: timer"),
        ] {
            assert!(acceptance(&bad).is_none());
        }
        assert!(acceptance("LW_LINUX_BOOT_ACCEPTED").is_none());
    }
}

#[cfg(feature = "qemu-arm-system")]
fn run_native(args: Rv1103LinuxArgs) -> ExitCode {
    use labwired_core::bus::SystemBus;
    use labwired_core::memory::LinearMemory;
    use labwired_core::system::rv1103_linux::*;
    use labwired_core::{AdvanceRequest, AdvanceStop, Cpu, Machine};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    let started = Instant::now();
    let mut bus = SystemBus::empty();
    let event_scheduler = bus.cycle_clock.scheduler_enabled();
    let mut receipt = serde_json::json!({
        "schema":1, "board":BOARD, "mode":"reduced-direct-zImage",
        "status":"FAIL", "guest_boot":"NOT_ACCEPTED", "physical_acceptance":"UNAVAILABLE",
        "source_revision":BACKEND_SOURCE_REVISION, "source_archive_sha256":BACKEND_SOURCE_ARCHIVE_SHA256,
        "compiled_backend_sha256":compiled_backend_sha256(),
        "cpu_hz":args.cpu_hz, "cpu_hz_default":CPU_HZ, "cpu_hz_override":args.cpu_hz != CPU_HZ,
        "timing":"one retired instruction tick; functional calibration, not physical timing",
        "timer_hz":TIMER_HZ, "idle_fast_forward":true, "event_scheduler":event_scheduler,
        "limits":{"cycles":args.max_cycles,"retirements":args.max_retirements,"fuel":args.max_fuel,"wall_seconds":args.wall_seconds,"console_bytes":args.console_bytes,"advance_window":4096,"batch_cap":64},
        "reductions":["direct zImage; no ROM, SD or vendor BSP boot", "single Cortex-A7; no CRU, reset, pinctrl, DMA or FIQ debugger", "no I2C, ESP, RF or smartbasket application acceptance"],
        "console":"", "cycles":0, "retirements_completed_windows":0, "fuel_completed_windows":0, "last_pc":null
    });
    let outcome = (|| -> Result<(), String> {
        if !event_scheduler {
            return Err(
                "RV1103 Linux requires event-scheduler for explicit idle fast-forward".into(),
            );
        }
        if args.cpu_hz == 0
            || args.max_cycles == 0
            || args.max_retirements == 0
            || args.max_fuel == 0
            || args.wall_seconds == 0
            || args.console_bytes == 0
            || args.console_bytes > 65536
        {
            return Err("limits must be positive; console limit must be <=64 KiB".into());
        }
        let artifacts = LinuxArtifacts::read(&args.artifacts).map_err(|e| e.to_string())?;
        receipt["artifacts"] = serde_json::to_value(&artifacts.manifest.artifacts).unwrap();
        receipt["actual_input_sha256"] = serde_json::json!({"zImage":sha256(&artifacts.kernel),"rv1103-direct.dtb":sha256(&artifacts.dtb),"initramfs.cpio":sha256(&artifacts.initramfs),"kernel.config":sha256(&artifacts.config)});
        bus.ram = LinearMemory::new(RAM_BYTES as usize, 0);
        bus.cpu_hz = args.cpu_hz;
        let loaded = load_linux(&mut bus, artifacts).map_err(|e| e.to_string())?;
        let sink = Arc::new(Mutex::new(Vec::new()));
        if !bus.attach_uart_tx_sink_named("uart2", sink.clone(), false) {
            return Err("UART2 console attachment failed".into());
        }
        let mut machine = Machine::new(loaded.cpu, bus);
        machine.config.idle_fast_forward_enabled = true;
        let mut console = Vec::new();
        let mut retired = 0u64;
        let mut fuel = 0u64;
        loop {
            receipt["last_pc"] = serde_json::json!(format!("{:#010x}", machine.cpu.get_pc()));
            receipt["cycles"] = serde_json::json!(machine.total_cycles);
            receipt["retirements_completed_windows"] = serde_json::json!(retired);
            receipt["fuel_completed_windows"] = serde_json::json!(fuel);
            receipt["idle_cycles"] = serde_json::json!(machine.idle_fast_forward_cycles_skipped);
            receipt["console"] = serde_json::json!(String::from_utf8_lossy(&console));
            let text = String::from_utf8_lossy(&console);
            if text.contains("Kernel panic") || text.contains("LW_LINUX_BOOT_FAILURE") {
                return Err("kernel panic or userspace failure marker".into());
            }
            if let Some(evidence) = acceptance(&text) {
                receipt["guest_evidence"] = serde_json::json!({"kernel":"6.18.56-0-lts","pid":1,"machine":"armv7l","proc_sys_devtmpfs":"mounted by PID1","uptime_before":evidence.uptime_before,"uptime_after":evidence.uptime_after,"uptime_delta":evidence.uptime_after-evidence.uptime_before,"sleep":"real guest sleep syscall"});
                return Ok(());
            }
            if console.len() >= args.console_bytes {
                return Err("console byte limit".into());
            }
            if started.elapsed().as_secs() >= args.wall_seconds {
                return Err("wall timeout".into());
            }
            if machine.total_cycles >= args.max_cycles {
                return Err("cycle limit".into());
            }
            if retired >= args.max_retirements {
                return Err("retirement limit".into());
            }
            if fuel >= args.max_fuel {
                return Err("fuel limit".into());
            }
            let window = 4096u64
                .min(args.max_retirements - retired)
                .min(args.max_fuel - fuel);
            let request = AdvanceRequest::run(Some(window))
                .with_cycle_limit(4096.min(args.max_cycles - machine.total_cycles))
                .with_batch_cap(std::num::NonZeroU32::new(64).unwrap());
            let result = machine.advance(request);
            let mut bytes = sink.lock().unwrap();
            let overflow = bytes.len() > args.console_bytes - console.len();
            let remaining = args.console_bytes - console.len();
            console.extend(bytes.drain(..).take(remaining));
            drop(bytes);
            if overflow {
                if let Ok(report) = &result {
                    receipt["retirements_completed_windows"] =
                        serde_json::json!(retired + report.primary_steps);
                    receipt["fuel_completed_windows"] =
                        serde_json::json!(fuel + report.fuel_consumed);
                } else {
                    receipt["partial_error_accounting"] = "completed windows only; failed native boundary may have partially progressed".into();
                }
                receipt["console"] = serde_json::json!(String::from_utf8_lossy(&console));
                receipt["last_pc"] = serde_json::json!(format!("{:#010x}", machine.cpu.get_pc()));
                receipt["cycles"] = serde_json::json!(machine.total_cycles);
                return Err("console byte limit (additional UART output refused)".into());
            }
            match result {
                Ok(report) => {
                    retired += report.primary_steps;
                    fuel += report.fuel_consumed;
                    receipt["retirements_completed_windows"] = serde_json::json!(retired);
                    receipt["fuel_completed_windows"] = serde_json::json!(fuel);
                    if report.stop == AdvanceStop::NoProgress {
                        receipt["console"] = serde_json::json!(String::from_utf8_lossy(&console));
                        receipt["last_pc"] =
                            serde_json::json!(format!("{:#010x}", machine.cpu.get_pc()));
                        receipt["cycles"] = serde_json::json!(machine.total_cycles);
                        return Err("Machine NoProgress".into());
                    }
                }
                Err(error) => {
                    receipt["console"] = serde_json::json!(String::from_utf8_lossy(&console));
                    receipt["last_pc"] =
                        serde_json::json!(format!("{:#010x}", machine.cpu.get_pc()));
                    receipt["cycles"] = serde_json::json!(machine.total_cycles);
                    receipt["partial_error_accounting"]="completed windows only; a failed native boundary may have partially progressed".into();
                    return Err(error.to_string());
                }
            }
        }
    })();
    receipt["wall_elapsed_seconds"] = serde_json::json!(started.elapsed().as_secs_f64());
    match &outcome {
        Ok(()) => {
            receipt["status"] = "PASS".into();
            receipt["guest_boot"] = "ACCEPTED".into();
            receipt["stop"] = "userspace acceptance".into();
        }
        Err(reason) => receipt["stop"] = reason.clone().into(),
    }
    // Create-new protects previous receipts and rejects symlink destinations.
    let written = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args.receipt)?;
        f.write_all(serde_json::to_string_pretty(&receipt).unwrap().as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()
    })();
    if let Err(error) = written {
        eprintln!("receipt write failed ({}): {error}", args.receipt.display());
        eprintln!("{receipt}");
        return ExitCode::FAILURE;
    }
    match outcome {
        Ok(()) => {
            println!(
                "RV1103 Linux guest accepted; receipt {}",
                args.receipt.display()
            );
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!(
                "RV1103 Linux failed: {reason}; receipt {}",
                args.receipt.display()
            );
            ExitCode::FAILURE
        }
    }
}
