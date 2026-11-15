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
///
/// There is no hand-copied name list for this configuration any more. The
/// expectation is DERIVED from the source of each model on the bus, by the
/// parser `walk_starvation_contract` already uses: see
/// [`featureless_expected_forcing`]. A model whose `needs_legacy_walk()` /
/// `uses_scheduler()` bodies change changes the expectation with it, so the
/// observed set (read off a real bus) and the expected set (read off the
/// source) can no longer be edited in lockstep from one place.
/// Bus peripherals whose `Debug` output does not begin with the Rust type that
/// implements them (their model is an enum, and `Debug` prints the variant), so
/// the source lookup cannot resolve them. Each carries its expectation by hand
/// and a reason. A name here that CAN now be resolved fails the test, so the
/// list only shrinks.
#[cfg(not(feature = "event-scheduler"))]
const UNRESOLVED_BY_DEBUG_NAME: &[(&str, bool, &str)] = &[
    (
        "rcc",
        false,
        "enum model (Debug prints the variant `Stm32L4`); no walk work",
    ),
    (
        "exti",
        true,
        "enum model (Debug prints the variant `Stm32L4`); `peripherals/exti.rs` keeps the \
         walk without the scheduler",
    ),
];

/// What a featureless build says about one impl, from its source bodies.
#[cfg(not(feature = "event-scheduler"))]
fn impl_forces_walk_featureless(
    needs_legacy_walk: Option<&str>,
    uses_scheduler: Option<&str>,
) -> Option<bool> {
    // A literal `false` never forces the walk.
    if needs_legacy_walk == Some("false") {
        return Some(false);
    }
    match uses_scheduler {
        // Always on the scheduler, or on it unless explicitly forced back
        // (UART): leaves the walk regardless of the feature in this bus.
        Some("true") | Some("!self.legacy_walk_forced") => Some(false),
        // Joins the scheduler only when `scheduler_mode()` is true, which is
        // false without the feature, so the default/negated
        // `needs_legacy_walk()` keeps it on the walk.
        Some("self.scheduler_mode()") => match needs_legacy_walk {
            None | Some("!self.scheduler_mode()") => Some(true),
            _ => None,
        },
        _ => None,
    }
}

/// Expected featureless forcing set, derived from model source. Returns the
/// sorted peripheral names and panics on a shape it cannot classify, so a new
/// shape needs a deliberate classifier change rather than a silent default.
#[cfg(not(feature = "event-scheduler"))]
fn featureless_expected_forcing(bus: &SystemBus) -> Vec<String> {
    use crate::tests::walk_starvation_contract::parse_peripheral_impls;
    let impls = parse_peripheral_impls();
    let mut out = Vec::new();
    let mut unresolved = Vec::new();
    for p in bus.peripherals.iter() {
        let dbg = format!("{:?}", p.dev);
        let ty: String = dbg
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let found: Vec<_> = impls.iter().filter(|i| i.ty == ty).collect();
        if found.is_empty() {
            unresolved.push(p.name.clone());
            continue;
        }
        // bxCAN's predicate depends on runtime state (an attached interconnect),
        // which the walk_starvation_contract allowlist already justifies; it is
        // not walk-forcing on this bus (no CanBus attached).
        if ty == "BxCan" {
            continue;
        }
        let verdicts: Vec<Option<bool>> = found
            .iter()
            .map(|i| {
                impl_forces_walk_featureless(i.body("needs_legacy_walk"), i.body("uses_scheduler"))
            })
            .collect();
        let first = verdicts[0].unwrap_or_else(|| {
            panic!(
                "peripheral `{}` ({ty}): needs_legacy_walk/uses_scheduler bodies have a shape \
                 this derivation does not classify: {:?}",
                p.name,
                found
                    .iter()
                    .map(|i| (
                        i.file.clone(),
                        i.body("needs_legacy_walk"),
                        i.body("uses_scheduler")
                    ))
                    .collect::<Vec<_>>()
            )
        });
        assert!(
            verdicts.iter().all(|v| *v == Some(first)),
            "peripheral `{}` ({ty}) has several impls that disagree",
            p.name
        );
        if first {
            out.push(p.name.clone());
        }
    }
    unresolved.sort();
    let mut declared: Vec<&str> = UNRESOLVED_BY_DEBUG_NAME
        .iter()
        .map(|(n, _, _)| *n)
        .collect();
    declared.sort();
    assert_eq!(
        unresolved.iter().map(String::as_str).collect::<Vec<_>>(),
        declared,
        "peripherals whose model cannot be resolved from source drifted from \
         UNRESOLVED_BY_DEBUG_NAME (a name that now resolves must be removed)"
    );
    for (name, forces, _) in UNRESOLVED_BY_DEBUG_NAME {
        if *forces {
            out.push((*name).to_string());
        }
    }
    out.sort();
    out
}

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

    let mut forcing: Vec<String> = bus
        .peripherals
        .iter()
        .filter(|p| p.dev.needs_legacy_walk() && !p.dev.uses_scheduler())
        .map(|p| p.name.clone())
        .collect();
    forcing.sort_unstable();

    #[cfg(feature = "event-scheduler")]
    let mut expected: Vec<String> = EXPECTED_WALK_FORCING
        .iter()
        .map(|s| s.to_string())
        .collect();
    #[cfg(not(feature = "event-scheduler"))]
    let mut expected: Vec<String> = featureless_expected_forcing(&bus);
    expected.sort_unstable();
    // Anti-vacuity: the featureless expectation is derived, so prove the
    // derivation found real forcing models rather than an empty set.
    #[cfg(not(feature = "event-scheduler"))]
    assert!(
        expected.len() >= 10,
        "derived featureless walk-forcing set is implausibly small: {expected:?}"
    );

    assert_eq!(
            forcing,
            expected,
            "walk-forcing set (needs_legacy_walk && !uses_scheduler) drifted from the \
             campaign inventory (currently: post-B1).\n  got ({}):      {:?}\n  expected ({}): {:?}\n\
             A model newly (un)marked `needs_legacy_walk()` or migrated to the scheduler \
             must update EXPECTED_WALK_FORCING (event-scheduler) or its source bodies (featureless, derived).",
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
