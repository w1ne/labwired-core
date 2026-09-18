// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The **`logic_gate` primitive** — a 74-series part whose model is a truth
//! table, an enable and a propagation delay.
//!
//! # The gap this fills
//!
//! Running a 39-project KiCad corpus through the importer leaves 74-series
//! logic as the third largest model gap: 139 dropped symbols, led by
//! `SN74LVTH125` (38 placements) and `SN74LVC1T45` (32). Not one of them can be
//! a register part. A gate has no bus: no address, no register file, nothing to
//! write and nothing to read back. It maps input pads to output pads through a
//! boolean function, with enable and direction pads that can take the outputs
//! off the wire, after a propagation delay.
//!
//! # Where it sits
//!
//! Exactly where [`DeclarativeGpioDevice`](super::declarative_gpio) sits: one
//! [`BusResidentDevice`] on the SAME `SystemBus::gpio_devices` list, serviced by
//! the SAME pass, reaching its pads through the SAME narrowed [`DevicePins`]
//! port. No engine type crosses that boundary and this primitive adds nothing
//! to the [`BusResidentDevice`] trait — `tests/bus_resident_device_port.rs` is
//! what holds that line.
//!
//! It is NOT a `gpio_device` with rules, and the reason is the delay. A rule
//! machine answers an EDGE; a gate answers a LEVEL, continuously, and it answers
//! it `tprop` later. Expressing "Y follows A after 9 ns" as rules would need one
//! rule per edge direction per gate plus a timer per gate, and an eight-bit
//! transceiver would be 32 rules and 16 timers that all say the same thing. The
//! truth table says it once.
//!
//! # Timing: why the floor is one cycle
//!
//! `tprop_ns` is converted to simulated CYCLES at attach with a floor of ONE.
//! A zero-delay gate would let firmware store an input and read the answer back
//! in the same instruction, which no real part does — and, worse, it would make
//! the model's answer depend on whether the bus happened to service the device
//! inside that store. One cycle is 12.5 ns at 80 MHz, the right order for the
//! 5–15 ns this family specifies.
//!
//! The device is serviced from two places, and it needs both:
//!
//! * the MMIO write hook, through
//!   [`edge_service_addrs`](BusResidentDevice::edge_service_addrs) — so the
//!   instant firmware moves an input pad, the new level is LATCHED and the
//!   deadline is armed at the right cycle. Without it a bit-banged input that
//!   moves and moves back inside one tick interval would never be seen at all;
//! * the peripheral tick — so the deadline can EXPIRE. The write hook only runs
//!   when firmware writes; a gate whose input settled has to come good on its
//!   own, with nothing writing anything.
//!
//! # What is deliberately not modelled
//!
//! Contention. Two parts driving one net is an electrical question and this
//! engine has one level source per pad, last writer wins. Also: no input
//! thresholds, no slew, no glitch filtering (a pulse shorter than `tprop` is
//! swallowed here, as it largely is on silicon, but not for the same reason),
//! and no supply rail — a level translator's two voltage domains are not
//! represented, only its direction.

use std::collections::BTreeMap;

use anyhow::{anyhow, Context, Result};
use labwired_config::expr::{EvalCtx, Expr};
use labwired_config::{DeviceDescriptor, LogicKind, LogicSpec};

use crate::bus::{BusResidentDevice, DevicePins};
use crate::sim_input::{InputChannel, SimInput, SimInputError};

/// One pad this part is wired to, resolved at attach.
///
/// Both ends are optional and a transceiver role carries BOTH: its ODR (what
/// the MCU drives out, which is what this part reads when the pad is an input
/// that way round) and its IDR (what the MCU samples, which is what this part
/// drives when the pad is an output that way round). A plain input has only the
/// ODR end; a plain output only the IDR end.
#[derive(Debug, Clone)]
pub struct LogicPad {
    /// Descriptor role name (`A1`, `Y1`, `OE1`, `DIR`).
    pub role: String,
    /// Output register of the pin — what the MCU is driving. `None` when this
    /// role is never observed.
    pub odr: Option<(u64, u8)>,
    /// Input register of the pin — what the MCU samples. `None` when this role
    /// is never driven.
    pub idr: Option<(u64, u8)>,
}

/// A compiled truth-table entry: which output, and the boolean function.
#[derive(Debug)]
struct TableEntry {
    output: usize,
    expr: Expr,
}

/// What decides each output's value.
#[derive(Debug)]
enum Plan {
    /// Combinational: one expression per output.
    Table(Vec<TableEntry>),
    /// Transceiver: `(a_role, b_role)` pairs plus the DIR pad and its polarity.
    Transceiver {
        dir: String,
        a_to_b_when: bool,
        pairs: Vec<(String, String)>,
    },
    /// Bus switch: per output, the role followed when S is low and when high.
    Switch {
        select: String,
        sources: Vec<(String, String)>,
    },
}

/// One enable pad, resolved to output INDICES so the service pass does no
/// string work per output per tick.
#[derive(Debug)]
struct Enable {
    pin: String,
    active_high: bool,
    outputs: Vec<usize>,
}

/// A 74-series logic part.
#[derive(Debug)]
pub struct DeclarativeLogicDevice {
    id: String,
    /// Pads read each pass (inputs + controls + both sides of a transceiver).
    observed: Vec<LogicPad>,
    /// Pads this part can drive, in `logic.outputs` order.
    driven: Vec<LogicPad>,
    /// Last level read from each observed pad, by role. `false` for a pad the
    /// port does not read back — see the note in [`Self::service`].
    levels: BTreeMap<String, bool>,
    plan: Plan,
    enables: Vec<Enable>,
    /// Hi-Z when disabled. `false` ⇒ a disabled output is driven LOW.
    hiz: bool,
    /// Propagation delay in simulated cycles. Never zero (see the module note).
    tprop_cycles: u64,
    /// The value each output SHOULD settle to. `None` is Hi-Z.
    desired: Vec<Option<bool>>,
    /// Cycle at which `desired[i]` becomes visible on the pad, while pending.
    deadline: Vec<Option<u64>>,
    /// What each output is currently driving. `None` is "not driving".
    applied: Vec<Option<bool>>,
    /// Output-register addresses this part must be serviced on SYNCHRONOUSLY,
    /// from the MMIO write hook rather than only the peripheral tick. See
    /// [`BusResidentDevice::edge_service_addrs`].
    edge_addrs: Vec<u64>,
}

/// The evaluation context a truth-table expression sees: pin levels, and
/// nothing else.
///
/// `var(NAME)` is how [`labwired_config::compile_table_expr`] spells a pin name
/// in the shared grammar, so `var` is the only accessor that answers anything.
/// Every other one returns 0 — a gate has no registers, no FIFOs and no state
/// machine, and answering 0 is the same answer the grammar's own context gives
/// for a name it does not know.
struct PinLevels<'a>(&'a BTreeMap<String, bool>);

impl EvalCtx for PinLevels<'_> {
    fn reg(&self, _: &str) -> i64 {
        0
    }
    fn field(&self, _: &str, _: &str) -> i64 {
        0
    }
    fn var(&self, name: &str) -> i64 {
        i64::from(self.0.get(name).copied().unwrap_or(false))
    }
    fn input(&self, _: &str) -> i64 {
        0
    }
    fn reported(&self, _: &str) -> i64 {
        0
    }
    fn fifo_len(&self, _: &str) -> i64 {
        0
    }
    fn written(&self) -> i64 {
        0
    }
    fn state(&self) -> &str {
        ""
    }
}

/// No stimulus channels: a gate is driven by its pins, not by the host.
const NO_CHANNELS: &[InputChannel] = &[];

impl DeclarativeLogicDevice {
    /// Build from a descriptor whose pads have already been resolved.
    ///
    /// `cpu_hz` converts `tprop_ns` into cycles — the same derived clock every
    /// other self-timed resident device uses, and it carries the same fidelity
    /// note: a PLL reconfiguration mid-run is not tracked, so a gate's delay is
    /// pinned to the clock the system was built with.
    pub fn new(
        id: String,
        descriptor: &DeviceDescriptor,
        observed: Vec<LogicPad>,
        driven: Vec<LogicPad>,
        cpu_hz: u64,
    ) -> Result<Self> {
        let spec =
            descriptor.behavior.logic.as_ref().ok_or_else(|| {
                anyhow!("logic_gate '{}' has no `logic:` block", descriptor.r#type)
            })?;
        spec.validate(&descriptor.r#type)?;

        let index_of = |role: &str| -> Result<usize> {
            spec.outputs
                .iter()
                .position(|o| o == role)
                .ok_or_else(|| anyhow!("'{role}' is not an output of '{}'", descriptor.r#type))
        };

        let plan = match spec.kind()? {
            LogicKind::Table => {
                let mut entries = Vec::with_capacity(spec.table.len());
                for (out, src) in &spec.table {
                    let (expr, _) =
                        labwired_config::compile_table_expr(src).with_context(|| {
                            format!("logic_gate '{}' table entry '{out}'", descriptor.r#type)
                        })?;
                    entries.push(TableEntry {
                        output: index_of(out)?,
                        expr,
                    });
                }
                Plan::Table(entries)
            }
            LogicKind::Transceiver => {
                let d = spec.direction.as_ref().expect("kind said transceiver");
                Plan::Transceiver {
                    dir: d.pin.clone(),
                    a_to_b_when: d.a_to_b_when == 1,
                    pairs: d.a.iter().cloned().zip(d.b.iter().cloned()).collect(),
                }
            }
            LogicKind::Switch => {
                let s = spec.select.as_ref().expect("kind said switch");
                Plan::Switch {
                    select: s.pin.clone(),
                    sources: s.low.iter().cloned().zip(s.high.iter().cloned()).collect(),
                }
            }
        };

        let mut enables = Vec::with_capacity(spec.enables.len());
        for en in &spec.enables {
            enables.push(Enable {
                pin: en.pin.clone(),
                active_high: matches!(en.active, labwired_config::ActiveLevel::High),
                outputs: en
                    .outputs
                    .iter()
                    .map(|o| index_of(o))
                    .collect::<Result<Vec<_>>>()?,
            });
        }

        let n = driven.len();
        // Every port that hosts an observed pad. A write to one can move an
        // input, and an input that moves between two ticks must still be seen.
        let mut edge_addrs: Vec<u64> = observed
            .iter()
            .filter_map(|p| p.odr)
            .map(|(a, _)| a)
            .collect();
        edge_addrs.sort_unstable();
        edge_addrs.dedup();
        Ok(Self {
            id,
            observed,
            driven,
            levels: BTreeMap::new(),
            plan,
            enables,
            hiz: spec.drive.hiz_when_disabled,
            tprop_cycles: tprop_cycles(spec.tprop_ns, cpu_hz),
            desired: vec![None; n],
            deadline: vec![None; n],
            applied: vec![None; n],
            edge_addrs,
        })
    }

    /// Propagation delay in simulated cycles, for tests and diagnostics.
    pub fn tprop_cycles(&self) -> u64 {
        self.tprop_cycles
    }

    /// What each output is currently DRIVING, in `logic.outputs` order.
    /// `None` is Hi-Z — the part is not writing that pad at all.
    pub fn driving(&self) -> &[Option<bool>] {
        &self.applied
    }

    fn level(&self, role: &str) -> bool {
        self.levels.get(role).copied().unwrap_or(false)
    }

    /// Is output `i` enabled right now? An output no enable mentions is always
    /// on, which is the '04/'08/'32/'00 case.
    fn enabled(&self, i: usize) -> bool {
        self.enables
            .iter()
            .filter(|e| e.outputs.contains(&i))
            .all(|e| {
                let level = self.level(&e.pin);
                if e.active_high {
                    level
                } else {
                    !level
                }
            })
    }

    /// The value every output should settle to right now. `None` is Hi-Z.
    fn compute(&self) -> Vec<Option<bool>> {
        let mut out: Vec<Option<bool>> = vec![None; self.driven.len()];

        // The function first, ignoring enables; the enable pass below is what
        // takes an output off the wire. Keeping them separate is what makes
        // `hiz_when_disabled: false` a one-line difference instead of a second
        // code path.
        match &self.plan {
            Plan::Table(entries) => {
                let ctx = PinLevels(&self.levels);
                for e in entries {
                    out[e.output] = Some(e.expr.eval(&ctx) != 0);
                }
            }
            Plan::Transceiver {
                dir,
                a_to_b_when,
                pairs,
            } => {
                let a_to_b = self.level(dir) == *a_to_b_when;
                for (a, b) in pairs {
                    // The DRIVEN side of the pair takes the level the OBSERVED
                    // side is at. The other side of the pair is not driven at
                    // all this way round — it is an input, and driving it would
                    // fight the MCU for the pad.
                    let (src, dst) = if a_to_b { (a, b) } else { (b, a) };
                    let Some(i) = self.driven.iter().position(|p| &p.role == dst) else {
                        continue;
                    };
                    out[i] = Some(self.level(src));
                }
            }
            Plan::Switch { select, sources } => {
                let high = self.level(select);
                for (i, (low_src, high_src)) in sources.iter().enumerate() {
                    let src = if high { high_src } else { low_src };
                    out[i] = Some(self.level(src));
                }
            }
        }

        for (i, slot) in out.iter_mut().enumerate() {
            if !self.enabled(i) {
                // `hiz_when_disabled: false` is the rare part that pulls a
                // disabled output LOW rather than releasing it.
                *slot = if self.hiz { None } else { Some(false) };
            }
        }
        out
    }
}

/// `tprop_ns` → simulated cycles, floored at one.
///
/// Rounded UP, because a gate that specifies 9 ns must not answer in 8. The
/// floor is the module note's argument: a zero-cycle gate answers inside the
/// store that moved its input, which no part does.
fn tprop_cycles(tprop_ns: u64, cpu_hz: u64) -> u64 {
    let cpu_hz = cpu_hz.max(1);
    // ns × Hz / 1e9, rounded up, in u128 so a fast clock and a long delay
    // cannot overflow on the way.
    let numerator = u128::from(tprop_ns) * u128::from(cpu_hz);
    let cycles = numerator.div_ceil(1_000_000_000u128);
    u64::try_from(cycles).unwrap_or(u64::MAX).max(1)
}

impl BusResidentDevice for DeclarativeLogicDevice {
    /// One pass: sample every observed pad, recompute the function, arm the
    /// propagation deadline for anything that changed, then publish whatever
    /// deadline has come due.
    ///
    /// The three steps are in that order for the reason a gate exists: the
    /// answer to THIS pass's inputs is not published in THIS pass. What is
    /// published is the answer to an input change that happened `tprop` ago.
    fn service(&mut self, pins: &mut dyn DevicePins, now: u64) {
        for pad in &self.observed {
            let Some((addr, bit)) = pad.odr else { continue };
            // An address that does not read back means the MCU is driving
            // nothing there; `false` is the same default every other resident
            // model takes for an undriven output. For a gate this is the
            // honest answer twice over — an unconnected TTL input floats and
            // reads as whatever noise decides, and a model that guessed HIGH
            // would make an unwired NAND assert its output.
            let level = pins.output_bit(addr, bit).unwrap_or(false);
            self.levels.insert(pad.role.clone(), level);
        }

        let desired = self.compute();
        for (i, want) in desired.into_iter().enumerate() {
            if want != self.desired[i] {
                self.desired[i] = want;
                self.deadline[i] = Some(now.saturating_add(self.tprop_cycles));
            }
            let Some(due) = self.deadline[i] else {
                continue;
            };
            if now < due {
                continue;
            }
            self.deadline[i] = None;
            if self.applied[i] == self.desired[i] {
                continue;
            }
            self.applied[i] = self.desired[i];
            let Some(level) = self.desired[i] else {
                // Hi-Z: RELEASE the pad. Deliberately no write at all — the pad
                // keeps whatever level it last had, because this engine has one
                // level source per pad and no resistor network to decide what
                // an undriven net settles to. See `LogicDrive::hiz_when_disabled`.
                continue;
            };
            let Some((addr, bit)) = self.driven[i].idr else {
                continue;
            };
            // ⚠️ BOTH SEAMS, the same pair every resident model drives.
            // `drive_idr_bit` is an ordinary store to the input register, which
            // lands only where the model lets one land (STM32). On silicon
            // whose input word is READ-ONLY (EFR32, SAM, ESP32-C3) the store is
            // correctly dropped and the pad would never move — `drive_input_bit`
            // is the external-world seam those models sample. Driving only one
            // is how a part goes inert on half the catalog.
            let _ = pins.drive_input_bit(addr, bit, level);
            pins.drive_idr_bit(addr, bit, level);
        }
    }

    /// Every output register this part watches, so an MMIO write to one
    /// services it synchronously. See the module note for why a tick alone is
    /// not enough — and why the tick is still needed as well.
    fn edge_service_addrs(&self) -> &[u64] {
        &self.edge_addrs
    }

    fn as_sim_input(&mut self) -> &mut dyn SimInput {
        self
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl SimInput for DeclarativeLogicDevice {
    /// None. A gate's inputs are PADS — the host drives them by driving the
    /// MCU pin they are wired to, or by placing whatever else drives that net.
    /// Advertising a fake `a1` channel would put a stimulus in the palette that
    /// bypasses the wiring the part exists to model.
    fn input_channels(&self) -> &'static [InputChannel] {
        NO_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        unreachable!("a logic_gate declares no channels, so require_channel always rejects")
    }

    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }
}

/// Validate the static descriptor contract for the `logic_gate` primitive.
///
/// Separate from construction, like every sibling primitive, so manifest
/// preflight can reject an incomplete pack without resolving a single pad.
pub(crate) fn validate_descriptor(desc: &DeviceDescriptor) -> Result<()> {
    if desc.behavior.primitive != "logic_gate" {
        anyhow::bail!(
            "declarative logic kit requires behavior.primitive: logic_gate, got '{}'",
            desc.behavior.primitive
        );
    }
    let spec = desc.behavior.logic.as_ref().ok_or_else(|| {
        anyhow!(
            "logic_gate '{}' declares no `logic:` block — it would attach, drive nothing, \
             and look like a working part",
            desc.r#type
        )
    })?;
    spec.validate(&desc.r#type)?;
    anyhow::ensure!(
        desc.behavior.rules.is_empty(),
        "logic_gate '{}' declares `rules:`. A gate's behaviour is its truth table; a rule \
         list beside one is a second model of the same pads, and nothing says which wins",
        desc.r#type
    );
    Ok(())
}

/// The `config:` key a role binds its pad label to.
///
/// Default is the lowercased role plus `_pin` (`A1` → `a1_pin`), which is what
/// every in-tree descriptor uses and what keeps a 20-role transceiver from
/// needing a 20-line `pins:` block. `behavior.pins` / `behavior.output_pins`
/// override it for a part whose key is spelled differently.
pub fn config_key_for(desc: &DeviceDescriptor, role: &str) -> String {
    desc.behavior
        .pins
        .get(role)
        .or_else(|| desc.behavior.output_pins.get(role))
        .cloned()
        .unwrap_or_else(|| format!("{}_pin", role.to_lowercase()))
}

/// Which roles are OBSERVED (pads the MCU drives) and which are DRIVEN (pads
/// this part drives), for a spec.
///
/// The split is not simply inputs-vs-outputs: a transceiver role is both, and
/// which end is live depends on the DIR pad at runtime. Both ends are bound at
/// attach so the runtime never has to re-resolve a pad.
pub fn pad_roles(spec: &LogicSpec) -> (Vec<String>, Vec<String>) {
    let mut observed: Vec<String> = spec.inputs.clone();
    observed.extend(spec.control_pins());
    (observed, spec.outputs.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap as Map;

    /// A [`DevicePins`] over a flat word map, so a test can drive an "MCU
    /// output" and read back what the part drove without a whole bus.
    ///
    /// ODR and IDR are separate addresses here, exactly as they are on an
    /// STM32 port, so a test can tell "the part drove the pad" apart from "the
    /// MCU is still driving it".
    #[derive(Default)]
    struct FakePads {
        words: Map<u64, u32>,
        /// Every `drive_idr_bit`, so a released output can be proven to have
        /// cost NO write rather than merely to have landed on the same level.
        idr_writes: Vec<(u64, u8, bool)>,
    }

    impl FakePads {
        fn set(&mut self, addr: u64, bit: u8, high: bool) {
            let w = self.words.entry(addr).or_insert(0);
            if high {
                *w |= 1 << bit;
            } else {
                *w &= !(1 << bit);
            }
        }
        fn get(&self, addr: u64, bit: u8) -> Option<bool> {
            self.words.get(&addr).map(|w| (w >> bit) & 1 != 0)
        }
    }

    impl DevicePins for FakePads {
        fn output_bit(&self, addr: u64, bit: u8) -> Option<bool> {
            self.get(addr, bit)
        }
        fn drive_idr_bit(&mut self, addr: u64, bit: u8, high: bool) {
            self.idr_writes.push((addr, bit, high));
            self.set(addr, bit, high);
        }
        fn drive_input_bit(&mut self, _: u64, _: u8, _: bool) -> bool {
            false
        }
    }

    const ODR: u64 = 0x4001_0014;
    const IDR: u64 = 0x4001_0010;

    /// 1 GHz, so one nanosecond is exactly one cycle and `tprop_ns` reads as a
    /// cycle count in these tests. The conversion itself is pinned separately
    /// by [`tprop_rounds_up_and_never_reaches_zero`].
    const GHZ: u64 = 1_000_000_000;

    /// Build a device from descriptor YAML, binding role `R` to ODR bit i (for
    /// an observed role) and IDR bit i (for a driven one), in declaration order.
    fn device(yaml: &str, cpu_hz: u64) -> DeclarativeLogicDevice {
        let desc = DeviceDescriptor::from_yaml(yaml).expect("descriptor parses");
        validate_descriptor(&desc).expect("descriptor validates");
        let spec = desc.behavior.logic.as_ref().expect("logic block");
        let (obs_roles, drv_roles) = pad_roles(spec);
        let observed = obs_roles
            .iter()
            .enumerate()
            .map(|(i, role)| LogicPad {
                role: role.clone(),
                odr: Some((ODR, i as u8)),
                idr: None,
            })
            .collect();
        let driven = drv_roles
            .iter()
            .enumerate()
            .map(|(i, role)| LogicPad {
                role: role.clone(),
                odr: None,
                idr: Some((IDR, i as u8)),
            })
            .collect();
        DeclarativeLogicDevice::new("u1".into(), &desc, observed, driven, cpu_hz)
            .expect("constructs")
    }

    /// Bit index of an observed role, matching [`device`]'s binding.
    fn obs_bit(yaml: &str, role: &str) -> u8 {
        let desc = DeviceDescriptor::from_yaml(yaml).unwrap();
        let (obs, _) = pad_roles(desc.behavior.logic.as_ref().unwrap());
        obs.iter().position(|r| r == role).expect("observed role") as u8
    }

    fn drv_bit(yaml: &str, role: &str) -> u8 {
        let desc = DeviceDescriptor::from_yaml(yaml).unwrap();
        let (_, drv) = pad_roles(desc.behavior.logic.as_ref().unwrap());
        drv.iter().position(|r| r == role).expect("driven role") as u8
    }

    /// Service enough passes for any pending propagation to land.
    fn settle(dev: &mut DeclarativeLogicDevice, pads: &mut FakePads, from: u64) -> u64 {
        let step = dev.tprop_cycles() + 1;
        for c in from..=(from + step) {
            dev.service(pads, c);
        }
        from + step
    }

    const NAND: &str = r#"
type: t_nand
behavior:
  primitive: logic_gate
  logic:
    inputs: [A, B]
    outputs: [Y]
    table: { Y: "!(A & B)" }
    tprop_ns: 4
"#;

    #[test]
    fn a_nand_follows_its_truth_table_on_the_pads() {
        let a = obs_bit(NAND, "A");
        let b = obs_bit(NAND, "B");
        let y = drv_bit(NAND, "Y");
        for (ai, bi, want) in [
            (false, false, true),
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            let mut dev = device(NAND, GHZ);
            let mut pads = FakePads::default();
            pads.set(ODR, a, ai);
            pads.set(ODR, b, bi);
            settle(&mut dev, &mut pads, 0);
            assert_eq!(
                pads.get(IDR, y),
                Some(want),
                "NAND({ai},{bi}) must be {want}"
            );
        }
    }

    /// The whole reason this is not a `gpio_device` with rules: an output moves
    /// `tprop` AFTER its input, and NOT BEFORE.
    #[test]
    fn an_output_moves_after_tprop_and_not_before() {
        let a = obs_bit(NAND, "A");
        let b = obs_bit(NAND, "B");
        let y = drv_bit(NAND, "Y");
        let mut dev = device(NAND, GHZ);
        assert_eq!(dev.tprop_cycles(), 4, "4 ns at 1 GHz is 4 cycles");
        let mut pads = FakePads::default();

        // Both inputs LOW ⇒ Y HIGH. Let it settle so the step below is the
        // only thing in flight.
        let mut now = settle(&mut dev, &mut pads, 0);
        assert_eq!(pads.get(IDR, y), Some(true), "precondition: Y idles HIGH");

        // Raise both inputs at cycle `now`: Y must fall exactly 4 cycles later.
        pads.set(ODR, a, true);
        pads.set(ODR, b, true);
        dev.service(&mut pads, now); // the store that moves the pads
        for step in 1..4 {
            dev.service(&mut pads, now + step);
            assert_eq!(
                pads.get(IDR, y),
                Some(true),
                "Y moved after {step} cycle(s); tprop is 4 and a gate is not instant"
            );
        }
        dev.service(&mut pads, now + 4);
        assert_eq!(
            pads.get(IDR, y),
            Some(false),
            "Y must fall on the 4th cycle after the input change"
        );
        now += 4;
        let _ = now;
    }

    /// A NEGATIVE CONTROL for the table itself. The three tests above would all
    /// still pass if the engine ignored `table:` and drove some fixed function,
    /// as long as that function happened to agree. This drives the SAME harness
    /// with a deliberately wrong entry (AND where NAND belongs) and asserts the
    /// pad ends up at the opposite level — so the table is provably the thing
    /// being read.
    #[test]
    fn a_wrong_table_entry_flips_the_pad() {
        const AND: &str = r#"
type: t_and
behavior:
  primitive: logic_gate
  logic:
    inputs: [A, B]
    outputs: [Y]
    table: { Y: "A & B" }
    tprop_ns: 4
"#;
        let y = drv_bit(NAND, "Y");
        let a = obs_bit(NAND, "A");
        let b = obs_bit(NAND, "B");
        for (ai, bi) in [(false, false), (false, true), (true, false), (true, true)] {
            let mut right = device(NAND, GHZ);
            let mut wrong = device(AND, GHZ);
            let mut p1 = FakePads::default();
            let mut p2 = FakePads::default();
            for p in [&mut p1, &mut p2] {
                p.set(ODR, a, ai);
                p.set(ODR, b, bi);
            }
            settle(&mut right, &mut p1, 0);
            settle(&mut wrong, &mut p2, 0);
            assert_eq!(
                p1.get(IDR, y).map(|v| !v),
                p2.get(IDR, y),
                "NAND and AND must disagree on every input combination; they agreed at \
                 ({ai},{bi}), which means the pad is not following `table:` at all"
            );
        }
    }

    const BUF125: &str = r#"
type: t_125
behavior:
  primitive: logic_gate
  logic:
    inputs: [A1, A2]
    outputs: [Y1, Y2]
    enables:
      - { pin: OE1, active: low, outputs: [Y1] }
      - { pin: OE2, active: low, outputs: [Y2] }
    table: { Y1: "A1", Y2: "A2" }
    tprop_ns: 2
"#;

    #[test]
    fn a_disabled_output_goes_hi_z_and_stops_writing_the_pad() {
        let a1 = obs_bit(BUF125, "A1");
        let oe1 = obs_bit(BUF125, "OE1");
        let y1 = drv_bit(BUF125, "Y1");
        let mut dev = device(BUF125, GHZ);
        let mut pads = FakePads::default();

        // Enabled (OE1 low), A1 high ⇒ Y1 high.
        pads.set(ODR, oe1, false);
        pads.set(ODR, a1, true);
        let now = settle(&mut dev, &mut pads, 0);
        assert_eq!(pads.get(IDR, y1), Some(true), "enabled buffer follows A1");
        assert_eq!(dev.driving()[0], Some(true));

        // Disable: the part must stop DRIVING. The pad keeps the level it had
        // — this engine has one level source per pad and no resistor network —
        // so the proof is that no further write happens at all.
        pads.set(ODR, oe1, true);
        pads.idr_writes.clear();
        let now = settle(&mut dev, &mut pads, now + 1);
        assert_eq!(dev.driving()[0], None, "a disabled output must be released");

        // ⚠️ THE ANTI-VACUITY HALF. "No writes" is also what a part that had
        // stopped working entirely would produce, so move A1 while disabled and
        // assert the pad STILL does not move: a released output is deaf to its
        // input, which a merely-idle one is not.
        pads.set(ODR, a1, false);
        let now = settle(&mut dev, &mut pads, now + 1);
        assert!(
            pads.idr_writes.is_empty(),
            "a released output must not write its pad, even when its input moves: {:?}",
            pads.idr_writes
        );
        assert_eq!(
            pads.get(IDR, y1),
            Some(true),
            "the pad keeps the level it last had; nothing else drives this net"
        );

        // Re-enable: it must come back, at the CURRENT input level.
        pads.set(ODR, oe1, false);
        settle(&mut dev, &mut pads, now + 1);
        assert_eq!(
            pads.get(IDR, y1),
            Some(false),
            "re-enabling must publish the input level the part has now, not the one it \
             held when it was disabled"
        );
    }

    #[test]
    fn each_gate_has_its_own_enable() {
        let (a1, a2) = (obs_bit(BUF125, "A1"), obs_bit(BUF125, "A2"));
        let (oe1, oe2) = (obs_bit(BUF125, "OE1"), obs_bit(BUF125, "OE2"));
        let y1 = drv_bit(BUF125, "Y1");
        let mut dev = device(BUF125, GHZ);
        let mut pads = FakePads::default();
        pads.set(ODR, a1, true);
        pads.set(ODR, a2, true);
        pads.set(ODR, oe1, false); // gate 1 enabled
        pads.set(ODR, oe2, true); // gate 2 disabled
        settle(&mut dev, &mut pads, 0);
        assert_eq!(dev.driving()[0], Some(true), "gate 1 is enabled");
        assert_eq!(
            dev.driving()[1],
            None,
            "gate 2 must stay released — a single part-wide enable would switch both"
        );
        assert_eq!(pads.get(IDR, y1), Some(true));
        // Not `pads.get(IDR, y2) == None`: gate 1's write materialised the IDR
        // word, so bit y2 reads back as a 0 nobody wrote. The proof that gate 2
        // is released is that it performed no write at all.
        assert_eq!(
            pads.idr_writes,
            vec![(IDR, y1, true)],
            "only the enabled gate may touch a pad: {:?}",
            pads.idr_writes
        );
    }

    #[test]
    fn hiz_false_drives_a_disabled_output_low() {
        const PULLED: &str = r#"
type: t_pulled
behavior:
  primitive: logic_gate
  logic:
    inputs: [A]
    outputs: [Y]
    enables: [{ pin: OE, active: low, outputs: [Y] }]
    table: { Y: "A" }
    tprop_ns: 1
    drive: { hiz_when_disabled: false }
"#;
        let a = obs_bit(PULLED, "A");
        let oe = obs_bit(PULLED, "OE");
        let y = drv_bit(PULLED, "Y");
        let mut dev = device(PULLED, GHZ);
        let mut pads = FakePads::default();
        pads.set(ODR, a, true);
        pads.set(ODR, oe, false);
        let now = settle(&mut dev, &mut pads, 0);
        assert_eq!(pads.get(IDR, y), Some(true));
        pads.set(ODR, oe, true);
        settle(&mut dev, &mut pads, now + 1);
        assert_eq!(
            dev.driving()[0],
            Some(false),
            "with hiz_when_disabled: false a disabled output is PULLED low, not released"
        );
        assert_eq!(pads.get(IDR, y), Some(false));
    }

    const XCVR: &str = r#"
type: t_245
behavior:
  primitive: logic_gate
  logic:
    inputs: [A1, B1]
    outputs: [A1, B1]
    enables: [{ pin: OE, active: low, outputs: [A1, B1] }]
    direction: { pin: DIR, a_to_b_when: 1, a: [A1], b: [B1] }
    tprop_ns: 3
"#;

    /// A transceiver pad is read one way round and driven the other, so this
    /// harness binds each side's ODR and IDR independently — which is exactly
    /// what attach does.
    fn transceiver() -> (DeclarativeLogicDevice, FakePads) {
        let desc = DeviceDescriptor::from_yaml(XCVR).unwrap();
        validate_descriptor(&desc).unwrap();
        let observed = vec![
            LogicPad {
                role: "A1".into(),
                odr: Some((ODR, 0)),
                idr: None,
            },
            LogicPad {
                role: "B1".into(),
                odr: Some((ODR, 1)),
                idr: None,
            },
            LogicPad {
                role: "OE".into(),
                odr: Some((ODR, 2)),
                idr: None,
            },
            LogicPad {
                role: "DIR".into(),
                odr: Some((ODR, 3)),
                idr: None,
            },
        ];
        let driven = vec![
            LogicPad {
                role: "A1".into(),
                odr: None,
                idr: Some((IDR, 0)),
            },
            LogicPad {
                role: "B1".into(),
                odr: None,
                idr: Some((IDR, 1)),
            },
        ];
        (
            DeclarativeLogicDevice::new("u2".into(), &desc, observed, driven, GHZ).unwrap(),
            FakePads::default(),
        )
    }

    #[test]
    fn a_transceiver_flips_which_side_it_drives_with_dir() {
        let (mut dev, mut pads) = transceiver();
        pads.set(ODR, 2, false); // OE low = enabled
        pads.set(ODR, 3, true); // DIR high = A to B
        pads.set(ODR, 0, true); // the MCU drives A1 high
        pads.set(ODR, 1, false); // and B1's own output register is low
        let now = settle(&mut dev, &mut pads, 0);
        assert_eq!(
            (dev.driving()[0], dev.driving()[1]),
            (None, Some(true)),
            "DIR high must drive the B side from A and leave A alone — driving A too \
             would have the part fighting the MCU for the pad it is reading"
        );
        assert_eq!(pads.get(IDR, 1), Some(true), "B1 followed A1");

        // Flip DIR. Now B is the input and A is driven, and the levels are the
        // other way round, so the answer changes as well as the direction.
        pads.set(ODR, 3, false);
        let now = settle(&mut dev, &mut pads, now + 1);
        assert_eq!(
            (dev.driving()[0], dev.driving()[1]),
            (Some(false), None),
            "DIR low must drive the A side from B"
        );
        assert_eq!(pads.get(IDR, 0), Some(false), "A1 followed B1's LOW");

        // OE high isolates BOTH buses.
        pads.set(ODR, 2, true);
        settle(&mut dev, &mut pads, now + 1);
        assert_eq!(
            (dev.driving()[0], dev.driving()[1]),
            (None, None),
            "OE high must take both sides off the wire"
        );
    }

    #[test]
    fn a_bus_switch_follows_its_select_pad() {
        const SW: &str = r#"
type: t_3257
behavior:
  primitive: logic_gate
  logic:
    inputs: [A1, B1]
    outputs: [Y1]
    select: { pin: S, low: [A1], high: [B1] }
    tprop_ns: 1
"#;
        let a1 = obs_bit(SW, "A1");
        let b1 = obs_bit(SW, "B1");
        let s = obs_bit(SW, "S");
        let y1 = drv_bit(SW, "Y1");
        let mut dev = device(SW, GHZ);
        let mut pads = FakePads::default();
        pads.set(ODR, a1, true);
        pads.set(ODR, b1, false);

        pads.set(ODR, s, false);
        let now = settle(&mut dev, &mut pads, 0);
        assert_eq!(pads.get(IDR, y1), Some(true), "S low selects the A side");

        pads.set(ODR, s, true);
        settle(&mut dev, &mut pads, now + 1);
        assert_eq!(pads.get(IDR, y1), Some(false), "S high selects the B side");
    }

    #[test]
    fn tprop_rounds_up_and_never_reaches_zero() {
        // 9 ns at 80 MHz is 0.72 cycles — a gate that answered in the same
        // store is not a gate, so the floor is one.
        assert_eq!(super::tprop_cycles(9, 80_000_000), 1);
        // 9 ns at 1 GHz is exactly 9.
        assert_eq!(super::tprop_cycles(9, 1_000_000_000), 9);
        // 11 ns at 480 MHz is 5.28 — rounded UP, because a part specifying
        // 11 ns must not answer in 10.4.
        assert_eq!(super::tprop_cycles(11, 480_000_000), 6);
        // A part that states no delay still gets one cycle.
        assert_eq!(super::tprop_cycles(0, 80_000_000), 1);
        // A clock this model never sees must not divide by zero.
        assert_eq!(super::tprop_cycles(5, 0), 1);
    }

    #[test]
    fn an_input_port_is_named_for_synchronous_service() {
        let dev = device(NAND, GHZ);
        assert_eq!(
            dev.edge_service_addrs(),
            &[ODR],
            "a gate must be serviced from the MMIO write hook — an input that moves and \
             moves back inside one tick interval would otherwise never be seen"
        );
    }

    #[test]
    fn a_logic_gate_exposes_no_stimulus_channels() {
        let mut dev = device(NAND, GHZ);
        assert!(dev.input_channels().is_empty());
        let err = dev.set_input("a", 1.0).unwrap_err();
        assert!(
            matches!(err, SimInputError::UnknownChannel(_)),
            "a gate's inputs are pads, not host channels: {err:?}"
        );
    }

    #[test]
    fn rules_beside_a_truth_table_are_refused() {
        let desc = DeviceDescriptor::from_yaml(
            r#"
type: t
behavior:
  primitive: logic_gate
  logic: { inputs: [A], outputs: [Y], table: { Y: "A" } }
  pins: { A: a_pin }
  rules:
    - on: { pin: A, edge: rising }
      do: [ { var: { name: n, value: 1 } } ]
"#,
        )
        .unwrap();
        let err = format!("{:#}", validate_descriptor(&desc).unwrap_err());
        assert!(err.contains("second model"), "{err}");
    }

    #[test]
    fn a_descriptor_with_no_logic_block_is_refused() {
        let desc =
            DeviceDescriptor::from_yaml("type: t\nbehavior:\n  primitive: logic_gate\n").unwrap();
        let err = format!("{:#}", validate_descriptor(&desc).unwrap_err());
        assert!(err.contains("`logic:`"), "{err}");
    }

    /// Every shipping 74-series descriptor must construct, not merely parse.
    #[test]
    fn every_shipping_logic_descriptor_builds() {
        let types = [
            "74hc04",
            "74hc00",
            "74hc08",
            "74hc32",
            "74hc125",
            "74lvc1t45",
            "74hc245",
            "74cbtlv3257",
        ];
        for ty in types {
            let yaml = labwired_config::embedded_device_yaml(ty)
                .unwrap_or_else(|| panic!("{ty} is embedded"));
            let desc = DeviceDescriptor::from_yaml(yaml)
                .unwrap_or_else(|e| panic!("{ty}.yaml must parse: {e:#}"));
            validate_descriptor(&desc).unwrap_or_else(|e| panic!("{ty}.yaml must validate: {e:#}"));
            let spec = desc.behavior.logic.as_ref().unwrap();
            let (obs, drv) = pad_roles(spec);
            let observed = obs
                .iter()
                .enumerate()
                .map(|(i, r)| LogicPad {
                    role: r.clone(),
                    odr: Some((ODR, i as u8)),
                    idr: None,
                })
                .collect();
            let driven = drv
                .iter()
                .enumerate()
                .map(|(i, r)| LogicPad {
                    role: r.clone(),
                    odr: None,
                    idr: Some((IDR, i as u8)),
                })
                .collect();
            let dev = DeclarativeLogicDevice::new(ty.into(), &desc, observed, driven, 80_000_000)
                .unwrap_or_else(|e| panic!("{ty} must construct: {e:#}"));
            assert!(dev.tprop_cycles() >= 1, "{ty} has a zero-cycle delay");
        }
    }
}
