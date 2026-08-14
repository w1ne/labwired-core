//! Report a CLI panic to `POST /v1/telemetry/failure`.
//!
//! Until now a panic on a user's machine produced a backtrace on their terminal
//! and nothing anywhere else. The playground got a reporter first, which left
//! the CLI — the surface that runs on the widest range of machines, and the one
//! the installer just spent a release learning to deliver — as the silent one.
//!
//! What leaves the machine is a fixed set of enumerated dimensions plus a
//! fingerprint. Never the panic message: it routinely carries a file path, a
//! chip name a customer has not announced, or a string from the firmware under
//! test. The fingerprint is a digest of the panic's source location, which
//! answers "is this the same crash as those other forty?" and nothing else.
//!
//! The hook still runs under `panic = "abort"` — abort happens after hooks —
//! so this works in release builds, where `catch_unwind` would not.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// One report per process. A panic inside a panic must not become a loop.
static REPORTED: AtomicBool = AtomicBool::new(false);

fn opted_out() -> bool {
    if std::env::var("LABWIRED_TELEMETRY").is_ok_and(|v| v == "0") {
        return true;
    }
    if std::env::var("DO_NOT_TRACK").is_ok_and(|v| !v.is_empty()) {
        return true;
    }
    // CI panics are our own runs; they are already visible as a red job, and a
    // matrix build would otherwise report the same fingerprint fifty times.
    // `LABWIRED_TELEMETRY=1` opts a CI machine back in deliberately.
    std::env::var("CI").is_ok_and(|v| !v.is_empty())
        && !std::env::var("LABWIRED_TELEMETRY").is_ok_and(|v| v == "1")
}

fn telemetry_url() -> String {
    std::env::var("LABWIRED_TELEMETRY_URL")
        .unwrap_or_else(|_| "https://api.labwired.com/v1/telemetry/failure".to_string())
}

/// Which OS and architecture this binary was built for, in the same vocabulary
/// the release archives and `install.sh` use.
fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("macos", "aarch64") => "darwin-aarch64",
        ("windows", "x86_64") => "windows-x86_64",
        _ => "unknown",
    }
}

/// A stable, non-reversible id for "this panic": FNV-1a over the source
/// location only. `src/commands/run.rs:412:9` groups; the message never enters.
pub fn fingerprint(location: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in location.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// Post one crash row. Best effort, 2 second budget, failures ignored.
fn report(location: &str) {
    if opted_out() || REPORTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let body = serde_json::json!({
        "surface": "cli",
        "event": "crash",
        "stage": "run",
        "platform": platform(),
        "release": env!("CARGO_PKG_VERSION"),
        "channel": "install.sh",
        "error_class": "panic",
        "fingerprint": fingerprint(location),
    });
    let _ = ureq::post(&telemetry_url())
        .timeout(Duration::from_secs(2))
        .send_json(&body);
}

/// Install the panic hook. Chains to the previous hook, so the backtrace the
/// user sees is unchanged — this adds a report, it does not replace output.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map_or_else(|| "unknown".to_string(), |l| format!("{}:{}", l.file(), l.line()));
        report(&location);
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_place_fingerprints_the_same_and_a_different_place_does_not() {
        assert_eq!(fingerprint("src/run.rs:412"), fingerprint("src/run.rs:412"));
        assert_ne!(fingerprint("src/run.rs:412"), fingerprint("src/run.rs:413"));
        assert_eq!(fingerprint("src/run.rs:412").len(), 8);
    }

    #[test]
    fn platform_uses_the_release_archive_vocabulary() {
        // Whatever this test runs on, the answer must be a slug install.sh
        // would recognise — never a bare `std::env::consts` value.
        let p = platform();
        assert!(
            ["linux-x86_64", "linux-aarch64", "darwin-x86_64", "darwin-aarch64", "windows-x86_64", "unknown"]
                .contains(&p),
            "unexpected platform slug: {p}"
        );
    }

    #[test]
    fn opting_out_is_honoured() {
        // Serialised through one test because they share process env.
        let restore = std::env::var("LABWIRED_TELEMETRY").ok();
        std::env::set_var("LABWIRED_TELEMETRY", "0");
        assert!(opted_out(), "LABWIRED_TELEMETRY=0 must opt out");
        std::env::remove_var("LABWIRED_TELEMETRY");

        std::env::set_var("DO_NOT_TRACK", "1");
        assert!(opted_out(), "DO_NOT_TRACK must opt out");
        std::env::remove_var("DO_NOT_TRACK");

        match restore {
            Some(v) => std::env::set_var("LABWIRED_TELEMETRY", v),
            None => std::env::remove_var("LABWIRED_TELEMETRY"),
        }
    }
}
