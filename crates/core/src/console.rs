// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WHICH CONSOLE THE DEVELOPER'S CABLE IS ON.
//!
//! An ESP32-C3 or -S3 has two consoles: UART0 and the chip's own
//! USB-Serial-JTAG block. Both are always there in silicon. Which one carries
//! `Serial` to the host is not a chip fact and not a firmware fact — it is a
//! BOARD fact, decided by what the board's USB socket is soldered to:
//!
//! * **native USB** (ESP32-C3 SuperMini, ESP32-S3 Zero) — the USB-C socket lands
//!   directly on the MCU's USB-Serial-JTAG. UART0 comes out on header pins
//!   (GPIO20/21 on the C3) with nothing attached, so a build whose `Serial` is
//!   UART0 prints into open air. That is a real bug that shipped: a hosted build
//!   flashed to a real SuperMini emitted the ROM and bootloader banners and then
//!   nothing at all.
//! * **bridge chip** (classic ESP32 devkits, Adafruit Feather ESP32 V2) — a
//!   CP210x/CH34x sits on UART0 and IS the USB device the host enumerates. Its
//!   `Serial` must stay on UART0; a classic ESP32 has no USB peripheral at all.
//!
//! The build side derives `ARDUINO_USB_CDC_ON_BOOT` from that same board fact.
//! This module is the twin's half of the rule, so the two cannot disagree: the
//! run manifest declares the board's console and the engine taps exactly it.
//!
//! ## Why one tap and not both
//!
//! Merging both consoles into one pane would make every run "work", which is
//! precisely the wrong answer for a twin: no real board hands you two consoles
//! on one cable, so a merged pane would hide the CDC-on-boot bug above instead
//! of reproducing it. It also breaks on the C3's faithful ROM path, where the
//! mask ROM prints its banner to BOTH consoles — a merged buffer renders every
//! ROM character twice.
//!
//! ## But it must not be silent
//!
//! Tapping one console means the other's bytes have nowhere to go, and "empty
//! Serial pane, no reason given" is the failure this file exists to prevent.
//! [`ConsoleCapture`] therefore records BOTH streams and shows one: the
//! untapped stream is not displayed, but [`ConsoleCapture::unheard_output`]
//! reports what the firmware said on it, so the twin can explain a silent pane
//! the way a logic analyzer clipped to the other pin would.

use labwired_config::SystemManifest;
use std::sync::{Arc, Mutex};

/// Peripheral name of the ESP32-C3/S3 USB-Serial-JTAG block on the bus.
pub const USB_SERIAL_JTAG: &str = "usb_serial_jtag";

/// The console a board's USB socket is physically wired to.
///
/// Parsed from the run manifest's `debug_uart:` key — the ONE place this
/// vocabulary is spelled. Every construction path asks here rather than
/// re-deriving it, so a new spelling cannot be honoured on one path and ignored
/// on another (which is how the browser ended up taking `usb_serial_jtag` on the
/// C3 ROM path and silently substituting UART0 everywhere else).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostConsole {
    /// The MCU's own USB-Serial-JTAG — a `deploy.usb: native` board.
    UsbSerialJtag,
    /// One named UART peripheral (`uart0`, `uart1`, …): a USB-UART bridge board,
    /// or a board that deliberately routes its console off the default UART.
    Uart(String),
    /// The manifest declares nothing. Preserves each path's historical default
    /// rather than guessing a board fact the manifest did not state.
    Undeclared,
}

impl HostConsole {
    /// Read the board's console declaration out of a run manifest.
    pub fn from_manifest(manifest: &SystemManifest) -> Self {
        match manifest.debug_uart.as_deref() {
            None => Self::Undeclared,
            Some(name) => Self::from_name(name),
        }
    }

    /// Parse one console name. Accepts both spellings of the USB block that
    /// have appeared in manifests.
    pub fn from_name(name: &str) -> Self {
        if name.eq_ignore_ascii_case(USB_SERIAL_JTAG)
            || name.eq_ignore_ascii_case("usb-serial-jtag")
        {
            Self::UsbSerialJtag
        } else {
            Self::Uart(name.to_string())
        }
    }

    /// Name to show a human. `Undeclared` reports the default it resolves to.
    pub fn label(&self) -> &str {
        match self {
            Self::UsbSerialJtag => USB_SERIAL_JTAG,
            Self::Uart(name) => name,
            Self::Undeclared => "uart",
        }
    }

    /// True when this declaration selects the USB-Serial-JTAG block.
    pub fn is_usb_serial_jtag(&self) -> bool {
        matches!(self, Self::UsbSerialJtag)
    }
}

/// Both consoles captured, one of them shown.
///
/// `heard` is the console the board's cable is on — the bytes the Serial pane
/// renders, and the bytes a real board would have delivered. `unheard` is the
/// other console, recorded and never shown, so that firmware printing into a
/// disconnected console is diagnosable instead of vanishing.
pub struct ConsoleCapture {
    tapped: HostConsole,
    other: HostConsole,
    heard: Arc<Mutex<Vec<u8>>>,
    unheard: Arc<Mutex<Vec<u8>>>,
}

impl ConsoleCapture {
    /// `tapped` is the console the board's socket is wired to; `other` is the
    /// console that exists in silicon but reaches no connector.
    pub fn new(tapped: HostConsole, other: HostConsole) -> Self {
        Self {
            tapped,
            other,
            heard: Arc::new(Mutex::new(Vec::new())),
            unheard: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// THE capture for a run: the console the manifest declares, plus the one
    /// that therefore has no connector on this board. Every construction path
    /// calls this instead of re-deriving the pairing, so the twin cannot tap one
    /// console on the ROM path and a different one on the ELF path.
    pub fn for_manifest(manifest: &SystemManifest) -> Self {
        let tapped = HostConsole::from_manifest(manifest);
        // On the ESP32-C3/S3 the pair is always {UART0, USB-Serial-JTAG}; a
        // board's socket is on exactly one of them. On a chip with only UARTs
        // the "other" console never receives a byte, so the report stays quiet.
        let other = if tapped.is_usb_serial_jtag() {
            HostConsole::Uart("uart0".to_string())
        } else {
            HostConsole::UsbSerialJtag
        };
        Self::new(tapped, other)
    }

    /// The console this run shows — what the board's USB socket is wired to.
    pub fn tapped(&self) -> &HostConsole {
        &self.tapped
    }

    /// Sink for the console the pane shows.
    pub fn heard_sink(&self) -> Arc<Mutex<Vec<u8>>> {
        self.heard.clone()
    }

    /// Sink for the console nothing is plugged into.
    pub fn unheard_sink(&self) -> Arc<Mutex<Vec<u8>>> {
        self.unheard.clone()
    }

    /// What the firmware said on the console the board is NOT wired to, minus
    /// anything the shown console also received.
    ///
    /// The subtraction matters on the C3's faithful ROM path: the mask ROM
    /// prints its banner to both consoles, so both streams start with the same
    /// bytes. Only what comes AFTER that shared prefix is genuinely unheard —
    /// i.e. output the developer would not see on this board. Reporting the
    /// banner as "unheard" would raise the alarm on every single run.
    pub fn unheard_output(&self) -> Vec<u8> {
        let heard = self.heard.lock().unwrap();
        let unheard = self.unheard.lock().unwrap();
        let shared = heard
            .iter()
            .zip(unheard.iter())
            .take_while(|(a, b)| a == b)
            .count();
        unheard[shared..].to_vec()
    }

    /// One-line explanation of a Serial pane that does not show what the
    /// firmware printed, or `None` when nothing was lost.
    ///
    /// Advisory, not fatal: a real board behaves exactly this way, so the run is
    /// not wrong — the developer just needs to be told which console their
    /// output went to and that their board has no cable on it.
    pub fn mismatch(&self) -> Option<String> {
        let unheard = self.unheard_output();
        if unheard.is_empty() {
            return None;
        }
        Some(format!(
            "firmware wrote {} bytes to {}, which this board's USB connector is not wired to \
             (its console is {}). A real board shows the same empty pane. If this board's socket \
             IS on {}, the run manifest must declare `debug_uart: {}`.",
            unheard.len(),
            self.other.label(),
            self.tapped.label(),
            self.other.label(),
            self.other.label(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with(debug_uart: Option<&str>) -> SystemManifest {
        let mut yaml = String::from("name: \"t\"\nchip: \"c\"\n");
        if let Some(v) = debug_uart {
            yaml.push_str(&format!("debug_uart: \"{v}\"\n"));
        }
        serde_yaml::from_str(&yaml).unwrap()
    }

    #[test]
    fn undeclared_manifest_selects_nothing() {
        assert_eq!(
            HostConsole::from_manifest(&manifest_with(None)),
            HostConsole::Undeclared
        );
    }

    #[test]
    fn both_usb_spellings_reach_the_same_console() {
        for spelling in ["usb_serial_jtag", "USB_SERIAL_JTAG", "usb-serial-jtag"] {
            assert_eq!(
                HostConsole::from_manifest(&manifest_with(Some(spelling))),
                HostConsole::UsbSerialJtag,
                "{spelling}"
            );
        }
    }

    #[test]
    fn a_uart_name_stays_a_uart_name() {
        assert_eq!(
            HostConsole::from_manifest(&manifest_with(Some("uart1"))),
            HostConsole::Uart("uart1".into())
        );
    }

    /// The banner both consoles receive is not unheard output.
    #[test]
    fn shared_prefix_is_not_reported_as_unheard() {
        let cap = ConsoleCapture::new(
            HostConsole::Uart("uart0".into()),
            HostConsole::UsbSerialJtag,
        );
        cap.heard_sink()
            .lock()
            .unwrap()
            .extend_from_slice(b"ESP-ROM banner\n");
        cap.unheard_sink()
            .lock()
            .unwrap()
            .extend_from_slice(b"ESP-ROM banner\n");
        assert!(cap.unheard_output().is_empty());
        assert_eq!(cap.mismatch(), None);
    }

    /// What only the untapped console got is reported, and the message names
    /// both consoles and the manifest key that would fix it.
    #[test]
    fn output_only_the_untapped_console_saw_is_reported() {
        let cap = ConsoleCapture::new(
            HostConsole::Uart("uart0".into()),
            HostConsole::UsbSerialJtag,
        );
        cap.heard_sink()
            .lock()
            .unwrap()
            .extend_from_slice(b"banner\n");
        cap.unheard_sink()
            .lock()
            .unwrap()
            .extend_from_slice(b"banner\nhello from CDC\n");

        assert_eq!(cap.unheard_output(), b"hello from CDC\n".to_vec());
        let msg = cap.mismatch().expect("a mismatch");
        assert!(msg.contains("usb_serial_jtag"), "{msg}");
        assert!(msg.contains("uart0"), "{msg}");
        assert!(msg.contains("debug_uart"), "{msg}");
    }

    /// Symmetric: tapping USB and printing on UART0 is just as much a mismatch.
    #[test]
    fn the_report_works_in_both_directions() {
        let cap = ConsoleCapture::new(
            HostConsole::UsbSerialJtag,
            HostConsole::Uart("uart0".into()),
        );
        cap.heard_sink()
            .lock()
            .unwrap()
            .extend_from_slice(b"banner\n");
        cap.unheard_sink()
            .lock()
            .unwrap()
            .extend_from_slice(b"banner\nhello from UART0\n");

        assert_eq!(cap.unheard_output(), b"hello from UART0\n".to_vec());
        assert!(cap.mismatch().unwrap().contains("uart0"));
    }

    /// A console with nothing on it at all is not a mismatch.
    #[test]
    fn a_silent_untapped_console_raises_nothing() {
        let cap = ConsoleCapture::new(
            HostConsole::Uart("uart0".into()),
            HostConsole::UsbSerialJtag,
        );
        cap.heard_sink()
            .lock()
            .unwrap()
            .extend_from_slice(b"lots of output\n");
        assert!(cap.unheard_output().is_empty());
        assert_eq!(cap.mismatch(), None);
    }
}
