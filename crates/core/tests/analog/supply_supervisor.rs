// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The MCU's supply comes from its circuit: a regulator lowered by the analog
//! engine drives `board.power.vdd_volts`, and the STM32F401's supply
//! supervisor holds the core in reset, releases it, and resets it on a
//! power-down or brown-out — with the reset cause in RCC_CSR where the
//! firmware reads it.
//!
//! The firmware is `tests/fixtures/stm32f401-supply-bootlog.elf` (source in
//! `tests/fixtures/supply-bootlog/`): on every boot it appends RCC_CSR to a
//! log in RAM, counts the boot, clears the flags with RMVF and spins
//! incrementing a heartbeat. So the reset sequence asserted here is the one the
//! FIRMWARE saw, not only the supervisor's own counters.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::power::SupplyResetCause;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Bus, Machine};
use std::path::Path;

const LOG_MAGIC: u32 = 0xB007_C0DE;
/// RCC_CSR after a power-on reset: PORRSTF | PINRSTF | BORRSTF.
const CSR_POWER_ON: u32 = 0x0E00_0000;
/// RCC_CSR after a brown-out reset: PINRSTF | BORRSTF.
const CSR_BROWN_OUT: u32 = 0x0600_0000;

/// 1 ms at the F401's 84 MHz.
const CYCLES_PER_MS: u64 = 84_000;

struct Rig {
    machine: Machine<CortexM>,
    session: CosimSession,
}

/// An F401 running the boot logger, with `bor_level` programmed (`None` is the
/// factory "BOR off"), and one analog model whose `vdd` probe is routed to
/// `route` (`board.power.vdd_volts`, or a plain store path for the negative
/// control).
fn rig(netlist: &str, bor_level: Option<&str>, route: &str) -> Rig {
    labwired_core::fidelity::reset();
    let chip_path = root("configs/chips/stm32f401.yaml");
    let mut descriptor = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    descriptor
        .supply_monitor
        .as_mut()
        .expect("the F401 descriptor declares a supply_monitor")
        .bor_level = bor_level.map(str::to_string);
    let netlist = netlist
        .lines()
        .map(|line| format!("        {line}\n"))
        .collect::<String>();
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: \"supply\"\n\
         chip: \"{chip}\"\n\
         external_devices: []\n\
         cosim_models:\n\
         \x20 - id: power\n\
         \x20   adapter: analog\n\
         \x20   step_ns: 5000\n\
         \x20   outputs: {{ vdd: {route} }}\n\
         \x20   config:\n\
         \x20     substeps: 10\n\
         \x20     integration: trap\n\
         \x20     probes: {{ vdd: \"v(vdd)\" }}\n\
         \x20     netlist_text: |\n{netlist}",
        chip = chip_path.display(),
    ))
    .expect("parse manifest");
    let mut bus = SystemBus::from_config(&descriptor, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&root("tests/fixtures/stm32f401-supply-bootlog.elf"))
        .expect("load boot-logger fixture");
    machine.load_firmware(&image).expect("load firmware");
    let session = CosimSession::new(&manifest.cosim_models, Path::new("."), &machine.bus)
        .expect("build session")
        .expect("one model");
    assert_eq!(session.binding_errors(), &[] as &[RoutingError]);
    Rig { machine, session }
}

impl Rig {
    fn run_ms(&mut self, ms: u64) {
        self.session
            .advance_budget(
                &mut self.machine,
                AdvanceRequest::run(None).with_cycle_limit(ms * CYCLES_PER_MS),
            )
            .expect("advance");
    }

    fn word(&mut self, addr: u64) -> u32 {
        self.machine.bus.read_u32(addr).expect("read RAM")
    }

    /// Boots the firmware counted, or 0 if it never ran at all.
    fn boots(&mut self) -> u32 {
        if self.word(0x2000_0000) == LOG_MAGIC {
            self.word(0x2000_0004)
        } else {
            0
        }
    }

    fn heartbeat(&mut self) -> u32 {
        if self.word(0x2000_0000) == LOG_MAGIC {
            self.word(0x2000_0008)
        } else {
            0
        }
    }

    /// RCC_CSR as the firmware read it at each boot.
    fn causes(&mut self) -> Vec<u32> {
        let boots = self.boots().min(8);
        (0..boots)
            .map(|i| self.word(0x2000_0010 + 4 * u64::from(i)))
            .collect()
    }
}

/// 9 V into an AMS1117-3.3, ramped up over 1 ms, with 10 µF on the rail and
/// the MCU as a 330 Ω load.
const REGULATOR_9V: &str = "Vin in 0 PULSE(0 9 0 1m 1m 1 2)
XU1 in vdd 0 AMS1117-3.3
Cout vdd 0 10u
Rmcu vdd 0 330";

#[test]
fn a_regulated_board_holds_the_core_until_por_then_boots_once() {
    let mut rig = rig(REGULATOR_9V, None, "board.power.vdd_volts");
    rig.run_ms(0);
    assert!(
        rig.machine.supply_status().routed,
        "routed from the first advance"
    );

    // 0.1 ms in, the input ramp is at 0.9 V and the regulator's output is
    // below the 1.72 V power-on threshold: not one instruction has run.
    rig.session
        .advance_budget(
            &mut rig.machine,
            AdvanceRequest::run(None).with_cycle_limit(CYCLES_PER_MS / 10),
        )
        .expect("advance");
    let early = rig.machine.supply_status();
    assert!(early.held_in_reset, "held at {:?} V", early.vdd_volts);
    assert!(early.vdd_volts.expect("the model has stepped") < 1.72);
    assert_eq!(rig.boots(), 0, "the firmware must not have run");

    rig.run_ms(5);
    let status = rig.machine.supply_status();
    let vdd = status.vdd_volts.expect("vdd");
    assert!((vdd - 3.3).abs() < 0.01, "regulated rail {vdd} V");
    assert!(!status.held_in_reset);
    assert_eq!((status.power_on_resets, status.brown_out_resets), (1, 0));
    assert_eq!(status.last_cause, Some(SupplyResetCause::PowerOn));
    assert_eq!(rig.boots(), 1);
    assert_eq!(rig.causes(), vec![CSR_POWER_ON]);
    assert!(rig.heartbeat() > 1000, "the firmware is running");
    // RMVF cleared the flags the firmware read.
    assert_eq!(rig.word(0x4002_3874) & 0xFE00_0000, 0);

    let gaps = labwired_core::fidelity::report().to_gaps();
    assert!(
        !gaps
            .iter()
            .any(|g| g.kind == labwired_core::fidelity::UNPOWERED_RAIL_ASSUMED),
        "a routed rail is not an assumed one: {gaps:?}"
    );
}

/// 4.0 V into an AMS1117-3.3 is dropout: the rail sits near 4.0 − 1.1 =
/// 2.9 V. With the F401's BOR level 3 programmed (2.92 V rising) that is under
/// the release threshold, so the MCU never boots.
const DROPOUT_4V0: &str = "Vin in 0 dc 4.0
XU1 in vdd 0 AMS1117-3.3
Cout vdd 0 10u
Rmcu vdd 0 330";

#[test]
fn a_regulator_in_dropout_below_bor_never_boots_the_core() {
    let mut rig = rig(DROPOUT_4V0, Some("bor3"), "board.power.vdd_volts");
    rig.run_ms(5);
    let status = rig.machine.supply_status();
    let vdd = status.vdd_volts.expect("vdd");
    assert!(
        vdd > 2.85 && vdd < 2.92,
        "AMS1117-3.3 at 4.0 V in dropout should sit near 2.9 V, got {vdd}"
    );
    assert!(status.held_in_reset);
    assert_eq!(status.release_volts, Some(2.92));
    assert_eq!((status.power_on_resets, status.brown_out_resets), (0, 0));
    assert_eq!(rig.boots(), 0, "the firmware never ran");
    assert_eq!(rig.heartbeat(), 0);
}

/// The control for the test above: the SAME rail with the factory option
/// bytes (BOR off, only the 1.72 V POR) boots — so it is the threshold that
/// kept the core down, not a broken rig.
#[test]
fn the_same_dropout_rail_boots_with_brown_out_reset_off() {
    let mut rig = rig(DROPOUT_4V0, None, "board.power.vdd_volts");
    rig.run_ms(5);
    assert!(!rig.machine.supply_status().held_in_reset);
    assert_eq!(rig.boots(), 1);
    assert_eq!(rig.causes(), vec![CSR_POWER_ON]);
}

/// Two sags on a 9 V input: to 3.6 V at 3 ms (the rail drops to ~2.5 V —
/// under BOR level 3's 2.83 V, above the 1.68 V power-down) and to 1 V at 6 ms
/// (the rail collapses: a power-down). The firmware must see three boots:
/// power-on, brown-out, power-on — in that order, in RCC_CSR.
const TWO_SAGS: &str = "Vin in 0 PULSE(9 3.6 3m 50u 50u 1m 10)
Vsag in2 in PULSE(0 -8 6m 50u 50u 1m 10)
XU1 in2 vdd 0 AMS1117-3.3
Cout vdd 0 10u
Rmcu vdd 0 330";

#[test]
fn a_sag_below_bor_is_a_brown_out_and_a_collapse_is_a_power_on_reset() {
    let mut rig = rig(TWO_SAGS, Some("bor3"), "board.power.vdd_volts");
    rig.run_ms(2);
    assert_eq!(rig.boots(), 1);
    let before = rig.heartbeat();

    // Inside the first sag: held, not counting.
    rig.run_ms(2); // t = 4 ms, mid-sag... the sag ends at 4.05 ms
    rig.session
        .advance_budget(
            &mut rig.machine,
            AdvanceRequest::run(None).with_cycle_limit(0),
        )
        .expect("advance");
    let mid = rig.machine.supply_status();
    assert!(
        mid.held_in_reset,
        "held in the sag at {:?} V",
        mid.vdd_volts
    );
    let during = rig.heartbeat();
    rig.session
        .advance_budget(
            &mut rig.machine,
            AdvanceRequest::run(None).with_cycle_limit(CYCLES_PER_MS / 50),
        )
        .expect("advance");
    assert_eq!(rig.heartbeat(), during, "no instruction runs while held");
    assert!(during > before);

    rig.run_ms(5); // t ≈ 9 ms: both sags over
    let status = rig.machine.supply_status();
    assert!(!status.held_in_reset);
    assert_eq!((status.power_on_resets, status.brown_out_resets), (2, 1));
    assert_eq!(rig.boots(), 3);
    assert_eq!(
        rig.causes(),
        vec![CSR_POWER_ON, CSR_BROWN_OUT, CSR_POWER_ON],
        "RCC_CSR at each boot, as the firmware read it"
    );
}

/// Negative control: the same two sags, but the rail routed to a plain store
/// path instead of `board.power.vdd_volts`. The MCU is on today's ideal rail —
/// it boots once and never resets — and the census says the rail is assumed.
#[test]
fn an_unrouted_rail_is_the_ideal_rail_and_says_so() {
    let mut rig = rig(TWO_SAGS, Some("bor3"), "power.vdd_volts");
    rig.run_ms(9);
    let status = rig.machine.supply_status();
    assert!(!status.routed && !status.held_in_reset);
    assert_eq!((status.power_on_resets, status.brown_out_resets), (0, 0));
    assert_eq!(rig.boots(), 1, "on the ideal rail the sags reset nothing");
    // No supply reset ran, so nothing set a flag: the CSR reads as it always
    // has on this model when nothing drives the supply.
    assert_eq!(rig.causes(), vec![0]);
    let gaps = labwired_core::fidelity::report().to_gaps();
    let note = gaps
        .iter()
        .find(|g| g.kind == labwired_core::fidelity::UNPOWERED_RAIL_ASSUMED)
        .unwrap_or_else(|| panic!("the census must name the assumed rail: {gaps:?}"));
    assert!(note.detail.contains("3.3 V"), "{}", note.detail);
}

/// A chip with no `supply_monitor:` refuses a VDD route at bind time, rather
/// than running a circuit that can never reset it.
#[test]
fn a_chip_without_a_supply_monitor_refuses_the_route() {
    let chip_path = root("configs/chips/stm32f401.yaml");
    let mut descriptor = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    descriptor.supply_monitor = None;
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: \"supply\"\nchip: \"{}\"\nexternal_devices: []\ncosim_models:\n  - id: p\n    \
         adapter: analog\n    step_ns: 5000\n    outputs: {{ vdd: board.power.vdd_volts }}\n    \
         config:\n      probes: {{ vdd: \"v(vdd)\" }}\n      netlist_text: |\n        V1 vdd 0 dc \
         3.3\n        R1 vdd 0 1k\n",
        chip_path.display()
    ))
    .expect("parse manifest");
    let mut bus = SystemBus::from_config(&descriptor, &manifest).expect("build bus");
    let _ = configure_cortex_m(&mut bus);
    let session = CosimSession::new(&manifest.cosim_models, Path::new("."), &bus)
        .expect("build session")
        .expect("one model");
    assert!(
        matches!(
            session.binding_errors(),
            [RoutingError::NoSupplyMonitor { .. }]
        ),
        "{:?}",
        session.binding_errors()
    );
}
