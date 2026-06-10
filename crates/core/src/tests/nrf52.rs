use crate::bus::SystemBus;
use crate::cpu::cortex_m::CortexM;
use crate::{Bus, Cpu, Machine};
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[test]
fn test_nrf52_full_smoke() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/nrf52832.yaml");

    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/nrf52-dk.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|_| panic!("Failed to load chip config at {:?}", chip_path));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|_| panic!("Failed to load system manifest at {:?}", system_path));

    let anchored_chip = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored_chip.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("Failed to build bus");

    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    // Thumb-1 Code for Cortex-M4F (nRF52)
    let code = vec![
        0x02, 0x48, // ldr r0, [pc, #8]  (loads 0x4000251C)
        0x4F, 0x21, // movs r1, #79 ('O')
        0x01, 0x60, // str r1, [r0, #0]
        0x4B, 0x21, // movs r1, #75 ('K')
        0x01, 0x60, // str r1, [r0, #0]
        0xFE, 0xE7, // b .
        0x1C, 0x25, 0x00, 0x40, // .word 0x4000251C (UART0 TXD)
    ];

    let load_addr = 0x00000000; // nRF52 flash base
    for (i, byte) in code.iter().enumerate() {
        bus.write_u8(load_addr + i as u64, *byte).unwrap();
    }

    let mut cpu = CortexM::new();
    cpu.set_pc(load_addr as u32);

    let mut machine = Machine::new(cpu, bus);

    for _ in 0..20 {
        machine.step().expect("Simulation failed");
    }

    let data = sink.lock().unwrap();
    assert_eq!(
        *data.last().expect("UART output empty"),
        75,
        "UART0 TXD should contain 'K' (75)"
    );
}

#[test]
fn xiao_nrf52840_sense_manifest_builds_with_uart_gpio_spi() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/nrf52840.yaml");

    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/seeed-xiao-nrf52840-sense.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|_| panic!("Failed to load chip config at {:?}", chip_path));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|_| panic!("Failed to load system manifest at {:?}", system_path));

    let anchored_chip = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored_chip.to_str().unwrap().to_string();

    let bus = SystemBus::from_config(&chip, &manifest).expect("Failed to build XIAO bus");
    let names: Vec<&str> = bus.peripherals.iter().map(|p| p.name.as_str()).collect();

    assert!(names.contains(&"uart0"), "uart0 missing: {names:?}");
    assert!(names.contains(&"gpio0"), "gpio0 missing: {names:?}");
    assert!(names.contains(&"gpio1"), "gpio1 missing: {names:?}");
    assert!(names.contains(&"spi0"), "spi0 missing: {names:?}");
}

#[test]
fn xiao_nrf52840_gpio_task_registers_drive_led_pins() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/nrf52840.yaml");

    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/seeed-xiao-nrf52840-sense.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored_chip = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored_chip.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("Failed to build XIAO bus");

    bus.write_u32(0x5000_0508, 1 << 26).unwrap();
    assert_eq!(bus.read_u32(0x5000_0504).unwrap() & (1 << 26), 1 << 26);

    bus.write_u32(0x5000_050C, 1 << 26).unwrap();
    assert_eq!(bus.read_u32(0x5000_0504).unwrap() & (1 << 26), 0);
}

/// Behavioural test: TIMER0 driven through onboarding manifest.
/// Configures BITMODE=32-bit, PRESCALER=0, CC[0]=5, enables COMPARE[0] IRQ,
/// starts the timer, then ticks the bus enough cycles for the compare to
/// fire. Asserts the event register and that the configured NVIC IRQ (8)
/// got pended.
#[test]
fn nrf52840_onboarding_timer0_fires_compare_and_pends_irq() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/onboarding/nrf52840.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/onboarding/nrf52840.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("onboarding bus");

    const TIMER0: u64 = 0x4000_8000;
    const TASKS_START: u64 = TIMER0;
    const EVENTS_COMPARE0: u64 = TIMER0 + 0x140;
    const INTENSET: u64 = TIMER0 + 0x304;
    const BITMODE: u64 = TIMER0 + 0x508;
    const PRESCALER: u64 = TIMER0 + 0x510;
    const CC0: u64 = TIMER0 + 0x540;
    const TIMER0_IRQ: u32 = 8;

    bus.write_u32(BITMODE, 3).unwrap(); // 32-bit
    bus.write_u32(PRESCALER, 0).unwrap();
    bus.write_u32(CC0, 5).unwrap();
    bus.write_u32(INTENSET, 1 << 16).unwrap(); // COMPARE[0]
    bus.write_u32(TASKS_START, 1).unwrap();

    // Tick the bus until COMPARE[0] fires or we time out.
    let mut compare_fired = false;
    let mut irq_pended = false;
    for _ in 0..200 {
        let (interrupts, _costs) = bus.tick_peripherals_fully();
        if interrupts.contains(&TIMER0_IRQ) {
            irq_pended = true;
        }
        if bus.read_u32(EVENTS_COMPARE0).unwrap() != 0 {
            compare_fired = true;
            break;
        }
    }

    assert!(compare_fired, "TIMER0 EVENTS_COMPARE[0] never fired");
    assert!(irq_pended, "TIMER0 IRQ (8) was never pended on NVIC");
}

/// Behavioural test: RTC0 driven through the onboarding manifest.
/// Configures PRESCALER=0, CC[0]=4, enables COMPARE[0] IRQ + EVTEN,
/// starts the RTC, ticks the bus, asserts EVENTS_COMPARE[0] and NVIC IRQ 11.
#[test]
fn nrf52840_onboarding_rtc0_fires_compare_and_pends_irq() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/onboarding/nrf52840.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/onboarding/nrf52840.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("onboarding bus");

    const RTC0: u64 = 0x4000_B000;
    const TASKS_START: u64 = RTC0;
    const EVENTS_COMPARE0: u64 = RTC0 + 0x140;
    const INTENSET: u64 = RTC0 + 0x304;
    const EVTENSET: u64 = RTC0 + 0x344;
    const PRESCALER: u64 = RTC0 + 0x508;
    const CC0: u64 = RTC0 + 0x540;
    const RTC0_IRQ: u32 = 11;

    bus.write_u32(PRESCALER, 0).unwrap();
    bus.write_u32(CC0, 4).unwrap();
    bus.write_u32(EVTENSET, 1 << 16).unwrap();
    bus.write_u32(INTENSET, 1 << 16).unwrap();
    bus.write_u32(TASKS_START, 1).unwrap();

    let mut compare_fired = false;
    let mut irq_pended = false;
    for _ in 0..200 {
        let (interrupts, _costs) = bus.tick_peripherals_fully();
        if interrupts.contains(&RTC0_IRQ) {
            irq_pended = true;
        }
        if bus.read_u32(EVENTS_COMPARE0).unwrap() != 0 {
            compare_fired = true;
            break;
        }
    }

    assert!(compare_fired, "RTC0 EVENTS_COMPARE[0] never fired");
    assert!(irq_pended, "RTC0 IRQ (11) was never pended on NVIC");
}

/// Behavioural test: Zephyr / nRF SDK clock_init boot pattern.
/// Firmware writes TASKS_HFCLKSTART then busy-loops on EVENTS_HFCLKSTARTED.
/// The bus must surface the event within a bounded number of ticks for
/// the firmware to make forward progress.
#[test]
fn nrf52840_onboarding_clock_boot_pattern_completes() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/onboarding/nrf52840.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/onboarding/nrf52840.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("onboarding bus");

    const CLOCK: u64 = 0x4000_0000;
    const TASKS_HFCLKSTART: u64 = CLOCK;
    const TASKS_LFCLKSTART: u64 = CLOCK + 0x008;
    const EVENTS_HFCLKSTARTED: u64 = CLOCK + 0x100;
    const EVENTS_LFCLKSTARTED: u64 = CLOCK + 0x104;
    const HFCLKSTAT: u64 = CLOCK + 0x40C;
    const LFCLKSRC: u64 = CLOCK + 0x518;

    // Issue both clock starts as a typical bring-up sequence would.
    bus.write_u32(LFCLKSRC, 1).unwrap(); // Xtal
    bus.write_u32(TASKS_HFCLKSTART, 1).unwrap();
    bus.write_u32(TASKS_LFCLKSTART, 1).unwrap();

    let mut hf_done = false;
    let mut lf_done = false;
    for _ in 0..16 {
        bus.tick_peripherals_fully();
        if bus.read_u32(EVENTS_HFCLKSTARTED).unwrap() != 0 {
            hf_done = true;
        }
        if bus.read_u32(EVENTS_LFCLKSTARTED).unwrap() != 0 {
            lf_done = true;
        }
        if hf_done && lf_done {
            break;
        }
    }

    assert!(hf_done, "EVENTS_HFCLKSTARTED never fired");
    assert!(lf_done, "EVENTS_LFCLKSTARTED never fired");
    // HFCLKSTAT.STATE should show running.
    assert_ne!(bus.read_u32(HFCLKSTAT).unwrap() & (1 << 16), 0);
}

/// Diagnostic: confirm the onboarding bus actually maps GPIO0 at the
/// standard Nordic base and that a direct OUTSET write lands on it.
#[test]
fn nrf52840_onboarding_gpio0_direct_outset_works() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/onboarding/nrf52840.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/onboarding/nrf52840.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    let bus = SystemBus::from_config(&chip, &manifest).expect("onboarding bus");

    let map: Vec<String> = bus
        .peripherals
        .iter()
        .map(|p| format!("{}@0x{:08X}+0x{:X}", p.name, p.base, p.size))
        .collect();
    let mut bus = bus;
    let write_res = bus.write_u32(0x5000_0508, 1 << 26);
    let read_res = bus.read_u32(0x5000_0504);
    assert!(
        read_res.as_ref().ok().copied().unwrap_or(0) & (1 << 26) == (1 << 26),
        "GPIO0 OUTSET via 0x5000_0508 didn't set OUT bit 26. \
         write_res={write_res:?} read_res={read_res:?} \
         peripherals={map:#?}"
    );
}

/// GPIOTE EVENTS_IN: a GPIO0 pin transition should fire EVENTS_IN[0]
/// when channel 0 is configured in Event mode with matching pin and
/// polarity. We drive the edge by writing to GPIO0.OUTSET — the IN
/// register tracks OUT for output-configured pins on Nordic silicon.
#[test]
fn nrf52840_onboarding_gpiote_event_in_fires_on_edge() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/onboarding/nrf52840.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/onboarding/nrf52840.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("onboarding bus");

    const GPIOTE: u64 = 0x4000_6000;
    const GPIOTE_CONFIG_0: u64 = GPIOTE + 0x510;
    const GPIOTE_INTENSET: u64 = GPIOTE + 0x304;
    const GPIOTE_EVENTS_IN_0: u64 = GPIOTE + 0x100;
    const GPIO0_DIRSET: u64 = 0x5000_0518;
    const GPIO0_OUTSET: u64 = 0x5000_0508;

    let pin: u32 = 3;
    // Configure pin 3 as output so IN tracks it.
    bus.write_u32(GPIO0_DIRSET, 1 << pin).unwrap();

    // GPIOTE ch0: Event mode (1), port 0, PSEL pin 3, polarity LO_TO_HI.
    let cfg = 1 | (pin << 8) | (1u32 << 16);
    bus.write_u32(GPIOTE_CONFIG_0, cfg).unwrap();
    bus.write_u32(GPIOTE_INTENSET, 1).unwrap(); // IN[0] interrupt

    // Initial tick — snapshots current GPIO IN (all zero) as baseline.
    bus.tick_peripherals_fully();
    assert_eq!(
        bus.read_u32(GPIOTE_EVENTS_IN_0).unwrap(),
        0,
        "EVENTS_IN[0] must be 0 before any edge"
    );

    // Drive the rising edge.
    bus.write_u32(GPIO0_OUTSET, 1 << pin).unwrap();

    // First tick after the edge: bus snapshots new GPIO IN, GPIOTE
    // observes the change.  Second tick: GPIOTE drains pending events.
    let mut irq_seen = false;
    for _ in 0..4 {
        let (irqs, _costs) = bus.tick_peripherals_fully();
        if irqs.contains(&6) {
            irq_seen = true;
        }
    }

    assert_eq!(
        bus.read_u32(GPIOTE_EVENTS_IN_0).unwrap(),
        1,
        "EVENTS_IN[0] should be set after rising edge on watched pin"
    );
    assert!(
        irq_seen,
        "GPIOTE IRQ 6 should pend when INTEN.IN[0] enabled"
    );
}

/// End-to-end PPI test: TIMER0 EVENTS_COMPARE[0] → PPI CH[0] →
/// GPIOTE TASKS_OUT[0] → GPIO0 pin 26.
///
/// This is the canonical "hardware-driven LED toggle" pattern in nRF SDK
/// firmware. Exercises every link in the cross-peripheral chain:
/// TIMER fires fired_events, PPI routes them, GPIOTE produces mmio_writes,
/// bus applies them to GPIO0 — all without any CPU instruction execution.
///
/// Silicon behaviour: GPIOTE Task mode drives the **pad** level, which is
/// observable in `GPIO.IN` (0x510), not in `GPIO.OUT` (0x504).  The OUT
/// register is only modified by CPU writes to OUTSET/OUTCLR; GPIOTE does
/// not touch it.  We therefore observe transitions on GPIO0.IN bit 26.
#[test]
fn nrf52840_onboarding_ppi_routes_timer_to_gpiote_pin() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/onboarding/nrf52840.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/onboarding/nrf52840.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("onboarding bus");

    const TIMER0: u64 = 0x4000_8000;
    const TIMER0_TASKS_START: u64 = TIMER0;
    const TIMER0_TASKS_CLEAR: u64 = TIMER0 + 0x00C;
    const TIMER0_EVENTS_COMPARE0: u64 = TIMER0 + 0x140;
    const TIMER0_BITMODE: u64 = TIMER0 + 0x508;
    const TIMER0_PRESCALER: u64 = TIMER0 + 0x510;
    const TIMER0_SHORTS: u64 = TIMER0 + 0x200;
    const TIMER0_CC0: u64 = TIMER0 + 0x540;

    const GPIOTE: u64 = 0x4000_6000;
    const GPIOTE_TASKS_OUT_0: u64 = GPIOTE;
    const GPIOTE_CONFIG_0: u64 = GPIOTE + 0x510;

    const PPI: u64 = 0x4001_F000;
    const PPI_CH0_EEP: u64 = PPI + 0x510;
    const PPI_CH0_TEP: u64 = PPI + 0x514;
    const PPI_CHENSET: u64 = PPI + 0x504;

    // Observe GPIO0.IN (0x510): GPIOTE drives the pad level here.
    // GPIO0.OUT (0x504) is untouched by GPIOTE tasks (silicon-faithful).
    const GPIO0_IN: u64 = 0x5000_0510;
    const LED_RED_PIN: u32 = 26;

    // 1. GPIOTE channel 0: Task mode, port 0, pin 26, polarity = Toggle.
    let gpiote_cfg = 3       // MODE = Task
        | (LED_RED_PIN << 8) // PSEL
        | (3u32 << 16); // POLARITY = Toggle
    bus.write_u32(GPIOTE_CONFIG_0, gpiote_cfg).unwrap();

    // 2. PPI channel 0: TIMER0.EVENTS_COMPARE[0] → GPIOTE.TASKS_OUT[0].
    bus.write_u32(PPI_CH0_EEP, TIMER0_EVENTS_COMPARE0 as u32)
        .unwrap();
    bus.write_u32(PPI_CH0_TEP, GPIOTE_TASKS_OUT_0 as u32)
        .unwrap();
    bus.write_u32(PPI_CHENSET, 1).unwrap();

    // 3. TIMER0: 32-bit, no prescaler, CC[0]=4, auto-clear on compare.
    bus.write_u32(TIMER0_BITMODE, 3).unwrap();
    bus.write_u32(TIMER0_PRESCALER, 0).unwrap();
    bus.write_u32(TIMER0_CC0, 4).unwrap();
    bus.write_u32(TIMER0_SHORTS, 1).unwrap(); // COMPARE[0]_CLEAR
    bus.write_u32(TIMER0_TASKS_CLEAR, 1).unwrap();
    bus.write_u32(TIMER0_TASKS_START, 1).unwrap();

    // 4. Run the bus enough cycles for several compares to fire.
    //    Track transitions on GPIO0.IN bit 26 (pad level driven by GPIOTE).
    let mut prior_in = bus.read_u32(GPIO0_IN).unwrap();
    let mut transitions = 0;
    let mut compare_observed_at: Vec<usize> = Vec::new();
    for tick in 0usize..200 {
        bus.tick_peripherals_fully();
        if bus.read_u32(TIMER0_EVENTS_COMPARE0).unwrap() != 0
            && compare_observed_at.last().copied() != Some(tick.saturating_sub(1))
        {
            compare_observed_at.push(tick);
        }
        let now_in = bus.read_u32(GPIO0_IN).unwrap();
        if (now_in & (1 << LED_RED_PIN)) != (prior_in & (1 << LED_RED_PIN)) {
            transitions += 1;
            prior_in = now_in;
        }
    }

    let last_in = bus.read_u32(GPIO0_IN).unwrap();
    // Confirm GPIO0.OUT was NOT modified by GPIOTE (silicon fidelity).
    let last_out = bus.read_u32(0x5000_0504u64).unwrap();
    assert_eq!(
        last_out & (1 << LED_RED_PIN),
        0,
        "GPIO0.OUT must NOT be modified by GPIOTE tasks (GPIOTE drives pad/IN only); \
         got OUT = 0x{last_out:08X}"
    );
    assert!(
        transitions >= 4,
        "expected >=4 GPIO0 pin {LED_RED_PIN} transitions in GPIO0.IN in 200 ticks, \
         got {transitions}. TIMER0 EVENTS_COMPARE[0] observed at ticks {compare_observed_at:?}; \
         final GPIO0.IN = 0x{last_in:08X}"
    );
}

/// Real-firmware end-to-end: load the precompiled
/// `firmware-nrf52840-timer-blinky` ELF, step the Cortex-M CPU, and
/// assert that GPIO0 OUT bit 26 toggles. This exercises the whole
/// stack — instruction decode, TIMER0 dynamics, EVENTS_COMPARE polling,
/// GPIO writes — in one go.
///
/// The ELF must be built before running:
/// ```text
/// cargo build --release --target thumbv7em-none-eabi \
///     -p firmware-nrf52840-timer-blinky
/// ```
///
/// If the ELF isn't present, the test prints a skip message and passes
/// (instead of failing CI on a missing artifact).
#[test]
fn nrf52840_onboarding_real_firmware_toggles_led() {
    use crate::cpu::cortex_m::CortexM;
    use crate::Machine;

    let elf_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/thumbv7em-none-eabi/release/firmware-nrf52840-timer-blinky");

    if !elf_path.exists() {
        // Don't fail CI when the prebuilt artifact is missing — but be
        // loud so a missing ELF doesn't look like a passing test.
        println!(
            "SKIPPED: ELF not built at {}. Run\n  cargo build --release --target thumbv7em-none-eabi -p firmware-nrf52840-timer-blinky\nfirst.",
            elf_path.display()
        );
        return;
    }
    println!("Loading firmware from {}", elf_path.display());

    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/onboarding/nrf52840.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/onboarding/nrf52840.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    let bus = SystemBus::from_config(&chip, &manifest).expect("onboarding bus");
    let cpu = CortexM::new();
    let mut machine = Machine::new(cpu, bus);

    // Inline ELF loader: goblin parses PT_LOAD segments, we feed them to
    // ProgramImage. Bypasses labwired-loader to avoid the dev-dep cycle
    // (loader depends on labwired-core, so importing it from labwired-core
    // tests would produce two crate versions in the dep graph).
    let elf_bytes = std::fs::read(&elf_path).expect("read ELF");
    let elf = goblin::elf::Elf::parse(&elf_bytes).expect("parse ELF");
    let mut image = crate::memory::ProgramImage::new(elf.entry, crate::Arch::Arm);
    for ph in &elf.program_headers {
        if ph.p_type != goblin::elf::program_header::PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let start = ph.p_paddr;
        let off = ph.p_offset as usize;
        let size = ph.p_filesz as usize;
        image.add_segment(start, elf_bytes[off..off + size].to_vec());
    }
    machine.load_firmware(&image).expect("load firmware");

    const GPIO0_OUT: u64 = 0x5000_0504;
    const LED_BIT: u32 = 1 << 26;

    let mut last_state = machine.bus.read_u32(GPIO0_OUT).unwrap_or(0) & LED_BIT;
    let mut transitions = 0usize;
    let max_steps = 200_000;

    for _ in 0..max_steps {
        machine.step().expect("step");
        let now = machine.bus.read_u32(GPIO0_OUT).unwrap_or(0) & LED_BIT;
        if now != last_state {
            transitions += 1;
            last_state = now;
            if transitions >= 4 {
                break;
            }
        }
    }

    println!("Firmware ran {transitions} GPIO0 pin 26 transitions.");
    assert!(
        transitions >= 4,
        "firmware should toggle GPIO0 pin 26 at least 4 times within \
         {max_steps} CPU steps; observed {transitions}"
    );
}

/// IRQ-driven firmware end-to-end: load the ISR-blinky ELF, step the
/// CPU, and confirm the TIMER0 interrupt handler ran (LED toggled).
/// Exercises the full NVIC + vector-dispatch path through the sim:
/// firmware writes NVIC ISER, TIMER raises IRQ via tick, bus pends it,
/// CortexM dispatches through VTOR + 0x60, handler runs, returns, WFI
/// loop resumes.
#[test]
fn nrf52840_onboarding_isr_firmware_toggles_led() {
    use crate::Machine;

    let elf_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/thumbv7em-none-eabi/release/firmware-nrf52840-isr-blinky");

    if !elf_path.exists() {
        println!(
            "SKIPPED: ELF not built at {}. Run\n  cargo build --release --target thumbv7em-none-eabi -p firmware-nrf52840-isr-blinky\nfirst.",
            elf_path.display()
        );
        return;
    }

    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/onboarding/nrf52840.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/onboarding/nrf52840.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("onboarding bus");
    // Add NVIC/SCB/DWT peripherals at the standard Cortex-M addresses;
    // chip yamls don't list them. Returns a CPU with VTOR shared with SCB.
    let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    let elf_bytes = std::fs::read(&elf_path).expect("read ELF");
    let elf = goblin::elf::Elf::parse(&elf_bytes).expect("parse ELF");

    // Find the ISR_COUNT symbol so we can read it directly out of RAM
    // after stepping — confirms the handler actually executed (vs the
    // LED being toggled some other way).
    let isr_count_addr = elf
        .syms
        .iter()
        .find_map(|sym| match elf.strtab.get_at(sym.st_name) {
            Some("ISR_COUNT") => Some(sym.st_value),
            _ => None,
        })
        .expect("ISR_COUNT symbol present in ELF");

    let mut image = crate::memory::ProgramImage::new(elf.entry, crate::Arch::Arm);
    for ph in &elf.program_headers {
        if ph.p_type != goblin::elf::program_header::PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        image.add_segment(
            ph.p_paddr,
            elf_bytes[ph.p_offset as usize..(ph.p_offset + ph.p_filesz) as usize].to_vec(),
        );
    }
    machine.load_firmware(&image).expect("load firmware");

    const GPIO0_OUT: u64 = 0x5000_0504;
    const LED_BIT: u32 = 1 << 26;

    let mut transitions = 0usize;
    let mut last_state = machine.bus.read_u32(GPIO0_OUT).unwrap_or(0) & LED_BIT;
    let max_steps = 500_000;
    let mut grace = 0usize;
    for _ in 0..max_steps {
        machine.step().expect("step");
        let now = machine.bus.read_u32(GPIO0_OUT).unwrap_or(0) & LED_BIT;
        if now != last_state {
            transitions += 1;
            last_state = now;
        }
        if transitions >= 4 {
            grace += 1;
            // Step a few extra cycles to let any in-flight ISR complete
            // its atomic increment of ISR_COUNT before we break out.
            if grace >= 200 {
                break;
            }
        }
    }

    let isr_count = machine.bus.read_u32(isr_count_addr).unwrap_or(0);
    println!("ISR ran {isr_count} times; GPIO0 pin 26 transitions = {transitions}");

    assert!(
        isr_count >= 4,
        "TIMER0 ISR should have fired at least 4 times; got {isr_count}"
    );
    assert!(
        transitions >= 4,
        "ISR should have toggled GPIO0 pin 26 at least 4 times; got {transitions}"
    );
}

/// Two-instance BLE loopback: spin up two independent nRF52840 sims,
/// load TX firmware on one and RX firmware on the other, then
/// interleave their CPU steps. The TX firmware drives the RADIO TASKS_TXEN
/// → TASKS_START → EVENTS_END cycle, pushing a packet onto the global
/// virtual air; the RX firmware drives TASKS_RXEN → TASKS_START →
/// EVENTS_END and consumes the packet, writing the first payload byte
/// + length + CRCSTATUS into well-known statics.
///
/// This is the "full simulation" smoke: two complete Cortex-M cores
/// running real Rust firmware that talks to a real RADIO model with
/// PACKETPTR Easy DMA, BLE whitening, CRC-24, address matching, and
/// cross-instance air routing.
#[test]
fn nrf52840_ble_loopback_through_virtual_air() {
    use crate::peripherals::nrf52::radio;
    use crate::Machine;

    let tx_elf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/thumbv7em-none-eabi/release/firmware-nrf52840-ble-tx");
    let rx_elf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/thumbv7em-none-eabi/release/firmware-nrf52840-ble-rx");

    if !tx_elf.exists() || !rx_elf.exists() {
        println!(
            "SKIPPED: build firmwares first:\n  \
             cargo build --release --target thumbv7em-none-eabi \
             -p firmware-nrf52840-ble-tx -p firmware-nrf52840-ble-rx"
        );
        return;
    }

    // Start with a clean virtual air so prior tests in this binary don't
    // leak frames into this one.
    radio::clear_virtual_air();

    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/onboarding/nrf52840.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/onboarding/nrf52840.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    let build_machine = |elf_path: &PathBuf| -> Machine<crate::cpu::cortex_m::CortexM> {
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("onboarding bus");
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        let elf_bytes = std::fs::read(elf_path).expect("read ELF");
        let elf = goblin::elf::Elf::parse(&elf_bytes).expect("parse ELF");
        let mut image = crate::memory::ProgramImage::new(elf.entry, crate::Arch::Arm);
        for ph in &elf.program_headers {
            if ph.p_type != goblin::elf::program_header::PT_LOAD || ph.p_filesz == 0 {
                continue;
            }
            image.add_segment(
                ph.p_paddr,
                elf_bytes[ph.p_offset as usize..(ph.p_offset + ph.p_filesz) as usize].to_vec(),
            );
        }
        machine.load_firmware(&image).expect("load firmware");
        machine
    };

    // Resolve the symbol addresses we want to peek at.
    let tx_elf_bytes = std::fs::read(&tx_elf).unwrap();
    let tx_elf_parsed = goblin::elf::Elf::parse(&tx_elf_bytes).unwrap();
    let tx_done_addr = tx_elf_parsed
        .syms
        .iter()
        .find_map(|s| match tx_elf_parsed.strtab.get_at(s.st_name) {
            Some("TX_DONE_COUNT") => Some(s.st_value),
            _ => None,
        })
        .expect("TX_DONE_COUNT symbol");

    let rx_elf_bytes = std::fs::read(&rx_elf).unwrap();
    let rx_elf_parsed = goblin::elf::Elf::parse(&rx_elf_bytes).unwrap();
    let find_rx_sym = |name: &str| -> u64 {
        rx_elf_parsed
            .syms
            .iter()
            .find_map(|s| match rx_elf_parsed.strtab.get_at(s.st_name) {
                Some(n) if n == name => Some(s.st_value),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing symbol: {name}"))
    };
    let rx_done_addr = find_rx_sym("RX_DONE_COUNT");
    let rx_length_addr = find_rx_sym("RX_LENGTH");
    let rx_first_payload_addr = find_rx_sym("RX_FIRST_PAYLOAD_BYTE");
    let rx_crc_addr = find_rx_sym("RX_CRC_STATUS");

    let mut tx_machine = build_machine(&tx_elf);
    let mut rx_machine = build_machine(&rx_elf);

    // Interleave steps. The RX firmware needs to be running its
    // TASKS_START + poll-loop while the TX firmware pushes onto the
    // air; otherwise the RX peripheral hasn't dequeued before the test
    // checks RAM.
    let max_steps = 1_000_000;
    let mut tx_done = false;
    let mut rx_done = false;
    for _ in 0..max_steps {
        if !tx_done {
            tx_machine.step().expect("tx step");
            if tx_machine.bus.read_u32(tx_done_addr).unwrap_or(0) > 0 {
                tx_done = true;
            }
        }
        if !rx_done {
            rx_machine.step().expect("rx step");
            if rx_machine.bus.read_u32(rx_done_addr).unwrap_or(0) > 0 {
                rx_done = true;
            }
        }
        if tx_done && rx_done {
            break;
        }
    }

    assert!(tx_done, "TX firmware never reached TX_DONE_COUNT increment");
    assert!(rx_done, "RX firmware never reached RX_DONE_COUNT increment");

    let length = rx_machine.bus.read_u32(rx_length_addr).unwrap_or(0);
    let first = rx_machine.bus.read_u32(rx_first_payload_addr).unwrap_or(0);
    let crc = rx_machine.bus.read_u32(rx_crc_addr).unwrap_or(0);

    println!("BLE loopback: length={length} first_payload_byte=0x{first:02X} crc_status={crc}");

    assert_eq!(length, 4, "RX should have observed LENGTH=4");
    assert_eq!(
        first, 0xC0,
        "RX should have observed first payload byte 0xC0"
    );
    assert_eq!(crc, 1, "CRC should validate end-to-end");
}

/// Validate the simulator against **real Arduino code**: a stock
/// `digitalWrite` blink sketch compiled by `arduino-cli` with the
/// Adafruit nRF52 BSP (`adafruit:nrf52:pca10056`). The ELF expects
/// the Adafruit bootloader + SoftDevice layout (app at 0x26000), so
/// after `load_firmware` we manually fix up SP/PC from the
/// application vector table and set VTOR=0x26000 as the bootloader
/// would.
///
/// Then we step the Cortex-M core and watch GPIO0.OUT bit 26 toggle —
/// proving that:
/// - the Arduino HAL's `pinMode` boils down to GPIO0 DIRSET writes
///   our model handles,
/// - `digitalWrite(HIGH/LOW)` lands in GPIO0 OUTSET/OUTCLR per the
///   Adafruit Arduino nRF52 wrapper,
/// - the busy-loop in `delayMicroseconds` (DWT.CYCCNT-based on
///   Adafruit's HAL) runs against our DWT model without spinning
///   forever.
///
/// Skipped if the ELF hasn't been built. Build with:
/// ```text
/// arduino-cli compile --fqbn adafruit:nrf52:pca10056 \
///     --output-dir target/arduino-blink/ \
///     /tmp/arduino-blink/arduino-blink.ino
/// cp target/arduino-blink/arduino-blink.ino.elf target/arduino-blink.elf
/// ```
#[test]
fn nrf52840_arduino_blink_toggles_gpio() {
    use crate::Machine;

    let elf_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/arduino-blink.elf");
    if !elf_path.exists() {
        println!(
            "SKIPPED: build the Arduino sketch first:\n  \
             arduino-cli compile --fqbn adafruit:nrf52:pca10056 \\\n    \
             --output-dir target/arduino-blink/ \\\n    \
             arduino-blink.ino\n  \
             cp target/arduino-blink/arduino-blink.ino.elf {}",
            elf_path.display()
        );
        return;
    }

    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/onboarding/nrf52840.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/onboarding/nrf52840.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("onboarding bus");
    let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    // Load PT_LOAD segments at their physical addresses. The Arduino
    // ELF places .text at 0x26000 (LMA) and .data at 0x2adcc (flash
    // copy, RAM @0x20006000 runtime).
    let elf_bytes = std::fs::read(&elf_path).expect("read Arduino ELF");
    let elf = goblin::elf::Elf::parse(&elf_bytes).expect("parse Arduino ELF");
    let mut image = crate::memory::ProgramImage::new(elf.entry, crate::Arch::Arm);
    for ph in &elf.program_headers {
        if ph.p_type != goblin::elf::program_header::PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        image.add_segment(
            ph.p_paddr,
            elf_bytes[ph.p_offset as usize..(ph.p_offset + ph.p_filesz) as usize].to_vec(),
        );
    }
    machine
        .load_firmware(&image)
        .expect("load Arduino firmware");

    // The Adafruit Bluefruit bootloader sets VTOR=0x26000 and jumps to
    // the application Reset_Handler whose address sits at 0x26004. We
    // pre-arm SP/VTOR and let the simulator run the C startup, main(),
    // FreeRTOS task creation, and vTaskStartScheduler. With SHPR3-aware
    // priority dispatch in the CortexM, SysTick at higher priority pends
    // PendSV (priority 0xFF) which then performs the context switch into
    // loopTask, which calls setup() once then loops loop().
    const APP_VTOR: u32 = 0x0002_6000;
    let sp = machine
        .bus
        .read_u32(APP_VTOR as u64)
        .expect("SP from app VT");
    let reset_handler = machine
        .bus
        .read_u32((APP_VTOR + 4) as u64)
        .expect("Reset_Handler from app VT");
    println!("Arduino app: SP=0x{sp:08X} Reset_Handler=0x{reset_handler:08X}");
    machine.cpu.set_sp(sp);
    machine
        .bus
        .write_u32(0xE000_ED08, APP_VTOR as u64 as u32)
        .ok();
    machine.cpu.set_pc(reset_handler & !1);

    // Run the sketch — watch GPIO0.OUT bit 26 transitions. Stop after
    // a handful of toggles to keep test wall-clock short.
    const GPIO0_OUT: u64 = 0x5000_0504;
    const LED_BIT: u32 = 1 << 26;
    let mut last_state = machine.bus.read_u32(GPIO0_OUT).unwrap_or(0) & LED_BIT;
    let mut transitions = 0usize;
    // FreeRTOS boot path (C startup + main + scheduler bring-up + first
    // PendSV context switch into loopTask) takes ~20–30M instructions
    // before setup() begins toggling. Budget generously for headroom.
    let max_steps = 80_000_000;
    for _ in 0..max_steps {
        if machine.step().is_err() {
            break;
        }
        let now = machine.bus.read_u32(GPIO0_OUT).unwrap_or(0) & LED_BIT;
        if now != last_state {
            transitions += 1;
            last_state = now;
            if transitions >= 4 {
                break;
            }
        }
    }

    let final_pc = machine.cpu.get_pc();
    let cyccnt = machine.bus.read_u32(0xE000_1004).unwrap_or(0xDEAD_BEEF);
    let dwt_ctrl = machine.bus.read_u32(0xE000_1000).unwrap_or(0xDEAD_BEEF);
    let dir = machine.bus.read_u32(0x5000_0514).unwrap_or(0);
    let out = machine.bus.read_u32(0x5000_0504).unwrap_or(0);
    println!(
        "Arduino blink: GPIO0 pin 26 transitions = {transitions}\n\
         CPU PC=0x{final_pc:08X}  DWT.CTRL=0x{dwt_ctrl:08X}  CYCCNT=0x{cyccnt:08X}\n\
         GPIO0 DIR=0x{dir:08X} OUT=0x{out:08X}"
    );
    assert!(
        transitions >= 4,
        "real Arduino sketch should toggle GPIO0 pin 26 ≥4 times within \
         {max_steps} CPU steps; observed {transitions}"
    );
}

#[test]
fn xiao_nrf52840_spim0_start_sets_end_event_and_amount() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/nrf52840.yaml");

    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/seeed-xiao-nrf52840-sense.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let anchored_chip = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored_chip.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("Failed to build XIAO bus");

    bus.write_u32(0x4000_3500, 7).unwrap();
    bus.write_u32(0x4000_3544, 0x2000_0000).unwrap();
    bus.write_u32(0x4000_3548, 4).unwrap();
    bus.write_u32(0x4000_3010, 1).unwrap();

    assert_eq!(bus.read_u32(0x4000_3118).unwrap(), 1);
    assert_eq!(bus.read_u32(0x4000_354C).unwrap(), 4);
}
