//! Explicit shared Linux basket acceptance; local native dogfood only.
use clap::Parser;
use std::{path::PathBuf, process::ExitCode};
#[derive(Parser, Debug)]
pub struct Rv1103BasketArgs {
    #[arg(long)]
    pub artifacts: PathBuf,
    #[arg(long)]
    pub receipt: PathBuf,
    #[arg(long, default_value_t = 4_000_000_000)]
    pub max_cycles: u64,
    #[arg(long, default_value_t = 2_000_000_000)]
    pub max_retirements: u64,
    #[arg(long, default_value_t = 4_000_000_000)]
    pub max_fuel: u64,
    #[arg(long, default_value_t = 3600)]
    pub wall_seconds: u64,
    #[arg(long, default_value_t = 65536)]
    pub console_bytes: usize,
}
impl Rv1103BasketArgs {
    fn validate(&self) -> Result<(), String> {
        if self.max_cycles == 0
            || self.max_cycles > 10_000_000_000
            || self.max_retirements == 0
            || self.max_retirements > 4_000_000_000
            || self.max_fuel == 0
            || self.max_fuel > 10_000_000_000
            || self.wall_seconds == 0
            || self.wall_seconds > 7200
            || self.console_bytes == 0
            || self.console_bytes > 65536
        {
            Err("positive bounded limits required: cycles/fuel<=10B, retirements<=4B, wall<=7200s, console<=64KiB".into())
        } else {
            Ok(())
        }
    }
}

#[cfg(feature = "qemu-arm-system")]
#[path = "basket_native.rs"]
mod native;

pub fn run(args: Rv1103BasketArgs) -> ExitCode {
    use labwired_core::system::{rv1103_basket, rv1103_linux};
    let mut receipt = serde_json::json!({
        "schema":1,"profile":rv1103_basket::PROFILE,"status":"FAIL","console":"",
        "physical_acceptance":"UNAVAILABLE","guest_acceptance":"NOT_RUN",
        "source_revision":rv1103_linux::BACKEND_SOURCE_REVISION,
        "compiled_backend_sha256":rv1103_linux::compiled_backend_sha256(),
        "trusted_prepared_sha256":rv1103_basket::TRUSTED_MANIFEST_SHA256,
        "canonical_manifest_sha256":rv1103_linux::sha256(rv1103_basket::CANONICAL_MANIFEST.as_bytes()),
        "canonical_export_sha256":rv1103_linux::sha256(rv1103_basket::CANONICAL_EXPORT.as_bytes()),
        "cpu_hz":rv1103_basket::FUNCTIONAL_CPU_HZ,"timer_hz":rv1103_linux::TIMER_HZ,
        "limits":{"cycles":args.max_cycles,"retirements":args.max_retirements,"fuel":args.max_fuel,
            "wall_seconds":args.wall_seconds,"console_bytes_per_fixture":args.console_bytes,"advance_window":4096,"batch_cap":64},
        "timing":"functional 24MHz instruction tick calibration; not physical/realtime",
        "supply_proxy":{"classification":"SYNTHETIC execution proxy, no RV1103 physical threshold claim",
            "raw_host_rail":"plant VBUS5V, not CPU VDD","healthy_raw5_to_functional_vdd":3.3,
            "por_rising_v":3.0,"pdr_falling_v":2.7,"external3v3_minimum_v":2.7},
        "reductions":["direct zImage, no ROM reboot","behavioral ESP-AT peer, no RF/vendor firmware","external3v3 disconnect is a declared carrier override of healthy actual plant output"]
    });
    if let Ok(exe) = std::env::current_exe().and_then(std::fs::read) {
        receipt["cli_sha256"] = rv1103_linux::sha256(&exe).into();
    }
    let result = args.validate().and_then(|()| {
        #[cfg(not(feature = "qemu-arm-system"))]
        {
            Err(
                "RV1103 basket requires qemu-arm-system and event-scheduler native features"
                    .to_owned(),
            )
        }
        #[cfg(feature = "qemu-arm-system")]
        {
            native::execute(&args, &mut receipt)
        }
    });
    match &result {
        Ok(()) => {
            receipt["status"] = "PASS".into();
            receipt["guest_acceptance"] = "ACCEPTED".into();
            receipt["stop"] = "all independent gates passed".into();
        }
        Err(e) => receipt["stop"] = e.clone().into(),
    }
    let bytes = serde_json::to_vec_pretty(&receipt).expect("receipt serialization");
    use std::io::Write;
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.receipt)
        .and_then(|mut f| {
            f.write_all(&bytes)?;
            f.write_all(b"\n")?;
            f.sync_all()
        });
    if let Err(e) = written {
        eprintln!(
            "Cannot preserve new receipt {}: {e}\n{}",
            args.receipt.display(),
            String::from_utf8_lossy(&bytes)
        );
        return ExitCode::FAILURE;
    }
    if let Err(e) = result {
        eprintln!(
            "RV1103 basket failed: {e}; full bounded console/evidence: {}",
            args.receipt.display()
        );
        ExitCode::FAILURE
    } else {
        println!("RV1103 basket accepted: {}", args.receipt.display());
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args() -> Rv1103BasketArgs {
        Rv1103BasketArgs::parse_from(["basket", "--artifacts", "a", "--receipt", "b"])
    }
    #[cfg(not(feature = "qemu-arm-system"))]
    #[test]
    fn default_backend_refusal_preserves_a_complete_failure_receipt() {
        let mut a = args();
        a.receipt = std::env::temp_dir().join(format!(
            "basket-default-refusal-{}.json",
            std::process::id()
        ));
        assert!(!a.receipt.exists());
        let path = a.receipt.clone();
        assert_eq!(run(a), ExitCode::FAILURE);
        let result = std::fs::read(&path);
        if result.is_ok() {
            std::fs::remove_file(&path).unwrap();
        }
        let receipt: serde_json::Value =
            serde_json::from_slice(&result.expect("every failure must retain a receipt")).unwrap();
        assert_eq!(receipt["status"], "FAIL");
        assert_eq!(receipt["console"], "");
        assert!(receipt["stop"]
            .as_str()
            .unwrap()
            .contains("qemu-arm-system"));
    }
    #[test]
    fn positive_bounded_limits_accept_and_zero_or_console_overflow_reject() {
        assert!(args().validate().is_ok());
        for which in 0..6 {
            let mut a = args();
            match which {
                0 => a.max_cycles = 0,
                1 => a.max_retirements = 0,
                2 => a.max_fuel = 0,
                3 => a.wall_seconds = 0,
                4 => a.console_bytes = 0,
                _ => a.console_bytes = 65537,
            }
            assert!(a.validate().is_err());
        }
    }
}
