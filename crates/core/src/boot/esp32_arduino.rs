// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! THE classic-ESP32 Arduino-ELF boot path.
//!
//! Why this module exists
//! ======================
//! Every other chip family that boots here has one: `boot::esp32c3_rom` owns the
//! C3 ROM path, `boot::esp32s3_rom` owns the S3's. Classic ESP32 had none. Its
//! recipe — bus, dual-core CPU pair, external devices from the manifest, stack
//! seeding, symbol-driven thunk install, APP_CPU flag policy — lived as
//! hand-copied sequences in whichever caller needed it: the wasm playground's
//! `install_arduino_esp32_quirks`, and each end-to-end test that boots an
//! Arduino ELF.
//!
//! That is the shape every defect fixed on this chip has had. A copy that misses
//! a step does not fail loudly; it boots a machine that is subtly wrong, and the
//! symptom surfaces somewhere else entirely — a blank panel, an empty UART sink,
//! a `loop()` that never runs. The stack seeds below are the sharpest example:
//! they are two magic DRAM addresses that must not collide with `.bss` or with
//! each other, and nothing about a wrong value announces itself.
//!
//! So: one home. Callers describe WHAT they are booting (an image, its symbols,
//! a manifest) and this module owns HOW. Adding a step here reaches every caller
//! at once, which is the only property that makes the step reliable.
//!
//! ELF parsing stays out
//! =====================
//! `symbol_addrs` and `image` are passed IN, exactly as
//! `install_arduino_esp32_profile` already requires. Reading object files is the
//! loader's job, and `labwired-loader` depends on this crate — so core cannot
//! depend on it without a cycle. Callers resolve both with
//! `labwired_loader::{load_elf, extract_arduino_esp32_thunks}`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::bus::SystemBus;
use crate::cpu::XtensaLx7;
use crate::memory::ProgramImage;
use crate::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32, install_arduino_esp32_profile,
    ArduinoEsp32Profile,
};
use crate::{Cpu, Machine};

/// PRO_CPU initial stack pointer.
///
/// Real silicon's BROM seeds SP near the top of DRAM before jumping to
/// `call_start_cpu0`. The sim skips BROM, so it must be seeded here.
pub const PRO_CPU_INITIAL_SP: u32 = 0x3FFE_0000;

/// APP_CPU initial stack pointer.
///
/// A SEPARATE DRAM region: above `.bss` (which ends around 0x3FFC_5CE8 on the
/// firmwares we ship) and below PRO_CPU's stack. The ROM sets this before
/// releasing APP_CPU to `call_start_cpu1`, whose first instruction is
/// `entry a1,32` — so an unseeded or overlapping value corrupts the first frame
/// rather than faulting somewhere you would think to look.
pub const APP_CPU_INITIAL_SP: u32 = 0x3FFD_8000;

/// How to bring up the machine. Defaults match the shipped browser path.
#[derive(Debug, Clone)]
pub struct ArduinoElfBootOpts {
    /// Attach a real second LX6 as APP_CPU (PRID 0xABAB → `xPortGetCoreID()==1`,
    /// halted until PRO_CPU releases it via `ets_set_appcpu_boot_addr`).
    ///
    /// arduino-esp32 pins `loopTask` to `CONFIG_ARDUINO_RUNNING_CORE=1`. With a
    /// real APP_CPU that is modelled and the firmware drives the whole rendezvous
    /// itself. With a single core there is nobody to mark the startup flags, so
    /// `appcpu_up_flag_addrs` has to forge them instead — see that field.
    pub dual_core: bool,
    /// Addresses to force-mark as "APP_CPU is up" for SINGLE-CORE frontends.
    ///
    /// Meaningless (and passed empty) when `dual_core` is set: the second CPU
    /// marks them for real. Forging them in the dual-core case is not merely
    /// redundant, it papers over a genuine bring-up failure.
    pub appcpu_up_flag_addrs: Vec<u32>,
    pub pro_cpu_sp: u32,
    pub app_cpu_sp: u32,
}

impl Default for ArduinoElfBootOpts {
    fn default() -> Self {
        Self {
            dual_core: true,
            appcpu_up_flag_addrs: Vec::new(),
            pro_cpu_sp: PRO_CPU_INITIAL_SP,
            app_cpu_sp: APP_CPU_INITIAL_SP,
        }
    }
}

/// A booted classic-ESP32 Arduino machine, plus the handles callers always want.
pub struct ArduinoElfMachine {
    pub machine: Machine<XtensaLx7>,
    /// Everything the firmware wrote to UART TX.
    ///
    /// Attached AFTER `configure_xtensa_esp32`, never before: the sink walks the
    /// peripherals already on the bus, so attaching it to an empty bus captures
    /// nothing — and an empty sink then reads as "the firmware never printed",
    /// which is a different and much more expensive conclusion.
    pub uart_sink: Arc<Mutex<Vec<u8>>>,
    pub profile: ArduinoEsp32Profile,
}

/// Build and boot a classic-ESP32 machine running an Arduino-ESP32 ELF.
///
/// Ordering here is load-bearing and is the reason this is one function rather
/// than a documented sequence:
///  1. bus + PRO_CPU, then the UART sink (peripherals must exist first);
///  2. external devices from the manifest, then `refresh_peripheral_index`;
///  3. `load_firmware` and seed PC — ELF segment loading clobbers patched bytes,
///     so it must precede the thunk install;
///  4. seed both stacks;
///  5. install the profile LAST, because it patches BREAK bytes into flash.
pub fn build_arduino_elf_machine(
    image: &ProgramImage,
    symbol_addrs: HashMap<&'static str, u32>,
    manifest: &labwired_config::SystemManifest,
    opts: &ArduinoElfBootOpts,
) -> Result<ArduinoElfMachine, String> {
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);

    attach_esp32_external_devices(&mut bus, manifest)
        .map_err(|e| format!("attach external devices from manifest: {e}"))?;
    bus.refresh_peripheral_index();

    let mut machine = Machine::new(cpu, bus);
    if opts.dual_core {
        machine = machine.with_secondary_cpu(XtensaLx7::new_app_cpu());
    }

    machine
        .load_firmware(image)
        .map_err(|e| format!("load firmware: {e}"))?;
    machine.cpu.set_pc(image.entry_point as u32);

    machine.cpu.set_sp(opts.pro_cpu_sp);
    if let Some(cpu1) = machine.cpu_secondary.as_mut() {
        cpu1.set_sp(opts.app_cpu_sp);
    }

    crate::peripherals::esp_xtensa_common::rom_thunks::set_appcpu_up_flags(
        opts.appcpu_up_flag_addrs.clone(),
    );

    let profile =
        install_arduino_esp32_profile(&mut machine, symbol_addrs, image.entry_point as u32)?;

    Ok(ArduinoElfMachine {
        machine,
        uart_sink,
        profile,
    })
}
