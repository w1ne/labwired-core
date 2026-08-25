//! Inventory: why `max_safe_tick_interval` stays 1 on each shipped WASM family.
//!
//! `max_safe_tick_interval` returns [`RECOMMENDED_TICK_INTERVAL`] (512) only when
//! `legacy_walk_disabled && !iolink && !hcsr04_forced_legacy` (see
//! `bus/policy.rs`). H5 `flash_models_ops` still forces CPU quantum 1 via
//! `requires_cycle_accurate` but no longer pins the peripheral tick interval.
//! Under `event-scheduler`, walk deletion auto-derives when every peripheral is
//! `uses_scheduler() || !needs_legacy_walk()`.
//!
//! This test builds each family's real chip+system bus with `walk_deleted =
//! None` (auto-derive), prints the walk-forcing set and non-forcer blockers,
//! and guards the green families (C3 / F103 / nRF / RP2040 / H563 / S3).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
#[cfg(feature = "event-scheduler")]
use labwired_core::bus::RECOMMENDED_TICK_INTERVAL;
use labwired_core::peripherals::components::IolinkMaster;
use labwired_core::peripherals::flash::Flash;
use labwired_core::peripherals::uart::Uart;
use std::path::PathBuf;

fn root(rel: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../..");
    p.push(rel);
    p
}

/// Walk-forcers: peripherals that still pin the legacy walk under derivation.
#[derive(Clone)]
struct ForcerRow {
    name: String,
    needs_legacy_walk: bool,
    uses_scheduler: bool,
}

struct Inventory {
    chip: &'static str,
    walk_deletable: bool,
    legacy_walk_disabled: bool,
    max_safe: u32,
    flash_models_ops: bool,
    has_iolink_master: bool,
    hcsr04_count: usize,
    hcsr04_scheduling_disabled: bool,
    forcers: Vec<ForcerRow>,
    /// Full peripheral roster with walk/scheduler status (for the doc dump).
    peripherals: Vec<ForcerRow>,
}

fn flash_models_ops(bus: &SystemBus) -> bool {
    bus.peripherals.iter().any(|p| {
        p.dev
            .as_any()
            .and_then(|a| a.downcast_ref::<Flash>())
            .is_some_and(|f| f.models_ops())
    })
}

fn has_iolink_master(bus: &SystemBus) -> bool {
    for p in &bus.peripherals {
        let Some(any) = p.dev.as_any() else {
            continue;
        };
        let Some(uart) = any.downcast_ref::<Uart>() else {
            continue;
        };
        for stream in &uart.attached_streams {
            if let Some(sa) = stream.as_any() {
                if sa.downcast_ref::<IolinkMaster>().is_some() {
                    return true;
                }
            }
        }
    }
    false
}

fn inventory(chip: &'static str, bus: &SystemBus) -> Inventory {
    let peripherals: Vec<ForcerRow> = bus
        .peripherals
        .iter()
        .map(|p| ForcerRow {
            name: p.name.clone(),
            needs_legacy_walk: p.dev.needs_legacy_walk(),
            uses_scheduler: p.dev.uses_scheduler(),
        })
        .collect();
    let forcers: Vec<ForcerRow> = peripherals
        .iter()
        .filter(|p| p.needs_legacy_walk && !p.uses_scheduler)
        .cloned()
        .collect();
    // walk_deletable ≡ empty forcer set (same predicate as derive_walk_deletable).
    let walk_deletable = forcers.is_empty();
    Inventory {
        chip,
        walk_deletable,
        legacy_walk_disabled: bus.legacy_walk_disabled,
        max_safe: bus.max_safe_tick_interval(),
        flash_models_ops: flash_models_ops(bus),
        has_iolink_master: has_iolink_master(bus),
        hcsr04_count: bus.hcsr04.len(),
        hcsr04_scheduling_disabled: bus.hcsr04_scheduling_disabled,
        forcers,
        peripherals,
    }
}

fn print_inventory(inv: &Inventory) {
    let forcer_names: Vec<&str> = inv.forcers.iter().map(|f| f.name.as_str()).collect();
    let hcsr04_forced_legacy = inv.hcsr04_count > 0 && inv.hcsr04_scheduling_disabled;
    println!("=== {} ===", inv.chip);
    println!("  walk_deletable (empty forcers): {}", inv.walk_deletable);
    println!(
        "  legacy_walk_disabled:           {}",
        inv.legacy_walk_disabled
    );
    println!("  flash_models_ops:               {}", inv.flash_models_ops);
    println!(
        "  has_iolink_master:              {}",
        inv.has_iolink_master
    );
    println!(
        "  hcsr04_count / forced_legacy:    {} / {}",
        inv.hcsr04_count, hcsr04_forced_legacy
    );
    println!("  max_safe_tick_interval:         {}", inv.max_safe);
    println!("  forcers ({}): {:?}", forcer_names.len(), forcer_names);
    for f in &inv.forcers {
        println!(
            "    - {}  needs_legacy_walk={} uses_scheduler={}",
            f.name, f.needs_legacy_walk, f.uses_scheduler
        );
    }
    if inv.forcers.is_empty() {
        println!("  (no walk-forcers)");
    }
    // Compact full roster for doc capture.
    println!("  full peripheral walk/scheduler status:");
    for p in &inv.peripherals {
        let role = if p.needs_legacy_walk && !p.uses_scheduler {
            "FORCER"
        } else if p.uses_scheduler {
            "scheduler"
        } else {
            "inert"
        };
        println!(
            "    [{role:9}] {:20} needs_legacy_walk={} uses_scheduler={}",
            p.name, p.needs_legacy_walk, p.uses_scheduler
        );
    }
    println!();
}

fn load_manifest(system_rel: &str) -> SystemManifest {
    let system_path = root(system_rel);
    let mut manifest = SystemManifest::from_file(&system_path).unwrap_or_else(|e| {
        panic!("load system manifest {system_path:?}: {e}");
    });
    // Anchor chip path so resolve_peripheral_path finds descriptors regardless
    // of cargo-test CWD.
    let anchored = system_path
        .parent()
        .expect("system path parent")
        .join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    // Inventory always auto-derives (never honor a hand walk_deleted hatch).
    manifest.walk_deleted = None;
    manifest
}

fn load_chip(chip_rel: &str) -> ChipDescriptor {
    let path = root(chip_rel);
    ChipDescriptor::from_file(&path).unwrap_or_else(|e| panic!("load chip {path:?}: {e}"))
}

fn bus_f103() -> SystemBus {
    let chip = load_chip("configs/chips/stm32f103.yaml");
    let manifest = load_manifest("examples/ssd1306-hello-lab/system.yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build f103 bus");
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    bus
}

fn bus_esp32c3() -> SystemBus {
    let chip = load_chip("configs/chips/esp32c3.yaml");
    let manifest = load_manifest("configs/systems/esp32c3-devkit.yaml");
    SystemBus::from_config(&chip, &manifest).expect("build esp32c3 bus")
}

fn bus_h563() -> SystemBus {
    let chip = load_chip("configs/chips/stm32h563.yaml");
    let manifest = load_manifest("configs/systems/nucleo-h563zi-demo.yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build h563 bus");
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    bus
}

fn bus_rp2040() -> SystemBus {
    // Opt out of in-tree bootrom so inventory sees the same peripheral set the
    // rest of the RP2040 tests assemble (bootrom is not a walk forcer anyway).
    std::env::set_var("LABWIRED_RP2040_BOOTROM", "");
    let chip = load_chip("configs/chips/rp2040.yaml");
    let manifest = load_manifest("configs/systems/rp2040-pico.yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build rp2040 bus");
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    bus
}

fn bus_nrf52840() -> SystemBus {
    let chip = load_chip("configs/chips/nrf52840.yaml");
    let manifest = load_manifest("configs/systems/nrf52840-dk.yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build nrf52840 bus");
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    bus
}

/// Silicon Labs BRD2709A (EFR32MG26) — the Series-2 family.
///
/// ⚠️ Worth listing separately from the other Cortex-M boards because its
/// walk-forcing set was the largest any shipped board had: TEN `Efr32s2Timer`
/// instances, FOUR shared-`I2c` instances in the EFR32 variant, the IADC, the
/// GPIO EXTI block and the `VirtualBle` controller — fifteen entries, of which
/// only three lived in EFR32-specific model files. A migration that moved the
/// obvious three would have left the board at `max_safe = 1` and reported
/// success.
fn bus_brd2709a() -> SystemBus {
    let chip = load_chip("configs/chips/efr32mg26.yaml");
    let manifest = load_manifest("configs/systems/brd2709a.yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build brd2709a bus");
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    bus
}

fn bus_esp32s3() -> SystemBus {
    // Production / WASM path — NOT SystemBus::from_config on chip YAML.
    // from_config stubs rmt/gdma/systimer (and other S3 models) with generic
    // placeholders, which previously produced a false walk-free inventory.
    // WasmSimulator::new_from_config_xtensa_esp32s3 and the firmware e2e /
    // oracle buses all call configure_xtensa_esp32s3 (→ register_esp32s3_
    // peripherals). Mirror that, then auto-derive walk deletion the same
    // way from_config does under walk_deleted = None so forcer emptiness
    // lines up with legacy_walk_disabled / max_safe for the inventory.
    let mut bus = SystemBus::new();
    let _ = labwired_core::system::xtensa::configure_xtensa_esp32s3(
        &mut bus,
        &labwired_core::system::xtensa::Esp32s3Opts::default(),
    );
    bus.recompute_walk_deletable();
    bus
}

/// Classic ESP32 — the family this inventory was missing.
///
/// Production path (`configure_xtensa_esp32`), not `from_config`: the classic
/// bus is assembled in code, and `from_config` on the chip YAML stubs the
/// models with generic placeholders, which would report a false walk-free
/// roster (the same trap `bus_esp32s3` documents).
///
/// `configure_xtensa_esp32` sets `legacy_walk_disabled` itself; re-derive over
/// the final peripheral set so the inventory reports the DERIVED property
/// rather than whatever the assembler latched. That difference is the whole
/// point of listing this family — see `esp32_classic_walk_forcers_are_named`.
fn bus_esp32_classic() -> SystemBus {
    let mut bus = SystemBus::new();
    let _ = labwired_core::system::xtensa::configure_xtensa_esp32(&mut bus);
    bus.recompute_walk_deletable();
    bus
}

/// Classic ESP32 is NOT walk-free, and this pins which models hold the walk.
///
/// It is the only shipped family absent from this inventory, and it is also the
/// family with the first confirmed tick-starvation defect: `configure_xtensa_
/// esp32` asserted `legacy_walk_disabled = true` under a comment claiming
/// `uart0` had migrated to the event scheduler. `Esp32Uart` never did — it
/// drains `tx_fifo` from `tick()` and nowhere else — so under `event-scheduler`
/// the hand flag deleted the walk out from under it and arduino-esp32 spun
/// forever in `uart_ll_write_txfifo`.
///
/// This test asserts the honest derived state: the forcer set is NON-empty and
/// contains the UARTs. It is deliberately not a "should be walk-free" gate —
/// asserting a property this bus does not have is how the defect got in.
#[test]
fn esp32_classic_walk_forcers_are_named() {
    let bus = bus_esp32_classic();
    let inv = inventory("esp32-classic", &bus);
    print_inventory(&inv);

    let forcing: Vec<&str> = inv.forcers.iter().map(|f| f.name.as_str()).collect();
    assert!(
        !forcing.is_empty(),
        "classic ESP32 reported an EMPTY walk-forcer set. Either a model was \
         genuinely migrated (update this test and the docs) or one is lying \
         about `needs_legacy_walk` — check `Esp32Uart` first."
    );
    assert!(
        forcing.iter().any(|n| n.starts_with("uart")),
        "expected a classic-ESP32 UART among the walk-forcers, got {forcing:?}. \
         `Esp32Uart::tick_elapsed` is the only thing that drains `tx_fifo`; if \
         it stops forcing the walk it must have gained a real event chain."
    );
    assert_eq!(
        inv.walk_deletable, inv.legacy_walk_disabled,
        "classic ESP32: derived walk_deletable ({}) != legacy_walk_disabled \
         ({}) after recompute — the derivation and the latch disagree",
        inv.walk_deletable, inv.legacy_walk_disabled
    );
}

/// PR-B gate: nRF52840 DK auto-derives walk deletion under `event-scheduler`
/// with `walk_deleted = None` and reaches `RECOMMENDED_TICK_INTERVAL` (512).
///
/// When migration is partial this test still fails with the remaining forcer
/// list — pin that set as EXPECTED only if the campaign cannot finish empty
/// in one PR (see docs/performance inventory).
#[test]
fn nrf52840_dk_is_walk_free_and_tick_512() {
    let bus = bus_nrf52840();
    let inv = inventory("nrf52840", &bus);
    print_inventory(&inv);

    #[cfg(feature = "event-scheduler")]
    {
        let forcing: Vec<&str> = inv.forcers.iter().map(|f| f.name.as_str()).collect();
        assert!(
            forcing.is_empty(),
            "nrf52840-dk still has walk-forcers under event-scheduler: {forcing:?}"
        );
        assert!(
            inv.legacy_walk_disabled,
            "nrf52840-dk: expected legacy_walk_disabled after auto-derive"
        );
        assert!(
            !inv.flash_models_ops && !inv.has_iolink_master,
            "nrf52840-dk: unexpected non-forcer max_safe blocker"
        );
        assert_eq!(
            inv.max_safe, RECOMMENDED_TICK_INTERVAL,
            "nrf52840-dk: expected max_safe={RECOMMENDED_TICK_INTERVAL}, got {}",
            inv.max_safe
        );
    }

    #[cfg(not(feature = "event-scheduler"))]
    {
        assert_eq!(inv.max_safe, 1, "featureless build must keep max_safe=1");
    }
}

/// PR-C gate: RP2040 Pico auto-derives walk deletion under `event-scheduler`
/// with `walk_deleted = None` and reaches `RECOMMENDED_TICK_INTERVAL` (512).
///
/// Inventory forcers (Task 1): dma, pio0, timer, spi0, i2c0, sio, xip_ssi,
/// usbctrl. Class-A inert models clear `needs_legacy_walk`; Class-B models
/// take a real event path (`uses_scheduler`). No `walk_deleted` YAML hatch.
#[test]
fn rp2040_pico_is_walk_free_and_tick_512() {
    let bus = bus_rp2040();
    let inv = inventory("rp2040", &bus);
    print_inventory(&inv);

    #[cfg(feature = "event-scheduler")]
    {
        let forcing: Vec<&str> = inv.forcers.iter().map(|f| f.name.as_str()).collect();
        assert!(
            forcing.is_empty(),
            "rp2040-pico still has walk-forcers under event-scheduler: {forcing:?}"
        );
        assert!(
            inv.legacy_walk_disabled,
            "rp2040-pico: expected legacy_walk_disabled after auto-derive"
        );
        assert!(
            !inv.flash_models_ops && !inv.has_iolink_master,
            "rp2040-pico: unexpected non-forcer max_safe blocker"
        );
        assert_eq!(
            inv.max_safe, RECOMMENDED_TICK_INTERVAL,
            "rp2040-pico: expected max_safe={RECOMMENDED_TICK_INTERVAL}, got {}",
            inv.max_safe
        );
    }

    #[cfg(not(feature = "event-scheduler"))]
    {
        assert_eq!(inv.max_safe, 1, "featureless build must keep max_safe=1");
    }
}

/// PR-D gate: NUCLEO-H563ZI demo auto-derives walk deletion under
/// `event-scheduler` with `walk_deleted = None` and reaches
/// `RECOMMENDED_TICK_INTERVAL` (512).
///
/// Inventory forcers (Task 1): gpdma1, fdcan1, rtc, pwr. Class-A: PwrH5.
/// Class-B: GPDMA / RtcV3 / FDCAN. **Single-node** FDCAN is intentional
/// walk-free (TX+IRQ events). **Multi-node** with CanBus `bus_rx` attached
/// forces the walk (absent on this demo bus) — honest interim, not a hatch.
/// H5 FLASH still sets `flash_models_ops` (CPU quantum 1 via
/// `requires_cycle_accurate`) but no longer blocks max_safe. No
/// `walk_deleted` YAML hatch.
#[test]
fn h563_is_walk_free_and_tick_512() {
    let bus = bus_h563();
    let inv = inventory("stm32h563", &bus);
    print_inventory(&inv);

    #[cfg(feature = "event-scheduler")]
    {
        let forcing: Vec<&str> = inv.forcers.iter().map(|f| f.name.as_str()).collect();
        assert!(
            forcing.is_empty(),
            "nucleo-h563zi-demo still has walk-forcers under event-scheduler: {forcing:?}"
        );
        assert!(
            inv.legacy_walk_disabled,
            "h563: expected legacy_walk_disabled after auto-derive"
        );
        assert!(
            inv.flash_models_ops,
            "h563: expected flash_models_ops (H5 FLASH erase/bank-swap drain)"
        );
        assert!(
            !inv.has_iolink_master,
            "h563: unexpected iolink max_safe blocker"
        );
        assert_eq!(
            inv.max_safe, RECOMMENDED_TICK_INTERVAL,
            "h563: expected max_safe={RECOMMENDED_TICK_INTERVAL} (flash_models_ops must not pin tick interval), got {}",
            inv.max_safe
        );
    }

    #[cfg(not(feature = "event-scheduler"))]
    {
        assert_eq!(inv.max_safe, 1, "featureless build must keep max_safe=1");
    }
}

/// BRD2709A (EFR32MG26) auto-derives walk deletion under `event-scheduler` and
/// reaches `RECOMMENDED_TICK_INTERVAL` (512).
///
/// The forcer inventory this had to empty: `timer0`..`timer9`
/// (`Efr32s2Timer` — lazy counter + closed-form wake), `i2c0`..`i2c3`
/// (`I2c::Efr32s2` — held-level re-assert chain), `iadc0`, `gpio_exti` and
/// `ble` (`VirtualBle` — advertising deadline plus a scan-cadence chain). No
/// `walk_deleted` YAML hatch; the executing proof that the migration is
/// behaviour-preserving is `efr32mg26_walk_differential`.
#[test]
fn brd2709a_is_walk_free_and_tick_512() {
    let bus = bus_brd2709a();
    let inv = inventory("efr32mg26", &bus);
    print_inventory(&inv);

    #[cfg(feature = "event-scheduler")]
    {
        let forcing: Vec<&str> = inv.forcers.iter().map(|f| f.name.as_str()).collect();
        assert!(
            forcing.is_empty(),
            "brd2709a still has walk-forcers under event-scheduler: {forcing:?}"
        );
        assert!(
            inv.legacy_walk_disabled,
            "brd2709a: expected legacy_walk_disabled after auto-derive"
        );
        assert!(
            !inv.flash_models_ops && !inv.has_iolink_master,
            "brd2709a: unexpected non-forcer max_safe blocker"
        );
        assert_eq!(
            inv.max_safe, RECOMMENDED_TICK_INTERVAL,
            "brd2709a: expected max_safe={RECOMMENDED_TICK_INTERVAL}, got {}",
            inv.max_safe
        );
    }

    #[cfg(not(feature = "event-scheduler"))]
    {
        assert_eq!(inv.max_safe, 1, "featureless build must keep max_safe=1");
    }
}

/// PR-E gate: ESP32-S3 production bus (`configure_xtensa_esp32s3`) auto-derives
/// walk deletion under `event-scheduler` and reaches `RECOMMENDED_TICK_INTERVAL`
/// (512). Class-A inert models clear `needs_legacy_walk`; Class-B models take
/// scheduler / matrix level export (or bus_tick for GDMA/RMT). No hand
/// `walk_deleted` hatch — `configure` ends with `recompute_walk_deletable`.
#[test]
fn esp32s3_is_walk_free_and_tick_512() {
    let bus = bus_esp32s3();
    let inv = inventory("esp32s3", &bus);
    print_inventory(&inv);

    #[cfg(feature = "event-scheduler")]
    {
        let forcing: Vec<&str> = inv.forcers.iter().map(|f| f.name.as_str()).collect();
        assert!(
            forcing.is_empty(),
            "esp32s3 still has walk-forcers under event-scheduler: {forcing:?}"
        );
        assert!(
            inv.legacy_walk_disabled,
            "esp32s3: expected legacy_walk_disabled after configure + recompute"
        );
        assert!(
            !inv.flash_models_ops && !inv.has_iolink_master,
            "esp32s3: unexpected non-forcer max_safe blocker"
        );
        assert_eq!(
            inv.max_safe, RECOMMENDED_TICK_INTERVAL,
            "esp32s3: expected max_safe={RECOMMENDED_TICK_INTERVAL}, got {}",
            inv.max_safe
        );
    }

    #[cfg(not(feature = "event-scheduler"))]
    {
        assert_eq!(inv.max_safe, 1, "featureless build must keep max_safe=1");
    }
}

/// Regression: green families flip walk-deletion and raise max_safe to 512
/// under `event-scheduler`.
#[test]
fn tick_interval_inventory_all_families() {
    let rows = [
        ("stm32f103", bus_f103()),
        ("esp32c3", bus_esp32c3()),
        ("stm32h563", bus_h563()),
        ("rp2040", bus_rp2040()),
        ("nrf52840", bus_nrf52840()),
        ("esp32s3", bus_esp32s3()),
    ];

    let inventories: Vec<Inventory> = rows
        .iter()
        .map(|(name, bus)| inventory(name, bus))
        .collect();

    for inv in &inventories {
        print_inventory(inv);
    }

    // Sanity: forcer emptiness must agree with legacy_walk_disabled under
    // auto-derive (walk_deleted = None). A mismatch would mean a non-peripheral
    // latch or a hand flag leaked through.
    for inv in &inventories {
        assert_eq!(
            inv.walk_deletable, inv.legacy_walk_disabled,
            "{}: walk_deletable ({}) != legacy_walk_disabled ({}) under auto-derive",
            inv.chip, inv.walk_deletable, inv.legacy_walk_disabled
        );
    }

    #[cfg(feature = "event-scheduler")]
    {
        // Green families: max_safe must already be RECOMMENDED_TICK_INTERVAL.
        // H563 keeps flash_models_ops (CPU quantum 1) but that no longer
        // pins the peripheral tick interval. ESP32-S3 (PR-E) is production
        // `configure_xtensa_esp32s3` + recompute.
        for name in [
            "stm32f103",
            "esp32c3",
            "nrf52840",
            "rp2040",
            "stm32h563",
            "esp32s3",
        ] {
            let inv = inventories
                .iter()
                .find(|i| i.chip == name)
                .expect("family present");
            assert!(
                inv.forcers.is_empty(),
                "{name} should have no walk-forcers, got {:?}",
                inv.forcers
                    .iter()
                    .map(|f| f.name.as_str())
                    .collect::<Vec<_>>()
            );
            assert!(
                inv.legacy_walk_disabled,
                "{name}: expected legacy_walk_disabled"
            );
            if name != "stm32h563" {
                assert!(!inv.flash_models_ops, "{name}: unexpected flash_models_ops");
            } else {
                assert!(
                    inv.flash_models_ops,
                    "stm32h563: expected flash_models_ops (CPU quantum still 1)"
                );
            }
            assert!(!inv.has_iolink_master, "{name}: unexpected iolink blocker");
            assert_eq!(
                inv.max_safe, RECOMMENDED_TICK_INTERVAL,
                "{name}: expected max_safe={RECOMMENDED_TICK_INTERVAL}, got {}",
                inv.max_safe
            );
        }
    }

    #[cfg(not(feature = "event-scheduler"))]
    {
        // Featureless builds never raise the interval.
        for inv in &inventories {
            assert_eq!(
                inv.max_safe, 1,
                "{}: featureless build must keep max_safe=1",
                inv.chip
            );
        }
        let _ = &inventories;
    }
}
