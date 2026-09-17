//! Pins the walk-free STM32 campaign's *remaining surface* on the L476
//! nokia5110-invaders bus as it is actually executed (`from_config` +
//! `configure_cortex_m`, exactly how every e2e/capture harness builds it).
//! The bus is built with any hand `walk_deleted` flag stripped, so the
//! assertion reflects only what the models themselves prove — not a manifest
//! override.
//!
//! The walk-forcing set is `needs_legacy_walk() && !uses_scheduler()` — the
//! exact predicate `derive_walk_deletable` negates (see this module's parent).
//! uart×6 / spi×3 are already event-migrated (`uses_scheduler()==true`) so
//! they carry the default `needs_legacy_walk()==true` but do NOT force the
//! walk; they are correctly excluded here.
//!
//! Why `configure_cortex_m`: the Cortex-M core installs the *real* SCB and
//! NVIC (the chip descriptor only carries inert placeholders for those ids)
//! and appends DWT (CYCCNT), which is not in the descriptor at all. In a
//! featureless build SCB's `tick()` drains software-pended exceptions and
//! DWT's advances CYCCNT — both real walk work. Omitting this step would hide
//! SCB's real tick and DWT entirely and under-report the surface, so the
//! runtime-faithful bus is the honest one to pin. (Under `event-scheduler`,
//! `configure_cortex_m` attaches the bus cycle clock to DWT, migrating it to
//! the lazy-read scheduler path — see the cfg-split lists below.)
//!
//! After batch **B0** (Class-A inert sweep) the forcing set is the plan's 21
//! Class-B instances still awaiting scheduler migration, PLUS the core DWT
//! (a lazy-read CYCCNT counter the plan calls out separately as the purest
//! read-sync case). Each later batch (SysTick+SCB, timers, DMA, the DWT
//! lazy-CYCCNT migration, …) moves a slice onto the scheduler, flipping its
//! `uses_scheduler()`, so this expected set shrinks batch by batch. Keep it
//! in lockstep with the plan's inventory.
//!
//! Batch **B1** (SysTick + SCB → scheduler) removes `systick` and `scb`
//! from the forcing set — but only in `event-scheduler` builds:
//! `uses_scheduler()` needs both the feature AND the bus-attached
//! [`crate::CycleClock`], so the featureless lane honestly keeps them on
//! the walk (the two lists below are cfg-split for exactly that reason).

use crate::bus::SystemBus;
use crate::system::cortex_m::configure_cortex_m;
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;

/// Walk-forcing ids on the runtime invaders bus after the I2C migration
/// (event-scheduler builds): the prior 6 (B2/B3 timers minus the 2 `dma*`,
/// with the DWT lazy-CYCCNT migration from #522) minus the 3 `i2c*` = 2.
/// The 3 `i2c*` NOW migrate: the STM32 F1/L4 transaction engine is self-paced
/// by the SAME held-level, delay-1 self-perpetuating event chain the Kinetis
/// variant uses. The chain runs `F1I2c::tick()`/`L4I2c::tick()` every cycle
/// while a transfer is *active* (countdown in flight OR SR2.BUSY set), so the
/// `&self`-read side effects (`rxne_consumed` / device byte pulls) are
/// observed by the already-live chain's next `on_event` exactly as the walk's
/// next `tick()` would — no event needs arming from the read path. Proven
/// byte-identical (registers + read bytes + NVIC-pend cycles) by the
/// `kinetis_scheduler` differential module (`f1_*`/`l4_*` cases). Remaining
/// plan Class-B on this bus: adc + exti.
///
/// bxCAN (`can1`) NO LONGER forces the walk: its `tick()` only drains a
/// `CanBus` mpsc interconnect, so `needs_legacy_walk()` now reports
/// `bus_rx.is_some()` — false on this bus (no multi-node CanBus is wired;
/// can-player replay is pushed by `service_can_log_players`, not the tick).
///
/// EXTI (`exti`) NO LONGER forces the walk: its held-level `explicit_irqs`
/// re-emission is driven by a delay-1 self-perpetuating event chain (armed on
/// the MMIO write that raises a masked pending line, stopping when firmware
/// clears PR). Byte-identical proof: `exti::scheduler_diff`.
///
/// ADC (`adc1`) NO LONGER forces the walk: its F1 conversion countdown is
/// event-scheduled (delay-1 chain armed on SWSTART, perpetuating through
/// continuous mode), and the legacy `cycles: 1` per-converting-tick cost is
/// normalized to zero in BOTH modes (SysTick B1 pattern) so `total_cycles`
/// agrees. Byte-identical proof: `adc::scheduler_diff`.
///
/// The forcing set is now EMPTY — with every Class-B walker migrated the
/// runtime invaders (L476) bus derives walk-deletion with no hand flag: the
/// campaign's full STM32 board flip.
#[cfg(feature = "event-scheduler")]
const EXPECTED_WALK_FORCING: &[&str] = &[];

/// Featureless builds: the scheduler does not exist, so SysTick and SCB
/// stay on the legacy walk. bxCAN (`can1`) is excluded regardless of the
/// feature — its walk-forcing is gated on an attached interconnect, not the
/// scheduler.
#[cfg(not(feature = "event-scheduler"))]
const EXPECTED_WALK_FORCING: &[&str] = &[
    "systick", "tim1", "tim2", "tim3", "tim4", "tim5", "tim6", "tim7", "tim8", "tim15", "tim16",
    "tim17", "dma1", "dma2", "i2c1", "i2c2", "i2c3", "adc1", "exti", "scb", "dwt",
];

fn invaders_bus_walk_stripped() -> SystemBus {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nokia5110-invaders-lab/system.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load invaders manifest");
    // Construct WITHOUT the lab's hand `walk_deleted: true`: the campaign
    // surface must come from the models, not the manifest escape hatch.
    manifest.walk_deleted = None;
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load l476 chip");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build invaders bus");
    // Install the real SCB/NVIC and the core DWT — the executed bus.
    let _ = configure_cortex_m(&mut bus);
    bus
}

#[test]
fn remaining_walk_forcing_set_matches_campaign_inventory() {
    let bus = invaders_bus_walk_stripped();

    let mut forcing: Vec<&str> = bus
        .peripherals
        .iter()
        .filter(|p| p.dev.needs_legacy_walk() && !p.dev.uses_scheduler())
        .map(|p| p.name.as_str())
        .collect();
    forcing.sort_unstable();

    let mut expected: Vec<&str> = EXPECTED_WALK_FORCING.to_vec();
    expected.sort_unstable();

    assert_eq!(
            forcing,
            expected,
            "walk-forcing set (needs_legacy_walk && !uses_scheduler) drifted from the \
             campaign inventory (currently: post-B1).\n  got ({}):      {:?}\n  expected ({}): {:?}\n\
             A model newly (un)marked `needs_legacy_walk()` or migrated to the scheduler \
             must update EXPECTED_WALK_FORCING to match the campaign's remaining surface.",
            forcing.len(),
            forcing,
            expected.len(),
            expected,
        );
}

#[test]
fn invaders_bus_now_flips_with_every_class_b_migrated() {
    // With I2C, EXTI and ADC all event-scheduled, the runtime invaders
    // (L476) bus has an EMPTY walk-forcing set and derives walk-deletion
    // with no hand `walk_deleted` flag — the campaign's full STM32 board
    // flip. (bxCAN never forced it here — its walk work is gated on an
    // attached CanBus interconnect, absent on this bus.)
    let bus = invaders_bus_walk_stripped();
    #[cfg(feature = "event-scheduler")]
    assert!(
        bus.derive_walk_deletable(),
        "invaders bus should be walk-deletable once every Class-B walker \
             (i2c/exti/adc) is migrated and the forcing set is empty"
    );
    // Featureless builds have no scheduler, so the migrated models honestly
    // stay on the walk and the bus does NOT flip.
    #[cfg(not(feature = "event-scheduler"))]
    assert!(
        !bus.derive_walk_deletable(),
        "featureless build keeps the walk (no scheduler to migrate onto)"
    );
}
