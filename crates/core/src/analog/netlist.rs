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
//! | Switch | `S<name> n1 n2 <ctrl> ron=<r> roff=<r>` |
//! | Diode | `D<name> n+ n- <model>` |
//! | BJT | `Q<name> nc nb ne <model>` |
//! | MOSFET | `M<name> nd ng ns nb <model> [w=<m>] [l=<m>]` |
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
}

impl Default for DiodeModel {
    fn default() -> Self {
        Self {
            is: 1e-14,
            n: 1.0,
            rs: 0.0,
        }
    }
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
}

impl ModelCard {
    /// The card's SPICE type keyword, for diagnostics.
    fn kind(&self) -> &'static str {
        match self {
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

/// `S<name> n1 n2 <ctrl> ron=<r> roff=<r>` — an ideal switch whose control is a
/// routed boolean input name, not a circuit node.
#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    /// Element name as written.
    pub name: String,
    /// First terminal.
    pub a: NodeRef,
    /// Second terminal.
    pub b: NodeRef,
    /// Name of the routed boolean input that opens and closes it.
    pub ctrl: String,
    /// Closed resistance in ohms.
    pub ron: f64,
    /// Open resistance in ohms.
    pub roff: f64,
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
    /// per inductor, in that order.
    pub fn branch_count(&self) -> usize {
        self.voltage_sources.len() + self.inductors.len()
    }

    /// Branch-current index of the named voltage source or inductor.
    pub fn branch_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        if let Some(index) = self
            .voltage_sources
            .iter()
            .position(|source| source.name.to_ascii_lowercase() == key)
        {
            return Some(index);
        }
        self.inductors
            .iter()
            .position(|inductor| inductor.name.to_ascii_lowercase() == key)
            .map(|index| self.voltage_sources.len() + index)
    }

    /// Name of the element owning branch current `index`.
    pub fn branch_name(&self, index: usize) -> &str {
        if index < self.voltage_sources.len() {
            &self.voltage_sources[index].name
        } else {
            &self.inductors[index - self.voltage_sources.len()].name
        }
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
    }

    /// True when the circuit holds at least one device whose stamp depends on
    /// the solution, i.e. when a step needs Newton iteration.
    ///
    /// This is the switch that keeps the linear engine exactly what it was:
    /// when it is false, [`super::mna::Solver::advance`] runs the same code,
    /// in the same order, on the same values as before diodes existed.
    pub fn is_nonlinear(&self) -> bool {
        !self.diodes.is_empty() || !self.bjts.is_empty() || !self.mosfets.is_empty()
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
        if !matches!(letter, 'R' | 'C' | 'L' | 'V' | 'I' | 'S' | 'D' | 'Q' | 'M') {
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
            'S' => {
                let shape = "S<name> n1 n2 <ctrl> ron=<r> roff=<r>";
                let rest = &tokens[1..];
                if rest.len() != 5 {
                    return Err(ctx.parse_err(format!("expected `{shape}`")));
                }
                let mut ron = None;
                let mut roff = None;
                for token in &rest[3..] {
                    let (key, value) = token
                        .split_once('=')
                        .ok_or_else(|| ctx.parse_err(format!("expected `{shape}`")))?;
                    match key.to_ascii_lowercase().as_str() {
                        "ron" => ron = Some(ctx.value(value, "ron")?),
                        "roff" => roff = Some(ctx.value(value, "roff")?),
                        other => {
                            return Err(ctx.parse_err(format!("unknown switch parameter `{other}`")))
                        }
                    }
                }
                let (Some(ron), Some(roff)) = (ron, roff) else {
                    return Err(ctx.parse_err(format!("expected `{shape}`")));
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
                });
            }
            _ => unreachable!("element letter filtered above"),
        }
    }

    bind_models(&mut circuit, &models, pending)?;
    Ok(circuit)
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
        let card = models
            .get(&key)
            .copied()
            .or_else(|| builtin_model(&device.model_name))
            .ok_or_else(|| {
                ctx.parse_err(format!(
                    "no `.model {}` line, and `{}` is not one of the built-in models \
                     `D`, `NPN`, `PNP`, `NMOS`, `PMOS`",
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
        other => {
            return Err(ctx.parse_err(format!(
                "`.model` type `{other}` is not one of `D`, `NPN`, `PNP`, `NMOS`, `PMOS`"
            )))
        }
    };

    for item in body.split([',', ' ', '\t']).filter(|part| !part.is_empty()) {
        let (param, raw) = item
            .split_once('=')
            .ok_or_else(|| ctx.parse_err(format!("`{item}` is not a `<param>=<value>` pair")))?;
        let value = ctx.value(raw, param)?;
        let param = param.to_ascii_uppercase();
        match (&mut card, param.as_str()) {
            (ModelCard::Diode(model), "IS") => model.is = value,
            (ModelCard::Diode(model), "N") => model.n = value,
            (ModelCard::Diode(model), "RS") => model.rs = value,
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
