// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32U575 register-level contract from the 60North Renode package.
//!
//! These tests drive the machine bus directly (no firmware): the same writes
//! the 60North peripheral Robot tests make, asserted against the same register
//! fields. They complement `stm32u575_60north.rs`, which runs the firmware.
//!
//! 60North is a Renode-derived reference, not silicon.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::device::I2cDevice;
use labwired_core::peripherals::spi_loopback::SpiLoopback;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use std::path::PathBuf;

const SYSTEM: &str = "configs/systems/nucleo-u575zi.yaml";

const RCC: u64 = 0x4602_0C00;
const PWR: u64 = 0x4602_0800;
const FLASH: u64 = 0x4002_2000;
const GPIOA: u64 = 0x4202_0000;
const GPIOB: u64 = 0x4202_0400;
const GPIOC: u64 = 0x4202_0800;
const GPIOI: u64 = 0x4202_2000;
const EXTI: u64 = 0x4602_2000;
const USART1: u64 = 0x4001_3800;
const UART4: u64 = 0x4000_4C00;
const LPUART1: u64 = 0x4600_2400;
const SPI1: u64 = 0x4001_3000;
const SPI3: u64 = 0x4600_2000;
const I2C1: u64 = 0x4000_5400;
const GPDMA1: u64 = 0x4002_0000;

struct Rig {
    machine: Machine<labwired_core::cpu::CortexM>,
}

impl Rig {
    fn new() -> Self {
        let sys_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(SYSTEM);
        let manifest: SystemManifest = SystemManifest::from_file(&sys_path).unwrap();
        let chip_path = sys_path.parent().unwrap().join(&manifest.chip);
        let chip = ChipDescriptor::from_file(&chip_path).unwrap();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let (mut cpu, _) = configure_cortex_m(&mut bus);
        // A `b .` loop in RAM gives `machine.step()` a safe instruction to run
        // while it drains flash erase/program ops recorded by MMIO writes.
        bus.write_u16(0x2000_0000, 0xE7FE).unwrap();
        cpu.set_pc(0x2000_0000);
        Self {
            machine: Machine::new(cpu, bus),
        }
    }

    /// Enable every peripheral clock these register tests touch.
    fn powered() -> Self {
        let mut r = Self::new();
        r.w(RCC + 0x88, 0x0000_0101); // AHB1: GPDMA1 + CRC
        r.w(RCC + 0x8C, 0x0000_01FF); // AHB2: GPIOA..GPIOI
        r.w(RCC + 0x94, 0x0000_0004); // AHB3: PWR
        r.w(RCC + 0x9C, 0x002E_C000); // APB1: SPI2, USART2/3, UART4, I2C1
        r.w(RCC + 0xA0, 0x0000_0200); // APB1ENR2: FDCAN1
        r.w(RCC + 0xA4, 0x0000_5000); // APB2: SPI1, USART1
        r.w(RCC + 0xA8, 0x0000_00E0); // APB3: SPI3, LPUART1, I2C3
        r
    }

    fn r(&self, addr: u64) -> u32 {
        self.machine.bus.read_u32(addr).unwrap()
    }

    fn w(&mut self, addr: u64, value: u32) {
        self.machine.bus.write_u32(addr, value).unwrap();
    }

    fn step(&mut self) {
        let _ = self.machine.step();
    }

    fn enable_gpio(&mut self, bits: u32) {
        let enr = RCC + 0x8C; // AHB2ENR1
        self.w(enr, self.r(enr) | bits);
    }

    fn enable(&mut self, reg: u64, bit: u32) {
        self.w(reg, self.r(reg) | bit);
    }
}

fn field(value: u32, pos: u32, width: u32) -> u32 {
    (value >> pos) & ((1 << width) - 1)
}

// ── RCC ─────────────────────────────────────────────────────────────────────

#[test]
fn rcc_reset_pll_gating_prescalers_and_backup_domain() {
    let mut r = Rig::new();
    // SW/SWS = MSIS after reset.
    assert_eq!(field(r.r(RCC + 0x1C), 0, 2), 0, "CFGR1.SW");
    assert_eq!(field(r.r(RCC + 0x1C), 2, 2), 0, "CFGR1.SWS");

    // PLL1 locks from MSIS with M=1, N=80, R=2.
    r.w(RCC + 0x28, 0x0004_0001);
    r.w(RCC + 0x34, 0x0101_024F);
    r.w(RCC + 0x00, r.r(RCC) | (1 << 24)); // PLL1ON
    assert_ne!(r.r(RCC) & (1 << 25), 0, "PLL1RDY");
    // A source that is not ready cannot be selected.
    r.w(RCC + 0x1C, 0x2);
    assert_eq!(field(r.r(RCC + 0x1C), 2, 2), 0, "SWS stays MSIS");
    r.w(RCC + 0x1C, 0x3);
    assert_eq!(field(r.r(RCC + 0x1C), 2, 2), 3, "SWS follows PLL1");

    // PLL1 configuration is frozen while PLL1 is on.
    let divr = r.r(RCC + 0x34);
    r.w(RCC + 0x34, 0x0101_013F);
    assert_eq!(r.r(RCC + 0x34), divr, "PLL1DIVR frozen while on");

    // Bus prescalers are stored: HPRE /2, PPRE1 /4, PPRE2 /2, PPRE3 /16.
    r.w(RCC + 0x20, 0x0000_0458);
    r.w(RCC + 0x24, 0x0000_0070);
    assert_eq!(field(r.r(RCC + 0x20), 0, 4), 0x8, "HPRE");
    assert_eq!(field(r.r(RCC + 0x20), 4, 3), 0x5, "PPRE1");
    assert_eq!(field(r.r(RCC + 0x20), 8, 3), 0x4, "PPRE2");
    assert_eq!(field(r.r(RCC + 0x24), 4, 3), 0x7, "PPRE3");

    // HSE absent: HSEON never reports HSERDY.
    r.w(RCC, r.r(RCC) | (1 << 16));
    assert_eq!(r.r(RCC) & (1 << 17), 0, "HSERDY without oscillator");

    // BDCR is protected until PWR.DBP.
    r.enable(RCC + 0x94, 1 << 2); // PWREN
    r.w(RCC + 0xF0, 0x0400_0000);
    assert_eq!(r.r(RCC + 0xF0), 0, "BDCR write ignored while protected");
    r.w(PWR + 0x28, 0x1); // DBPR
    r.w(RCC + 0xF0, 0x0400_0000);
    assert_eq!(r.r(RCC + 0xF0) & 0x0C00_0000, 0x0C00_0000, "LSION|LSIRDY");

    // CSR reset flags and RMVF.
    assert_eq!(r.r(RCC + 0xF4), 0x0C00_4400, "CSR reset");
    r.w(RCC + 0xF4, 0x0080_4400);
    assert_eq!(r.r(RCC + 0xF4), 0x0000_4400, "RMVF clears reset flags");

    // Clock enables are stored.
    r.w(RCC + 0x88, 0xD000_0101);
    assert_ne!(r.r(RCC + 0x88) & 1, 0, "GPDMA1EN");
    r.w(RCC + 0xA4, 0x0000_5000);
    assert_eq!(r.r(RCC + 0xA4) & 0x5000, 0x5000, "SPI1EN+USART1EN");
    r.w(RCC + 0xA4, 0);
    assert_eq!(r.r(RCC + 0xA4), 0, "enable clears");
}

// ── PWR ─────────────────────────────────────────────────────────────────────

#[test]
fn pwr_voltage_range_booster_regulator_and_dbpr() {
    let mut r = Rig::powered();
    for vos in 0..4u32 {
        r.w(PWR + 0x0C, vos << 16);
        assert_eq!(field(r.r(PWR + 0x0C), 16, 2), vos, "VOSR.VOS");
        assert_ne!(r.r(PWR + 0x0C) & 0x8000, 0, "VOSRDY");
        assert_eq!(field(r.r(PWR + 0x3C), 16, 2), vos, "SVMSR.ACTVOS");
        assert_ne!(r.r(PWR + 0x3C) & 0x8000, 0, "ACTVOSRDY");
    }
    assert_eq!(r.r(PWR + 0x0C) & 0x4000, 0, "BOOSTRDY off");
    r.w(PWR + 0x0C, (3 << 16) | (1 << 18));
    assert_eq!(
        r.r(PWR + 0x0C) & 0xC000,
        0xC000,
        "BOOSTEN|BOOSTRDY|VOSRDY"
    );
    // Regulator selection mirrors CR3.REGSEL into SVMSR.REGS.
    assert_eq!(r.r(PWR + 0x3C) & 2, 0, "LDO selected");
    r.w(PWR + 0x08, 2);
    assert_ne!(r.r(PWR + 0x3C) & 2, 0, "SMPS selected");
    r.w(PWR + 0x08, 0);
    assert_eq!(r.r(PWR + 0x3C) & 2, 0, "LDO selected again");
    r.w(PWR + 0x28, 1);
    assert_eq!(r.r(PWR + 0x28), 1, "DBPR");
}

// ── FLASH ───────────────────────────────────────────────────────────────────

#[test]
fn flash_latency_lock_keys_and_sequence_errors() {
    let mut r = Rig::new();
    for latency in 0..16 {
        r.w(FLASH, latency);
        assert_eq!(field(r.r(FLASH), 0, 4), latency, "ACR latency");
    }
    // Locked out of reset: control writes are ignored.
    assert_eq!(r.r(FLASH + 0x28), 0xC000_0000, "NSCR LOCK|OPTLOCK");
    r.w(FLASH + 0x28, 0x2);
    assert_eq!(r.r(FLASH + 0x28), 0xC000_0000, "locked write ignored");

    // Wrong key locks until reset.
    r.w(FLASH + 0x08, 0x1234_5678);
    r.w(FLASH + 0x08, 0x4567_0123);
    r.w(FLASH + 0x08, 0xCDEF_89AB);
    assert_ne!(r.r(FLASH + 0x28) & 0x8000_0000, 0, "wrong key stays locked");

    // Right sequence unlocks; LOCK can be set again by writing it.
    let mut r = Rig::new();
    r.w(FLASH + 0x08, 0x4567_0123);
    r.w(FLASH + 0x08, 0xCDEF_89AB);
    assert_eq!(r.r(FLASH + 0x28) & 0x8000_0000, 0, "unlocked");
    r.w(FLASH + 0x28, 0x8000_0000);
    assert_ne!(r.r(FLASH + 0x28) & 0x8000_0000, 0, "re-locked");

    // OPTLOCK clears only after the control register is unlocked.
    let mut r = Rig::new();
    r.w(FLASH + 0x10, 0x0819_2A3B);
    r.w(FLASH + 0x10, 0x4C5D_6E7F);
    assert_ne!(r.r(FLASH + 0x28) & 0x4000_0000, 0, "OPTLOCK still set");
    r.w(FLASH + 0x08, 0x4567_0123);
    r.w(FLASH + 0x08, 0xCDEF_89AB);
    r.w(FLASH + 0x10, 0x0819_2A3B);
    r.w(FLASH + 0x10, 0x4C5D_6E7F);
    assert_eq!(r.r(FLASH + 0x28) & 0x4000_0000, 0, "OPTLOCK cleared");

    // STRT without PER/MER raises PGSERR, not OPERR; ERR still needs ERRIE
    // to reach the IRQ line.
    let mut r = Rig::new();
    r.w(FLASH + 0x08, 0x4567_0123);
    r.w(FLASH + 0x08, 0xCDEF_89AB);
    r.w(FLASH + 0x28, 0x0001_0000);
    assert_ne!(r.r(FLASH + 0x20) & (1 << 7), 0, "PGSERR");
    assert_eq!(r.r(FLASH + 0x20) & (1 << 1), 0, "OPERR clear");
    r.w(FLASH + 0x20, 0x80);
    r.w(FLASH + 0x28, (1 << 24) | (1 << 16)); // ERRIE | STRT
    assert_ne!(r.r(FLASH + 0x20) & 0x82, 0, "PGSERR|OPERR");
    r.w(FLASH + 0x20, 0x82);
    assert_eq!(r.r(FLASH + 0x20), 0, "NSSR W1C");

    // EOP only when EOPIE was set.
    let mut r = Rig::new();
    r.w(FLASH + 0x08, 0x4567_0123);
    r.w(FLASH + 0x08, 0xCDEF_89AB);
    r.w(FLASH + 0x28, 0x2 | (1 << 16)); // PER | STRT, no EOPIE
    assert_eq!(r.r(FLASH + 0x20) & 1, 0, "EOP not reported when disabled");
    r.w(FLASH + 0x28, (1 << 24) | 0x2 | (1 << 16));
    assert_ne!(r.r(FLASH + 0x20) & 1, 0, "EOP reported when enabled");
}

#[test]
fn flash_page_and_mass_erase_touch_only_the_selected_region() {
    let bank1 = 0x0800_0000u64;
    let bank2 = 0x0810_0000u64;
    let page = 0x2000u64;
    let mut r = Rig::new();
    r.w(FLASH + 0x08, 0x4567_0123);
    r.w(FLASH + 0x08, 0xCDEF_89AB);
    // Program one aligned quad-word in each region (the U5 program path needs
    // PG and four successive 32-bit stores).
    r.w(FLASH + 0x28, 1); // PG
    for (base, word) in [
        (bank1 + 2 * page, 0x1122_3344u32),
        (bank1 + 3 * page, 0x5566_7788),
        (bank2 + page, 0x99AA_BBCC),
        (bank2 + 0x0F_FFF0, 0xDEAD_BEEF),
    ] {
        for k in 0..4u64 {
            r.machine.bus.write_u32(base + k * 4, word).unwrap();
        }
    }
    r.w(FLASH + 0x28, 0); // end programming
    // Page 2 of bank 1 (PER, PNB=2, STRT).
    r.w(FLASH + 0x28, 0x2 | (2 << 3) | (1 << 16));
    r.step();
    assert_eq!(r.r(bank1 + 2 * page), 0xFFFF_FFFF, "page erased");
    assert_eq!(r.r(bank1 + 3 * page), 0x5566_7788, "next page kept");
    assert_eq!(r.r(bank2 + page), 0x99AA_BBCC, "other bank kept");
    // Mass erase bank 2 only.
    r.w(FLASH + 0x28, 0x8000 | (1 << 15) | (1 << 16)); // MER2|STRT
    r.step();
    assert_eq!(r.r(bank2 + page), 0xFFFF_FFFF, "bank 2 erased");
    assert_eq!(r.r(bank2 + 0x0F_FFFC), 0xFFFF_FFFF, "bank 2 last word");
    assert_eq!(r.r(bank1 + 3 * page), 0x5566_7788, "bank 1 kept");
    // Mass erase bank 1 only.
    r.w(FLASH + 0x28, 0x4 | (1 << 16)); // MER1|STRT
    r.step();
    assert_eq!(r.r(bank1 + 3 * page), 0xFFFF_FFFF, "bank 1 erased");
}

// ── GPIO / EXTI ─────────────────────────────────────────────────────────────

fn gpio_reset_values_are(port: u64, moder: u32, ospeedr: u32, pupdr: u32) {
    let mut r = Rig::new();
    r.enable_gpio(0x1FF);
    assert_eq!(r.r(port), moder, "{port:#x} MODER");
    assert_eq!(r.r(port + 0x04), 0, "OTYPER");
    assert_eq!(r.r(port + 0x08), ospeedr, "OSPEEDR");
    assert_eq!(r.r(port + 0x0C), pupdr, "PUPDR");
    assert_eq!(r.r(port + 0x10), 0, "IDR");
    assert_eq!(r.r(port + 0x14), 0, "ODR");
    assert_eq!(r.r(port + 0x20), 0, "AFRL");
    assert_eq!(r.r(port + 0x24), 0, "AFRH");
    assert_eq!(r.r(port + 0x2C), 0, "HSLVR");
    assert_eq!(r.r(port + 0x30), 0x0000_FFFF, "SECCFGR");
}

#[test]
fn gpio_reset_values_and_set_reset_priority() {
    gpio_reset_values_are(GPIOA, 0xABFF_FFFF, 0x0C00_0000, 0x6400_0000);
    gpio_reset_values_are(GPIOB, 0xFFFF_FEBF, 0x0000_00C0, 0x0000_0100);
    gpio_reset_values_are(GPIOC, 0xFFFF_FFFF, 0, 0);
    gpio_reset_values_are(GPIOI, 0x0000_FFFF, 0, 0);

    let mut r = Rig::new();
    r.enable_gpio(1); // GPIOA
    r.w(GPIOA, 0xABFF_F7FF); // PA5 output
    let pin5 = 1u32 << 5;
    r.w(GPIOA + 0x18, pin5); // BSRR set
    assert_ne!(r.r(GPIOA + 0x14) & pin5, 0, "BSRR sets ODR");
    r.w(GPIOA + 0x18, pin5 << 16); // BSRR reset
    assert_eq!(r.r(GPIOA + 0x14) & pin5, 0, "BSRR resets ODR");
    r.w(GPIOA + 0x18, pin5);
    r.w(GPIOA + 0x28, pin5); // BRR
    assert_eq!(r.r(GPIOA + 0x14) & pin5, 0, "BRR resets ODR");
    r.w(GPIOA + 0x18, (pin5 << 16) | pin5);
    assert_ne!(r.r(GPIOA + 0x14) & pin5, 0, "set wins over reset");
    // HSLVR / SECCFGR are storage.
    r.w(GPIOA + 0x2C, 0x8001);
    r.w(GPIOA + 0x30, 0x0F0F);
    assert_eq!(r.r(GPIOA + 0x2C), 0x8001);
    assert_eq!(r.r(GPIOA + 0x30) & 0xFFFF, 0x0F0F);
    // LCKR sequence locks MODER for the selected pin.
    r.w(GPIOA + 0x1C, 0x0001_0020);
    r.w(GPIOA + 0x1C, 0x0000_0020);
    r.w(GPIOA + 0x1C, 0x0001_0020);
    assert_eq!(r.r(GPIOA + 0x1C), 0x0001_0020);
    r.w(GPIOA, 0xABFF_FFFF);
    assert_eq!(r.r(GPIOA), 0xABFF_F7FF, "locked PA5 config kept");
}

#[test]
fn gpio_inputs_and_exti_edges() {
    let mut r = Rig::new();
    r.enable_gpio(0x7); // GPIOA/B/C
    // PC13 input, PB13 input.
    r.w(GPIOC, 0xF3FF_FFFF);
    r.w(GPIOB, 0xF3FF_FEBF);
    let nvic_iser = 0xE000_E100u64;
    let nvic_ispr = 0xE000_E200u64;
    r.w(nvic_iser, 1 << 24); // EXTI13_IRQn
    r.w(EXTI + 0x6C, 0x200); // EXTICR4: PC13
    r.w(EXTI + 0x00, 0x2000); // RTSR1 line 13
    r.w(EXTI + 0x80, 0x2000); // IMR1 line 13

    let c = r.machine.bus.find_peripheral_index_by_name("gpioc").unwrap();
    let b = r.machine.bus.find_peripheral_index_by_name("gpiob").unwrap();
    r.machine.bus.set_peripheral_gpio_input(c, 13, true);
    r.step();
    assert_ne!(r.r(GPIOC + 0x10) & 0x2000, 0, "IDR sees the input");
    assert_eq!(r.r(EXTI + 0x0C), 0x2000, "RPR1");
    assert_ne!(r.r(nvic_ispr) & (1 << 24), 0, "NVIC pending");
    // Listening only to the selected port.
    r.w(EXTI + 0x0C, 0x2000);
    r.w(nvic_ispr, 1 << 24);
    r.machine.bus.set_peripheral_gpio_input(b, 13, true);
    r.step();
    assert_eq!(r.r(EXTI + 0x0C), 0, "PB13 ignored while PC13 selected");
    r.w(EXTI + 0x6C, 0x100); // PB13
    r.machine.bus.set_peripheral_gpio_input(b, 13, false);
    r.machine.bus.set_peripheral_gpio_input(b, 13, true);
    r.step();
    assert_eq!(r.r(EXTI + 0x0C), 0x2000, "PB13 now selected");
    // Software trigger regardless of edge selection.
    r.w(EXTI + 0x0C, 0x2000);
    r.w(EXTI + 0x08, 0x2000);
    assert_eq!(r.r(EXTI + 0x08), 0, "SWIER self-clears");
    assert_eq!(r.r(EXTI + 0x0C), 0x2000, "SWIER pends line 13");
}

// ── USART / LPUART ──────────────────────────────────────────────────────────

#[test]
fn usart_reset_values_prescaler_and_fifo_flags() {
    let mut r = Rig::powered();
    // Instance-specific ISR reset (RM0456 66.8.11 / 67.7.9).
    assert_eq!(r.r(USART1 + 0x1C), 0x0280_00C0, "USART1 ISR reset");
    assert_eq!(r.r(UART4 + 0x1C), 0x0080_00C0, "UART4 ISR reset");
    assert_eq!(r.r(LPUART1 + 0x1C), 0x0000_00C0, "LPUART1 ISR reset");
    for base in [USART1, UART4, LPUART1] {
        assert_eq!(r.r(base), 0, "CR1");
        assert_eq!(r.r(base + 0x0C), 0, "BRR");
        assert_eq!(r.r(base + 0x2C), 0, "PRESC");
        assert_eq!(r.r(base + 0x30), 0x8000_0000, "AUTOCR");
    }
    // PRESC is writable while UE=0, frozen while UE=1.
    r.w(USART1 + 0x2C, 5);
    assert_eq!(r.r(USART1 + 0x2C), 5, "PRESC stored");
    r.w(USART1, 1); // UE
    r.w(USART1 + 0x2C, 7);
    assert_eq!(r.r(USART1 + 0x2C), 5, "PRESC frozen while enabled");
    r.w(USART1, 0);
    r.w(USART1 + 0x2C, 3);
    assert_eq!(r.r(USART1 + 0x2C), 3);

    // FIFO mode: RXFTCFG=010 (half, 4 chars); RXFT after 4, RXFF after 8.
    let mut r = Rig::new();
    r.w(USART1 + 0x08, (2 << 25) | (1 << 28)); // CR3 RXFTCFG | RXFTIE
    r.w(USART1, 1 | (1 << 29)); // UE | FIFOEN
    let rx = r
        .machine
        .bus
        .attach_uart_rx_source_named("usart1")
        .unwrap();
    rx.lock().unwrap().extend(b"abc");
    for _ in 0..4_000 {
        r.step();
    }
    assert_eq!(r.r(USART1 + 0x1C) & (1 << 26), 0, "RXFT below threshold");
    rx.lock().unwrap().extend(b"d");
    for _ in 0..4_000 {
        r.step();
    }
    assert_ne!(r.r(USART1 + 0x1C) & (1 << 26), 0, "RXFT at half");
    assert_eq!(r.r(USART1 + 0x1C) & (1 << 24), 0, "RXFF not yet");
    rx.lock().unwrap().extend(b"efghij");
    for _ in 0..4_000 {
        r.step();
    }
    assert_ne!(r.r(USART1 + 0x1C) & (1 << 24), 0, "RXFF at depth 8");

    // TXFE is set from reset on USARTs, only in FIFO mode on LPUART.
    let mut r = Rig::new();
    assert_ne!(r.r(USART1 + 0x1C) & (1 << 23), 0, "USART1 TXFE");
    assert_eq!(r.r(LPUART1 + 0x1C) & (1 << 23), 0, "LPUART TXFE off");
    r.w(LPUART1, 1 << 29); // FIFOEN
    assert_ne!(r.r(LPUART1 + 0x1C) & (1 << 23), 0, "LPUART TXFE in FIFO");
}

// ── SPI ─────────────────────────────────────────────────────────────────────

struct Echo;
impl labwired_core::peripherals::device::SpiDevice for Echo {
    fn cs_pin(&self) -> &str {
        "CS"
    }
    fn transfer(&mut self, mosi: u8) -> u8 {
        mosi
    }
}

#[test]
fn spi3_limits_loopback_threshold_and_irq() {
    let mut r = Rig::powered();
    // SPI3 is limited: DSIZE 4..15 only, FTHLV 0..3, TSIZE up to 0x3FF.
    r.w(SPI1 + 0x08, 0x0007_01E7);
    r.w(SPI3 + 0x08, 0x0007_01E7);
    assert_eq!(field(r.r(SPI1 + 0x08), 5, 4), 0xF, "SPI1 FTHLV");
    assert_eq!(field(r.r(SPI3 + 0x08), 5, 4), 0x3, "SPI3 FTHLV");
    r.w(SPI1 + 0x08, 0x0007_001F);
    r.w(SPI3 + 0x08, 0x0007_001F);
    assert_eq!(field(r.r(SPI1 + 0x08), 0, 5), 0x1F, "SPI1 max DSIZE");
    assert_eq!(field(r.r(SPI3 + 0x08), 0, 5), 0x0F, "SPI3 max DSIZE");
    r.w(SPI1 + 0x04, 0xFFFF);
    r.w(SPI3 + 0x04, 0xFFFF);
    assert_eq!(field(r.r(SPI1 + 0x04), 0, 16), 0xFFFF, "SPI1 TSIZE");
    assert_eq!(field(r.r(SPI3 + 0x04), 0, 16), 0x03FF, "SPI3 TSIZE");

    // Full-duplex loopback: RXP after each frame, EOT + CSTART clear at end.
    let mut r = Rig::new();
    r.machine
        .bus
        .attach_spi_device("spi1", Box::new(SpiLoopback::default()))
        .unwrap();
    r.w(SPI1 + 0x08, 0x0007_0007);
    r.w(SPI1 + 0x0C, (1 << 22) | (1 << 26)); // MASTER | SSM
    r.w(SPI1 + 0x04, 2);
    r.w(SPI1 + 0x00, (1 << 12) | 1); // SSI | SPE
    r.w(SPI1 + 0x00, (1 << 12) | 1 | (1 << 9)); // CSTART
    r.w(SPI1 + 0x20, 0xA5);
    assert_ne!(r.r(SPI1 + 0x14) & 1, 0, "RXP");
    assert_eq!(r.r(SPI1 + 0x14) & (1 << 3), 0, "EOT not yet");
    r.w(SPI1 + 0x20, 0x5A);
    assert_ne!(r.r(SPI1 + 0x14) & (1 << 3), 0, "EOT");
    assert_eq!(r.r(SPI1 + 0x00) & (1 << 9), 0, "CSTART cleared");
    assert_eq!(r.r(SPI1 + 0x30), 0xA5, "first RX frame");
    assert_eq!(r.r(SPI1 + 0x30), 0x5A, "second RX frame");
    r.w(SPI1 + 0x18, 1 << 3); // EOTC
    assert_eq!(r.r(SPI1 + 0x14) & (1 << 3), 0, "EOT cleared");

    // RXPIE gates the interrupt line.
    let mut r = Rig::new();
    r.machine
        .bus
        .attach_spi_device("spi1", Box::new(Echo))
        .unwrap();
    r.w(SPI1 + 0x08, 0x0007_0007);
    r.w(SPI1 + 0x0C, (1 << 22) | (1 << 26));
    r.w(SPI1 + 0x04, 1);
    r.w(SPI1 + 0x10, 1); // RXPIE
    r.w(SPI1 + 0x00, (1 << 12) | 1);
    r.w(SPI1 + 0x00, (1 << 12) | 1 | (1 << 9));
    r.w(SPI1 + 0x20, 0x42);
    assert!(r.r(SPI1 + 0x14) & 1 != 0);
    // The SPI1 IRQ is NVIC line 59.
    assert!(r.machine.bus.is_nvic_irq_pending(59), "SPI1 IRQ pending");
    r.w(SPI1 + 0x30, 0);
    assert_eq!(r.r(SPI1 + 0x14) & 1, 0, "RXDR read clears RXP");
}

// ── I2C ─────────────────────────────────────────────────────────────────────

struct EchoI2c {
    address: u8,
    bytes: std::collections::VecDeque<u8>,
}

impl I2cDevice for EchoI2c {
    fn address(&self) -> u8 {
        self.address
    }
    fn read(&mut self) -> u8 {
        self.bytes.pop_front().unwrap_or(0)
    }
    fn write(&mut self, data: u8) {
        self.bytes.push_back(data);
    }
}

#[test]
fn i2c_write_read_and_nack() {
    let mut r = Rig::powered();
    r.machine
        .bus
        .attach_i2c_slave("i2c1", Box::new(EchoI2c { address: 0x42, bytes: Default::default() }))
        .unwrap();
    r.w(I2C1, 1); // PE
    // Write two bytes, AUTOEND.
    r.w(
        I2C1 + 0x04,
        (0x42 << 1) | (2 << 16) | (1 << 25) | (1 << 13),
    );
    assert_ne!(r.r(I2C1 + 0x18) & 2, 0, "TXIS");
    r.w(I2C1 + 0x28, 0x11);
    r.w(I2C1 + 0x28, 0x22);
    assert_ne!(r.r(I2C1 + 0x18) & (1 << 5), 0, "STOPF");
    r.w(I2C1 + 0x1C, 1 << 5);
    assert_eq!(r.r(I2C1 + 0x18) & (1 << 5), 0, "STOPF W1C");

    // Read back the two bytes echoed by the target.
    r.w(I2C1 + 0x04, (0x42 << 1) | (1 << 10) | (2 << 16) | (1 << 25) | (1 << 13));
    assert_ne!(r.r(I2C1 + 0x18) & 4, 0, "RXNE");
    assert_eq!(r.r(I2C1 + 0x24), 0x11);
    assert_eq!(r.r(I2C1 + 0x24), 0x22);
    assert_ne!(r.r(I2C1 + 0x18) & (1 << 5), 0, "STOPF");

    // No target: NACK + STOP.
    let mut r = Rig::new();
    r.w(I2C1, 1);
    r.w(
        I2C1 + 0x04,
        (0x50 << 1) | (1 << 16) | (1 << 25) | (1 << 13),
    );
    assert_ne!(r.r(I2C1 + 0x18) & (1 << 4), 0, "NACKF");
    assert_ne!(r.r(I2C1 + 0x18) & (1 << 5), 0, "STOPF");
    r.w(I2C1 + 0x1C, 1 << 4);
    assert_eq!(r.r(I2C1 + 0x18) & (1 << 4), 0, "NACKF W1C");
}

// ── GPDMA ───────────────────────────────────────────────────────────────────

#[test]
fn gpdma_bndt_bytes_irq_split_and_peripheral_routing() {
    let mut r = Rig::powered();
    r.enable(RCC + 0x88, 1); // GPDMA1EN
    let chan = |ch: u64| GPDMA1 + 0x50 + 0x80 * ch;
    // Channel 0 memory-to-memory: 32-bit words, SINC|DINC, BNDT = 8 bytes.
    let src = 0x2000_1000u64;
    let dst = 0x2000_2000u64;
    r.w(src, 0x1111_1111);
    r.w(src + 4, 0x2222_2222);
    r.w(dst + 8, 0xAAAA_AAAA);
    r.w(chan(0) + 0x40, (2 << 0) | (1 << 3) | (2 << 16) | (1 << 19));
    r.w(chan(0) + 0x44, 1 << 9); // SWREQ
    r.w(chan(0) + 0x48, 8); // BNDT bytes
    r.w(chan(0) + 0x4C, src as u32);
    r.w(chan(0) + 0x50, dst as u32);
    r.w(chan(0) + 0x14, (1 << 8) | 1); // TCIE | EN
    for _ in 0..20 {
        r.step();
    }
    assert_eq!(r.r(dst), 0x1111_1111);
    assert_eq!(r.r(dst + 4), 0x2222_2222);
    assert_eq!(r.r(dst + 8), 0xAAAA_AAAA, "BNDT is bytes, not beats");
    assert_eq!(r.r(chan(0) + 0x48) & 0xFFFF, 0, "BNDT drained");
    assert_ne!(r.r(chan(0) + 0x10) & (1 << 8), 0, "TCF");
    assert!(r.machine.bus.is_nvic_irq_pending(29), "channel 0 IRQ 29");

    // Channel 8 and 15 use NVIC 80..87, not 37..44.
    for (ch, irq) in [(8u64, 80u32), (15, 87)] {
        let mut r = Rig::new();
        r.enable(RCC + 0x88, 1);
        r.w(chan(ch) + 0x40, (2 << 0) | (1 << 3) | (2 << 16) | (1 << 19));
        r.w(chan(ch) + 0x44, 1 << 9);
        r.w(chan(ch) + 0x48, 8);
        r.w(chan(ch) + 0x4C, src as u32);
        r.w(chan(ch) + 0x50, dst as u32);
        r.w(chan(ch) + 0x14, (1 << 8) | 1); // TCIE | EN
        for _ in 0..20 {
            r.step();
        }
        assert!(
            r.machine.bus.is_nvic_irq_pending(irq),
            "channel {ch} pends IRQ {irq}"
        );
    }

    // Peripheral request routing: channel 0 REQSEL = SPI1_TX (7), enabled in
    // peripheral-request mode. A SPI TX request moves one byte to TXDR.
    let mut r = Rig::new();
    r.enable(RCC + 0x88, 1); // GPDMA1
    r.enable(RCC + 0xA4, 1 << 12); // SPI1EN
    r.w(SPI1 + 0x08, 0x0007_0007);
    r.w(SPI1 + 0x0C, (1 << 22) | (1 << 26));
    r.w(SPI1 + 0x04, 1);
    r.w(SPI1 + 0x00, (1 << 12) | 1);
    r.w(SPI1 + 0x00, (1 << 12) | 1 | (1 << 9));
    r.w(chan(0) + 0x40, (1 << 3) | (2 << 16)); // SINC, DDW 32? byte source
    r.w(chan(0) + 0x44, 7); // REQSEL = SPI1_TX, SWREQ=0
    r.w(chan(0) + 0x48, 1);
    r.w(chan(0) + 0x4C, src as u32);
    r.w(chan(0) + 0x50, SPI1 as u32 + 0x20);
    r.w(chan(0) + 0x14, 1); // EN
    r.w(SPI1 + 0x08, r.r(SPI1 + 0x08) | (1 << 15)); // TXDMAEN
    r.w(src, 0x0000_00A5);
    for _ in 0..5_000 {
        r.step();
    }
    assert_ne!(r.r(SPI1 + 0x14) & (1 << 4), 0, "SPI TXTF after DMA frame");
}
