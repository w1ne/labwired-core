// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! An unmodified arduino-pico 6.1.0 sketch for `waveshare_rp2350_zero`
//! (fixture `tests/fixtures/rp2350-zero-arduino-serial.elf`) on the
//! `rp2350-zero` twin.
//!
//! Sketch:
//! ```c
//! void setup() { Serial1.begin(115200); Serial1.println("RP2350_ARDUINO_BOOT"); }
//! void loop() {}
//! ```
//! Built with PlatformIO `platform = maxgerhardt/platform-raspberrypi`,
//! `board = waveshare_rp2350_zero`, `board_build.core = earlephilhower`
//! (framework-arduinopico 1.60100.0 = arduino-pico 6.1.0). sha256
//! 51c1526e4bee5c376a7b7e32004ec551dc41cd189103a30ce0e7e2073e0915f2.
//!
//! The image starts with arduino-pico's OTA stage at 0x10000000 (its own
//! vector table), which hands over to the sketch's vectors at 0x10003000.
//! Both stages call boot-ROM APIs through the halfword at 0x16.
//!
//! Boot faults this image hit, in order (each fixed in this change):
//! 1. UsageFault UNDEFINSTR at 0x1008 — no boot ROM; `rom_table_lookup`
//!    (halfword 0x16) read 0x1000 out of the flash-aliased vector table.
//! 2. BusFault at 0x5011004C — USBCTRL not mapped (runtime_init_usb_power_down).
//! 3. BusFault at 0x40058000 — clk_rst window stopped short of PLL_USB.
//! 4. BusFault at 0x400E080C — BOOTRAM boot locks not mapped.
//! 5. Hang in hw_claim_lock — LDAEXB set no exclusive reservation, so the
//!    paired STREXB always failed.
//!
//! Then (fixed in the DCP change): `runtime_init_per_core_enable_coprocessors`
//! executes `mrc p4,#0,r0,c0,c0,#1` (DCP RCMP, datasheet 3.6.2), and
//! `spinlock_set_extexclall` writes SCS ACTLR (0xE000E008). With the DCP
//! (`coprocessors: [rp2350_dcp]`) and ACTLR modelled, the sketch prints its
//! banner on UART0 (`first_serial_output`).

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use std::sync::{Arc, Mutex};

const FIXTURE: &str = "tests/fixtures/rp2350-zero-arduino-serial.elf";
/// `mrc p4,#0,r0,c0,c0,#1` inside runtime_init_per_core_enable_coprocessors.
const DCP_RCMP_PC: u32 = 0x1000_5958;
/// The sketch's weak `isr_hardfault` (`bkpt`).
const HARDFAULT_HANDLER: u32 = 0x1000_311c;
const STEP_BUDGET: u32 = 200_000;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn load() -> (ChipDescriptor, SystemManifest) {
    let chip_path = root("configs/chips/rp2350.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("rp2350 chip");
    let mut manifest =
        SystemManifest::from_file(root("configs/systems/rp2350-zero.yaml")).expect("system");
    manifest.chip = chip_path.to_str().unwrap().to_string();
    (chip, manifest)
}

struct Run {
    stop_pc: u32,
    steps: u32,
    flash_alias_entered: bool,
    cfsr: u32,
    uart: Vec<u8>,
}

fn run(stop_at: Option<u32>, budget: u32) -> Run {
    let path = root(FIXTURE);
    assert!(path.is_file(), "fixture missing: {}", path.display());
    let (chip, manifest) = load();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&path).expect("load elf");
    m.load_firmware(&image).expect("load firmware");

    let mut flash_alias_entered = false;
    let mut steps = 0;
    while steps < budget {
        let pc = m.cpu.get_pc();
        if Some(pc) == stop_at || pc == HARDFAULT_HANDLER {
            break;
        }
        if (0x1000..0x1100).contains(&pc) {
            flash_alias_entered = true;
        }
        if m.step().is_err() {
            break;
        }
        steps += 1;
    }
    let cfsr = m.bus.read_u32(0xE000_ED28).unwrap_or(0);
    let uart = uart.lock().unwrap().clone();
    Run {
        stop_pc: m.cpu.get_pc(),
        steps,
        flash_alias_entered,
        cfsr,
        uart,
    }
}

/// With the boot ROM, both stages' ROM lookups resolve, and the whole
/// pre-main hardware init (USB power-down, clocks/PLLs, boot locks,
/// hw_claim_lock) runs fault-free up to the first unmodelled instruction.
#[test]
fn boots_through_rom_and_runtime_init_to_the_dcp() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("LABWIRED_RP2350_BOOTROM");
    let r = run(Some(DCP_RCMP_PC), STEP_BUDGET);
    assert!(
        !r.flash_alias_entered,
        "a ROM lookup branched into the flash alias"
    );
    assert_eq!(
        r.stop_pc, DCP_RCMP_PC,
        "boot must reach runtime_init_per_core_enable_coprocessors without a fault \
         (stopped at {:#x} after {} steps, CFSR {:#x})",
        r.stop_pc, r.steps, r.cfsr
    );
    assert_eq!(r.cfsr, 0, "no fault status before the DCP instruction");
}

/// The reported 2026-11-01 fault: without the ROM window the OTA stage's
/// `rom_func_lookup` jumps to 0x1000 and HardFaults (UNDEFINSTR at 0x1008).
#[test]
fn without_the_rom_the_lookup_hardfaults_in_the_flash_alias() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("LABWIRED_RP2350_BOOTROM", "");
    let r = run(Some(DCP_RCMP_PC), STEP_BUDGET);
    std::env::remove_var("LABWIRED_RP2350_BOOTROM");
    assert!(
        r.flash_alias_entered,
        "expected the lookup to land at 0x1000"
    );
    assert_ne!(r.stop_pc, DCP_RCMP_PC);
}

/// First serial output on UART0 (Serial1, GP0): the whole boot — OTA stage,
/// boot ROM lookups, runtime_init (DCP enable, ACTLR, clocks, locks) and
/// setup() — with no fault. The banner lands within ~200k instructions.
#[test]
fn first_serial_output() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("LABWIRED_RP2350_BOOTROM");
    let r = run(None, 1_000_000);
    assert_eq!(r.cfsr, 0, "boot faulted (stopped at {:#x})", r.stop_pc);
    assert_ne!(
        r.stop_pc, HARDFAULT_HANDLER,
        "boot entered the HardFault handler"
    );
    let text = String::from_utf8_lossy(&r.uart);
    assert!(
        text.contains("RP2350_ARDUINO_BOOT"),
        "no banner (stopped at {:#x}, CFSR {:#x}): {text:?}",
        r.stop_pc,
        r.cfsr
    );
}

/// SYSINFO layout per RP2350 datasheet 12.15.1.2, Table 1306: PACKAGE_SEL at
/// 0x04 (1 = QFN60, RP2350A), PLATFORM at 0x08 (ASIC = bit 1).
#[test]
fn sysinfo_package_sel_and_platform_offsets() {
    let (chip, manifest) = load();
    let bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    assert_eq!(bus.read_u32(0x4000_0004).unwrap(), 1, "PACKAGE_SEL = QFN60");
    assert_eq!(bus.read_u32(0x4000_0008).unwrap(), 0b10, "PLATFORM = ASIC");
}
