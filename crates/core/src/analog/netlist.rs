// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Parser for the SPICE subset the in-core analog engine solves.
//!
//! The subset is deliberately small — linear two-terminal elements, independent
//! DC sources and ideal switches — because that is exactly what a deterministic
//! MNA transient solver can integrate exactly, in the browser, with no
//! dependencies. Anything outside it (diodes, transistors, subcircuits, model
//! libraries) is not approximated and not silently dropped: it is a hard error
//! that names the line and points at the ngspice adapter, which does support it.
//!
//! | Element | Line |
//! |---|---|
//! | Resistor | `R<name> n1 n2 <value>` |
//! | Capacitor | `C<name> n1 n2 <value> [ic=<v>]` |
//! | Inductor | `L<name> n1 n2 <value> [ic=<i>]` |
//! | Voltage source | `V<name> n+ n- dc <value>` |
//! | Current source | `I<name> n+ n- dc <value>` |
//! | Switch | `S<name> n1 n2 <ctrl> ron=<r> roff=<r>` |
//!
//! Plus `*` comments, `;`/`$` trailing comments, `.end` and
//! `.ic V(node)=<v>`. Node `0` and `gnd` are ground.
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

/// `V<name> n+ n- dc <volts>`.
#[derive(Debug, Clone, PartialEq)]
pub struct VoltageSource {
    /// Element name as written, e.g. `Vgpio`.
    pub name: String,
    /// Positive terminal.
    pub p: NodeRef,
    /// Negative terminal.
    pub n: NodeRef,
    /// DC value from the netlist; routed inputs replace it once time runs.
    pub dc: f64,
}

/// `I<name> n+ n- dc <amps>`. Positive current flows from `p` through the
/// source to `n`, as in SPICE.
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentSource {
    /// Element name as written.
    pub name: String,
    /// Positive terminal.
    pub p: NodeRef,
    /// Negative terminal.
    pub n: NodeRef,
    /// DC value from the netlist; routed inputs replace it once time runs.
    pub dc: f64,
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

/// Parse a netlist into a [`Circuit`].
pub fn parse_netlist(text: &str) -> Result<Circuit, AnalogError> {
    let mut circuit = Circuit::default();
    let mut ended = false;

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
        if ended {
            return Err(ctx.parse_err("netlist continues after `.end`"));
        }

        let tokens: Vec<&str> = line.split_whitespace().collect();
        let head = tokens[0];

        if let Some(directive) = head.strip_prefix('.') {
            match directive.to_ascii_lowercase().as_str() {
                "end" => ended = true,
                "ic" => parse_ic(&mut circuit, &ctx, &tokens[1..])?,
                _ => return Err(ctx.unsupported()),
            }
            continue;
        }

        let letter = head
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or(' ');
        if !matches!(letter, 'R' | 'C' | 'L' | 'V' | 'I' | 'S') {
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
                    "V<name> n+ n- dc <value>"
                } else {
                    "I<name> n+ n- dc <value>"
                };
                let rest = &tokens[1..];
                if rest.len() < 3 {
                    return Err(ctx.parse_err(format!("expected `{shape}`")));
                }
                let (np, nn) = (rest[0], rest[1]);
                let value = match rest.len() {
                    3 => rest[2],
                    4 if rest[2].eq_ignore_ascii_case("dc") => rest[3],
                    _ => {
                        return Err(ctx.parse_err(format!(
                            "expected `{shape}`; only `dc` sources are modelled in-core"
                        )))
                    }
                };
                let dc = ctx.value(value, "source value")?;
                let p = circuit.intern(np);
                let n = circuit.intern(nn);
                if letter == 'V' {
                    circuit.voltage_sources.push(VoltageSource {
                        name: head.to_string(),
                        p,
                        n,
                        dc,
                    });
                } else {
                    circuit.current_sources.push(CurrentSource {
                        name: head.to_string(),
                        p,
                        n,
                        dc,
                    });
                }
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

    Ok(circuit)
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
