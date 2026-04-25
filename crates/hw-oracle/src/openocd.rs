//! OpenOCD subprocess wrapper for ESP32-S3 hardware testing.
//!
//! Spawns an OpenOCD daemon and communicates with it via its TCL server (TCP
//! port 6666 by default). Commands are terminated with `\x1a` (ASCII 26); so
//! are responses.

use anyhow::{anyhow, bail, Context, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A live OpenOCD process with an active TCL socket connection.
pub struct OpenOcd {
    child: Child,
    sock: TcpStream,
}

impl OpenOcd {
    /// Spawn OpenOCD with the default ESP32-S3 USB-JTAG configuration.
    ///
    /// Equivalent to:
    /// ```text
    /// openocd -f interface/esp_usb_jtag.cfg -f target/esp32s3.cfg
    /// ```
    pub fn spawn_default() -> Result<Self> {
        Self::spawn_with_args(&[
            "-f",
            "interface/esp_usb_jtag.cfg",
            "-f",
            "target/esp32s3.cfg",
        ])
    }

    /// Spawn OpenOCD with explicit `-f` config arguments, plus `init`.
    ///
    /// The caller passes the raw arguments that appear after `openocd`, e.g.
    /// `&["-f", "interface/esp_usb_jtag.cfg", "-f", "target/esp32s3.cfg"]`.
    ///
    /// If the USB JTAG device (VID 0x303a / PID 0x1001) is present but in an
    /// error state from a previous session, this function issues a USB reset
    /// before spawning so that `libusb_bulk_write` errors are avoided.
    pub fn spawn_with_args(args: &[&str]) -> Result<Self> {
        // Kill any lingering openocd processes that might hold the USB device.
        let _ = Command::new("pkill").args(["-x", "openocd"]).status();
        std::thread::sleep(Duration::from_millis(300));

        // Reset the ESP32-S3 USB-JTAG adapter if possible, to clear any
        // dirty state left by a previous (unclean) OpenOCD session.
        reset_esp_usb_jtag();

        // Build the full argument list: user args + `init` command so the
        // target is brought up and the TCL port starts listening.
        let mut full_args: Vec<&str> = args.to_vec();
        full_args.extend_from_slice(&["-c", "init"]);

        let child = Command::new("openocd")
            .args(&full_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn openocd – is it in PATH?")?;

        // Poll for the TCL port (6666) to become ready, up to 30 s.
        // On a cold USB connection init + JTAG scan takes ~7 s.
        let addr = "127.0.0.1:6666";
        let deadline = Instant::now() + Duration::from_secs(30);
        let sock = loop {
            match TcpStream::connect(addr) {
                Ok(s) => break s,
                Err(_) => {
                    if Instant::now() >= deadline {
                        bail!(
                            "openocd TCL port 6666 did not become ready within 30 s"
                        );
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        };

        sock.set_read_timeout(Some(Duration::from_secs(10)))
            .context("set_read_timeout")?;
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .context("set_write_timeout")?;

        Ok(Self { child, sock })
    }

    /// Send a raw TCL command string and return the response (without the
    /// trailing `\x1a` EOF byte).
    pub fn tcl(&mut self, cmd: &str) -> Result<String> {
        // Write command + \x1a terminator.
        self.sock
            .write_all(cmd.as_bytes())
            .context("tcl write cmd")?;
        self.sock
            .write_all(&[0x1a])
            .context("tcl write terminator")?;

        // Read byte-by-byte until \x1a or EOF.
        let mut response = Vec::new();
        let mut buf = [0u8; 1];
        loop {
            match self.sock.read(&mut buf) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    if buf[0] == 0x1a {
                        break;
                    }
                    response.push(buf[0]);
                }
                // EAGAIN / EWOULDBLOCK / timed-out: retry.
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => return Err(e).context("tcl read response"),
            }
        }
        Ok(String::from_utf8_lossy(&response).into_owned())
    }

    // ── Target control ────────────────────────────────────────────────────────

    /// Assert reset and immediately halt the CPU.
    pub fn reset_halt(&mut self) -> Result<()> {
        self.tcl("reset halt")?;
        Ok(())
    }

    /// Resume execution from the current PC.
    pub fn resume(&mut self) -> Result<()> {
        self.tcl("resume")?;
        Ok(())
    }

    /// Halt the CPU.
    pub fn halt(&mut self) -> Result<()> {
        self.tcl("halt")?;
        Ok(())
    }

    /// Execute a single instruction and halt again.
    pub fn step(&mut self) -> Result<()> {
        self.tcl("step")?;
        Ok(())
    }

    // ── Register access ───────────────────────────────────────────────────────

    /// Read a named CPU register (e.g. `"a0"`, `"pc"`).
    ///
    /// Sends `reg <name>` and parses the hex value from the response of the
    /// form `a0 (/32): 0x00000000`.
    pub fn read_register(&mut self, name: &str) -> Result<u32> {
        let resp = self.tcl(&format!("reg {}", name))?;
        parse_hex_from_response(&resp)
            .with_context(|| format!("read_register({name}): bad response: {resp:?}"))
    }

    /// Write a 32-bit value into a named register.
    pub fn write_register(&mut self, name: &str, v: u32) -> Result<()> {
        self.tcl(&format!("reg {} 0x{:08x}", name, v))?;
        Ok(())
    }

    // ── Memory access ─────────────────────────────────────────────────────────

    /// Read `count` consecutive 32-bit words starting at `addr`.
    ///
    /// Sends `mdw 0xADDR count` and parses lines of the form
    /// `0x40370000: xxxxxxxx yyyyyyyy ...`.
    pub fn read_memory(&mut self, addr: u32, count: usize) -> Result<Vec<u32>> {
        if count == 0 {
            return Ok(vec![]);
        }
        let resp = self.tcl(&format!("mdw 0x{:08x} {}", addr, count))?;
        parse_mdw_response(&resp, count)
            .with_context(|| format!("read_memory(0x{addr:08x}, {count}): bad response: {resp:?}"))
    }

    /// Write 32-bit `words` starting at `addr` using one `mww` per word.
    pub fn write_memory(&mut self, addr: u32, words: &[u32]) -> Result<()> {
        for (i, &word) in words.iter().enumerate() {
            let a = addr.wrapping_add((i as u32) * 4);
            self.tcl(&format!("mww 0x{:08x} 0x{:08x}", a, word))
                .with_context(|| format!("write_memory word {i} at 0x{a:08x}"))?;
        }
        Ok(())
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Ask OpenOCD to shut down cleanly, then wait for the process to exit.
    pub fn shutdown(mut self) -> Result<()> {
        // Best-effort: send the shutdown command.  Ignore errors here because
        // the connection might drop before we read a response.
        let _ = self.tcl("shutdown");
        self.child.wait().context("waiting for openocd to exit")?;
        Ok(())
    }
}

impl Drop for OpenOcd {
    fn drop(&mut self) {
        // Kill the child if it's still running (e.g. on panic / early return).
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── Parsing helpers ──────────────────────────────────────────────────────────

/// Extract the first `0x[0-9a-fA-F]+` token from `s` and parse it as u32.
fn parse_hex_from_response(s: &str) -> Result<u32> {
    for token in s.split_whitespace() {
        if let Some(hex) = token.strip_prefix("0x").or_else(|| token.strip_prefix("0X")) {
            // Strip any trailing non-hex characters (e.g. a comma).
            let hex = hex.trim_end_matches(|c: char| !c.is_ascii_hexdigit());
            if !hex.is_empty() {
                return u32::from_str_radix(hex, 16)
                    .with_context(|| format!("parse hex '{hex}'"));
            }
        }
    }
    Err(anyhow!("no 0x… token found in: {s:?}"))
}

/// Parse the output of `mdw 0xADDR N` into a `Vec<u32>`.
///
/// Each line looks like:
/// ```text
/// 0x40370000: 00fff0c6 e5004136 120cfff9 0000f01d
/// ```
/// Words are separated by spaces; up to 4 per line.
fn parse_mdw_response(s: &str, expected: usize) -> Result<Vec<u32>> {
    let mut words = Vec::with_capacity(expected);
    for line in s.lines() {
        // Lines start with "0xADDR: w0 w1 w2 w3"
        if let Some(rest) = line.find(": ").map(|i| &line[i + 2..]) {
            for tok in rest.split_whitespace() {
                let tok = tok.trim();
                if tok.is_empty() {
                    continue;
                }
                let w = u32::from_str_radix(tok, 16)
                    .with_context(|| format!("parse mdw word '{tok}'"))?;
                words.push(w);
                if words.len() == expected {
                    return Ok(words);
                }
            }
        }
    }
    if words.len() < expected {
        bail!("expected {expected} words, got {}: {s:?}", words.len());
    }
    Ok(words)
}

/// Issue a USB reset to the ESP32-S3 USB-JTAG adapter (VID 0x303a / PID
/// 0x1001) to clear any dirty state left by a previous OpenOCD session.
///
/// This is a best-effort operation: if the device cannot be found or reset,
/// the function silently returns (the subsequent `openocd` invocation will
/// fail with a meaningful error instead).
fn reset_esp_usb_jtag() {
    #[allow(unused_imports)]
    use std::fs;

    // Use /sys/bus/usb/devices to find the device by idVendor/idProduct.
    let Ok(devices) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return;
    };
    for entry in devices.flatten() {
        let base = entry.path();
        let vendor = std::fs::read_to_string(base.join("idVendor"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let product = std::fs::read_to_string(base.join("idProduct"))
            .unwrap_or_default()
            .trim()
            .to_string();
        if vendor.eq_ignore_ascii_case("303a") && product.eq_ignore_ascii_case("1001") {
            // Found it.  Read busnum + devnum to build the device path.
            let busnum = std::fs::read_to_string(base.join("busnum"))
                .unwrap_or_default()
                .trim()
                .parse::<u32>()
                .unwrap_or(0);
            let devnum = std::fs::read_to_string(base.join("devnum"))
                .unwrap_or_default()
                .trim()
                .parse::<u32>()
                .unwrap_or(0);
            if busnum == 0 || devnum == 0 {
                return;
            }
            let dev_path = format!("/dev/bus/usb/{:03}/{:03}", busnum, devnum);
            // USBDEVFS_RESET ioctl = 0x5514
            const USBDEVFS_RESET: libc::c_ulong = 0x5514;
            unsafe {
                let path = std::ffi::CString::new(dev_path).unwrap();
                let fd = libc::open(path.as_ptr(), libc::O_WRONLY);
                if fd >= 0 {
                    libc::ioctl(fd, USBDEVFS_RESET, 0usize);
                    libc::close(fd);
                    // Brief pause for the OS to re-enumerate the device.
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
            return;
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialise all ignored (hardware) tests so only one OpenOCD instance runs
    // at a time.  Cargo runs tests in parallel by default; the USB-JTAG device
    // and port 6666 can only be held by one process.
    static HW_LOCK: Mutex<()> = Mutex::new(());

    // Unit tests for parsing helpers — no hardware required.

    #[test]
    fn parse_hex_reg_response() {
        let resp = "a0 (/32): 0x00000000";
        assert_eq!(parse_hex_from_response(resp).unwrap(), 0u32);

        let resp2 = "pc (/32): 0x40380060";
        assert_eq!(parse_hex_from_response(resp2).unwrap(), 0x40380060u32);
    }

    #[test]
    fn parse_mdw_response_single_line() {
        let resp = "0x40370000: 00fff0c6 e5004136 120cfff9 0000f01d \n";
        let words = parse_mdw_response(resp, 4).unwrap();
        assert_eq!(words, vec![0x00fff0c6, 0xe5004136, 0x120cfff9, 0x0000f01d]);
    }

    #[test]
    fn parse_mdw_response_multi_line() {
        let resp = "0x40370000: aabbccdd 11223344 \n0x40370008: deadbeef cafebabe \n";
        let words = parse_mdw_response(resp, 4).unwrap();
        assert_eq!(
            words,
            vec![0xaabbccdd, 0x11223344, 0xdeadbeef, 0xcafebabe]
        );
    }

    /// Live hardware test — requires ESP32-S3 board connected via USB-JTAG.
    /// Run with: cargo test -p labwired-hw-oracle -- --ignored
    #[test]
    #[ignore]
    fn openocd_halts_and_reads_reg() {
        let _guard = HW_LOCK.lock().unwrap();
        let mut oc = OpenOcd::spawn_default().unwrap();
        oc.reset_halt().unwrap();
        let a0 = oc.read_register("a0").unwrap();
        let _ = a0;
        oc.shutdown().unwrap();
    }

    /// Live hardware test — reads 4 words from IRAM and writes/reads back a
    /// scratch value in DRAM.
    #[test]
    #[ignore]
    fn openocd_read_write_memory() {
        let _guard = HW_LOCK.lock().unwrap();
        let mut oc = OpenOcd::spawn_default().unwrap();
        oc.reset_halt().unwrap();

        // Read 4 words from the start of the ESP32-S3 IRAM mirror.
        let words = oc.read_memory(0x40370000, 4).unwrap();
        assert_eq!(words.len(), 4);

        // Write and read back a scratch value in DRAM (internal SRAM 2).
        let scratch_addr = 0x3FC9_0000_u32;
        oc.write_memory(scratch_addr, &[0xDEAD_BEEF]).unwrap();
        let readback = oc.read_memory(scratch_addr, 1).unwrap();
        assert_eq!(readback[0], 0xDEAD_BEEF);

        oc.shutdown().unwrap();
    }

    /// Live hardware test — write a register value and read it back.
    #[test]
    #[ignore]
    fn openocd_write_read_register() {
        let _guard = HW_LOCK.lock().unwrap();
        let mut oc = OpenOcd::spawn_default().unwrap();
        oc.reset_halt().unwrap();

        // Write a recognisable sentinel into a0 and read it back.
        oc.write_register("a0", 0x1234_5678).unwrap();
        let v = oc.read_register("a0").unwrap();
        assert_eq!(v, 0x1234_5678);

        oc.shutdown().unwrap();
    }
}
