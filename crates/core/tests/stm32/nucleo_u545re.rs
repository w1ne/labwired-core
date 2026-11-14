// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NUCLEO-U545RE-Q (STM32U545RET6Q) machine-run proof: the twin drives the
//! board's own pins. Board facts (ST stm32u5xx-nucleo-bsp MB1841A, ST open pin
//! data .ioc, Zephyr nucleo_u545re_q): LD2 green = PA5 (active high), B1 user
//! button = PC13 (active high, pull-down), VCP = USART1 PA9 TX / PA10 RX AF7.
//!
//! Both fixtures are real arm-none-eabi-gcc ELFs, committed under
//! tests/fixtures: a 350-byte bare-metal blinky and the Cube HAL smoke
//! (examples/nucleo-u545re/board_firmware, MSI->PLL 160 MHz, SMPS, ICACHE).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::flash::u5;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const GPIOA: u64 = 0x4202_0000;
const GPIOC: u64 = 0x4202_0800;
const ODR: u64 = 0x14;
const FLASH_IF: u64 = 0x4002_2000;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

struct Rig {
    machine: Machine<labwired_core::cpu::CortexM>,
    uart: Arc<Mutex<Vec<u8>>>,
    gpioc_idx: usize,
}

fn rig(elf: &str) -> Rig {
    let sys_path = root("configs/systems/nucleo-u545re.yaml");
    let manifest: SystemManifest =
        serde_yaml::from_str(&std::fs::read_to_string(&sys_path).unwrap()).unwrap();
    let chip = ChipDescriptor::from_file(sys_path.parent().unwrap().join(&manifest.chip)).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build nucleo-u545re bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let gpioc_idx = bus.find_peripheral_index_by_name("gpioc").expect("gpioc");
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&root(&format!("tests/fixtures/{elf}"))).unwrap();
    machine.load_firmware(&image).unwrap();
    Rig {
        machine,
        uart,
        gpioc_idx,
    }
}

impl Rig {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.uart.lock().unwrap()).into_owned()
    }
    fn run(&mut self, steps: u64) {
        for _ in 0..steps {
            self.machine.step().expect("step");
        }
    }
    fn ld2(&self) -> bool {
        self.machine.bus.read_u32(GPIOA + ODR).unwrap() & (1 << 5) != 0
    }
}

#[test]
fn blinky_prints_ok_on_usart1_vcp() {
    let mut r = rig("nucleo-u545re-blinky.elf");
    r.run(20_000);
    assert!(
        r.text().starts_with("OK\n"),
        "VCP USART1 got {:?}",
        r.text()
    );
}

#[test]
fn blinky_configures_the_board_pins_and_toggles_ld2_on_pa5() {
    let mut r = rig("nucleo-u545re-blinky.elf");
    let (mut seen_hi, mut seen_lo, mut edges, mut last) = (false, false, 0, false);
    for _ in 0..40_000 {
        r.machine.step().unwrap();
        let v = r.ld2();
        seen_hi |= v;
        seen_lo |= !v;
        if v != last {
            edges += 1;
            last = v;
        }
    }
    assert!(seen_hi && seen_lo && edges >= 3, "PA5 edges={edges}");
    let bus = &r.machine.bus;
    let moder_a = bus.read_u32(GPIOA).unwrap();
    assert_eq!((moder_a >> 10) & 3, 1, "PA5 general-purpose output");
    assert_eq!((moder_a >> 18) & 3, 2, "PA9 alternate function");
    assert_eq!((moder_a >> 20) & 3, 2, "PA10 alternate function");
    assert_eq!(
        (bus.read_u32(GPIOA + 0x24).unwrap() >> 4) & 0xFF,
        0x77,
        "PA9/PA10 AF7 = USART1"
    );
    assert_eq!((bus.read_u32(GPIOC).unwrap() >> 26) & 3, 0, "PC13 input");
    assert_eq!(
        (bus.read_u32(GPIOC + 0x0C).unwrap() >> 26) & 3,
        2,
        "PC13 pull-down"
    );
}

#[test]
fn button_pc13_press_and_release_reach_the_vcp() {
    let mut r = rig("nucleo-u545re-blinky.elf");
    r.run(20_000);
    let idx = r.gpioc_idx;
    assert!(r.machine.bus.set_peripheral_gpio_input(idx, 13, true));
    r.run(5_000);
    assert!(
        r.text().contains("B1=1\n"),
        "press not seen: {:?}",
        r.text()
    );
    assert!(r.machine.bus.set_peripheral_gpio_input(idx, 13, false));
    r.run(5_000);
    assert!(
        r.text().contains("B1=0\n"),
        "release not seen: {:?}",
        r.text()
    );
}

#[test]
fn vcp_rx_pa10_is_echoed_back_on_tx_pa9() {
    let mut r = rig("nucleo-u545re-blinky.elf");
    r.run(20_000);
    let rx = r
        .machine
        .bus
        .attach_uart_rx_source_named("usart1")
        .expect("usart1 rx");
    rx.lock().unwrap().extend(b"Q");
    r.run(5_000);
    assert!(r.text().ends_with('Q'), "echo missing: {:?}", r.text());
}

/// ~20M steps through `Machine::step` take about a minute, so this is not in
/// the PR gate; run it with `--ignored` (evidence in examples/nucleo-u545re/VALIDATION.md).
/// The user button is held down from reset so the first report line shows B1=1.
#[test]
#[ignore = "20M-step Cube HAL run (~1 min); run with --ignored"]
fn cube_hal_firmware_reaches_160mhz_blinks_ld2_and_reports_b1() {
    let mut r = rig("nucleo-u545re-cubehal.elf");
    let idx = r.gpioc_idx;
    r.machine.bus.set_peripheral_gpio_input(idx, 13, true);
    r.run(20_000_000);
    assert!(r.text().contains("U545-HAL OK\r\n"), "got {:?}", r.text());
    assert!(
        r.text().contains("BLINK 0 LD2=1 B1=1\r\n"),
        "HAL blink + HAL_GPIO_ReadPin(PC13): {:?}",
        r.text()
    );
    assert!(r.ld2(), "LD2 (PA5) high after the first HAL_GPIO_TogglePin");
}

#[test]
fn memory_map_is_the_u545_one() {
    let mut r = rig("nucleo-u545re-blinky.elf");
    let b = &mut r.machine.bus;
    // SRAM1+SRAM2 = 256 KiB at 0x2000_0000; SRAM4 16 KiB at 0x2800_0000.
    assert!(b.write_u32(0x2003_FFFC, 0xA5A5_0001).is_ok());
    assert_eq!(b.read_u32(0x2003_FFFC).unwrap(), 0xA5A5_0001);
    assert!(b.read_u32(0x2004_0000).is_err(), "no SRAM3 on U545");
    assert!(b.write_u32(0x2800_3FFC, 7).is_ok());
    assert!(b.read_u32(0x2800_4000).is_err(), "SRAM4 is 16 KiB");
    // Flash is 512 KiB.
    assert!(b.read_u32(0x0807_FFFC).is_ok());
    assert!(b.read_u32(0x0808_0000).is_err(), "flash ends at 512 KiB");
}

#[test]
fn peripherals_the_u545_lacks_are_unmapped() {
    let r = rig("nucleo-u545re-blinky.elf");
    for (name, addr) in [
        ("USART2", 0x4000_4400u64),
        ("GPIOF", 0x4202_1400),
        ("GPIOI", 0x4202_2000),
    ] {
        assert!(
            r.machine.bus.read_u32(addr).is_err(),
            "{name} at {addr:#x} must be unmapped on STM32U545"
        );
    }
    for (name, id) in [
        ("USART1", "usart1"),
        ("USART3", "usart3"),
        ("GPIOH", "gpioh"),
    ] {
        assert!(
            r.machine.bus.find_peripheral_index_by_name(id).is_some(),
            "{name} missing"
        );
    }
}

#[test]
fn dbgmcu_idcode_is_the_u545_one() {
    let r = rig("nucleo-u545re-blinky.elf");
    assert_eq!(r.machine.bus.read_u32(0xE004_4000).unwrap(), 0x1002_6455);
}

#[test]
fn flash_banks_are_256k_so_bker_erase_hits_the_second_half() {
    let mut r = rig("nucleo-u545re-blinky.elf");
    let m = &mut r.machine;
    let bank = 0x4_0000u64; // 256 KiB
    let page = 2u64;
    let bank1 = u5::FLASH_BASE + page * u5::PAGE_SIZE;
    let bank2 = u5::FLASH_BASE + bank + page * u5::PAGE_SIZE;
    for a in [bank1, bank2] {
        for (i, b) in 0xDEAD_BEEFu32.to_le_bytes().iter().enumerate() {
            m.bus.flash.write_u8(a + i as u64, *b);
        }
    }
    m.bus
        .write_u32(FLASH_IF + u5::NSKEYR_OFF, 0x4567_0123)
        .unwrap();
    m.bus
        .write_u32(FLASH_IF + u5::NSKEYR_OFF, 0xCDEF_89AB)
        .unwrap();
    m.bus
        .write_u32(
            FLASH_IF + u5::NSCR_OFF,
            u5::NSCR_PER | u5::NSCR_BKER | ((page as u32) << u5::NSCR_PNB_SHIFT) | u5::NSCR_STRT,
        )
        .unwrap();
    m.step().unwrap();
    assert_eq!(
        m.bus.flash.read_u32(bank2).unwrap(),
        0xFFFF_FFFF,
        "bank 2 page erased at +256 KiB"
    );
    assert_eq!(
        m.bus.flash.read_u32(bank1).unwrap(),
        0xDEAD_BEEF,
        "bank 1 page untouched"
    );
    let _ = m.cpu.get_pc();
}
