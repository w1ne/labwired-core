// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! labwired-flasher — Linux-native NUCLEO debug-probe reflasher.
//!
//! ## Why
//!
//! Many NUCLEO boards ship with ST-Link V2-1 firmware on the on-board
//! debug probe (STM32F103). It's common to flash SEGGER's J-Link OB
//! firmware onto the same hardware to gain JLinkExe / RTT / SWO support.
//! The standard tools to switch between the two (SEGGER's STLinkReflash,
//! ST's STLinkUpgrade) are Windows-only or require a heavyweight Java
//! GUI; neither is friendly for headless Linux workflows.
//!
//! This tool is a Linux-native CLI replacement: it enumerates the
//! debug probe over USB, reports which firmware it's currently running,
//! and (with the bundled firmware blobs from the system's existing
//! SEGGER / STMicroelectronics installs) drives the firmware-update
//! handshake without launching any GUI.
//!
//! ## What it does today
//!
//! - `info` — enumerate connected ST-Link / J-Link OB / DFU bootloader
//!   probes and print VID:PID, serial, current firmware variant.
//! - `revert-instructions` — print the exact, copy-pasteable steps to
//!   convert J-Link OB back to ST-Link via STLinkReflash in a Windows
//!   VM with USB redirection. (Native protocol implementation is the
//!   next milestone — see TODO at the bottom of this file.)
//!
//! ## Roadmap (non-blocking — protocol RE in progress)
//!
//! - `to-stlink` — issue the SEGGER vendor-specific "enter STM32 ROM
//!   bootloader" control transfer, wait for the device to re-enumerate
//!   as `0483:DF11`, then DFU-flash the native ST-Link firmware
//!   extracted from STM32CubeProgrammer's STLinkUpgrade.jar.
//! - `to-jlink` — symmetric path: trigger ST-Link bootloader entry,
//!   then DFU-flash SEGGER's `JLink_OB_STM32F103.bin` from the local
//!   `/opt/SEGGER/JLink_*/Firmwares/` install.
//!
//! Until those land, the `revert-instructions` subcommand documents
//! the working manual workflow with the user's existing tooling.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rusb::{Device, DeviceDescriptor, GlobalContext};
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(version, about = "Linux-native NUCLEO debug-probe reflasher")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Enumerate connected debug probes and report current firmware variant.
    Info,
    /// Print the manual workflow for converting J-Link OB -> ST-Link via
    /// a Windows VM with STLinkReflash.exe (used while the native protocol
    /// implementation is still being reverse-engineered).
    RevertInstructions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeKind {
    StLinkV1,
    StLinkV2,
    StLinkV21,
    StLinkV3,
    JLinkOb,
    Stm32DfuBootloader,
    Unknown,
}

impl ProbeKind {
    fn classify(vid: u16, pid: u16) -> Self {
        match (vid, pid) {
            (0x0483, 0x3744) => Self::StLinkV1,
            (0x0483, 0x3748) => Self::StLinkV2,
            (0x0483, 0x374b) => Self::StLinkV21,
            (0x0483, 0x3752) => Self::StLinkV21, // V2-1 with composite VCP
            (0x0483, 0x374e | 0x374f | 0x3753 | 0x3754) => Self::StLinkV3,
            (0x0483, 0xdf11) => Self::Stm32DfuBootloader,
            (0x1366, _) => Self::JLinkOb,
            _ => Self::Unknown,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::StLinkV1 => "ST-Link V1 (native ST firmware)",
            Self::StLinkV2 => "ST-Link V2 (native ST firmware)",
            Self::StLinkV21 => "ST-Link V2-1 (native ST firmware)",
            Self::StLinkV3 => "ST-Link V3 (native ST firmware)",
            Self::JLinkOb => "SEGGER J-Link OB (J-Link firmware on STM32F1 debug probe)",
            Self::Stm32DfuBootloader => "STM32 ROM DFU bootloader (ready for firmware flash)",
            Self::Unknown => "unknown / not a recognised debug probe",
        }
    }

    fn is_probe(self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

fn read_strings(
    device: &Device<GlobalContext>,
    desc: &DeviceDescriptor,
) -> (Option<String>, Option<String>, Option<String>) {
    let Ok(handle) = device.open() else {
        return (None, None, None);
    };
    let langs = match handle.read_languages(Duration::from_millis(200)) {
        Ok(l) if !l.is_empty() => l,
        _ => return (None, None, None),
    };
    let lang = langs[0];
    let manufacturer = handle
        .read_manufacturer_string(lang, desc, Duration::from_millis(200))
        .ok();
    let product = handle
        .read_product_string(lang, desc, Duration::from_millis(200))
        .ok();
    let serial = handle
        .read_serial_number_string(lang, desc, Duration::from_millis(200))
        .ok();
    (manufacturer, product, serial)
}

fn cmd_info() -> Result<()> {
    let devices = rusb::devices().context("rusb::devices() failed — is libusb-1.0 installed?")?;
    let mut found = 0;
    let mut last_kind = ProbeKind::Unknown;
    for device in devices.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        let kind = ProbeKind::classify(desc.vendor_id(), desc.product_id());
        if !kind.is_probe() {
            continue;
        }
        found += 1;
        last_kind = kind;
        let (manufacturer, product, serial) = read_strings(&device, &desc);
        println!("[{}] {:04x}:{:04x}", found, desc.vendor_id(), desc.product_id());
        println!("    kind:         {}", kind.description());
        if let Some(m) = manufacturer {
            println!("    manufacturer: {m}");
        }
        if let Some(p) = product {
            println!("    product:      {p}");
        }
        if let Some(s) = serial {
            println!("    serial:       {s}");
        }
        println!(
            "    bus/address:  {}/{}",
            device.bus_number(),
            device.address()
        );
        println!();
    }
    if found == 0 {
        println!("No ST-Link / J-Link OB / STM32 DFU bootloader devices found.");
        println!("Plug in a NUCLEO board and try again.");
        return Ok(());
    }
    println!("Found {found} debug probe(s).");
    if found == 1 {
        match last_kind {
            ProbeKind::JLinkOb => {
                println!();
                println!("Hint: this probe runs SEGGER J-Link OB. To convert it back to");
                println!("native ST-Link firmware, run:");
                println!("    labwired-flasher revert-instructions");
            }
            ProbeKind::Stm32DfuBootloader => {
                println!();
                println!("Hint: probe is in STM32 ROM DFU mode — ready to receive a");
                println!("firmware payload via STLinkUpgrade.jar or dfu-util.");
            }
            ProbeKind::StLinkV21 | ProbeKind::StLinkV2 | ProbeKind::StLinkV1 | ProbeKind::StLinkV3 => {
                println!();
                println!("Hint: native ST-Link firmware detected. The board's Virtual COM");
                println!("Port should appear at /dev/ttyACM<N> (run `dmesg | tail` to find N).");
            }
            ProbeKind::Unknown => {}
        }
    }
    Ok(())
}

fn cmd_revert_instructions() -> Result<()> {
    println!("Manual workflow: convert SEGGER J-Link OB -> native ST-Link firmware");
    println!("on a NUCLEO debug probe, using a Windows VM as the USB host.");
    println!();
    println!("Why this script and not the native flasher?");
    println!("  The native protocol path (vendor-specific control transfer to put");
    println!("  the J-Link OB into STM32 ROM DFU mode, then DFU-flash native");
    println!("  ST-Link firmware) is still being reverse-engineered. Until then,");
    println!("  this routes through SEGGER's STLinkReflash.exe inside a Windows");
    println!("  VM, which gets the same job done with vendor-supplied tooling.");
    println!();
    println!("Prerequisites (one-time):");
    println!("  - Windows VM (GNOME Boxes / virt-manager / VirtualBox).");
    println!("  - VM has USB redirection enabled.");
    println!("  - On the Windows side: download STLinkReflash.exe + JLinkARM.dll");
    println!("    from https://www.segger.com/downloads/jlink/STLinkReflash");
    println!("    (the download is a ZIP served as .exe — unzip it).");
    println!();
    println!("Per-board steps:");
    println!("  1. Plug the NUCLEO into the Linux host. Confirm with:");
    println!("       labwired-flasher info");
    println!("     The board should show as 'SEGGER J-Link OB'.");
    println!();
    println!("  2. In your VM client (GNOME Boxes: properties -> Devices -> USB),");
    println!("     redirect the 'SEGGER J-Link' device into the Windows VM.");
    println!();
    println!("  3. Inside the VM, open a cmd prompt and run:");
    println!("       cd %USERPROFILE%\\Downloads");
    println!("       STLinkReflash.exe");
    println!("     A) Accept the SEGGER licence terms.");
    println!("     1) Choose 'Restore ST-Link' (option 1 in the menu).");
    println!("     The tool will:");
    println!("       - Force the J-Link OB into DFU bootloader mode.");
    println!("       - Wait for STM32F103 ROM bootloader to enumerate.");
    println!("       - Flash native ST-Link firmware.");
    println!("       - Restart the device.");
    println!();
    println!("  4. Unredirect the USB device from the VM and confirm on Linux:");
    println!("       labwired-flasher info");
    println!("     The board should now show as 'ST-Link V2-1 (native ST firmware)'.");
    println!();
    println!("  5. The Virtual COM Port at /dev/ttyACM<n> is now reliable for");
    println!("     byte-for-byte UART capture (no more J-Link OB packet drops).");
    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Info => cmd_info(),
        Cmd::RevertInstructions => cmd_revert_instructions(),
    }
}
