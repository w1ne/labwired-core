//! Reference-derived AT32F421G8U7 contract tests (Artery BSP 2.1.9 headers,
//! RM_AT32F421 V2.05). No silicon or flight claims.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus,
    peripherals::{
        flash::{Flash, FlashOp, FlashRegisterLayout},
        rcc::{Rcc, RccRegisterLayout},
    },
    Bus, FlashStoreOutcome, Peripheral,
};
use std::path::Path;

fn bus() -> SystemBus {
    let chip = ChipDescriptor::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/at32f421g8u7.yaml"),
    )
    .unwrap();
    let system: SystemManifest =
        serde_yaml::from_str("name: at32-test\nchip: at32f421g8u7\nexternal_devices: []\n")
            .unwrap();
    SystemBus::from_config(&chip, &system).unwrap()
}

#[test]
fn exact_package_memory_clock_gates_and_pad_routes() {
    let mut b = bus();
    assert_eq!(b.flash.data.len(), 65536);
    assert_eq!(b.ram.data.len(), 16384);
    b.write_u32(0x48000014, 1 << 5).unwrap();
    assert_eq!(b.read_u32(0x48000014).unwrap(), 0, "GPIO unclocked");
    b.write_u32(0x40021014, (1 << 17) | (1 << 18)).unwrap();
    b.write_u32(0x48000000, 1 << 10).unwrap();
    b.write_u32(0x48000014, 1 << 5).unwrap();
    assert_eq!(b.read_u32(0x48000014).unwrap(), 1 << 5);
    b.write_u32(0x48000000, 2 << 4).unwrap();
    b.write_u32(0x48000020, 1 << 8).unwrap();
    let gpio = b.find_peripheral_index_by_name("gpioa").unwrap();
    assert_eq!(
        b.peripherals[gpio]
            .dev
            .gpio_routing(2)
            .unwrap()
            .func
            .as_deref(),
        Some("USART2_TX")
    );
    b.write_u32(0x48000400, 2 << 8).unwrap();
    b.write_u32(0x48000420, 1 << 16).unwrap();
    let gpio = b.find_peripheral_index_by_name("gpiob").unwrap();
    assert_eq!(
        b.peripherals[gpio]
            .dev
            .gpio_routing(4)
            .unwrap()
            .func
            .as_deref(),
        Some("TMR3_CH1")
    );
    assert!(
        labwired_core::peripherals::stm32g0_clock::validate_timed_uart_clock(&b, "usart2").is_err(),
        "shared AF1 must not misidentify AT32 CRM as STM32G0 RCC"
    );
}

#[test]
fn crm_never_announces_an_unfitted_external_clock() {
    let mut r = Rcc::new_with_layout(RccRegisterLayout::At32F421);
    assert_eq!(r.read_u32(0).unwrap(), 0x83);
    r.write_u32(0, 0x0101_0001).unwrap();
    r.write_u32(4, (1 << 16) | 2).unwrap();
    assert_eq!(r.read_u32(0).unwrap() & ((1 << 17) | (1 << 25)), 0);
    assert_eq!(r.read_u32(4).unwrap() & 12, 0);
    r.set_hse_present(true);
    assert_eq!(
        r.read_u32(0).unwrap() & ((1 << 17) | (1 << 25)),
        (1 << 17) | (1 << 25)
    );
    assert_eq!(r.read_u32(4).unwrap() & 12, 8);
    r.write_u32(0x14, u32::MAX).unwrap();
    assert_eq!(
        r.read_u32(0x14).unwrap(),
        0x004e_0055,
        "only implemented AHB clocks"
    );
}

/// RM_AT32F421 V2.05 §5.2: KEY1/KEY2 unlock, byte/half-word/word program
/// with FPRGM, PRGMERR on a location that is not erased (unless the data is
/// zero), sector/bank erase, and a wrong key sequence as a bus error.
#[test]
fn flash_unlock_program_erase_geometry_and_failure_paths() {
    let mut f = Flash::new_with_layout(FlashRegisterLayout::At32F421);
    assert_eq!(f.flash_geometry(), (65536, 1024));
    assert!(
        matches!(
            f.flash_cpu_store(0xfc00, 4, 0, u32::MAX),
            FlashStoreOutcome::Unsupported(_)
        ),
        "store while locked"
    );
    assert!(f.write_u32(0x10, 1).is_ok(), "CTRL is ignored while locked");
    assert_eq!(f.read_u32(0x10).unwrap(), 0x0002_0080);
    f.write_u32(4, 0x45670123).unwrap();
    f.write_u32(4, 0xcdef89ab).unwrap();
    f.write_u32(0x10, 1).unwrap();
    for width in [1, 2, 4] {
        assert_eq!(
            f.flash_cpu_store(0xfc00, width, 0x12, u32::MAX),
            FlashStoreOutcome::Program,
            "width {width}"
        );
    }
    assert_eq!(f.read_u32(0x0c).unwrap(), 0x20, "ODF");
    assert_eq!(
        f.flash_cpu_store(0xfc00, 4, 0x12345678, 0xffff_00ff),
        FlashStoreOutcome::Consumed,
        "not erased"
    );
    assert_eq!(f.read_u32(0x0c).unwrap() & 4, 4, "PRGMERR");
    assert_eq!(
        f.flash_cpu_store(0xfc00, 4, 0, 0x1234_5678),
        FlashStoreOutcome::Program,
        "all-zero data programs a written location"
    );
    assert!(matches!(
        f.flash_cpu_store(0xfc01, 4, 0, u32::MAX),
        FlashStoreOutcome::Unsupported(_)
    ));
    f.write_u32(0x0c, 0x34).unwrap();
    assert_eq!(f.read_u32(0x0c).unwrap(), 0);
    f.write_u32(0x14, 0x0800fc00).unwrap();
    assert_eq!(f.read_u32(0x14).unwrap(), 0, "FLASH_ADDR is write-only");
    f.write_u32(0x10, 2 | 64).unwrap();
    assert_eq!(
        f.drain_pending_op(),
        Some(FlashOp::EraseSector {
            bank: 0,
            sector: 63
        })
    );
    f.write_u32(0x10, 4 | 64).unwrap();
    assert_eq!(
        f.drain_pending_op(),
        Some(FlashOp::EraseBothBanks),
        "AT32 has one 64KiB bank"
    );
    assert!(
        f.write_u32(0x10, 64).is_err(),
        "ERSTR without an erase kind"
    );
    f.write_u32(0x14, 0x0801_0000).unwrap();
    assert!(
        f.write_u32(0x10, 2 | 64).is_err(),
        "sector outside main flash"
    );
    assert!(f.write_u32(0x10, 1 << 4).is_err(), "USD programming");
    f.write_u32(0x10, 128).unwrap();
    assert!(matches!(
        f.flash_cpu_store(0xfc00, 4, 0, u32::MAX),
        FlashStoreOutcome::Unsupported(_)
    ));
    // A wrong key sequence is a bus error and locks out until reset.
    f.write_u32(4, 0x45670123).unwrap();
    assert!(f.write_u32(4, 0x1111_1111).is_err());
    f.write_u32(4, 0x45670123).unwrap_err();
    f.on_system_reset();
    f.write_u32(4, 0x45670123).unwrap();
    f.write_u32(4, 0xcdef89ab).unwrap();
    assert_eq!(f.read_u32(0x10).unwrap() & 0x80, 0);
}

/// DMA1 through the descriptor: five channels, so no CCR6 and no CSELR.
/// The memory-to-memory direction (DTD=0 reads PADDR) is exercised by the
/// BSP smoke below and by the shared `dma` unit tests.
#[test]
fn dma_has_five_channels() {
    let mut b = bus();
    b.write_u32(0x40021014, 1).unwrap(); // AHBEN DMA1.
    assert!(b.write_u32(0x40020000 + 0x6c, 1).is_err(), "no channel 6");
    assert!(b.write_u32(0x40020000 + 0xa8, 1).is_err(), "no CSELR");
    b.write_u32(0x40020000 + 0x58 + 4, 3).unwrap();
    assert_eq!(b.read_u32(0x40020000 + 0x58 + 4).unwrap(), 3, "CDTCNT5");
}

#[test]
fn exint_gpio_groups_use_vendor_vectors_and_only_existing_lines() {
    let mut b = bus();
    let exti = b.find_peripheral_index_by_name("exti").unwrap();
    for (bit, irq) in [(0, 5), (3, 6), (15, 7)] {
        b.write_u32(0x40010400, 1 << bit).unwrap();
        b.write_u32(0x40010410, 1 << bit).unwrap();
        let mut high = Vec::new();
        b.peripherals[exti].dev.irq_line_levels(&mut |line, level| {
            if level {
                high.push(line);
            }
        });
        assert_eq!(high, vec![irq], "line {bit}");
        b.write_u32(0x40010414, 1 << bit).unwrap();
    }
    b.write_u32(0x40010400, u32::MAX).unwrap();
    assert_eq!(b.read_u32(0x40010400).unwrap(), 0x002b_ffff);
}

#[test]
fn resets_release_gpio_and_relock_flash_without_erasing_storage_or_external_inputs() {
    let mut b = bus();
    b.write_u32(0x40021014, 1 << 17).unwrap();
    b.write_u32(0x48000000, 1 << 10).unwrap();
    b.write_u32(0x48000014, 1 << 5).unwrap();
    let gpio = b.find_peripheral_index_by_name("gpioa").unwrap();
    assert!(b.set_peripheral_gpio_input(gpio, 0, true));
    b.write_u32(0x40021028, 1 << 17).unwrap();
    b.write_u32(0x48000014, 1 << 5).unwrap(); // Held reset: ignored.
    b.write_u32(0x40021028, 0).unwrap();
    assert_eq!(b.read_u32(0x48000000).unwrap(), 0);
    assert_eq!(b.read_u32(0x48000014).unwrap(), 0);
    assert_eq!(b.peripherals[gpio].dev.read_gpio_input(0), Some(true));
    b.write_u32(0x40022004, 0x45670123).unwrap();
    b.write_u32(0x40022004, 0xcdef89ab).unwrap();
    b.write_u32(0x40022010, 1).unwrap();
    b.flash.data[0xfc00] = 0x5a;
    for p in &mut b.peripherals {
        p.dev.on_system_reset();
    }
    assert_eq!(b.read_u32(0x40022010).unwrap() & 0x80, 0x80);
    assert_eq!(b.flash.data[0xfc00], 0x5a);
    assert_eq!(b.peripherals[gpio].dev.read_gpio_input(0), Some(true));
}

#[test]
fn absent_registers_are_errors_in_the_new_profiles() {
    let mut b = bus();
    b.write_u32(0x40021018, 1 << 11).unwrap();
    for address in [0x40021038, 0x40022030, 0x40022084] {
        assert!(b.read_u32(address).is_err(), "unknown read {address:#x}");
        assert!(
            b.write_u32(address, 1).is_err(),
            "unknown write {address:#x}"
        );
    }
}

#[test]
fn warm_reset_records_the_actual_source_without_inventing_power_on() {
    let mut crm = Rcc::new_with_layout(RccRegisterLayout::At32F421);
    crm.write_u32(0x24, 1 << 24).unwrap(); // Clear the latched reset causes.
    crm.on_system_reset();
    crm.on_peripheral_reset(labwired_core::PeripheralResetCause::Watchdog);
    assert_eq!(crm.read_u32(0x24).unwrap() & 0xfc00_0000, 1 << 29);
    crm.write_u32(0x24, 1 << 24).unwrap();
    crm.on_node_reset();
    assert_eq!(crm.read_u32(0x24).unwrap() & 0xfc00_0000, 1 << 26);
    crm.on_supply_reset(labwired_core::power::SupplyResetCause::PowerOn);
    assert_eq!(crm.read_u32(0x24).unwrap() & 0x0c00_0000, 0x0c00_0000);
}

#[test]
fn regulated_supply_holds_cpu_and_resets_at32_registers_on_power_return() {
    use labwired_core::{Cpu, Machine};
    let mut b = bus();
    let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut b);
    let mut m = Machine::new(cpu, b);
    let elf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/at32f421g8u7-smoke.elf");
    m.load_firmware(&labwired_loader::load_elf(&elf).unwrap())
        .unwrap();
    m.attach_supply();
    m.set_supply_volts(0.0);
    let initial_pc = m.cpu.get_pc();
    m.step().unwrap();
    assert!(m.supply_status().held_in_reset);
    assert_eq!(m.cpu.get_pc(), initial_pc);
    m.set_supply_volts(2.05);
    m.step().unwrap();
    assert!(m.supply_status().held_in_reset);
    m.set_supply_volts(2.06);
    m.step().unwrap();
    assert!(!m.supply_status().held_in_reset);
    assert_eq!(m.supply_status().power_on_resets, 1);
    let gpio = m.bus.find_peripheral_index_by_name("gpioa").unwrap();
    m.bus.write_u32(0x40021014, 1 << 17).unwrap();
    m.bus.write_u32(0x48000000, 1 << 10).unwrap();
    m.bus.write_u32(0x48000014, 1 << 5).unwrap();
    m.bus.set_peripheral_gpio_input(gpio, 0, true);
    m.bus.write_u32(0x40022004, 0x45670123).unwrap();
    m.bus.write_u32(0x40022004, 0xcdef89ab).unwrap();
    m.bus.flash.data[0xfc00] = 0x5a;
    m.bus.write_u32(0x20002000, 0x12345678).unwrap();
    m.set_supply_volts(1.87);
    m.step().unwrap();
    assert!(m.supply_status().held_in_reset);
    m.set_supply_volts(2.06);
    m.step().unwrap();
    assert_eq!(m.supply_status().power_on_resets, 2);
    // Read the actual register bank, not a zero produced by its reset clock gate.
    assert_eq!(m.bus.peripherals[gpio].dev.read_u32(0).unwrap(), 0);
    assert_eq!(m.bus.peripherals[gpio].dev.read_u32(0x14).unwrap(), 0);
    assert_eq!(m.bus.peripherals[gpio].dev.read_gpio_input(0), Some(true));
    assert_eq!(m.bus.read_u32(0x40022010).unwrap() & 0x80, 0x80);
    assert_eq!(m.bus.flash.data[0xfc00], 0x5a);
    // Existing generic supervisor retains SRAM; no analog data-retention claim.
    assert_eq!(m.bus.read_u32(0x20002000).unwrap(), 0x12345678);
    assert_eq!(
        m.bus.read_u32(0x40021024).unwrap() & 0x0c00_0000,
        0x0c00_0000
    );
}

/// The committed Artery SDK smoke (examples/at32f421g8u7/firmware) through the
/// ordinary machine: PLL switch to 120 MHz, flash erase/program of the last
/// sector, a DMA memory-to-memory copy and real TMR6 interrupts toggling PA5.
#[test]
fn vendor_sdk_smoke_runs_flash_dma_and_timer_irq_paths() {
    use labwired_core::Machine;
    let mut b = bus();
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    b.attach_uart_tx_sink(sink.clone(), false);
    let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut b);
    let mut m = Machine::new(cpu, b);
    let elf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/at32f421g8u7-smoke.elf");
    m.load_firmware(&labwired_loader::load_elf(&elf).unwrap())
        .unwrap();
    let gpioa = m.bus.find_peripheral_index_by_name("gpioa").unwrap();
    let (mut previous, mut edges) = (None, 0);
    for step in 0..200_000 {
        m.step()
            .unwrap_or_else(|e| panic!("smoke step {step}: {e}"));
        let level = m.bus.peripherals[gpioa].dev.read_gpio_output(5);
        if level.is_some() && previous.is_some() && level != previous {
            edges += 1;
        }
        previous = level;
    }
    let uart = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    for line in [
        "AT32 START",
        "FLASH OK",
        "DMA OK",
        "TIMER IRQ OK",
        "AT32F421G8U7 IO OK",
    ] {
        assert!(uart.contains(line), "missing {line:?} in {uart:?}");
    }
    assert!(!uart.contains("FAIL"), "{uart}");
    assert!(edges >= 8, "TMR6 handler produced only {edges} PA5 edges");
    // The program/erase landed in flash storage, not only in a register.
    let word = u32::from_le_bytes(m.bus.flash.data[0xfc00..0xfc04].try_into().unwrap());
    assert_eq!(word, 0x1234_5678);
}

/// Artery CMSIS `at32f421.h` IRQn_Type: TMR1_BRK_OVF_TRG_HALL_IRQn = 13 and
/// TMR1_CH_IRQn = 14. A compare flag must not raise the overflow vector.
#[test]
fn tmr1_splits_overflow_and_channel_vectors() {
    let mut b = bus();
    b.write_u32(0x40021018, 1 << 11).unwrap(); // CRM APB2EN TMR1.
    let tim1 = b.find_peripheral_index_by_name("tim1").unwrap();
    let lines = |bus: &SystemBus| {
        let own = bus.peripherals[tim1].dev.irq_line_level();
        let mut channel = None;
        bus.peripherals[tim1]
            .dev
            .irq_line_levels(&mut |irq, level| {
                assert_eq!(irq, 14);
                channel = Some(level);
            });
        (own, channel)
    };
    // UG reloads CNT=0, latching UIF and (CCR1 == CNT == 0) CC1IF together.
    b.write_u32(0x40012c0c, 1 << 1).unwrap(); // IDEN: C1IEN only.
    b.write_u32(0x40012c14, 1).unwrap(); // SWEVT: OVFSWTR.
    assert_eq!(lines(&b), (Some(false), Some(true)));
    b.write_u32(0x40012c0c, 1).unwrap(); // IDEN: OVFIEN only.
    assert_eq!(lines(&b), (Some(true), Some(false)));
    b.write_u32(0x40012c10, 0).unwrap(); // Clear the flags (rc_w0).
    assert_eq!(lines(&b), (Some(false), Some(false)));
}

/// CRM peripheral resets act only on blocks with a register reset model:
/// the GPIO ports (AHBRST). Every other reset bit refuses instead of leaving
/// the block's registers untouched.
#[test]
fn crm_peripheral_resets_apply_to_modeled_targets_and_refuse_the_rest() {
    let mut b = bus();
    for (offset, bit, block) in [
        (0x0c, 0, "SCFGCMP"),
        (0x0c, 1, "EXINT"),
        (0x0c, 9, "ADC1"),
        (0x0c, 11, "TMR1"),
        (0x0c, 14, "USART1"),
        (0x10, 1, "TMR3"),
        (0x10, 17, "USART2"),
    ] {
        assert!(
            b.write_u32(0x40021000 + offset, 1 << bit).is_err(),
            "{block} reset must refuse until its model resets"
        );
    }
    for bit in [17, 18, 19, 22] {
        b.write_u32(0x40021028, 1 << bit).unwrap();
        b.write_u32(0x40021028, 0).unwrap();
    }
    assert!(b.write_u32(0x40021028, 1 << 16).is_err(), "no GPIOE");
}

/// The WDT is the shared STM32-layout IWDG with `reset_on_expiry`: an
/// unreloaded timeout resets the running machine and CRM records the
/// watchdog cause (CTRLSTS WDTRSTF, bit 29).
#[test]
fn wdt_expiry_resets_the_machine_and_records_the_watchdog_cause() {
    use labwired_core::{Cpu, Machine};
    let mut b = bus();
    let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut b);
    let mut m = Machine::new(cpu, b);
    let elf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/at32f421g8u7-smoke.elf");
    m.load_firmware(&labwired_loader::load_elf(&elf).unwrap())
        .unwrap();
    let reset_pc = m.cpu.get_pc();
    for _ in 0..1000 {
        m.step().unwrap();
    }
    assert_ne!(m.cpu.get_pc(), reset_pc);
    m.bus.write_u32(0x40021024, 1 << 24).unwrap(); // Clear reset flags.
    m.bus.write_u32(0x40003000, 0x5555).unwrap();
    m.bus.write_u32(0x40003004, 0).unwrap(); // /4
    m.bus.write_u32(0x40003008, 0).unwrap(); // 4 LICK ticks = 100 us.
    m.bus.write_u32(0x40003000, 0xcccc).unwrap();
    let mut reset = false;
    for _ in 0..100_000 {
        m.step().unwrap();
        if m.bus.read_u32(0x40021024).unwrap() & (1 << 29) != 0 {
            reset = true;
            break;
        }
    }
    assert!(reset, "WDT expiry never reset the machine");
    assert_eq!(
        m.cpu.get_pc(),
        reset_pc,
        "core restarted at its reset vector"
    );
}
