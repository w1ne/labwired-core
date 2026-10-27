// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Parser for the SPICE subset the in-core analog engine solves.
//!
//! The subset is deliberately small, because everything in it has to stay
//! deterministic in the browser with no dependencies. Anything outside it
//! (subcircuits, model libraries, AC/DC sweeps, device capacitances) is not
//! approximated and not silently dropped: it is a hard error that names the
//! line and points at the ngspice adapter, which does support it.
//!
//! ## Elements
//!
//! | Element | Line |
//! |---|---|
//! | Resistor | `R<name> n1 n2 <value>` |
//! | Capacitor | `C<name> n1 n2 <value> [ic=<v>]` |
//! | Inductor | `L<name> n1 n2 <value> [ic=<i>]` |
//! | Voltage source | `V<name> n+ n- <source>` |
//! | Current source | `I<name> n+ n- <source>` |
//! | Switch | `S<name> n1 n2 <ctrl> ron=<r> roff=<r>` or `S<name> n1 n2 nc+ nc- vt=<v> [vh=<v>] [invert=0\|1] ron=<r> roff=<r>` |
//! | Voltage switch | `S<name> n1 n2 nc+ nc- vt=<v> [vh=<v>] [invert=0|1] ron=<r> roff=<r>` |
//! | Diode | `D<name> n+ n- <model>` |
//! | BJT | `Q<name> nc nb ne <model>` |
//! | MOSFET | `M<name> nd ng ns nb <model> [w=<m>] [l=<m>]` |
//! | VCVS | `E<name> n+ n- nc+ nc- <gain>` |
//! | VCCS | `G<name> n+ n- nc+ nc- <transconductance>` |
//! | CCCS | `F<name> n+ n- <vsource> <gain>` |
//! | CCVS | `H<name> n+ n- <vsource> <transresistance>` |
//! | Op-amp / comparator | `X<name> in+ in- v+ v- out <model>` |
//!
//! The four dependent sources are SPICE's linear ones and nothing more: `POLY`,
//! `VALUE={...}` and `TABLE` forms are refused by name. `E` and `H` each add a
//! branch current, numbered after every voltage source and inductor; `F` and
//! `H` read the branch current of the element they name, which must be one that
//! carries one (a `V`, `L`, `E` or `H`).
//!
//! `X` is SPICE's subcircuit call, and the in-core engine has no subcircuits.
//! It accepts an `X` line only when its model names an `OPAMP` or `COMP` card
//! (built in, or declared with `.model`); the pin order is the one vendor
//! op-amp and comparator models use (`IN+ IN- VCC VEE OUT`). Any other `X` line
//! is a real subcircuit call and is refused with the pointer to ngspice. The
//! element is lowered, at parse time, into the primitives above; see
//! [`OpAmpModel`] and [`ComparatorModel`] for exactly what it becomes.
//!
//! `<source>` is `[dc] <value>`, `SIN(vo va freq [td [theta]])` or
//! `PULSE(v1 v2 [td [tr [tf [pw [per]]]]])`, spelled as in SPICE. A source that
//! carries a function is driven by the clock, so it cannot also be a routed
//! input — [`super::adapter`] rejects a manifest that wires one.
//!
//! ## Models
//!
//! `.model <name> D|NPN|PNP|NMOS|PMOS (<param>=<value> ...)` declares a model
//! card; parameters may also be written without the parentheses, and a card may
//! be declared after the elements that use it. Parameters this engine does not
//! model (every capacitance, every temperature coefficient, `VAF`, `IKF`,
//! `GAMMA`, …) are **accepted and ignored** rather than rejected, so a vendor
//! model pasted from a datasheet still runs — with the large-signal DC
//! behaviour it describes and none of its charge storage. See
//! [`super::device`] for what that costs.
//!
//! An element may also name one of five built-in cards and skip `.model`
//! entirely: `D` (1N4148-class, `IS=2.52n N=1.752`), `NPN`, `PNP` (β = 100),
//! `NMOS`, `PMOS` (`VTO=±1 V`, `KP=20u`, `LAMBDA=0.02`). Those are *this
//! engine's* convenience values and are not ngspice's parameter defaults; a
//! parameter omitted from a real `.model` line gets ngspice's default, so a
//! deck written out in full means the same thing to both engines.
//!
//! Five more built-in cards name part families, for the same reason: an
//! imported board says `1N4733A` or `LM358`, not a parameter list. They are
//! `1N4733A` (5.1 V zener), `SMAJ5.0A` (5 V TVS), `IDEAL_OPAMP`, `LM358` and
//! `LM393`; [`builtin_model`] gives their values and where each came from.
//! They are *-like* cards — the datasheet's headline numbers, not the vendor's
//! macro model.
//!
//! A diode card may carry `BV` (reverse breakdown voltage), `IBV` (the current
//! at `BV`, default 1 mA) and `NBV` (breakdown emission coefficient, default
//! `N`); with `BV` set the diode is solved with ngspice's three-region level-1
//! equation, and with it unset the diode is exactly what it was before
//! breakdown existed. `.model <name> OPAMP(...)` and `.model <name> COMP(...)`
//! are this engine's own card types, not ngspice's.
//!
//! ## Directives and comments
//!
//! Plus `*` comments, `;`/`$` trailing comments, `.end`, `.ic V(node)=<v>` and
//! `.options`/`.print`/`.tran`/`.save`/`.control`…`.endc`, which are accepted
//! and ignored so that one deck can be handed to both this engine and ngspice.
//!
//! Note on the first line: a classic SPICE deck treats line 1 as a title and
//! ignores it. This parser does not, because silently dropping an element line
//! is the worst failure mode a netlist parser has. Put a `*` on your comment.

use std::collections::BTreeMap;
use std::fmt;

/// A node reference: `None` is ground, `Some(i)` indexes [`Circuit::node_name`].
pub type NodeRef = Option<usize>;

/// Everything that can go wrong between a netlist string and a solved step.
///
/// Every parse variant carries the 1-based line number and the line's text, so
/// a manifest can report which line of which file the user has to fix.
#[derive(Debug, Clone, PartialEq)]
pub enum AnalogError {
    /// A line is in the subset's vocabulary but malformed.
    Parse {
        /// 1-based line number in the netlist.
        line: usize,
        /// The offending line, trimmed.
        text: String,
        /// What was wrong with it.
        message: String,
    },
    /// A line names an element this engine does not model at all.
    Unsupported {
        /// 1-based line number in the netlist.
        line: usize,
        /// The offending line, trimmed.
        text: String,
    },
    /// The circuit is bigger than the in-core dense solver accepts.
    TooLarge {
        /// Nodes + branch currents the circuit needs.
        unknowns: usize,
        /// The ceiling.
        max: usize,
    },
    /// The MNA matrix has no unique solution (a floating node, a shorted
    /// voltage source, a zero-valued resistor).
    Singular {
        /// The unknown at which elimination found no pivot.
        row: usize,
    },
    /// Newton–Raphson ran out of iterations on one step of a nonlinear
    /// circuit.
    ///
    /// This is the variant that exists so a hard circuit reports *where* it
    /// gave up instead of quietly filling the trace with `NaN`: the caller
    /// gets the step, the simulated time, and the unknown that was still
    /// moving when the budget ran out.
    NoConvergence {
        /// 0-based internal step index since the solver was built, or `None`
        /// for the DC operating point.
        step: Option<u64>,
        /// Simulated time at the end of the step, in seconds.
        time: f64,
        /// Iterations attempted before giving up.
        iterations: u32,
        /// Name of the unknown with the largest residual movement.
        unknown: String,
        /// How much that unknown still moved on the last iteration.
        delta: f64,
    },
    /// The manifest `config` block is not a usable analog configuration.
    Config(String),
}

impl AnalogError {
    /// 1-based netlist line this error is about, when it is about a line.
    pub fn line(&self) -> Option<usize> {
        match self {
            Self::Parse { line, .. } | Self::Unsupported { line, .. } => Some(*line),
            _ => None,
        }
    }

    /// The netlist line's text, when this error is about a line.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Parse { text, .. } | Self::Unsupported { text, .. } => Some(text),
            _ => None,
        }
    }
}

impl fmt::Display for AnalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse {
                line,
                text,
                message,
            } => write!(f, "netlist line {line}: {message} (in `{text}`)"),
            // The exact wording the design calls for: it has to tell the user
            // which adapter does support the line they wrote.
            Self::Unsupported { text, .. } => write!(
                f,
                "element `{text}` needs ngspice; use `adapter: external_process` \
                 with `tools/cosim/labwired_ngspice.py`"
            ),
            Self::TooLarge { unknowns, max } => write!(
                f,
                "circuit needs {unknowns} unknowns (nodes + branch currents), over the \
                 in-core limit of {max}; use `adapter: external_process` with \
                 `tools/cosim/labwired_ngspice.py`"
            ),
            Self::Singular { row } => write!(
                f,
                "circuit matrix is singular at unknown {row}: check for a floating node, \
                 a zero-ohm resistor or two voltage sources in parallel"
            ),
            Self::NoConvergence {
                step,
                time,
                iterations,
                unknown,
                delta,
            } => {
                let where_ = match step {
                    Some(step) => format!("step {step} (t = {time:e} s)"),
                    None => "the DC operating point".to_string(),
                };
                write!(
                    f,
                    "Newton iteration did not converge at {where_} after {iterations} \
                     iterations; `{unknown}` was still moving by {delta:e} per iteration. \
                     Try a smaller `substeps` interval, or use `adapter: external_process` \
                     with `tools/cosim/labwired_ngspice.py`"
                )
            }
            Self::Config(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for AnalogError {}

/// `R<name> n1 n2 <ohms>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Resistor {
    /// Element name as written, e.g. `R1`.
    pub name: String,
    /// First terminal.
    pub a: NodeRef,
    /// Second terminal.
    pub b: NodeRef,
    /// Resistance in ohms.
    pub ohms: f64,
}

/// `C<name> n1 n2 <farads> [ic=<volts>]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Capacitor {
    /// Element name as written.
    pub name: String,
    /// Positive terminal (the one `ic=` is measured at, relative to `b`).
    pub a: NodeRef,
    /// Negative terminal.
    pub b: NodeRef,
    /// Capacitance in farads.
    pub farads: f64,
    /// Initial voltage across the element, overriding the operating point.
    pub ic: Option<f64>,
}

/// `L<name> n1 n2 <henries> [ic=<amps>]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Inductor {
    /// Element name as written.
    pub name: String,
    /// Terminal the branch current flows into.
    pub a: NodeRef,
    /// Terminal the branch current flows out of.
    pub b: NodeRef,
    /// Inductance in henries.
    pub henries: f64,
    /// Initial current `a` → `b`, overriding the operating point.
    pub ic: Option<f64>,
}

/// How an independent source's value depends on time.
///
/// The operating point is solved at `t = 0` with [`Waveform::at`] evaluated
/// there, which is what SPICE's `.tran` does when it is not given `uic`, so a
/// deck starts from the same state in both engines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Waveform {
    /// A constant. This is the only kind a routed input may drive.
    Dc(f64),
    /// `SIN(vo va freq [td [theta]])`:
    /// `vo + va·exp(−(t−td)·theta)·sin(2π·freq·(t−td))` after `td`, `vo`
    /// before it.
    Sin {
        /// Offset, volts or amps.
        offset: f64,
        /// Peak amplitude.
        amplitude: f64,
        /// Frequency in hertz.
        frequency: f64,
        /// Delay before the sine starts, seconds.
        delay: f64,
        /// Exponential damping factor, 1/s.
        theta: f64,
    },
    /// `PULSE(v1 v2 [td [tr [tf [pw [per]]]]])` — a trapezoidal pulse train.
    Pulse {
        /// Initial (and resting) value.
        v1: f64,
        /// Pulsed value.
        v2: f64,
        /// Delay before the first edge, seconds.
        delay: f64,
        /// Rise time, seconds.
        rise: f64,
        /// Fall time, seconds.
        fall: f64,
        /// Time held at `v2`, seconds, not counting the edges.
        width: f64,
        /// Period, seconds.
        period: f64,
    },
}

impl Waveform {
    /// Value at `t` seconds.
    ///
    /// Deterministic: `sin`, `exp` and `floor` come from `libm` rather than
    /// from the platform's libm, for the reason [`super::device`] gives.
    pub fn at(&self, t: f64) -> f64 {
        match *self {
            Self::Dc(value) => value,
            Self::Sin {
                offset,
                amplitude,
                frequency,
                delay,
                theta,
            } => {
                if t <= delay {
                    offset
                } else {
                    let elapsed = t - delay;
                    let envelope = if theta == 0.0 {
                        1.0
                    } else {
                        libm::exp(-elapsed * theta)
                    };
                    offset
                        + amplitude
                            * envelope
                            * libm::sin(2.0 * core::f64::consts::PI * frequency * elapsed)
                }
            }
            Self::Pulse {
                v1,
                v2,
                delay,
                rise,
                fall,
                width,
                period,
            } => {
                if t < delay {
                    return v1;
                }
                let mut phase = t - delay;
                if period > 0.0 && phase >= period {
                    phase -= libm::floor(phase / period) * period;
                }
                if rise > 0.0 && phase < rise {
                    v1 + (v2 - v1) * (phase / rise)
                } else if phase < rise + width {
                    v2
                } else if fall > 0.0 && phase < rise + width + fall {
                    v2 + (v1 - v2) * ((phase - rise - width) / fall)
                } else {
                    v1
                }
            }
        }
    }

    /// True when a routed input may drive this source.
    pub fn is_constant(&self) -> bool {
        matches!(self, Self::Dc(_))
    }
}

/// `V<name> n+ n- <source>`.
#[derive(Debug, Clone, PartialEq)]
pub struct VoltageSource {
    /// Element name as written, e.g. `Vgpio`.
    pub name: String,
    /// Positive terminal.
    pub p: NodeRef,
    /// Negative terminal.
    pub n: NodeRef,
    /// Value at `t = 0`; routed inputs replace it once time runs.
    pub dc: f64,
    /// Time dependence. `Dc` sources are the ones a routed input may drive.
    pub wave: Waveform,
}

/// `I<name> n+ n- <source>`. Positive current flows from `p` through the
/// source to `n`, as in SPICE.
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentSource {
    /// Element name as written.
    pub name: String,
    /// Positive terminal.
    pub p: NodeRef,
    /// Negative terminal.
    pub n: NodeRef,
    /// Value at `t = 0`; routed inputs replace it once time runs.
    pub dc: f64,
    /// Time dependence. `Dc` sources are the ones a routed input may drive.
    pub wave: Waveform,
}

/// Which way round a three-terminal device's junctions face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// NPN or N-channel: the device conducts for positive controlling
    /// voltages, and its sign factor is `+1`.
    N,
    /// PNP or P-channel: every terminal voltage and current is mirrored, and
    /// its sign factor is `−1`.
    P,
}

impl Polarity {
    /// `+1.0` for [`Polarity::N`], `−1.0` for [`Polarity::P`] — the factor
    /// that turns circuit coordinates into device coordinates.
    pub fn sign(self) -> f64 {
        match self {
            Self::N => 1.0,
            Self::P => -1.0,
        }
    }
}

/// Resolved parameters of a `.model <name> D(...)` card.
///
/// Defaults are ngspice's, so a `.model` line that omits a parameter means the
/// same thing to both engines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiodeModel {
    /// Saturation current `IS`, amps.
    pub is: f64,
    /// Emission coefficient `N`.
    pub n: f64,
    /// Ohmic series resistance `RS`. Non-zero adds one internal node.
    pub rs: f64,
    /// Reverse breakdown voltage `BV`, volts, positive. `None` is a diode
    /// with no breakdown at all, solved exactly as before `BV` existed.
    pub bv: Option<f64>,
    /// Current at the breakdown knee `IBV`, amps. ngspice-47's default, 1 mA.
    pub ibv: f64,
    /// Breakdown emission coefficient `NBV`. `None` means "same as `N`",
    /// which is ngspice's default.
    pub nbv: Option<f64>,
}

impl Default for DiodeModel {
    fn default() -> Self {
        Self {
            is: 1e-14,
            n: 1.0,
            rs: 0.0,
            bv: None,
            ibv: 1e-3,
            nbv: None,
        }
    }
}

/// How a comparator's output stage drives its pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparatorOutput {
    /// Open collector / open drain: pulls `out` down to `v-` through `ROUT`
    /// when `in+ < in-`, and lets it float otherwise. Needs a pull-up. This is
    /// the LM393/LM339 output.
    OpenCollector,
    /// Push-pull: drives `out` to either rail through `ROUT`.
    PushPull,
}

/// Resolved parameters of a `.model <name> OPAMP(...)` card.
///
/// The element is a voltage-feedback op-amp macro, lowered into primitives:
///
/// ```text
///   gain node p:  G  = AOL/Rg · (v(in+) − v(in−) + VOS)  into p
///                 Rg = 1 MΩ   and   Cg = AOL/(2π·GBW·Rg)   from p to ground
///   clamps:       D  p → (v+ − DROP_HI)      D  (v− + DROP_LO) → p
///   output:       Thevenin v(p) behind ROUT, i.e.  G = 1/ROUT into out,
///                 R = ROUT from out to ground
/// ```
///
/// So the DC gain is `AOL`, the single pole sits at `GBW/AOL`, the unity-gain
/// frequency is `GBW`, and the output cannot leave `[v− + DROP_LO, v+ −
/// DROP_HI]` by more than one clamp-diode drop (a few mV — the clamp diode's
/// emission coefficient is 0.02, so its knee is 50× sharper than silicon's).
/// The clamps are real diodes solved with SPICE's `pnjlim`, which is what lets
/// Newton walk a saturated output back into the linear region. The rails come
/// from the `v+`/`v−` pins, so the same card works at ±15 V and at 3.3 V.
///
/// What it does not model: input bias current, input capacitance, slew-rate
/// limiting beyond what the pole gives, output current limiting, and supply
/// current — the output stage's current returns through ground, not through
/// the supply pins. Only the clamp current reaches `v+`/`v−`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpAmpModel {
    /// DC open-loop gain `AOL`, V/V.
    pub aol: f64,
    /// Gain-bandwidth product `GBW`, Hz.
    pub gbw: f64,
    /// Output resistance `ROUT`, ohms.
    pub rout: f64,
    /// Input offset voltage `VOS`, volts, added to `v(in+) − v(in−)`.
    pub vos: f64,
    /// `DROP_HI`: how far below `v+` the output saturates, volts.
    pub drop_hi: f64,
    /// `DROP_LO`: how far above `v−` the output saturates, volts.
    pub drop_lo: f64,
}

impl Default for OpAmpModel {
    fn default() -> Self {
        Self {
            aol: 1e5,
            gbw: 1e6,
            rout: 100.0,
            vos: 0.0,
            drop_hi: 0.0,
            drop_lo: 0.0,
        }
    }
}

/// Resolved parameters of a `.model <name> COMP(...)` card.
///
/// A comparator is an op-amp with no compensation, a decision node of fixed
/// 1 V span, and an output stage:
///
/// ```text
///   decision node q (referenced to v−):
///       G  = AOL/Rq · (v(in−) − v(in+) − VOS + VHYS·(v(q) − v(v−) − ½))  into q
///       Rq = 1 MΩ,  Cq = AOL/(2π·GBW·Rq),  diode-clamped to [v−, v− + 1 V]
///   OUTPUT=OC:  an N-channel switch from out to v−, gate q, VTO = 0.5 V,
///               on-resistance ROUT at full drive
///   OUTPUT=PP:  a second node p = −10⁴·(v(q) − v(v−) − ½), clamped to the
///               rails, driving out through ROUT like the op-amp's output
/// ```
///
/// `VHYS` is internal positive feedback: the input thresholds sit at
/// `−VOS ± VHYS/2`, a window `VHYS` volts wide, independent of the supply.
/// `GBW` here sets the response time: the decision node slews at
/// `2π·GBW·overdrive` volts per second across its 1 V span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComparatorModel {
    /// DC gain `AOL`, V/V.
    pub aol: f64,
    /// Gain-bandwidth product `GBW`, Hz.
    pub gbw: f64,
    /// Output on-resistance (open collector) or output resistance (push-pull),
    /// ohms.
    pub rout: f64,
    /// Input offset voltage `VOS`, volts.
    pub vos: f64,
    /// Hysteresis window `VHYS`, volts. Zero is none.
    pub vhys: f64,
    /// `OUTPUT=OC` (default) or `OUTPUT=PP`.
    pub output: ComparatorOutput,
}

impl Default for ComparatorModel {
    fn default() -> Self {
        Self {
            aol: 2e5,
            gbw: 10e6,
            rout: 60.0,
            vos: 0.0,
            vhys: 0.0,
            output: ComparatorOutput::OpenCollector,
        }
    }
}

/// Which regulator a `.model <name> LDO|LDOADJ|BUCK(...)` card describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegulatorKind {
    /// `LDO`: a fixed linear regulator, pins `in out gnd`. The output is
    /// referenced to the `gnd` pin.
    Linear,
    /// `LDOADJ`: an adjustable three-terminal linear regulator (LM317), pins
    /// `in out adj`. It holds `v(out) − v(adj)` at `VREF` and drives `IADJ`
    /// out of the `adj` pin; the external divider sets the output.
    LinearAdjustable,
    /// `BUCK`: an averaged step-down converter, pins `in out gnd` (fixed,
    /// `VOUT` set) or `in out gnd fb` (adjustable, `VREF` set).
    Buck,
}

/// Resolved parameters of a regulator card: `LDO`, `LDOADJ` or `BUCK`.
///
/// Every regulator lowers into the same core, written about a reference pin
/// `ref` (the `gnd` pin, or the `adj` pin of an `LDOADJ`) and one internal
/// set-point node `a` that holds the output the regulator is trying to make,
/// `v(out) − v(ref)`:
///
/// ```text
///   set point a:  Norton drive to VSET through Ra = 1 MΩ
///                 (VSET = VOUT, or VREF for LDOADJ; plus LINE·(v(in) − v(ref)
///                 − VSET − VDO) of line regulation). An adjustable BUCK
///                 instead drives a with an error amplifier:
///                 AOL/Ra · (VREF − (v(fb) − v(ref))).
///   dropout:      D  a → hd,  hd = max(DMAX·(v(in) − v(ref)) − VDO, 0)
///                 (the max: 1 kΩ and a clamp to ground, buffered by an E)
///   current limit D  a → a + K·(ILIM − I(out)),  K = 100 V/A
///   output:       E  oi = v(ref) + v(a),  then ROUT from oi to out
///   input:        LDO/LDOADJ: F draws the output current from in into ref
///                 (a pass element: I(in) = I(out));
///                 BUCK: a power-balance sink draws
///                 (v(out) − v(ref)) · I(out) / (EFF · max(v(in) − v(ref), 0.1 V))
///                 from in into ref.
///   quiescent:    IQ from in to ref (IADJ from in out of the adj pin), scaled
///                 by the set point, so it is IQ while regulating and zero with
///                 no input.
/// ```
///
/// So `v(out) = min(VSET, DMAX·v(in) − VDO) − ROUT·I(out)` while the output
/// current is under `ILIM`, and the current is held at `ILIM` past it. The
/// dropout clamps are the op-amp macro's clamp diode ([`MACRO_CLAMP_DIODE`]),
/// the current-limit clamp a softer one ([`REGULATOR_LIMIT_DIODE`]): in
/// dropout the output sits one clamp drop (≈ 8 mV at the set-point node's
/// 3 µA) above `v(in) − VDO`.
///
/// What it does not model: dropout that grows with load current, ripple and
/// switching (the buck is averaged: no inductor current, no loop dynamics),
/// the LM317's minimum load current, thermal shutdown, and reverse current
/// from the output into the input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegulatorModel {
    /// Topology.
    pub kind: RegulatorKind,
    /// Fixed output voltage `VOUT`, volts. Zero on an adjustable part.
    pub vout: f64,
    /// Reference `VREF`, volts: `v(out) − v(adj)` of an `LDOADJ`, `v(fb)` of
    /// an adjustable `BUCK`.
    pub vref: f64,
    /// Dropout / headroom `VDO`, volts: the least `v(in) − v(out)` (for a
    /// buck, `DMAX·v(in) − v(out)`) the part regulates at.
    pub vdo: f64,
    /// Maximum duty cycle `DMAX` of a buck, 0..=1. 1 for a linear part.
    pub dmax: f64,
    /// Output current limit `ILIM`, amps.
    pub ilim: f64,
    /// Quiescent current `IQ` (`IADJ` on an `LDOADJ`), amps.
    pub iq: f64,
    /// Output resistance `ROUT`, ohms: the load regulation, ΔV/ΔI.
    pub rout: f64,
    /// Line regulation `LINE`, V/V: ΔVout/ΔVin while regulating.
    pub line: f64,
    /// Conversion efficiency `EFF` of a buck, 0..=1.
    pub eff: f64,
    /// Error-amplifier gain `AOL` of an adjustable buck, V/V.
    pub aol: f64,
}

impl RegulatorModel {
    /// The card's defaults for `kind`: a working part, not a datasheet.
    pub fn defaults(kind: RegulatorKind) -> Self {
        let common = Self {
            kind,
            vout: 0.0,
            vref: 0.0,
            vdo: 0.0,
            dmax: 1.0,
            ilim: 1.0,
            iq: 0.0,
            rout: 0.01,
            line: 0.0,
            eff: 1.0,
            aol: 1e5,
        };
        match kind {
            RegulatorKind::Linear => Self {
                vout: 3.3,
                vdo: 0.3,
                ..common
            },
            RegulatorKind::LinearAdjustable => Self {
                vref: 1.25,
                vdo: 1.5,
                ilim: 1.5,
                iq: 50e-6,
                ..common
            },
            RegulatorKind::Buck => Self {
                eff: 0.9,
                ilim: 3.0,
                ..common
            },
        }
    }

    /// True for a buck whose output is set by a divider into `fb`.
    pub fn is_adjustable_buck(&self) -> bool {
        self.kind == RegulatorKind::Buck && self.vref > 0.0
    }

    /// The set point a fixed part (or an `LDOADJ`) holds `v(out) − v(ref)` at.
    pub fn setpoint(&self) -> f64 {
        match self.kind {
            RegulatorKind::LinearAdjustable => self.vref,
            RegulatorKind::Linear | RegulatorKind::Buck => self.vout,
        }
    }
}

/// Gain of a regulator's current-limit sense, volts per amp.
///
/// The limit clamp compares `K·(ILIM − I(out))` against the clamp diode's
/// knee, so it conducts a few milliamps past `ILIM` at 100 V/A (see
/// [`REGULATOR_LIMIT_DIODE`]). Sensing the output voltage across `ROUT`
/// instead would put the knee at `(clamp drop)/ROUT`, which on a 10 mΩ part
/// is amperes.
pub const REGULATOR_LIMIT_GAIN: f64 = 100.0;

/// The current-limit clamp's diode: a plain `IS=1e-12 N=1` junction.
///
/// Softer than [`MACRO_CLAMP_DIODE`] on purpose. The limit loop runs through
/// the output current, whose slope against the set point is `K/ROUT` (10⁴ on
/// a 10 mΩ part); with the macro clamp's 50× sharper knee on top, the loop is
/// too stiff for SPICE's timestep control (ngspice stops with "timestep too
/// small" at the knee). At N = 1 the knee is ~0.4 V at the set point's
/// microamps — 4 mA of limit error at K = 100 V/A.
pub const REGULATOR_LIMIT_DIODE: DiodeModel = DiodeModel {
    is: 1e-12,
    n: 1.0,
    rs: 0.0,
    bv: None,
    ibv: 1e-3,
    nbv: None,
};

/// Floor on the input voltage a buck's power-balance sink divides by, volts.
///
/// Below it the converter delivers almost nothing (its output is clamped to
/// `DMAX·v(in) − VDO`), and dividing a vanishing output power by a vanishing
/// input voltage is 0/0; the floor turns that into "a vanishing input current".
pub const POWER_SINK_VMIN: f64 = 0.1;

/// The averaged buck's input: a current
/// `(v(op) − v(on)) · I / (eff · max(v(p) − v(n), POWER_SINK_VMIN))` flows from
/// `p` through the element to `n`, where `I` is the output current, i.e. minus
/// the branch current of `control` (an `E` sourcing the output).
///
/// Only a `BUCK` lowering creates one; there is no netlist letter for it.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerSink {
    /// Element name (`<X name>#pin`).
    pub name: String,
    /// Input terminal the current leaves the circuit at.
    pub p: NodeRef,
    /// Terminal the current returns at (the regulator's `gnd` pin).
    pub n: NodeRef,
    /// Positive output terminal the delivered power is measured at.
    pub op: NodeRef,
    /// Negative output terminal.
    pub on: NodeRef,
    /// The `E` element sourcing the output current.
    pub control: String,
    /// That element's branch-current index.
    pub control_branch: usize,
    /// Conversion efficiency, 0 < eff <= 1.
    pub eff: f64,
}

/// Resolved parameters of a `.model <name> NPN|PNP(...)` card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BjtModel {
    /// NPN or PNP.
    pub polarity: Polarity,
    /// Transport saturation current `IS`, amps.
    pub is: f64,
    /// Forward current gain `BF`.
    pub bf: f64,
    /// Reverse current gain `BR`.
    pub br: f64,
    /// Forward emission coefficient `NF`.
    pub nf: f64,
    /// Reverse emission coefficient `NR`.
    pub nr: f64,
}

impl BjtModel {
    fn defaults(polarity: Polarity) -> Self {
        Self {
            polarity,
            is: 1e-16,
            bf: 100.0,
            br: 1.0,
            nf: 1.0,
            nr: 1.0,
        }
    }
}

/// Resolved parameters of a `.model <name> NMOS|PMOS(...)` card, SPICE
/// level 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MosModel {
    /// N-channel or P-channel.
    pub polarity: Polarity,
    /// Zero-bias threshold `VTO`, volts — **negative for a normal PMOS**, as
    /// in ngspice.
    pub vto: f64,
    /// Transconductance parameter `KP`, A/V².
    pub kp: f64,
    /// Channel-length modulation `LAMBDA`, 1/V.
    pub lambda: f64,
    /// Default channel width `W`, metres (the element line may override it).
    pub w: f64,
    /// Default channel length `L`, metres (the element line may override it).
    pub l: f64,
}

impl MosModel {
    fn defaults(polarity: Polarity) -> Self {
        Self {
            polarity,
            vto: 0.0,
            kp: 2e-5,
            lambda: 0.0,
            w: 1e-4,
            l: 1e-4,
        }
    }
}

/// One `.model` card, before it is attached to an element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModelCard {
    /// A `D` card.
    Diode(DiodeModel),
    /// An `NPN` or `PNP` card.
    Bjt(BjtModel),
    /// An `NMOS` or `PMOS` card.
    Mos(MosModel),
    /// An `OPAMP` card (this engine's type, not ngspice's).
    OpAmp(OpAmpModel),
    /// A `COMP` card (this engine's type, not ngspice's).
    Comparator(ComparatorModel),
    /// An `LDO`, `LDOADJ` or `BUCK` card (this engine's types).
    Regulator(RegulatorModel),
}

impl ModelCard {
    /// The card's SPICE type keyword, for diagnostics.
    fn kind(&self) -> &'static str {
        match self {
            Self::OpAmp(_) => "OPAMP",
            Self::Comparator(_) => "COMP",
            Self::Regulator(model) => match model.kind {
                RegulatorKind::Linear => "LDO",
                RegulatorKind::LinearAdjustable => "LDOADJ",
                RegulatorKind::Buck => "BUCK",
            },
            Self::Diode(_) => "D",
            Self::Bjt(model) => match model.polarity {
                Polarity::N => "NPN",
                Polarity::P => "PNP",
            },
            Self::Mos(model) => match model.polarity {
                Polarity::N => "NMOS",
                Polarity::P => "PMOS",
            },
        }
    }
}

/// The five model names an element may use with no `.model` line of its own.
///
/// These are this engine's convenience values, **not** ngspice's parameter
/// defaults: `NMOS` here is a working enhancement FET, whereas ngspice's
/// level-1 defaults (`VTO = 0`) describe a device that is never off. Write a
/// `.model` line when the deck has to mean the same thing to both engines.
fn builtin_model(name: &str) -> Option<ModelCard> {
    match name.to_ascii_uppercase().as_str() {
        // 1N4148-class small-signal silicon switching diode.
        "D" => Some(ModelCard::Diode(DiodeModel {
            is: 2.52e-9,
            n: 1.752,
            rs: 0.0,
            ..DiodeModel::default()
        })),
        // 1N4733A-like 5.1 V, 1 W zener. The datasheet's headline point is
        // VZ = 5.1 V at IZT = 49 mA with ZZT ≤ 7 Ω. Level-1 breakdown puts the
        // junction at BV when IBV flows, and RS adds IZT·RS on top, so
        // BV = 4.8 V with RS = 6 Ω lands VZ(49 mA) at 5.09 V; the dynamic
        // impedance there is RS + N·Vt/IZT = 6.6 Ω. The price of one RS for
        // both directions is a soft forward knee: 1.9 V at 200 mA, where the
        // datasheet says ≤ 1.2 V.
        "1N4733A" => Some(ModelCard::Diode(DiodeModel {
            is: 1e-11,
            n: 1.2,
            rs: 6.0,
            bv: Some(4.8),
            ibv: 49e-3,
            nbv: None,
        })),
        // SMAJ5.0A-like 400 W unidirectional TVS. Datasheet: VBR 6.40–7.07 V
        // at IT = 10 mA, VC = 9.2 V at IPP = 43.5 A. BV sits mid-window at
        // 6.7 V; RS = 52.5 mΩ is what puts the junction plus RS·IPP at 9.2 V
        // for 43.5 A.
        "SMAJ5.0A" => Some(ModelCard::Diode(DiodeModel {
            is: 1e-12,
            n: 1.0,
            rs: 0.0525,
            bv: Some(6.7),
            ibv: 10e-3,
            nbv: None,
        })),
        // An op-amp that is ideal for any circuit a board puts around it:
        // 120 dB, 10 MHz, 1 Ω, rail to rail.
        "IDEAL_OPAMP" => Some(ModelCard::OpAmp(OpAmpModel {
            aol: 1e6,
            gbw: 10e6,
            rout: 1.0,
            ..OpAmpModel::default()
        })),
        // LM358-like (TI LM358 datasheet, typical): AOL 100 V/mV, GBW
        // 0.7 MHz, output swings to within 1.5 V of V+ and to ground. ROUT is
        // not a datasheet number; 50 Ω is a plausible bipolar class-AB stage.
        // VOS is zero: a typical offset has no sign, so any value here would
        // be a wrong one for most parts.
        "LM358" => Some(ModelCard::OpAmp(OpAmpModel {
            aol: 1e5,
            gbw: 0.7e6,
            rout: 50.0,
            vos: 0.0,
            drop_hi: 1.5,
            drop_lo: 0.0,
        })),
        // LM393-like (TI LM393 datasheet, typical): AOL 200 V/mV, open
        // collector sinking 4 mA at VOL = 150 mV (so ROUT = 37.5 Ω), and a
        // 1.3 µs response to 5 mV of overdrive — which is what GBW = 12 MHz
        // gives the 1 V decision node: 0.5 V / (2π · 12 MHz · 5 mV) = 1.3 µs.
        "LM393" => Some(ModelCard::Comparator(ComparatorModel {
            aol: 2e5,
            gbw: 12e6,
            rout: 37.5,
            ..ComparatorModel::default()
        })),
        // ── Regulators. *-like* cards: the datasheet's typical headline
        // numbers, lowered as described on `RegulatorModel`. ROUT is the
        // load-regulation spec over its stated current range; LINE the
        // line-regulation spec over its stated input range. ──
        //
        // AMS1117-3.3 / -5.0 (Advanced Monolithic Systems AMS1117 datasheet,
        // electrical characteristics, typical): dropout 1.1 V at 800 mA,
        // current limit 1.1 A (900 mA min), quiescent current 5 mA, line
        // regulation 1 mV over 4.75–12 V (6.5–12 V for -5.0), load
        // regulation 1 mV over 0–800 mA.
        "AMS1117-3.3" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 3.3,
            vdo: 1.1,
            ilim: 1.1,
            iq: 5e-3,
            rout: 1e-3 / 0.8,
            line: 1e-3 / (12.0 - 4.75),
            ..RegulatorModel::defaults(RegulatorKind::Linear)
        })),
        "AMS1117-5.0" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 5.0,
            vdo: 1.1,
            ilim: 1.1,
            iq: 5e-3,
            rout: 1e-3 / 0.8,
            line: 1e-3 / (12.0 - 6.5),
            ..RegulatorModel::defaults(RegulatorKind::Linear)
        })),
        // LM7805 (TI LM340/LM7805 datasheet, typical at 25 °C): dropout 2 V
        // at 1 A, short-circuit current 2.1 A, quiescent current 5 mA, line
        // regulation 3 mV over 7.5–20 V, load regulation 15 mV over
        // 5 mA–1.5 A.
        "LM7805" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 5.0,
            vdo: 2.0,
            ilim: 2.1,
            iq: 5e-3,
            rout: 15e-3 / 1.495,
            line: 3e-3 / (20.0 - 7.5),
            ..RegulatorModel::defaults(RegulatorKind::Linear)
        })),
        // MCP1700-3.3 (Microchip MCP1700 datasheet, typical): dropout 178 mV
        // at 250 mA, short-circuit current 408 mA, quiescent current 1.6 µA,
        // line regulation 0.05 %/V, load regulation −0.4 % over 0.1–250 mA.
        "MCP1700-3.3" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 3.3,
            vdo: 0.178,
            ilim: 0.408,
            iq: 1.6e-6,
            rout: 0.004 * 3.3 / 0.2499,
            line: 0.0005 * 3.3,
            ..RegulatorModel::defaults(RegulatorKind::Linear)
        })),
        // A generic 3.3 V LDO: 300 mV dropout, 500 mA limit, 50 µA quiescent.
        // Not a part; the card to reach for when a schematic says "LDO".
        "LDO" | "LDO-3.3" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 3.3,
            vdo: 0.3,
            ilim: 0.5,
            iq: 50e-6,
            rout: 0.01,
            ..RegulatorModel::defaults(RegulatorKind::Linear)
        })),
        // LM317 (TI LM317 datasheet, typical): VREF 1.25 V, IADJ 50 µA,
        // current limit 2.2 A, line regulation 0.01 %/V, load regulation
        // 0.1 % over 10 mA–1.5 A. Dropout is read off the dropout curve at
        // 1 A: about 1.7 V.
        "LM317" => Some(ModelCard::Regulator(RegulatorModel {
            vref: 1.25,
            vdo: 1.7,
            ilim: 2.2,
            iq: 50e-6,
            rout: 0.001 * 5.0 / 1.49,
            line: 0.0001 * 1.25,
            ..RegulatorModel::defaults(RegulatorKind::LinearAdjustable)
        })),
        // MP1584 (MPS MP1584EN datasheet, typical): feedback 0.8 V, peak
        // current limit 4 A (a 3 A part), quiescent current 100 µA, ~90 %
        // efficient at 12 V → 5 V, 1 A. VDO is not a datasheet number: the
        // 150 mΩ high-side switch at 3 A, about 0.45 V.
        "MP1584" => Some(ModelCard::Regulator(RegulatorModel {
            vref: 0.8,
            vdo: 0.45,
            ilim: 4.0,
            iq: 100e-6,
            rout: 0.01,
            eff: 0.9,
            ..RegulatorModel::defaults(RegulatorKind::Buck)
        })),
        // LM2596 (TI LM2596 datasheet, typical): feedback 1.23 V (-ADJ),
        // switch current limit 4.5 A, quiescent current 5 mA, switch
        // saturation 1.16 V at 3 A (the VDO), efficiency 80 % for the 5 V
        // part at 12 V, 3 A (73 % for -ADJ at 3 V).
        "LM2596-5.0" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 5.0,
            vdo: 1.16,
            ilim: 4.5,
            iq: 5e-3,
            rout: 0.01,
            eff: 0.80,
            ..RegulatorModel::defaults(RegulatorKind::Buck)
        })),
        "LM2596-ADJ" => Some(ModelCard::Regulator(RegulatorModel {
            vref: 1.23,
            vdo: 1.16,
            ilim: 4.5,
            iq: 5e-3,
            rout: 0.01,
            eff: 0.73,
            ..RegulatorModel::defaults(RegulatorKind::Buck)
        })),
        "NPN" => Some(ModelCard::Bjt(BjtModel::defaults(Polarity::N))),
        "PNP" => Some(ModelCard::Bjt(BjtModel::defaults(Polarity::P))),
        "NMOS" => Some(ModelCard::Mos(MosModel {
            vto: 1.0,
            lambda: 0.02,
            ..MosModel::defaults(Polarity::N)
        })),
        "PMOS" => Some(ModelCard::Mos(MosModel {
            vto: -1.0,
            lambda: 0.02,
            ..MosModel::defaults(Polarity::P)
        })),
        _ => None,
    }
}

/// `D<name> n+ n- <model>` — a Shockley junction diode.
#[derive(Debug, Clone, PartialEq)]
pub struct Diode {
    /// Element name as written, e.g. `D1`.
    pub name: String,
    /// Anode, the terminal the netlist names first.
    pub anode: NodeRef,
    /// Cathode.
    pub cathode: NodeRef,
    /// The junction's anode side: the same node as [`Self::anode`] when
    /// `RS = 0`, and the internal node `<name>#internal` when it is not.
    pub junction_anode: NodeRef,
    /// Model name as written, for diagnostics.
    pub model_name: String,
    /// Resolved model parameters.
    pub model: DiodeModel,
    /// The breakdown voltage the reverse exponential is actually written
    /// about — ngspice's `tBrkdwnV`, i.e. `BV` moved so that the current at
    /// `−BV` is `IBV` — or `None` when the model has no `BV`. See
    /// [`super::device::breakdown_voltage`].
    pub breakdown: Option<f64>,
}

/// `E<name> n+ n- nc+ nc- <gain>` — voltage-controlled voltage source:
/// `v(n+) − v(n−) = gain · (v(nc+) − v(nc−))`. Adds a branch current.
#[derive(Debug, Clone, PartialEq)]
pub struct Vcvs {
    /// Element name as written.
    pub name: String,
    /// Positive output terminal.
    pub p: NodeRef,
    /// Negative output terminal.
    pub n: NodeRef,
    /// Positive controlling node.
    pub cp: NodeRef,
    /// Negative controlling node.
    pub cn: NodeRef,
    /// Voltage gain, V/V.
    pub gain: f64,
}

/// `G<name> n+ n- nc+ nc- <gm>` — voltage-controlled current source. A current
/// `gm · (v(nc+) − v(nc−))` flows from `n+` through the source to `n−`, as in
/// SPICE.
#[derive(Debug, Clone, PartialEq)]
pub struct Vccs {
    /// Element name as written.
    pub name: String,
    /// Terminal the current leaves the circuit at.
    pub p: NodeRef,
    /// Terminal the current returns to the circuit at.
    pub n: NodeRef,
    /// Positive controlling node.
    pub cp: NodeRef,
    /// Negative controlling node.
    pub cn: NodeRef,
    /// Transconductance, siemens.
    pub gm: f64,
}

/// `F<name> n+ n- <vsource> <gain>` — current-controlled current source: a
/// current `gain · i(vsource)` flows from `n+` through the source to `n−`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cccs {
    /// Element name as written.
    pub name: String,
    /// Terminal the current leaves the circuit at.
    pub p: NodeRef,
    /// Terminal the current returns to the circuit at.
    pub n: NodeRef,
    /// Name of the element whose branch current controls this one.
    pub control: String,
    /// That element's branch-current index (see [`Circuit::branch_index`]).
    pub control_branch: usize,
    /// Current gain, A/A.
    pub gain: f64,
}

/// `H<name> n+ n- <vsource> <r>` — current-controlled voltage source:
/// `v(n+) − v(n−) = r · i(vsource)`. Adds a branch current.
#[derive(Debug, Clone, PartialEq)]
pub struct Ccvs {
    /// Element name as written.
    pub name: String,
    /// Positive output terminal.
    pub p: NodeRef,
    /// Negative output terminal.
    pub n: NodeRef,
    /// Name of the element whose branch current controls this one.
    pub control: String,
    /// That element's branch-current index (see [`Circuit::branch_index`]).
    pub control_branch: usize,
    /// Transresistance, ohms.
    pub ohms: f64,
}

/// `Q<name> nc nb ne <model>` — an Ebers–Moll bipolar transistor.
#[derive(Debug, Clone, PartialEq)]
pub struct Bjt {
    /// Element name as written, e.g. `Q1`.
    pub name: String,
    /// Collector.
    pub c: NodeRef,
    /// Base.
    pub b: NodeRef,
    /// Emitter.
    pub e: NodeRef,
    /// Model name as written, for diagnostics.
    pub model_name: String,
    /// Resolved model parameters.
    pub model: BjtModel,
}

/// `M<name> nd ng ns nb <model> [w=<m>] [l=<m>]` — a level-1 MOSFET.
#[derive(Debug, Clone, PartialEq)]
pub struct Mosfet {
    /// Element name as written, e.g. `M1`.
    pub name: String,
    /// Drain.
    pub d: NodeRef,
    /// Gate.
    pub g: NodeRef,
    /// Source.
    pub s: NodeRef,
    /// Bulk. Required by the syntax and tied to the channel through `GMIN`,
    /// but it does not shift the threshold: there is no body effect.
    pub bulk: NodeRef,
    /// Model name as written, for diagnostics.
    pub model_name: String,
    /// Resolved model parameters.
    pub model: MosModel,
    /// `KP·W/L`, A/V² — the transconductance the solver actually stamps.
    pub beta: f64,
}

/// Voltage threshold control for a switch: closes from the voltage between
/// `pos` and `neg` rather than from a routed boolean input.
///
/// `|V(pos) − V(neg)|` is compared to `vt` with hysteresis `vh`. With
/// `invert == false` (NO contact) the switch closes at `vt` and opens at
/// `vt − vh`. With `invert == true` (NC contact) those edges are reversed, so
/// the contact is closed while the coil is de-energised.
#[derive(Debug, Clone, PartialEq)]
pub struct VoltageSwitchControl {
    /// Positive sense node.
    pub pos: NodeRef,
    /// Negative sense node.
    pub neg: NodeRef,
    /// Closing threshold in volts (`|V|` rising through `vt`).
    pub vt: f64,
    /// Hysteresis in volts; the opening threshold is `vt − vh` (clamped at 0).
    pub vh: f64,
    /// When true, the contact is the complement (NC) of the NO threshold rule.
    pub invert: bool,
}

/// `S<name> n1 n2 <ctrl> ron=<r> roff=<r>` — an ideal switch whose control is a
/// routed boolean input name, not a circuit node.
///
/// Or `S<name> n1 n2 nc+ nc- vt=<v> [vh=<v>] [invert=0|1] ron=<r> roff=<r>` —
/// the same switch closed by the absolute voltage between `nc+` and `nc-`.
/// When `voltage` is `Some`, `ctrl` is empty and the adapter does not expose a
/// routed input for this element.
#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    /// Element name as written.
    pub name: String,
    /// First terminal.
    pub a: NodeRef,
    /// Second terminal.
    pub b: NodeRef,
    /// Name of the routed boolean input that opens and closes it.
    /// Empty when [`Self::voltage`] is set.
    pub ctrl: String,
    /// Closed resistance in ohms.
    pub ron: f64,
    /// Open resistance in ohms.
    pub roff: f64,
    /// Voltage-threshold control, when the switch is not digitally routed.
    pub voltage: Option<VoltageSwitchControl>,
}

/// A parsed netlist: elements in file order plus the node table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Circuit {
    node_names: Vec<String>,
    node_index: BTreeMap<String, usize>,
    /// Resistors, in netlist order.
    pub resistors: Vec<Resistor>,
    /// Capacitors, in netlist order.
    pub capacitors: Vec<Capacitor>,
    /// Inductors, in netlist order.
    pub inductors: Vec<Inductor>,
    /// Voltage sources, in netlist order.
    pub voltage_sources: Vec<VoltageSource>,
    /// Current sources, in netlist order.
    pub current_sources: Vec<CurrentSource>,
    /// Switches, in netlist order.
    pub switches: Vec<Switch>,
    /// Diodes, in netlist order.
    pub diodes: Vec<Diode>,
    /// Bipolar transistors, in netlist order.
    pub bjts: Vec<Bjt>,
    /// MOSFETs, in netlist order.
    pub mosfets: Vec<Mosfet>,
    /// Voltage-controlled voltage sources (`E`), in netlist order.
    pub vcvs: Vec<Vcvs>,
    /// Voltage-controlled current sources (`G`), in netlist order.
    pub vccs: Vec<Vccs>,
    /// Current-controlled current sources (`F`), in netlist order.
    pub cccs: Vec<Cccs>,
    /// Current-controlled voltage sources (`H`), in netlist order.
    pub ccvs: Vec<Ccvs>,
    /// Power-balance sinks, one per lowered `BUCK`.
    pub power_sinks: Vec<PowerSink>,
    /// `.ic V(node)=value` entries, in netlist order.
    pub node_ic: Vec<(usize, f64)>,
}

impl Circuit {
    /// Number of non-ground nodes (the `N` of the MNA system).
    pub fn node_count(&self) -> usize {
        self.node_names.len()
    }

    /// Name of node `index`, as first written in the netlist.
    pub fn node_name(&self, index: usize) -> &str {
        &self.node_names[index]
    }

    /// Resolve a node name. `Some(None)` is ground; `None` means the netlist
    /// never mentions that name — which is why probes can be checked up front
    /// instead of silently reading zero.
    pub fn node(&self, name: &str) -> Option<NodeRef> {
        let key = name.trim().to_ascii_lowercase();
        if is_ground(&key) {
            return Some(None);
        }
        self.node_index.get(&key).map(|index| Some(*index))
    }

    /// Number of branch currents (the `M`): one per voltage source, then one
    /// per inductor, then one per `E`, then one per `H`, in that order.
    ///
    /// The dependent sources come last so that adding one to a netlist never
    /// renumbers a voltage source's or inductor's branch.
    pub fn branch_count(&self) -> usize {
        self.voltage_sources.len() + self.inductors.len() + self.vcvs.len() + self.ccvs.len()
    }

    /// Branch-current index of the named voltage source, inductor, `E` or `H`.
    pub fn branch_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.voltage_sources
            .iter()
            .map(|e| &e.name)
            .chain(self.inductors.iter().map(|e| &e.name))
            .chain(self.vcvs.iter().map(|e| &e.name))
            .chain(self.ccvs.iter().map(|e| &e.name))
            .position(|candidate| candidate.to_ascii_lowercase() == key)
    }

    /// Name of the element owning branch current `index`.
    pub fn branch_name(&self, index: usize) -> &str {
        let mut index = index;
        if index < self.voltage_sources.len() {
            return &self.voltage_sources[index].name;
        }
        index -= self.voltage_sources.len();
        if index < self.inductors.len() {
            return &self.inductors[index].name;
        }
        index -= self.inductors.len();
        if index < self.vcvs.len() {
            return &self.vcvs[index].name;
        }
        &self.ccvs[index - self.vcvs.len()].name
    }

    /// `N + M`: the dimension of the MNA system.
    pub fn unknowns(&self) -> usize {
        self.node_count() + self.branch_count()
    }

    fn intern(&mut self, name: &str) -> NodeRef {
        let key = name.trim().to_ascii_lowercase();
        if is_ground(&key) {
            return None;
        }
        if let Some(index) = self.node_index.get(&key) {
            return Some(*index);
        }
        let index = self.node_names.len();
        self.node_names.push(name.trim().to_string());
        self.node_index.insert(key, index);
        Some(index)
    }

    fn has_element(&self, name: &str) -> bool {
        let key = name.to_ascii_lowercase();
        let matches = |candidate: &str| candidate.to_ascii_lowercase() == key;
        self.resistors.iter().any(|e| matches(&e.name))
            || self.capacitors.iter().any(|e| matches(&e.name))
            || self.inductors.iter().any(|e| matches(&e.name))
            || self.voltage_sources.iter().any(|e| matches(&e.name))
            || self.current_sources.iter().any(|e| matches(&e.name))
            || self.switches.iter().any(|e| matches(&e.name))
            || self.diodes.iter().any(|e| matches(&e.name))
            || self.bjts.iter().any(|e| matches(&e.name))
            || self.mosfets.iter().any(|e| matches(&e.name))
            || self.vcvs.iter().any(|e| matches(&e.name))
            || self.vccs.iter().any(|e| matches(&e.name))
            || self.cccs.iter().any(|e| matches(&e.name))
            || self.ccvs.iter().any(|e| matches(&e.name))
            || self.power_sinks.iter().any(|e| matches(&e.name))
    }

    /// True when the circuit holds at least one device whose stamp depends on
    /// the solution, i.e. when a step needs Newton iteration.
    ///
    /// This is the switch that keeps the linear engine exactly what it was:
    /// when it is false, [`super::mna::Solver::advance`] runs the same code,
    /// in the same order, on the same values as before diodes existed.
    pub fn is_nonlinear(&self) -> bool {
        !self.diodes.is_empty()
            || !self.bjts.is_empty()
            || !self.mosfets.is_empty()
            || !self.power_sinks.is_empty()
    }

    /// True when any independent source carries a transient function, so the
    /// solver has to re-evaluate sources against the clock each step.
    pub fn has_waveforms(&self) -> bool {
        self.voltage_sources.iter().any(|s| !s.wave.is_constant())
            || self.current_sources.iter().any(|s| !s.wave.is_constant())
    }
}

fn is_ground(lowercased: &str) -> bool {
    lowercased == "0" || lowercased == "gnd" || lowercased == "ground"
}

/// Parse a SPICE value with an optional engineering suffix (`10k`, `100n`,
/// `2.2meg`, `1e-6`). Trailing unit letters are ignored, as in SPICE, so
/// `100nF` and `10kohm` mean what they look like.
pub fn parse_spice_value(raw: &str) -> Option<f64> {
    let text = raw.trim();
    let bytes = text.as_bytes();
    let mut index = 0;

    if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
        index += 1;
    }
    let digits_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
    }
    if index == digits_start || (index == digits_start + 1 && bytes[digits_start] == b'.') {
        return None;
    }
    // An exponent only counts when digits actually follow it: `1e-6` is a
    // number, `1exp` is the number 1 with a suffix SPICE ignores.
    let mut exponent_at = None;
    if index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
        let mut probe = index + 1;
        if probe < bytes.len() && (bytes[probe] == b'+' || bytes[probe] == b'-') {
            probe += 1;
        }
        if probe < bytes.len() && bytes[probe].is_ascii_digit() {
            while probe < bytes.len() && bytes[probe].is_ascii_digit() {
                probe += 1;
            }
            exponent_at = Some(index);
            index = probe;
        }
    }

    let suffix = text[index..].to_ascii_lowercase();
    if suffix.starts_with("mil") {
        return text[..index].parse::<f64>().ok().map(|mils| mils * 25.4e-6);
    }
    let power: i32 = if suffix.starts_with("meg") {
        6
    } else {
        match suffix.as_bytes().first() {
            None => 0,
            Some(b't') => 12,
            Some(b'g') => 9,
            Some(b'k') => 3,
            Some(b'm') => -3,
            Some(b'u') => -6,
            Some(b'n') => -9,
            Some(b'p') => -12,
            Some(b'f') => -15,
            // Anything else is a unit spelling (`5ohm`, `3volt`), not a scale.
            Some(_) => 0,
        }
    };

    // Scale by moving the decimal exponent rather than multiplying, so `100n`
    // is the same double as `100e-9` — the value a user would get writing it
    // out — instead of `100.0 * 1e-9`, which is one ulp away from it.
    let (mantissa, exponent) = match exponent_at {
        Some(at) => (&text[..at], text[at + 1..index].parse::<i32>().ok()?),
        None => (&text[..index], 0),
    };
    let mantissa = mantissa.strip_suffix('.').unwrap_or(mantissa);
    format!("{mantissa}e{}", exponent + power).parse().ok()
}

/// Strip a `*` comment line and any `;` / `$` trailing comment.
fn strip_comment(line: &str) -> &str {
    let cut = line.find([';', '$']).unwrap_or(line.len());
    line[..cut].trim()
}

struct LineCtx<'a> {
    number: usize,
    text: &'a str,
}

impl LineCtx<'_> {
    fn parse_err(&self, message: impl Into<String>) -> AnalogError {
        AnalogError::Parse {
            line: self.number,
            text: self.text.to_string(),
            message: message.into(),
        }
    }

    fn unsupported(&self) -> AnalogError {
        AnalogError::Unsupported {
            line: self.number,
            text: self.text.to_string(),
        }
    }

    fn value(&self, raw: &str, what: &str) -> Result<f64, AnalogError> {
        parse_spice_value(raw)
            .ok_or_else(|| self.parse_err(format!("{what} `{raw}` is not a SPICE value")))
    }
}

/// One device line as parsed, before its `.model` card is known.
///
/// Model cards may be written after the elements that use them, so the parser
/// collects the terminals in one pass and binds the parameters in a second.
struct PendingDevice {
    line: usize,
    text: String,
    letter: char,
    name: String,
    nodes: Vec<NodeRef>,
    model_name: String,
    width: Option<f64>,
    length: Option<f64>,
}

/// Parse a netlist into a [`Circuit`].
pub fn parse_netlist(text: &str) -> Result<Circuit, AnalogError> {
    let mut circuit = Circuit::default();
    let mut ended = false;
    let mut models: BTreeMap<String, ModelCard> = BTreeMap::new();
    let mut pending: Vec<PendingDevice> = Vec::new();
    let mut in_control = false;
    // Line of every `F`/`H`, in the order they were pushed, for the error when
    // the element they name carries no branch current.
    let mut controls: Vec<(char, usize, String)> = Vec::new();

    for (offset, raw_line) in text.lines().enumerate() {
        let number = offset + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('*') {
            continue;
        }
        let line = strip_comment(trimmed);
        if line.is_empty() {
            continue;
        }
        let ctx = LineCtx { number, text: line };
        // A `.control` block is ngspice's scripting language, not a circuit.
        // Skipping it wholesale is what lets one deck drive both engines: the
        // in-core adapter takes its run length from the manifest, ngspice
        // takes it from the block.
        if in_control {
            if line.eq_ignore_ascii_case(".endc") {
                in_control = false;
            }
            continue;
        }
        if ended {
            return Err(ctx.parse_err("netlist continues after `.end`"));
        }

        let tokens: Vec<&str> = line.split_whitespace().collect();
        let head = tokens[0];

        if let Some(directive) = head.strip_prefix('.') {
            match directive.to_ascii_lowercase().as_str() {
                "end" => ended = true,
                "ic" => parse_ic(&mut circuit, &ctx, &tokens[1..])?,
                "model" => parse_model(&mut models, &ctx, &tokens[1..])?,
                "control" => in_control = true,
                // Analysis and housekeeping cards. This engine's run length,
                // step and outputs come from the manifest, so these say
                // nothing it can act on — but rejecting them would mean a deck
                // cannot be shared with ngspice, which is the whole point of
                // spelling the subset in SPICE.
                "options" | "option" | "tran" | "op" | "print" | "plot" | "save" | "probe"
                | "width" | "temp" | "nodeset" | "title" | "endc" => {}
                _ => return Err(ctx.unsupported()),
            }
            continue;
        }

        let letter = head
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or(' ');
        if !matches!(
            letter,
            'R' | 'C' | 'L' | 'V' | 'I' | 'S' | 'D' | 'Q' | 'M' | 'E' | 'G' | 'F' | 'H' | 'X'
        ) {
            return Err(ctx.unsupported());
        }
        if head.len() < 2 {
            return Err(ctx.parse_err(format!("element `{head}` has no name after `{letter}`")));
        }
        if circuit.has_element(head) {
            return Err(ctx.parse_err(format!("element `{head}` is declared twice")));
        }

        match letter {
            'R' => {
                let rest = &tokens[1..];
                if rest.len() != 3 {
                    return Err(ctx.parse_err("expected `R<name> n1 n2 <value>`"));
                }
                let ohms = ctx.value(rest[2], "resistance")?;
                if ohms == 0.0 {
                    return Err(ctx.parse_err("resistance must be non-zero"));
                }
                let a = circuit.intern(rest[0]);
                let b = circuit.intern(rest[1]);
                circuit.resistors.push(Resistor {
                    name: head.to_string(),
                    a,
                    b,
                    ohms,
                });
            }
            'C' => {
                let (n1, n2, value, ic) =
                    two_nodes_value_ic(&ctx, &tokens[1..], "C<name> n1 n2 <value> [ic=<v>]")?;
                let farads = ctx.value(value, "capacitance")?;
                if farads <= 0.0 {
                    return Err(ctx.parse_err("capacitance must be positive"));
                }
                let ic = ic.map(|raw| ctx.value(raw, "ic")).transpose()?;
                let a = circuit.intern(n1);
                let b = circuit.intern(n2);
                circuit.capacitors.push(Capacitor {
                    name: head.to_string(),
                    a,
                    b,
                    farads,
                    ic,
                });
            }
            'L' => {
                let (n1, n2, value, ic) =
                    two_nodes_value_ic(&ctx, &tokens[1..], "L<name> n1 n2 <value> [ic=<i>]")?;
                let henries = ctx.value(value, "inductance")?;
                if henries <= 0.0 {
                    return Err(ctx.parse_err("inductance must be positive"));
                }
                let ic = ic.map(|raw| ctx.value(raw, "ic")).transpose()?;
                let a = circuit.intern(n1);
                let b = circuit.intern(n2);
                circuit.inductors.push(Inductor {
                    name: head.to_string(),
                    a,
                    b,
                    henries,
                    ic,
                });
            }
            'V' | 'I' => {
                let shape = if letter == 'V' {
                    "V<name> n+ n- <source>"
                } else {
                    "I<name> n+ n- <source>"
                };
                let rest = &tokens[1..];
                if rest.len() < 3 {
                    return Err(ctx.parse_err(format!("expected `{shape}`")));
                }
                let (np, nn) = (rest[0], rest[1]);
                let wave = parse_source(&ctx, &rest[2..], shape)?;
                let dc = wave.at(0.0);
                let p = circuit.intern(np);
                let n = circuit.intern(nn);
                if letter == 'V' {
                    circuit.voltage_sources.push(VoltageSource {
                        name: head.to_string(),
                        p,
                        n,
                        dc,
                        wave,
                    });
                } else {
                    circuit.current_sources.push(CurrentSource {
                        name: head.to_string(),
                        p,
                        n,
                        dc,
                        wave,
                    });
                }
            }
            'D' | 'Q' | 'M' => {
                let (terminals, shape) = match letter {
                    'D' => (2, "D<name> n+ n- <model>"),
                    'Q' => (3, "Q<name> nc nb ne <model>"),
                    _ => (4, "M<name> nd ng ns nb <model> [w=<m>] [l=<m>]"),
                };
                let rest = &tokens[1..];
                if rest.len() < terminals + 1 {
                    return Err(ctx.parse_err(format!("expected `{shape}`")));
                }
                let mut width = None;
                let mut length = None;
                for token in &rest[terminals + 1..] {
                    let (key, value) = token
                        .split_once('=')
                        .ok_or_else(|| ctx.parse_err(format!("expected `{shape}`")))?;
                    match key.to_ascii_lowercase().as_str() {
                        "w" if letter == 'M' => width = Some(ctx.value(value, "w")?),
                        "l" if letter == 'M' => length = Some(ctx.value(value, "l")?),
                        other => {
                            return Err(ctx.parse_err(format!(
                                "unknown parameter `{other}`; expected `{shape}`"
                            )))
                        }
                    }
                }
                if matches!(width, Some(w) if w <= 0.0) || matches!(length, Some(l) if l <= 0.0) {
                    return Err(ctx.parse_err("w and l must be positive"));
                }
                // The terminals are interned now so that node numbering still
                // follows netlist order; the model is bound after the last
                // line, because `.model` may come after its elements.
                let nodes = rest[..terminals]
                    .iter()
                    .map(|name| circuit.intern(name))
                    .collect();
                pending.push(PendingDevice {
                    line: ctx.number,
                    text: line.to_string(),
                    letter,
                    name: head.to_string(),
                    nodes,
                    model_name: rest[terminals].to_string(),
                    width,
                    length,
                });
            }
            'E' | 'G' => {
                let shape = if letter == 'E' {
                    "E<name> n+ n- nc+ nc- <gain>"
                } else {
                    "G<name> n+ n- nc+ nc- <gm>"
                };
                let rest = &tokens[1..];
                refuse_behavioural_source(&ctx, rest)?;
                if rest.len() != 5 {
                    return Err(ctx.parse_err(format!("expected `{shape}`")));
                }
                let value = ctx.value(rest[4], if letter == 'E' { "gain" } else { "gm" })?;
                let p = circuit.intern(rest[0]);
                let n = circuit.intern(rest[1]);
                let cp = circuit.intern(rest[2]);
                let cn = circuit.intern(rest[3]);
                let name = head.to_string();
                if letter == 'E' {
                    circuit.vcvs.push(Vcvs {
                        name,
                        p,
                        n,
                        cp,
                        cn,
                        gain: value,
                    });
                } else {
                    circuit.vccs.push(Vccs {
                        name,
                        p,
                        n,
                        cp,
                        cn,
                        gm: value,
                    });
                }
            }
            'F' | 'H' => {
                let shape = if letter == 'F' {
                    "F<name> n+ n- <vsource> <gain>"
                } else {
                    "H<name> n+ n- <vsource> <r>"
                };
                let rest = &tokens[1..];
                refuse_behavioural_source(&ctx, rest)?;
                if rest.len() != 4 {
                    return Err(ctx.parse_err(format!("expected `{shape}`")));
                }
                let value = ctx.value(rest[3], if letter == 'F' { "gain" } else { "r" })?;
                let p = circuit.intern(rest[0]);
                let n = circuit.intern(rest[1]);
                let name = head.to_string();
                let control = rest[2].to_string();
                // The controlling element may be declared later, and its
                // branch number is only final once every `V`, `L`, `E` and `H`
                // (the lowered op-amps' included) exists; bound after the
                // last line.
                controls.push((letter, ctx.number, line.to_string()));
                if letter == 'F' {
                    circuit.cccs.push(Cccs {
                        name,
                        p,
                        n,
                        control,
                        control_branch: usize::MAX,
                        gain: value,
                    });
                } else {
                    circuit.ccvs.push(Ccvs {
                        name,
                        p,
                        n,
                        control,
                        control_branch: usize::MAX,
                        ohms: value,
                    });
                }
            }
            'X' => {
                let rest = &tokens[1..];
                if rest.len() < 2 {
                    return Err(ctx.unsupported());
                }
                // Whether this is an op-amp or a real subcircuit call depends on
                // a `.model` line that may not have been read yet, so it is
                // decided with the other devices, after the last line.
                let (model_name, terminals) = rest.split_last().expect("checked above");
                let nodes = terminals.iter().map(|name| circuit.intern(name)).collect();
                pending.push(PendingDevice {
                    line: ctx.number,
                    text: line.to_string(),
                    letter,
                    name: head.to_string(),
                    nodes,
                    model_name: model_name.to_string(),
                    width: None,
                    length: None,
                });
            }
            'S' => {
                let digital = "S<name> n1 n2 <ctrl> ron=<r> roff=<r>";
                let voltage = "S<name> n1 n2 nc+ nc- vt=<v> [vh=<v>] [invert=0|1] ron=<r> roff=<r>";
                let rest = &tokens[1..];
                let voltage_form = rest.iter().any(|token| {
                    token
                        .split_once('=')
                        .map(|(key, _)| key.eq_ignore_ascii_case("vt"))
                        .unwrap_or(false)
                });
                if voltage_form {
                    if rest.len() < 6 {
                        return Err(ctx.parse_err(format!("expected `{voltage}`")));
                    }
                    let mut ron = None;
                    let mut roff = None;
                    let mut vt = None;
                    let mut vh = 0.0;
                    let mut invert = false;
                    for token in &rest[4..] {
                        let (key, value) = token
                            .split_once('=')
                            .ok_or_else(|| ctx.parse_err(format!("expected `{voltage}`")))?;
                        match key.to_ascii_lowercase().as_str() {
                            "ron" => ron = Some(ctx.value(value, "ron")?),
                            "roff" => roff = Some(ctx.value(value, "roff")?),
                            "vt" => vt = Some(ctx.value(value, "vt")?),
                            "vh" => vh = ctx.value(value, "vh")?,
                            "invert" => {
                                let parsed = ctx.value(value, "invert")?;
                                if parsed != 0.0 && parsed != 1.0 {
                                    return Err(ctx.parse_err("invert must be 0 or 1"));
                                }
                                invert = parsed != 0.0;
                            }
                            other => {
                                return Err(ctx.parse_err(format!(
                                    "unknown switch parameter `{other}`"
                                )))
                            }
                        }
                    }
                    let (Some(ron), Some(roff), Some(vt)) = (ron, roff, vt) else {
                        return Err(ctx.parse_err(format!("expected `{voltage}`")));
                    };
                    if ron <= 0.0 || roff <= 0.0 {
                        return Err(ctx.parse_err("ron and roff must be positive"));
                    }
                    if vt <= 0.0 {
                        return Err(ctx.parse_err("vt must be positive"));
                    }
                    if vh < 0.0 || vh > vt {
                        return Err(ctx.parse_err("vh must be in [0, vt]"));
                    }
                    let a = circuit.intern(rest[0]);
                    let b = circuit.intern(rest[1]);
                    let pos = circuit.intern(rest[2]);
                    let neg = circuit.intern(rest[3]);
                    circuit.switches.push(Switch {
                        name: head.to_string(),
                        a,
                        b,
                        ctrl: String::new(),
                        ron,
                        roff,
                        voltage: Some(VoltageSwitchControl {
                            pos,
                            neg,
                            vt,
                            vh,
                            invert,
                        }),
                    });
                } else {
                    if rest.len() != 5 {
                        return Err(ctx.parse_err(format!("expected `{digital}`")));
                    }
                    let mut ron = None;
                    let mut roff = None;
                    for token in &rest[3..] {
                        let (key, value) = token
                            .split_once('=')
                            .ok_or_else(|| ctx.parse_err(format!("expected `{digital}`")))?;
                        match key.to_ascii_lowercase().as_str() {
                            "ron" => ron = Some(ctx.value(value, "ron")?),
                            "roff" => roff = Some(ctx.value(value, "roff")?),
                            other => {
                                return Err(ctx.parse_err(format!(
                                    "unknown switch parameter `{other}`"
                                )))
                            }
                        }
                    }
                    let (Some(ron), Some(roff)) = (ron, roff) else {
                        return Err(ctx.parse_err(format!("expected `{digital}`")));
                    };
                    if ron <= 0.0 || roff <= 0.0 {
                        return Err(ctx.parse_err("ron and roff must be positive"));
                    }
                    let a = circuit.intern(rest[0]);
                    let b = circuit.intern(rest[1]);
                    circuit.switches.push(Switch {
                        name: head.to_string(),
                        a,
                        b,
                        ctrl: rest[2].to_string(),
                        ron,
                        roff,
                        voltage: None,
                    });
                }
            }
            _ => unreachable!("element letter filtered above"),
        }
    }

    bind_models(&mut circuit, &models, pending)?;
    bind_controls(&mut circuit, &controls)?;
    Ok(circuit)
}

/// Refuse the nonlinear dependent-source forms by name, so they read as
/// "outside the subset" rather than as a malformed linear source.
fn refuse_behavioural_source(ctx: &LineCtx<'_>, rest: &[&str]) -> Result<(), AnalogError> {
    let behavioural = rest.iter().any(|token| {
        let upper = token.to_ascii_uppercase();
        upper.starts_with("POLY")
            || upper.starts_with("VALUE")
            || upper.starts_with("TABLE")
            || upper.starts_with("LAPLACE")
            || upper.contains('{')
    });
    if behavioural {
        Err(ctx.unsupported())
    } else {
        Ok(())
    }
}

/// Resolve each `F`/`H` to the branch current of the element it names.
///
/// Done last because the branch numbering is final only once every `V`, `L`,
/// `E` and `H` exists, including the ones an op-amp lowers into.
fn bind_controls(
    circuit: &mut Circuit,
    controls: &[(char, usize, String)],
) -> Result<(), AnalogError> {
    let resolve = |circuit: &Circuit, letter: char, index: usize, name: &str| {
        circuit.branch_index(name).ok_or_else(|| {
            let message = format!(
                "`{name}` carries no branch current; an F or H element must name a \
                 V, L, E or H element"
            );
            // A lowered regulator's own F comes after every written one and
            // names an E it just made, so only a written line can fail here;
            // the fallback keeps a would-be bug an error rather than a panic.
            match controls.iter().filter(|(l, _, _)| *l == letter).nth(index) {
                Some((_, line, text)) => AnalogError::Parse {
                    line: *line,
                    text: text.clone(),
                    message,
                },
                None => AnalogError::Config(message),
            }
        })
    };
    for index in 0..circuit.cccs.len() {
        let branch = resolve(circuit, 'F', index, &circuit.cccs[index].control)?;
        circuit.cccs[index].control_branch = branch;
    }
    for index in 0..circuit.ccvs.len() {
        let branch = resolve(circuit, 'H', index, &circuit.ccvs[index].control)?;
        circuit.ccvs[index].control_branch = branch;
    }
    for index in 0..circuit.power_sinks.len() {
        let name = circuit.power_sinks[index].control.clone();
        let branch = circuit.branch_index(&name).ok_or_else(|| {
            AnalogError::Config(format!("power sink control `{name}` carries no branch"))
        })?;
        circuit.power_sinks[index].control_branch = branch;
    }
    Ok(())
}

/// Attach each device's model card, once every `.model` line has been read.
fn bind_models(
    circuit: &mut Circuit,
    models: &BTreeMap<String, ModelCard>,
    pending: Vec<PendingDevice>,
) -> Result<(), AnalogError> {
    for device in pending {
        let ctx = LineCtx {
            number: device.line,
            text: &device.text,
        };
        let key = device.model_name.to_ascii_lowercase();
        let resolved = models
            .get(&key)
            .copied()
            .or_else(|| builtin_model(&device.model_name));
        if device.letter == 'X' {
            // Only an op-amp, comparator or regulator card makes an `X` line
            // ours; any other `X` is a call into a subcircuit library.
            match resolved {
                Some(ModelCard::OpAmp(model)) => {
                    lower_opamp(circuit, &ctx, &device, model)?;
                }
                Some(ModelCard::Comparator(model)) => {
                    lower_comparator(circuit, &ctx, &device, model)?;
                }
                Some(ModelCard::Regulator(model)) => {
                    lower_regulator(circuit, &ctx, &device, model)?;
                }
                _ => return Err(ctx.unsupported()),
            }
            continue;
        }
        let card = resolved.ok_or_else(|| {
            ctx.parse_err(format!(
                "no `.model {}` line, and `{}` is not one of the built-in models \
                     `D`, `NPN`, `PNP`, `NMOS`, `PMOS`, `1N4733A`, `SMAJ5.0A`",
                device.model_name, device.model_name
            ))
        })?;
        let wrong_kind = |wanted: &str| {
            ctx.parse_err(format!(
                "`{}` is a {} element but `{}` is a {} model",
                device.name,
                wanted,
                device.model_name,
                card.kind()
            ))
        };
        match device.letter {
            'D' => {
                let ModelCard::Diode(model) = card else {
                    return Err(wrong_kind("D"));
                };
                if model.is <= 0.0 || model.n <= 0.0 || model.rs < 0.0 {
                    return Err(
                        ctx.parse_err("a diode model needs IS > 0, N > 0 and RS >= 0".to_string())
                    );
                }
                let breakdown = match model.bv {
                    None => None,
                    Some(bv) => {
                        let nbv = model.nbv.unwrap_or(model.n);
                        if bv <= 0.0 || model.ibv <= 0.0 || nbv <= 0.0 {
                            return Err(ctx.parse_err(
                                "a diode model with BV needs BV > 0, IBV > 0 and NBV > 0"
                                    .to_string(),
                            ));
                        }
                        Some(super::device::breakdown_voltage(
                            bv, model.ibv, model.is, nbv,
                        ))
                    }
                };
                // A series resistance needs somewhere to drop its voltage, so
                // it gets the internal node SPICE also creates. It is a real
                // node: it costs an unknown and `v(d1#internal)` probes it.
                let junction_anode = if model.rs > 0.0 {
                    circuit.intern(&format!("{}#internal", device.name))
                } else {
                    device.nodes[0]
                };
                circuit.diodes.push(Diode {
                    name: device.name,
                    anode: device.nodes[0],
                    cathode: device.nodes[1],
                    junction_anode,
                    model_name: device.model_name,
                    model,
                    breakdown,
                });
            }
            'Q' => {
                let ModelCard::Bjt(model) = card else {
                    return Err(wrong_kind("Q"));
                };
                if model.is <= 0.0 || model.bf <= 0.0 || model.br <= 0.0 {
                    return Err(
                        ctx.parse_err("a BJT model needs IS > 0, BF > 0 and BR > 0".to_string())
                    );
                }
                if model.nf <= 0.0 || model.nr <= 0.0 {
                    return Err(ctx.parse_err("a BJT model needs NF > 0 and NR > 0".to_string()));
                }
                circuit.bjts.push(Bjt {
                    name: device.name,
                    c: device.nodes[0],
                    b: device.nodes[1],
                    e: device.nodes[2],
                    model_name: device.model_name,
                    model,
                });
            }
            'M' => {
                let ModelCard::Mos(model) = card else {
                    return Err(wrong_kind("M"));
                };
                if model.kp <= 0.0 {
                    return Err(ctx.parse_err("a MOSFET model needs KP > 0".to_string()));
                }
                let w = device.width.unwrap_or(model.w);
                let l = device.length.unwrap_or(model.l);
                if w <= 0.0 || l <= 0.0 {
                    return Err(ctx.parse_err("a MOSFET model needs W > 0 and L > 0".to_string()));
                }
                circuit.mosfets.push(Mosfet {
                    name: device.name,
                    d: device.nodes[0],
                    g: device.nodes[1],
                    s: device.nodes[2],
                    bulk: device.nodes[3],
                    model_name: device.model_name,
                    model,
                    beta: model.kp * w / l,
                });
            }
            _ => unreachable!("device letter filtered above"),
        }
    }
    Ok(())
}

/// Parse the value part of a `V`/`I` line: `[dc] <value>`, `SIN(...)` or
/// `PULSE(...)`.
///
/// SPICE allows the argument list to be written with or without commas and
/// with the parenthesis detached from the keyword (`SIN (0 5 1k)`), so the
/// tokens are re-joined and split on the punctuation rather than trusted to
/// arrive one per argument.
fn parse_source(ctx: &LineCtx<'_>, tokens: &[&str], shape: &str) -> Result<Waveform, AnalogError> {
    let joined = tokens.join(" ");
    let upper = joined.trim().to_ascii_uppercase();

    for keyword in ["SIN", "PULSE"] {
        let Some(rest) = upper.strip_prefix(keyword) else {
            continue;
        };
        let rest = rest.trim_start();
        if !rest.starts_with('(') {
            continue;
        }
        let body = joined.trim()[joined.trim().len() - rest.len()..].trim();
        let body = body
            .strip_prefix('(')
            .and_then(|inner| inner.strip_suffix(')'))
            .ok_or_else(|| ctx.parse_err(format!("`{keyword}(` is missing its closing `)`")))?;
        let args: Vec<f64> = body
            .split([',', ' ', '\t'])
            .filter(|token| !token.is_empty())
            .map(|token| ctx.value(token, "source parameter"))
            .collect::<Result<_, _>>()?;
        let arg = |index: usize, fallback: f64| args.get(index).copied().unwrap_or(fallback);

        if keyword == "SIN" {
            if args.len() < 3 || args.len() > 5 {
                return Err(ctx.parse_err("expected `SIN(vo va freq [td [theta]])`"));
            }
            return Ok(Waveform::Sin {
                offset: args[0],
                amplitude: args[1],
                frequency: args[2],
                delay: arg(3, 0.0),
                theta: arg(4, 0.0),
            });
        }
        if args.len() < 2 || args.len() > 7 {
            return Err(ctx.parse_err("expected `PULSE(v1 v2 [td [tr [tf [pw [per]]]]])`"));
        }
        let period = arg(6, 0.0);
        let rise = arg(3, 0.0);
        let fall = arg(4, 0.0);
        let width = arg(5, period);
        if rise < 0.0 || fall < 0.0 || width < 0.0 || period < 0.0 {
            return Err(ctx.parse_err("PULSE times must not be negative"));
        }
        if period > 0.0 && rise + width + fall > period {
            return Err(ctx.parse_err(
                "PULSE tr + pw + tf is longer than per, so the pulse never returns to v1"
                    .to_string(),
            ));
        }
        return Ok(Waveform::Pulse {
            v1: args[0],
            v2: args[1],
            delay: arg(2, 0.0),
            rise,
            fall,
            width,
            period,
        });
    }

    let raw = match tokens.len() {
        1 => tokens[0],
        2 if tokens[0].eq_ignore_ascii_case("dc") => tokens[1],
        _ => {
            return Err(ctx.parse_err(format!(
                "expected `{shape}`, where `<source>` is `[dc] <value>`, \
                 `SIN(vo va freq [td [theta]])` or `PULSE(v1 v2 [td [tr [tf [pw [per]]]]])`"
            )))
        }
    };
    Ok(Waveform::Dc(ctx.value(raw, "source value")?))
}

/// Parse `.model <name> <type>(<param>=<value> ...)`.
///
/// Unknown parameters are accepted and dropped on purpose: a vendor model card
/// carries a dozen charge and temperature parameters this engine has no term
/// for, and refusing the card would mean the user has to hand-edit every
/// datasheet model to run it. The docs say what survives; [`super::device`]
/// says what it costs.
fn parse_model(
    models: &mut BTreeMap<String, ModelCard>,
    ctx: &LineCtx<'_>,
    tokens: &[&str],
) -> Result<(), AnalogError> {
    if tokens.len() < 2 {
        return Err(ctx.parse_err("expected `.model <name> <type>(<param>=<value> ...)`"));
    }
    let name = tokens[0];
    let key = name.to_ascii_lowercase();
    if models.contains_key(&key) {
        return Err(ctx.parse_err(format!("`.model {name}` is declared twice")));
    }

    // `NPN(BF=100)`, `NPN (BF=100)` and `NPN BF=100` all mean the same thing.
    let rest = tokens[1..].join(" ");
    let (kind, body) = match rest.find('(') {
        Some(at) => (
            rest[..at].trim().to_string(),
            rest[at + 1..]
                .trim_end()
                .strip_suffix(')')
                .ok_or_else(|| ctx.parse_err("`.model` is missing its closing `)`"))?
                .to_string(),
        ),
        None => {
            let mut parts = rest.splitn(2, char::is_whitespace);
            (
                parts.next().unwrap_or("").trim().to_string(),
                parts.next().unwrap_or("").to_string(),
            )
        }
    };

    let mut card = match kind.to_ascii_uppercase().as_str() {
        "D" => ModelCard::Diode(DiodeModel::default()),
        "NPN" => ModelCard::Bjt(BjtModel::defaults(Polarity::N)),
        "PNP" => ModelCard::Bjt(BjtModel::defaults(Polarity::P)),
        "NMOS" => ModelCard::Mos(MosModel::defaults(Polarity::N)),
        "PMOS" => ModelCard::Mos(MosModel::defaults(Polarity::P)),
        "OPAMP" => ModelCard::OpAmp(OpAmpModel::default()),
        "COMP" => ModelCard::Comparator(ComparatorModel::default()),
        "LDO" => ModelCard::Regulator(RegulatorModel::defaults(RegulatorKind::Linear)),
        "LDOADJ" => ModelCard::Regulator(RegulatorModel::defaults(RegulatorKind::LinearAdjustable)),
        "BUCK" => ModelCard::Regulator(RegulatorModel::defaults(RegulatorKind::Buck)),
        other => {
            return Err(ctx.parse_err(format!(
                "`.model` type `{other}` is not one of `D`, `NPN`, `PNP`, `NMOS`, `PMOS`, \
                 `OPAMP`, `COMP`, `LDO`, `LDOADJ`, `BUCK`"
            )))
        }
    };

    for item in body.split([',', ' ', '\t']).filter(|part| !part.is_empty()) {
        let (param, raw) = item
            .split_once('=')
            .ok_or_else(|| ctx.parse_err(format!("`{item}` is not a `<param>=<value>` pair")))?;
        // The one parameter that is a word, not a number.
        if let ModelCard::Comparator(model) = &mut card {
            if param.eq_ignore_ascii_case("OUTPUT") {
                model.output = match raw.to_ascii_uppercase().as_str() {
                    "OC" | "OD" => ComparatorOutput::OpenCollector,
                    "PP" => ComparatorOutput::PushPull,
                    other => {
                        return Err(ctx.parse_err(format!(
                            "comparator `OUTPUT={other}` is not `OC` (open collector), \
                             `OD` (open drain) or `PP` (push-pull)"
                        )))
                    }
                };
                continue;
            }
        }
        let value = ctx.value(raw, param)?;
        let param = param.to_ascii_uppercase();
        match (&mut card, param.as_str()) {
            (ModelCard::Diode(model), "IS") => model.is = value,
            (ModelCard::Diode(model), "N") => model.n = value,
            (ModelCard::Diode(model), "RS") => model.rs = value,
            (ModelCard::Diode(model), "BV") => model.bv = Some(value),
            (ModelCard::Diode(model), "IBV") => model.ibv = value,
            (ModelCard::Diode(model), "NBV") => model.nbv = Some(value),
            (ModelCard::OpAmp(model), "AOL") => model.aol = value,
            (ModelCard::OpAmp(model), "GBW") => model.gbw = value,
            (ModelCard::OpAmp(model), "ROUT") => model.rout = value,
            (ModelCard::OpAmp(model), "VOS") => model.vos = value,
            (ModelCard::OpAmp(model), "DROP_HI") => model.drop_hi = value,
            (ModelCard::OpAmp(model), "DROP_LO") => model.drop_lo = value,
            (ModelCard::Comparator(model), "AOL") => model.aol = value,
            (ModelCard::Comparator(model), "GBW") => model.gbw = value,
            (ModelCard::Comparator(model), "ROUT") => model.rout = value,
            (ModelCard::Comparator(model), "VOS") => model.vos = value,
            (ModelCard::Comparator(model), "VHYS") => model.vhys = value,
            // This engine's own card types have no vendor parameters to
            // tolerate, so a misspelt one is an error rather than a silently
            // ideal op-amp.
            (ModelCard::OpAmp(_), other) => {
                return Err(ctx.parse_err(format!(
                    "`{other}` is not an OPAMP parameter; expected AOL, GBW, ROUT, VOS, \
                     DROP_HI, DROP_LO"
                )))
            }
            (ModelCard::Regulator(model), param) => {
                let kind = model.kind;
                let allowed: &[&str] = match kind {
                    RegulatorKind::Linear => &["VOUT", "VDO", "ILIM", "IQ", "ROUT", "LINE"],
                    RegulatorKind::LinearAdjustable => {
                        &["VREF", "VDO", "ILIM", "IADJ", "ROUT", "LINE"]
                    }
                    RegulatorKind::Buck => &[
                        "VOUT", "VREF", "VDO", "DMAX", "ILIM", "IQ", "ROUT", "EFF", "AOL",
                    ],
                };
                if !allowed.contains(&param) {
                    return Err(ctx.parse_err(format!(
                        "`{param}` is not a {} parameter; expected {}",
                        ModelCard::Regulator(*model).kind(),
                        allowed.join(", ")
                    )));
                }
                match param {
                    "VOUT" => model.vout = value,
                    "VREF" => model.vref = value,
                    "VDO" => model.vdo = value,
                    "DMAX" => model.dmax = value,
                    "ILIM" => model.ilim = value,
                    "IQ" | "IADJ" => model.iq = value,
                    "ROUT" => model.rout = value,
                    "LINE" => model.line = value,
                    "EFF" => model.eff = value,
                    "AOL" => model.aol = value,
                    _ => unreachable!("checked against `allowed`"),
                }
            }
            (ModelCard::Comparator(_), other) => {
                return Err(ctx.parse_err(format!(
                    "`{other}` is not a COMP parameter; expected AOL, GBW, ROUT, VOS, VHYS, \
                     OUTPUT"
                )))
            }
            (ModelCard::Bjt(model), "IS") => model.is = value,
            (ModelCard::Bjt(model), "BF") => model.bf = value,
            (ModelCard::Bjt(model), "BR") => model.br = value,
            (ModelCard::Bjt(model), "NF") => model.nf = value,
            (ModelCard::Bjt(model), "NR") => model.nr = value,
            (ModelCard::Mos(model), "VTO" | "VT0") => model.vto = value,
            (ModelCard::Mos(model), "KP") => model.kp = value,
            (ModelCard::Mos(model), "LAMBDA") => model.lambda = value,
            (ModelCard::Mos(model), "W") => model.w = value,
            (ModelCard::Mos(model), "L") => model.l = value,
            // A level this engine cannot honour is the one parameter worth
            // refusing: silently solving a BSIM card with Shichman-Hodges
            // would be wrong by orders of magnitude, not by a capacitance.
            (ModelCard::Mos(_), "LEVEL") if value != 1.0 => {
                return Err(ctx.parse_err(format!(
                    "MOSFET `LEVEL={value}` is not modelled in-core; only level 1 \
                     (Shichman-Hodges) is. Use `adapter: external_process` with \
                     `tools/cosim/labwired_ngspice.py`"
                )))
            }
            _ => {}
        }
    }

    models.insert(key, card);
    Ok(())
}

// ---------------------------------------------------------------------------
// Op-amp and comparator lowering
// ---------------------------------------------------------------------------

/// Resistance of the op-amp's and comparator's internal gain nodes, ohms.
///
/// Any value gives the same transfer function (the transconductance and the
/// capacitor scale with it); 1 MΩ keeps a saturated stage's clamp current in
/// the tenths of an amp rather than the kiloamps a 1 Ω node would push through
/// its clamp diode, which is what keeps the clamp within millivolts of the rail.
pub const MACRO_GAIN_RESISTANCE: f64 = 1e6;

/// The internal clamp diode of the op-amp and comparator macros.
///
/// `N = 0.02` puts its thermal voltage at 0.52 mV: a knee 50× sharper than a
/// silicon junction, so a clamped output sits within ~15 mV of its rail at
/// half an amp of clamp current. It is an ordinary diode to the solver —
/// limited by `pnjlim`, stamped like any other — which is the point: the
/// saturation nonlinearity is one Newton already knows how to converge.
pub const MACRO_CLAMP_DIODE: DiodeModel = DiodeModel {
    is: 1e-12,
    n: 0.02,
    rs: 0.0,
    bv: None,
    ibv: 1e-3,
    nbv: None,
};

/// Loop gain of the push-pull comparator's output node, V/V from the 1 V
/// decision node. Large enough that the output is always hard against a rail
/// except for the few nanoseconds the decision node spends crossing.
pub const COMPARATOR_PP_GAIN: f64 = 1e4;

/// Builds the primitives one `X` line lowers into, named `<X name>#<part>`.
struct Lowering<'c> {
    circuit: &'c mut Circuit,
    prefix: String,
}

impl Lowering<'_> {
    fn node(&mut self, part: &str) -> NodeRef {
        let name = format!("{}#{part}", self.prefix);
        self.circuit.intern(&name)
    }

    fn name(&self, part: &str) -> String {
        format!("{}#{part}", self.prefix)
    }

    fn resistor(&mut self, part: &str, a: NodeRef, b: NodeRef, ohms: f64) {
        let name = self.name(part);
        self.circuit.resistors.push(Resistor { name, a, b, ohms });
    }

    fn capacitor(&mut self, part: &str, a: NodeRef, b: NodeRef, farads: f64) {
        let name = self.name(part);
        self.circuit.capacitors.push(Capacitor {
            name,
            a,
            b,
            farads,
            ic: None,
        });
    }

    fn vccs(&mut self, part: &str, p: NodeRef, n: NodeRef, cp: NodeRef, cn: NodeRef, gm: f64) {
        let name = self.name(part);
        self.circuit.vccs.push(Vccs {
            name,
            p,
            n,
            cp,
            cn,
            gm,
        });
    }

    fn current(&mut self, part: &str, p: NodeRef, n: NodeRef, amps: f64) {
        let name = self.name(part);
        self.circuit.current_sources.push(CurrentSource {
            name,
            p,
            n,
            dc: amps,
            wave: Waveform::Dc(amps),
        });
    }

    fn voltage(&mut self, part: &str, p: NodeRef, n: NodeRef, volts: f64) {
        let name = self.name(part);
        self.circuit.voltage_sources.push(VoltageSource {
            name,
            p,
            n,
            dc: volts,
            wave: Waveform::Dc(volts),
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn vcvs(&mut self, part: &str, p: NodeRef, n: NodeRef, cp: NodeRef, cn: NodeRef, gain: f64) {
        let name = self.name(part);
        self.circuit.vcvs.push(Vcvs {
            name,
            p,
            n,
            cp,
            cn,
            gain,
        });
    }

    fn clamp(&mut self, part: &str, anode: NodeRef, cathode: NodeRef) {
        let name = self.name(part);
        self.circuit.diodes.push(Diode {
            name,
            anode,
            cathode,
            junction_anode: anode,
            model_name: "clamp".to_string(),
            model: MACRO_CLAMP_DIODE,
            breakdown: None,
        });
    }

    /// A rail offset by `drop` volts toward the inside of the supply, or the
    /// rail itself when `drop` is zero (no extra unknowns).
    fn offset_rail(&mut self, part: &str, rail: NodeRef, drop: f64, below: bool) -> NodeRef {
        if drop == 0.0 {
            return rail;
        }
        let node = self.node(part);
        let source = format!("v{part}");
        if below {
            // v(rail) − v(node) = drop
            self.voltage(&source, rail, node, drop);
        } else {
            // v(node) − v(rail) = drop
            self.voltage(&source, node, rail, drop);
        }
        node
    }
}

fn macro_pins(
    ctx: &LineCtx<'_>,
    device: &PendingDevice,
    kind: &str,
) -> Result<[NodeRef; 5], AnalogError> {
    <[NodeRef; 5]>::try_from(device.nodes.as_slice()).map_err(|_| {
        ctx.parse_err(format!(
            "`{}` is an {kind} with {} pins; expected `X<name> in+ in- v+ v- out {}`",
            device.name,
            device.nodes.len(),
            device.model_name
        ))
    })
}

fn check_macro(ctx: &LineCtx<'_>, aol: f64, gbw: f64, rout: f64) -> Result<(), AnalogError> {
    if !(aol > 1.0 && gbw > 0.0 && rout > 0.0) {
        return Err(ctx.parse_err("an OPAMP/COMP model needs AOL > 1, GBW > 0 and ROUT > 0"));
    }
    Ok(())
}

/// Lower an op-amp `X` line; see [`OpAmpModel`] for the circuit.
fn lower_opamp(
    circuit: &mut Circuit,
    ctx: &LineCtx<'_>,
    device: &PendingDevice,
    model: OpAmpModel,
) -> Result<(), AnalogError> {
    let [inp, inn, vcc, vee, out] = macro_pins(ctx, device, "op-amp")?;
    check_macro(ctx, model.aol, model.gbw, model.rout)?;
    if model.drop_hi < 0.0 || model.drop_lo < 0.0 {
        return Err(ctx.parse_err("DROP_HI and DROP_LO must not be negative"));
    }
    let rg = MACRO_GAIN_RESISTANCE;
    let gm = model.aol / rg;
    let cg = model.aol / (2.0 * core::f64::consts::PI * model.gbw * rg);

    let mut lower = Lowering {
        circuit,
        prefix: device.name.clone(),
    };
    let p = lower.node("p");
    // gm·(v(in+) − v(in−)) flows from ground through the source into p.
    lower.vccs("g", None, p, inp, inn, gm);
    lower.resistor("rg", p, None, rg);
    lower.capacitor("cg", p, None, cg);
    if model.vos != 0.0 {
        lower.current("ios", None, p, gm * model.vos);
    }
    let hi = lower.offset_rail("hi", vcc, model.drop_hi, true);
    let lo = lower.offset_rail("lo", vee, model.drop_lo, false);
    lower.clamp("dhi", p, hi);
    lower.clamp("dlo", lo, p);
    // Thevenin v(p) behind ROUT, as a Norton pair: no extra unknown.
    lower.vccs("go", None, out, p, None, 1.0 / model.rout);
    lower.resistor("ro", out, None, model.rout);
    Ok(())
}

/// Lower a comparator `X` line; see [`ComparatorModel`] for the circuit.
fn lower_comparator(
    circuit: &mut Circuit,
    ctx: &LineCtx<'_>,
    device: &PendingDevice,
    model: ComparatorModel,
) -> Result<(), AnalogError> {
    let [inp, inn, vcc, vee, out] = macro_pins(ctx, device, "comparator")?;
    check_macro(ctx, model.aol, model.gbw, model.rout)?;
    if model.vhys < 0.0 {
        return Err(ctx.parse_err("VHYS must not be negative"));
    }
    let rq = MACRO_GAIN_RESISTANCE;
    let gm = model.aol / rq;
    let cq = model.aol / (2.0 * core::f64::consts::PI * model.gbw * rq);

    let mut lower = Lowering {
        circuit,
        prefix: device.name.clone(),
    };
    // The decision node q, referenced to v−: high means "in− is above in+",
    // i.e. the output should be LOW.
    let q = lower.node("q");
    lower.vccs("g", vee, q, inn, inp, gm);
    lower.resistor("rq", q, vee, rq);
    lower.capacitor("cq", q, vee, cq);
    if model.vos != 0.0 {
        // −gm·VOS into q, i.e. +VOS on v(in+) − v(in−).
        lower.current("ios", q, vee, gm * model.vos);
    }
    if model.vhys != 0.0 {
        // gm·VHYS·(v(q) − v(v−) − ½) into q: positive feedback that holds
        // whichever state q is in until the input crosses by VHYS/2.
        let gh = gm * model.vhys;
        lower.vccs("gh", vee, q, q, vee, gh);
        lower.current("ih", q, vee, 0.5 * gh);
    }
    let q_hi = lower.offset_rail("qhi", vee, 1.0, false);
    lower.clamp("dqh", q, q_hi);
    lower.clamp("dql", vee, q);

    match model.output {
        ComparatorOutput::OpenCollector => {
            // An N-channel switch, gate q against v−: VTO = 0.5 V is the middle
            // of q's span, and KP = 2/ROUT makes the on-resistance at
            // v(q) − v(v−) = 1 V exactly ROUT.
            let name = lower.name("mo");
            let kp = 2.0 / model.rout;
            let mos = MosModel {
                polarity: Polarity::N,
                vto: 0.5,
                kp,
                lambda: 0.0,
                w: 1.0,
                l: 1.0,
            };
            lower.circuit.mosfets.push(Mosfet {
                name,
                d: out,
                g: q,
                s: vee,
                bulk: vee,
                model_name: "comparator-output".to_string(),
                model: mos,
                beta: kp,
            });
        }
        ComparatorOutput::PushPull => {
            let p = lower.node("p");
            let rp = MACRO_GAIN_RESISTANCE;
            let gp = COMPARATOR_PP_GAIN / rp;
            // −gp·(v(q) − v(v−) − ½) into p, referenced to ground.
            lower.vccs("gp", p, None, q, vee, gp);
            lower.current("ip", None, p, 0.5 * gp);
            lower.resistor("rp", p, None, rp);
            lower.clamp("dph", p, vcc);
            lower.clamp("dpl", vee, p);
            lower.vccs("go", None, out, p, None, 1.0 / model.rout);
            lower.resistor("ro", out, None, model.rout);
        }
    }
    Ok(())
}

/// The pin order of a regulator `X` line, for the error when it is wrong.
fn regulator_shape(model: &RegulatorModel) -> &'static str {
    match model.kind {
        RegulatorKind::Linear => "in out gnd",
        RegulatorKind::LinearAdjustable => "in out adj",
        RegulatorKind::Buck if model.is_adjustable_buck() => "in out gnd fb",
        RegulatorKind::Buck => "in out gnd",
    }
}

/// Lower a regulator `X` line; see [`RegulatorModel`] for the circuit.
fn lower_regulator(
    circuit: &mut Circuit,
    ctx: &LineCtx<'_>,
    device: &PendingDevice,
    model: RegulatorModel,
) -> Result<(), AnalogError> {
    if model.kind == RegulatorKind::Buck && (model.vout > 0.0) == model.is_adjustable_buck() {
        return Err(ctx.parse_err(
            "a BUCK model sets exactly one of VOUT (fixed, pins `in out gnd`) and VREF \
             (adjustable, pins `in out gnd fb`)",
        ));
    }
    let shape = regulator_shape(&model);
    let wanted = shape.split(' ').count();
    if device.nodes.len() != wanted {
        return Err(ctx.parse_err(format!(
            "`{}` is a regulator with {} pins; expected `X<name> {shape} {}`",
            device.name,
            device.nodes.len(),
            device.model_name
        )));
    }
    let (input, out, reference) = (device.nodes[0], device.nodes[1], device.nodes[2]);
    let feedback = device.nodes.get(3).copied();
    let valid = model.ilim > 0.0
        && model.rout > 0.0
        && model.vdo >= 0.0
        && model.iq >= 0.0
        && model.dmax > 0.0
        && model.dmax <= 1.0
        && model.eff > 0.0
        && model.eff <= 1.0
        && model.aol > 1.0
        && model.line.is_finite();
    if !valid {
        return Err(ctx.parse_err(
            "a regulator model needs ILIM > 0, ROUT > 0, VDO >= 0, IQ >= 0, \
             0 < DMAX <= 1, 0 < EFF <= 1 and AOL > 1",
        ));
    }
    let adjustable_buck = model.is_adjustable_buck();
    let setpoint = if adjustable_buck {
        model.vref
    } else {
        model.setpoint()
    };
    if setpoint <= 0.0 {
        return Err(ctx.parse_err("a regulator's VOUT or VREF must be positive"));
    }

    let ra = MACRO_GAIN_RESISTANCE;
    let mut lower = Lowering {
        circuit,
        prefix: device.name.clone(),
    };

    // The set point a, as a ground-referenced voltage: what v(out) − v(ref)
    // should be.
    let a = lower.node("a");
    lower.resistor("ra", a, None, ra);
    match feedback {
        Some(fb) => {
            // Error amplifier: AOL/Ra · (VREF − (v(fb) − v(ref))) into a.
            let gm = model.aol / ra;
            lower.current("iref", None, a, gm * model.vref);
            lower.vccs("gerr", a, None, fb, reference, gm);
        }
        None => {
            // VSET plus LINE·(v(in) − v(ref) − VSET − VDO): the line term is
            // zero at the input where regulation begins.
            let base = setpoint - model.line * (setpoint + model.vdo);
            lower.current("iset", None, a, base / ra);
            if model.line != 0.0 {
                lower.vccs("gline", None, a, input, reference, model.line / ra);
            }
        }
    }

    // Dropout: a ≤ hd = max(DMAX·(v(in) − v(ref)) − VDO, 0). The max is a
    // clamp diode holding hf at ground through 1 kΩ when the headroom is
    // negative; an E buffers it into hd, so the set point's clamp current
    // never flows through that 1 kΩ.
    let h0 = lower.node("h0");
    lower.vcvs("eh", h0, None, input, reference, model.dmax);
    let h1 = lower.node("h1");
    lower.voltage("vh", h0, h1, model.vdo);
    let hf = lower.node("hf");
    lower.resistor("rh", h1, hf, 1e3);
    lower.clamp("dhz", None, hf);
    let hd = lower.node("hd");
    lower.vcvs("ehd", hd, None, hf, None, 1.0);
    lower.clamp("dh", a, hd);

    // Output: v(oi) = v(ref) + v(a), behind ROUT.
    let oi = lower.node("oi");
    lower.vcvs("eo", oi, reference, a, None, 1.0);
    lower.resistor("ro", oi, out, model.rout);
    let eo = lower.name("eo");

    // Current limit: a ≤ a + K·(ILIM − I(out)), i.e. the clamp conducts once
    // I(out) passes ILIM (by the clamp's few millivolts over K). The output
    // current comes from the output E's branch (H: v(m) = K·i(eo) = −K·I).
    let m = lower.node("m");
    let name = lower.name("hm");
    lower.circuit.ccvs.push(Ccvs {
        name,
        p: m,
        n: None,
        control: eo.clone(),
        control_branch: usize::MAX,
        ohms: REGULATOR_LIMIT_GAIN,
    });
    let l0 = lower.node("l0");
    lower.vcvs("el", l0, m, a, None, 1.0);
    let l1 = lower.node("l1");
    lower.voltage("vl", l1, l0, REGULATOR_LIMIT_GAIN * model.ilim);
    let name = lower.name("dl");
    lower.circuit.diodes.push(Diode {
        name,
        anode: a,
        cathode: l1,
        junction_anode: a,
        model_name: "limit-clamp".to_string(),
        model: REGULATOR_LIMIT_DIODE,
        breakdown: None,
    });

    // Input current. The output E draws I(out) from ref; a linear part takes
    // exactly that from in (F cancels it at ref), a buck takes the power.
    if model.kind == RegulatorKind::Buck {
        let name = lower.name("pin");
        lower.circuit.power_sinks.push(PowerSink {
            name,
            p: input,
            n: reference,
            op: out,
            on: reference,
            control: eo,
            control_branch: usize::MAX,
            eff: model.eff,
        });
    } else {
        let name = lower.name("fi");
        lower.circuit.cccs.push(Cccs {
            name,
            p: input,
            n: reference,
            control: eo,
            control_branch: usize::MAX,
            gain: -1.0,
        });
    }

    // Quiescent (or adjust-pin) current, from in into ref, in proportion to
    // how far the part is into regulation.
    if model.iq > 0.0 {
        match feedback {
            Some(fb) => lower.vccs("gq", input, reference, fb, reference, model.iq / model.vref),
            None => lower.vccs("gq", input, reference, a, None, model.iq / setpoint),
        }
    }
    Ok(())
}

#[allow(clippy::type_complexity)]
fn two_nodes_value_ic<'a>(
    ctx: &LineCtx<'_>,
    tokens: &[&'a str],
    shape: &str,
) -> Result<(&'a str, &'a str, &'a str, Option<&'a str>), AnalogError> {
    match tokens.len() {
        3 => Ok((tokens[0], tokens[1], tokens[2], None)),
        4 => {
            let (key, value) = tokens[3]
                .split_once('=')
                .ok_or_else(|| ctx.parse_err(format!("expected `{shape}`")))?;
            if !key.eq_ignore_ascii_case("ic") {
                return Err(ctx.parse_err(format!("unknown parameter `{key}`; expected `{shape}`")));
            }
            Ok((tokens[0], tokens[1], tokens[2], Some(value)))
        }
        _ => Err(ctx.parse_err(format!("expected `{shape}`"))),
    }
}

fn parse_ic(circuit: &mut Circuit, ctx: &LineCtx<'_>, tokens: &[&str]) -> Result<(), AnalogError> {
    // `.ic V(a)=5 V(b)=1.25`, tolerating spaces around `=`.
    let joined = tokens.join(" ").replace(" =", "=").replace("= ", "=");
    if joined.trim().is_empty() {
        return Err(ctx.parse_err("expected `.ic V(node)=<value>`"));
    }
    for item in joined.split_whitespace() {
        let (lhs, rhs) = item
            .split_once('=')
            .ok_or_else(|| ctx.parse_err("expected `.ic V(node)=<value>`"))?;
        let lhs = lhs.trim();
        let node_name = lhs
            .strip_prefix('V')
            .or_else(|| lhs.strip_prefix('v'))
            .and_then(|rest| rest.strip_prefix('('))
            .and_then(|rest| rest.strip_suffix(')'))
            .ok_or_else(|| {
                ctx.parse_err(format!(
                    "`{lhs}` is not a node reference; expected `V(node)`"
                ))
            })?;
        let value = ctx.value(rhs, "initial condition")?;
        let node = circuit.intern(node_name);
        match node {
            Some(index) => circuit.node_ic.push((index, value)),
            None => return Err(ctx.parse_err("`.ic` cannot set the ground node")),
        }
    }
    Ok(())
}
