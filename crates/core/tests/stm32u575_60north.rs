// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! 60North STM32U5 behavioural acceptance suite.
//!
//! The 60North `renode-stm32u5` package (platform `.repl`, peripheral models,
//! Robot tests and firmware) is the behavioural acceptance reference for the
//! STM32U575 target. Its 13 prebuilt ELFs are run here against the LabWired
//! U575 system and checked on the result symbols/registers the Robot tests
//! check — not on "exit 0" or a UART banner.
//!
//! Firmware is an external, non-committed artifact: point
//! `RENODE_U5_FIRMWARE` at the directory holding the ELFs (default is the
//! 60North worktree build directory).
//!
//! 60North is a Renode-derived reference, not silicon. Passing here means
//! "matches the reference model's documented behaviour", not "matches a bench
//! part".

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::device::I2cDevice;
use labwired_core::peripherals::spi_loopback::SpiLoopback;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Bus, Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const DEFAULT_FW_DIR: &str = "/home/andrii/projects/renode-stm32u5-60north/build/firmware";
const SYSTEM_NUCLEO: &str = "configs/systems/nucleo-u575zi.yaml";

const GPIOA: u64 = 0x4202_0000;
const GPIOB: u64 = 0x4202_0400;
const GPIOC: u64 = 0x4202_0800;
const GPIOG: u64 = 0x4202_1800;
const GPIOH: u64 = 0x4202_1C00;
const ODR: u64 = 0x14;
const RCC_CFGR1: u64 = 0x4602_0C1C;
const FLASH_ACR: u64 = 0x4002_2000;
const PWR_VOSR: u64 = 0x4602_080C;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn fw_dir() -> PathBuf {
    std::env::var("RENODE_U5_FIRMWARE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_FW_DIR))
}

struct Rig {
    machine: Machine<labwired_core::cpu::CortexM>,
    uart: Arc<Mutex<Vec<u8>>>,
    elf: Vec<u8>,
}

impl Rig {
    fn boot(elf_name: &str, system_rel: &str) -> Self {
        let sys_path = root(system_rel);
        let manifest: SystemManifest =
            SystemManifest::from_file(&sys_path).unwrap_or_else(|e| panic!("{system_rel}: {e:#}"));
        let chip_path = sys_path.parent().unwrap().join(&manifest.chip);
        let chip = ChipDescriptor::from_file(&chip_path)
            .unwrap_or_else(|e| panic!("chip {}: {e:#}", chip_path.display()));
        let mut bus = SystemBus::from_config(&chip, &manifest)
            .unwrap_or_else(|e| panic!("build {system_rel}: {e:#}"));
        let uart = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(uart.clone(), false);
        let (cpu, _) = configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        // Same batching contract as `labwired run`: without a tick interval
        // wider than one the advance path pins the CPU to single steps and a
        // HAL_Delay(50) costs tens of seconds per example.
        let interval = machine.bus.max_safe_tick_interval();
        machine.config.peripheral_tick_interval = interval;
        machine.bus.config.peripheral_tick_interval = interval;
        let elf_path = fw_dir().join(elf_name);
        let elf = std::fs::read(&elf_path).unwrap_or_else(|e| {
            panic!(
                "60North firmware {} is unreadable ({e}); set RENODE_U5_FIRMWARE",
                elf_path.display()
            )
        });
        let image = labwired_loader::load_elf(&elf_path).unwrap();
        machine.load_firmware(&image).unwrap();
        Self {
            machine,
            uart,
            elf,
        }
    }

    fn symbol(&self, name: &str) -> u64 {
        u64::from(
            labwired_loader::resolve_symbol_in_elf(&self.elf, name)
                .unwrap_or_else(|| panic!("symbol {name} not found in 60North ELF")),
        )
    }

    fn u32_at(&self, addr: u64) -> u32 {
        self.machine.bus.read_u32(addr).unwrap()
    }

    fn symbol_u32(&self, name: &str) -> u32 {
        self.u32_at(self.symbol(name))
    }

    fn step(&mut self, n: u64) {
        for _ in 0..n {
            self.machine.step().expect("step");
        }
    }

    /// Run a cycle budget through the batched/idle-fast-forward path (the same
    /// one `labwired run` uses). `HAL_Delay` busy-waits cost real steps on the
    /// single-step path, so the examples' 50-250 ms delays need this.
    fn run_cycles(&mut self, steps: u64) {
        self.machine
            .advance(AdvanceRequest::run(Some(steps)))
            .expect("advance");
    }

    fn run_until<F: Fn(&Self) -> bool>(&mut self, max: u64, what: &str, done: F) {
        let mut spent = 0u64;
        while spent < max {
            if done(self) {
                return;
            }
            let chunk = (max - spent).min(100_000);
            self.machine
                .advance(AdvanceRequest::run(Some(chunk)))
                .expect("advance");
            spent += chunk;
        }
        if done(self) {
            return;
        }
        panic!(
            "timed out after {max} steps waiting for {what}\n{}",
            self.diag()
        );
    }

    fn bytes_at(&self, addr: u64, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| self.machine.bus.read_u8(addr + i as u64).unwrap())
            .collect()
    }

    fn symbol_bytes(&self, name: &str, len: usize) -> Vec<u8> {
        self.bytes_at(self.symbol(name), len)
    }

    /// Symbol/PC/fault snapshot used only to make a timeout diagnosable.
    fn diag(&self) -> String {
        let mut out = format!("PC={:#010x}\n", self.machine.cpu.get_pc());
        for name in [
            "step",
            "failedStep",
            "failedStage",
            "stage",
            "halStatus",
            "ready",
            "transferComplete",
            "transferError",
            "receiveComplete",
            "dataOk",
            "ringHead",
            "receivedCount",
        ] {
            if let Some(addr) = labwired_loader::resolve_symbol_in_elf(&self.elf, name) {
                if let Ok(v) = self.machine.bus.read_u32(u64::from(addr)) {
                    out.push_str(&format!("{name}={v:#x} "));
                }
            }
        }
        let groups: [(&str, u64, &[u64]); 4] = [
            ("SPI1", 0x4001_3000, &[0x00, 0x04, 0x08, 0x0C, 0x10, 0x14, 0x18, 0x20, 0x30]),
            ("GPDMA", 0x4002_0000, &[0x64, 0x94, 0x98, 0x9C, 0xA0]),
            ("DMA1", 0x4002_0080, &[0x14, 0x44, 0x48, 0x4C, 0x50]),
            ("RCC", 0x4602_0C00, &[0x00, 0x04, 0x1C]),
        ];
        for (label, base, offs) in groups {
            out.push_str(&format!("{label}:"));
            for off in offs {
                out.push_str(&format!(" {off:x}={:#x}", self.u32_at(base + off)));
            }
            out.push(' ');
        }
        out
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.uart.lock().unwrap()).into_owned()
    }

    fn gpio_odr(&self, base: u64, pin: u8) -> bool {
        self.u32_at(base + ODR) & (1 << pin) != 0
    }

    fn gpio_idx(&self, name: &str) -> usize {
        self.machine
            .bus
            .find_peripheral_index_by_name(name)
            .unwrap_or_else(|| panic!("peripheral {name}"))
    }

    fn set_input(&mut self, port: &str, pin: u8, level: bool) {
        let idx = self.gpio_idx(port);
        self.machine
            .bus
            .set_peripheral_gpio_input(idx, pin, level);
    }

    fn uart_rx(&self, name: &str) -> Arc<Mutex<std::collections::VecDeque<u8>>> {
        self.machine
            .bus
            .attach_uart_rx_source_named(name)
            .unwrap_or_else(|| panic!("UART {name} has no RX source"))
    }
}

fn assert_symbol(rig: &Rig, name: &str, expected: u32) {
    assert_eq!(rig.symbol_u32(name), expected, "symbol {name}");
}

// ── Bare-metal core ─────────────────────────────────────────────────────────

#[test]
fn core_smoke_maps_memories_and_services_interrupts() {
    let mut r = Rig::boot("core-smoke.elf", SYSTEM_NUCLEO);
    r.step(60_000);
    let done = 1u32 << 31;
    assert_eq!(r.symbol_u32("passedChecks") & done, done, "CHECK_DONE");
    assert_eq!(r.symbol_u32("flashSizeKb"), 2048, "F_SIZE = 2048 KB");
    assert_eq!(r.symbol_u32("irqLogCount"), 2, "first and last IRQ lines");
    for flag in 0..9 {
        let bit = 1u32 << flag;
        assert_eq!(
            r.symbol_u32("passedChecks") & bit,
            bit,
            "passedChecks bit {flag}"
        );
    }
    let ticks = r.symbol_u32("sysTickCount");
    r.step(4_000_000);
    assert!(
        r.symbol_u32("sysTickCount") > ticks,
        "SysTick keeps ticking"
    );
    assert_eq!(r.symbol_u32("faultCount"), 0);
}

#[test]
fn core_smoke_restarts_after_machine_reset() {
    let mut r = Rig::boot("core-smoke.elf", SYSTEM_NUCLEO);
    r.step(2_000_000);
    r.machine.reset();
    r.step(2_000_000);
    let done = 1u32 << 31;
    assert_eq!(r.symbol_u32("passedChecks") & done, done);
}

// ── Bare-metal clock ────────────────────────────────────────────────────────

#[test]
fn clock_init_reaches_160mhz_and_done_stage() {
    let mut r = Rig::boot("clock-init.elf", SYSTEM_NUCLEO);
    r.run_until(2_000_000, "clock stage 6", |r| {
        r.symbol_u32("stage") == 6 || r.symbol_u32("failedStage") != 0
    });
    assert_eq!(r.symbol_u32("failedStage"), 0, "failedStage");
    assert_eq!(r.symbol_u32("stage"), 6, "stage");
    assert_eq!((r.u32_at(RCC_CFGR1) >> 2) & 3, 3, "SWS = PLL1");
    assert_ne!(r.u32_at(0x4602_0C00) & (1 << 25), 0, "PLL1RDY");
    assert_eq!(r.u32_at(FLASH_ACR) & 0xF, 4, "flash latency 4");
    assert_eq!((r.u32_at(PWR_VOSR) >> 16) & 3, 3, "VOS range 1");
    assert_eq!(r.u32_at(PWR_VOSR) & 0xC000, 0xC000, "VOSRDY|BOOSTRDY");
}

// ── HAL clock ───────────────────────────────────────────────────────────────

#[test]
fn hal_clock_completes_and_reports_160mhz() {
    let mut r = Rig::boot("hal-clock.elf", SYSTEM_NUCLEO);
    r.run_until(2_000_000, "HAL clock step 7", |r| {
        r.symbol_u32("step") == 7 || r.symbol_u32("failedStep") != 0
    });
    assert_eq!(r.symbol_u32("failedStep"), 0);
    assert_eq!(r.symbol_u32("halStatus"), 0);
    assert_eq!(r.symbol_u32("step"), 7);
    for s in [
        "sysclkHz",
        "hclkHz",
        "pclk1Hz",
        "pclk2Hz",
        "pclk3Hz",
        "systemCoreClockHz",
    ] {
        assert_eq!(r.symbol_u32(s), 160_000_000, "{s}");
    }
    let before = r.symbol_u32("tickBeforeDelay");
    let after = r.symbol_u32("tickAfterDelay");
    assert_eq!(after - before, 11, "HAL_Delay(10) waits 11 ticks");
}

// ── HAL GPIO/EXTI ───────────────────────────────────────────────────────────

#[test]
fn hal_gpio_toggles_mirrors_and_counts_rising_edges() {
    let mut r = Rig::boot("hal-gpio.elf", SYSTEM_NUCLEO);
    r.run_until(1_000_000, "GPIO ready", |r| r.symbol_u32("ready") == 1);
    assert_eq!(r.symbol_u32("toggleOk"), 1);
    assert_eq!(r.symbol_u32("mirroredLevel"), 0);

    r.set_input("gpiob", 13, true);
    r.step(50_000);
    assert_eq!(r.symbol_u32("mirroredLevel"), 1);
    assert!(r.gpio_odr(GPIOA, 5), "PA5 mirrors PB13");
    r.set_input("gpiob", 13, false);
    r.step(50_000);
    assert_eq!(r.symbol_u32("mirroredLevel"), 0);
    assert!(!r.gpio_odr(GPIOA, 5));

    assert_eq!(r.symbol_u32("risingEdges"), 0);
    r.set_input("gpioc", 13, true);
    r.step(50_000);
    assert_eq!(r.symbol_u32("risingEdges"), 1);
    assert_eq!(r.symbol_u32("lastRisingPin"), 0x2000);
    r.set_input("gpioc", 13, false);
    r.step(50_000);
    assert_eq!(r.symbol_u32("risingEdges"), 1, "falling edge ignored");
    r.set_input("gpioc", 13, true);
    r.step(50_000);
    assert_eq!(r.symbol_u32("risingEdges"), 2);
}

// ── HAL UART ────────────────────────────────────────────────────────────────

#[test]
fn hal_uart_prints_configures_baud_and_echoes() {
    let mut r = Rig::boot("hal-uart.elf", SYSTEM_NUCLEO);
    r.run_until(3_000_000, "UART receiving", |r| {
        r.text().contains("Interrupt transmit done")
            && r.symbol_u32("step") >= 12
    });
    assert_eq!(r.symbol_u32("failedStep"), 0);
    assert_eq!(r.symbol_u32("usartBrr"), 1389);
    let rx = r.uart_rx("usart1");
    rx.lock().unwrap().extend(b"hello renode");
    r.run_until(4_000_000, "uppercase echo", |r| {
        r.text().contains("HELLO RENODE")
    });
    assert_eq!(r.symbol_u32("receivedCount"), 12);
}

// ── HAL SPI (polling / interrupt) ───────────────────────────────────────────

#[test]
fn hal_spi_loops_back_8_and_16_bit_frames() {
    let mut r = Rig::boot("hal-spi.elf", SYSTEM_NUCLEO);
    r.machine
        .bus
        .attach_spi_device("spi1", Box::new(SpiLoopback::default()))
        .unwrap();
    r.machine
        .bus
        .attach_spi_device("spi3", Box::new(SpiLoopback::default()))
        .unwrap();
    r.run_until(4_000_000, "SPI step 12", |r| {
        r.symbol_u32("step") == 12 || r.symbol_u32("failedStep") != 0
    });
    assert_eq!(r.symbol_u32("failedStep"), 0, "{}", r.diag());
    assert_eq!(r.symbol_u32("step"), 12, "{}", r.diag());
    for s in ["polling8Ok", "interrupt8Ok", "polling16Ok", "limited8Ok"] {
        assert_eq!(r.symbol_u32(s), 1, "{s}");
    }
    assert_eq!(r.symbol_u32("transferComplete"), 1);
}

// ── HAL SPI DMA + loopback ──────────────────────────────────────────────────

#[test]
fn hal_spi_dma_transmits_all_bytes() {
    let mut r = Rig::boot("hal-spi-dma.elf", SYSTEM_NUCLEO);
    r.run_until(4_000_000, "SPI DMA step 6", |r| {
        r.symbol_u32("step") == 6 || r.symbol_u32("failedStep") != 0
    });
    assert_eq!(r.symbol_u32("failedStep"), 0);
    assert_eq!(r.symbol_u32("step"), 6);
    assert_eq!(r.symbol_u32("transferError"), 0);
    assert_eq!(r.symbol_u32("transferComplete"), 1);
}

#[test]
fn hal_spi_dma_loopback_receives_what_it_sent() {
    let mut r = Rig::boot("hal-spi-dma-loopback.elf", SYSTEM_NUCLEO);
    r.machine
        .bus
        .attach_spi_device("spi1", Box::new(SpiLoopback::default()))
        .unwrap();
    r.run_until(6_000_000, "SPI DMA loopback step 6", |r| {
        r.symbol_u32("step") == 6 || r.symbol_u32("failedStep") != 0
    });
    assert_eq!(r.symbol_u32("failedStep"), 0);
    assert_eq!(r.symbol_u32("step"), 6);
    assert_eq!(r.symbol_u32("transferError"), 0);
    assert_eq!(r.symbol_u32("transferComplete"), 1);
    assert_eq!(r.symbol_u32("dataOk"), 1);
}

// ── HAL UART DMA ────────────────────────────────────────────────────────────

#[test]
fn hal_uart_dma_receives_text() {
    let mut r = Rig::boot("hal-uart-dma.elf", SYSTEM_NUCLEO);
    r.run_until(3_000_000, "UART DMA ready banner", |r| {
        r.text().contains("STM32U575 HAL UART DMA ready")
    });
    let rx = r.uart_rx("usart1");
    rx.lock().unwrap().extend(b"hello renode");
    r.run_until(4_000_000, "UART DMA receive complete", |r| {
        r.symbol_u32("step") == 10 || r.symbol_u32("failedStep") != 0
    });
    assert_eq!(r.symbol_u32("failedStep"), 0);
    assert_eq!(r.symbol_u32("step"), 10);
    assert_eq!(r.symbol_u32("receiveError"), 0);
    assert_eq!(r.symbol_u32("receiveComplete"), 1);
    assert_eq!(r.symbol_u32("dataOk"), 1);
}

// ── ST examples ─────────────────────────────────────────────────────────────

#[test]
fn st_spi_fullduplex_polling_master_sends_and_receives() {
    let mut r = Rig::boot("st-spi-fullduplex-polling-master.elf", SYSTEM_NUCLEO);
    r.machine
        .bus
        .attach_spi_device("spi1", Box::new(SpiLoopback::default()))
        .unwrap();
    // Hold the user button from boot, so the example's wait loop exits at its
    // first check instead of after a 100 ms toggle delay.
    r.set_input("gpioc", 13, true);
    r.run_until(5_000_000, "SPI transfer outcome", |r| {
        r.gpio_odr(GPIOB, 7) || r.gpio_odr(GPIOG, 2)
    });
    r.set_input("gpioc", 13, false);
    assert!(!r.gpio_odr(GPIOG, 2), "LED3 error off");
    assert!(r.gpio_odr(GPIOB, 7), "LED2 on after successful transfer");
    assert!(r.gpio_odr(GPIOC, 7), "LED1 on after successful transfer");
    // BUFFERSIZE = sizeof(aTxBuffer) - 1 = 0x73.
    let tx = r.symbol_bytes("aTxBuffer", 0x73);
    let rx = r.symbol_bytes("aRxBuffer", 0x73);
    assert_eq!(rx, tx, "SPI loopback received buffer equals the sent buffer");
}

struct EchoI2c {
    address: u8,
    pending: std::collections::VecDeque<u8>,
}

impl EchoI2c {
    fn new(address: u8) -> Self {
        Self {
            address,
            pending: std::collections::VecDeque::new(),
        }
    }
}

impl I2cDevice for EchoI2c {
    fn address(&self) -> u8 {
        self.address
    }
    fn read(&mut self) -> u8 {
        self.pending.pop_front().unwrap_or(0)
    }
    fn write(&mut self, data: u8) {
        self.pending.push_back(data);
    }
}

#[test]
fn st_i2c_twoboards_master_writes_and_reads_back() {
    let mut r = Rig::boot("st-i2c-twoboards-polling-master.elf", SYSTEM_NUCLEO);
    // I2C_ADDRESS is 0x30F (10-bit); the L4-profile controller matches on
    // SADD[7:1], i.e. 0x07.
    r.machine
        .bus
        .attach_i2c_slave("i2c1", Box::new(EchoI2c::new(0x07)))
        .unwrap();
    // First press/release: the example waits 50 ms (debounce) then transmits.
    r.set_input("gpioc", 13, true);
    r.run_cycles(10_000_000);
    r.set_input("gpioc", 13, false);
    r.run_until(5_000_000, "I2C transmit outcome", |r| {
        r.gpio_odr(GPIOB, 7) || r.gpio_odr(GPIOG, 2)
    });
    assert!(r.gpio_odr(GPIOB, 7), "LED2 on after transmit");
    assert!(!r.gpio_odr(GPIOG, 2), "LED3 error off after transmit");
    // Second press/release: receive; LED2 off and buffers must match.
    r.set_input("gpioc", 13, true);
    r.run_cycles(10_000_000);
    r.set_input("gpioc", 13, false);
    r.run_until(5_000_000, "I2C receive outcome", |r| {
        !r.gpio_odr(GPIOB, 7) || r.gpio_odr(GPIOG, 2)
    });
    assert!(!r.gpio_odr(GPIOB, 7), "LED2 off after receive");
    assert!(!r.gpio_odr(GPIOG, 2), "LED3 error off after receive");
    // TXBUFFERSIZE = sizeof(aTxBuffer) - 1 = 0xA2.
    let tx = r.symbol_bytes("aTxBuffer", 0xA2);
    let rx = r.symbol_bytes("aRxBuffer", 0xA2);
    assert_eq!(rx, tx, "I2C echo received buffer equals the sent buffer");
}

#[test]
fn st_dma_flash_to_ram_copies_and_blinks() {
    let mut r = Rig::boot("st-dma-flash-to-ram.elf", SYSTEM_NUCLEO);
    r.run_until(8_000_000, "GPDMA completion", |r| {
        r.symbol_u32("transferCompleteDetected") != 0 || r.symbol_u32("transferErrorDetected") != 0
    });
    assert_eq!(r.symbol_u32("transferErrorDetected"), 0, "DMA error flag");
    assert_eq!(r.symbol_u32("transferCompleteDetected"), 1, "DMA complete");
    assert!(!r.gpio_odr(GPIOG, 2), "LED3 error off");
    let src = r.symbol_bytes("aSRC_Const_Buffer", 128);
    let dst = r.symbol_bytes("aDST_Buffer", 128);
    assert_eq!(dst, src, "GPDMA copied all 128 bytes");
}

#[test]
fn st_fdcan_loopback_receives_its_own_frames() {
    let mut r = Rig::boot("st-fdcan-loopback.elf", SYSTEM_NUCLEO);
    r.run_until(8_000_000, "FDCAN frame processing", |r| {
        r.symbol_bytes("RxData", 12) != [0u8; 12] || r.gpio_odr(GPIOH, 7)
    });
    assert!(!r.gpio_odr(GPIOH, 7), "LED6 error off");
    // The example retrieves FIFO0 (TxData0), then FIFO1 twice; the last
    // retrieved payload is TxData2.
    let rx = r.symbol_bytes("RxData", 12);
    let expected = r.symbol_bytes("TxData2", 12);
    assert_eq!(rx, expected, "last FDCAN loopback payload");
}
