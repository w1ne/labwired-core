// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **The `logic_gate` primitive's schema** — a 74-series part as a truth table.
//!
//! # Why a register map cannot reach this family
//!
//! A 39-project KiCad corpus run through the importer leaves 74-series logic as
//! the third largest model gap: 139 dropped symbols, led by `SN74LVTH125` (38×)
//! and `SN74LVC1T45` (32×). None of them can be a `i2c_device` or a
//! `spi_device`, because **a gate has no bus**. It has no address, no register,
//! nothing to write and nothing to read back. What it has is a set of input
//! pads, a set of output pads, a boolean function from one to the other, an
//! enable that can take the outputs off the wire entirely, and a propagation
//! delay. That is the whole datasheet, and that is exactly this schema.
//!
//! # The three ways a part says what its outputs do
//!
//! Exactly one of these must be present, because they are three different
//! parts, not three spellings of one:
//!
//! * [`table`](LogicSpec::table) — a **combinational gate**. Each output role
//!   maps to a boolean expression over the input roles: `Y1: "A1"` (a buffer),
//!   `Y1: "!(A1 & B1)"` (a NAND), `Y1: "A1 ^ B1"` (an XOR). This covers
//!   '04/'00/'08/'32/'86/'125 and everything shaped like them.
//! * [`direction`](LogicSpec::direction) — a **bidirectional transceiver**
//!   ('245, LVC1T45). One pad pair per bit; a DIR pad decides which side of
//!   each pair is the input and which is the output. The same pad is an input
//!   in one direction and an output in the other, which is why a transceiver
//!   role is bound to BOTH ends of its pad rather than to one.
//! * [`select`](LogicSpec::select) — a **bus switch / 1-of-2 mux**
//!   ('CBTLV3257). One S pad picks, per output, between an A-side and a B-side
//!   input.
//!
//! A `select` is expressible as a `table` (`Y1: "(!S & A1) | (S & B1)"`) and a
//! `direction` is not. Both are kept as their own block anyway, because a
//! reviewer holding the datasheet is checking "is the A side listed in pin
//! order" and not "is the mux algebra right" — and because a `direction` part
//! needs a genuinely different pad binding that the engine has to be told
//! about, not infer from the shape of an expression.
//!
//! # The expression grammar is the one we already have
//!
//! [`crate::expr`] is the Phase C integer expression language every Tier-2
//! rule guard is written in — `! & | ^ ~ ( )`, C precedence with the
//! datasheet-shaped `==`/`&` exception. A truth-table entry is compiled with
//! THAT parser, not a second one: [`compile_table_expr`] rewrites each bare pin
//! name to the `var(NAME)` call the grammar already has and hands the result to
//! [`Expr::parse`](crate::expr::Expr::parse). So the operators, the precedence,
//! the parenthesis rules and the error messages are shared, and a gate cannot
//! develop its own dialect of `&`.
//!
//! The consequence a part author sees: **a pin role must be a plain
//! identifier** — `[A-Za-z_][A-Za-z0-9_]*`. The datasheets spell the '125's
//! gates `1A`/`1Y`/`1OE`, which starts with a digit and is not one, so the
//! in-tree descriptors use `A1`/`Y1`/`OE1`. [`validate`](LogicSpec::validate)
//! rejects anything else by name rather than letting it fail as a parse error
//! nobody can read.
//!
//! # Hi-Z, honestly
//!
//! See [`LogicDrive::hiz_when_disabled`]. Short version: this twin has no
//! resistor network, so "Hi-Z" is "the part stops driving" and the pad keeps
//! whatever level it last had. It is not "the pad floats to the MCU's pull-up",
//! because the MCU's pull configuration is not modelled as a level source.

use crate::*;

/// A pin role name a truth-table expression can mention.
///
/// Deliberately the C identifier rule and nothing more: these names become
/// `var(NAME)` in the shared expression grammar, so a role that is not an
/// identifier is not spellable in a table at all.
pub fn is_valid_role(role: &str) -> bool {
    let mut chars = role.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Which level of an enable pin turns its outputs ON.
///
/// Defaults to `low` because every 74-series output enable in the corpus is
/// `OE`-bar: '125, '244, '245, 'LVC1T45 and 'CBTLV3257 all enable on a LOW.
/// A default of `high` would be silently wrong for all of them.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActiveLevel {
    #[default]
    Low,
    High,
}

impl ActiveLevel {
    /// Is `level` the asserted one?
    pub fn asserted(self, level: bool) -> bool {
        match self {
            ActiveLevel::Low => !level,
            ActiveLevel::High => level,
        }
    }
}

/// One output-enable pad and the outputs it gates.
///
/// A '125 has FOUR of these, one per gate, which is the whole reason this is a
/// list of `{ pin, outputs }` rather than one part-wide enable pin: modelling
/// the '125 with a single OE would make all four buffers switch together, and
/// the resulting part would pass a one-gate test and be wrong on the board.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogicEnable {
    /// Role name of the enable pad (an MCU output this part observes).
    pub pin: String,
    /// Level that ENABLES. See [`ActiveLevel`] for why this defaults to `low`.
    #[serde(default)]
    pub active: ActiveLevel,
    /// Output roles this enable gates. Every name must be in
    /// [`LogicSpec::outputs`].
    pub outputs: Vec<String>,
}

/// A bidirectional transceiver's direction control.
///
/// `a` and `b` are the two sides, paired BY INDEX: `a[i]` and `b[i]` are the
/// two ends of one bit. When the DIR pad reads
/// [`a_to_b_when`](Self::a_to_b_when) the A side is observed and the B side is
/// driven; otherwise the other way round.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogicDirection {
    /// Role name of the direction pad.
    pub pin: String,
    /// DIR level that means "A drives B". Datasheet default for the '245 and
    /// the LVC1T45 is 1.
    #[serde(default = "one")]
    pub a_to_b_when: u8,
    /// A-side roles, in bit order.
    pub a: Vec<String>,
    /// B-side roles, in the SAME bit order.
    pub b: Vec<String>,
}

fn one() -> u8 {
    1
}

/// A bus switch / 1-of-2 multiplexer.
///
/// [`low`](Self::low) and [`high`](Self::high) are paired by index with
/// [`LogicSpec::outputs`]: output `i` follows `low[i]` while the S pad reads 0
/// and `high[i]` while it reads 1.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogicSelect {
    /// Role name of the select pad.
    pub pin: String,
    /// Input role each output follows when S is LOW, in output order.
    pub low: Vec<String>,
    /// Input role each output follows when S is HIGH, in output order.
    pub high: Vec<String>,
}

/// What a disabled output does.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogicDrive {
    /// `true` (the default, and the truth for every part in this family): a
    /// disabled output is **released**, not forced.
    ///
    /// ## What Hi-Z means on this twin
    ///
    /// The engine's pad model has exactly two level sources: what the MCU
    /// drives out of a pin configured as an output, and the external level a
    /// bus-resident device last applied through
    /// [`DevicePins`](../../labwired_core/bus/trait.DevicePins.html). There is
    /// no resistor network and no bus arbitration: `GpioPort` computes the
    /// input word as "ODR for the bits driven push-pull, the latched external
    /// level for the rest". Nothing decays, and nothing floats.
    ///
    /// So a released output here means the part stops WRITING the pad, and the
    /// pad holds whatever level it last held — the part's own last driven
    /// value, or the MCU's own output if firmware has since turned the pin
    /// around. That is the correct model for the case this exists for (a '245
    /// whose OE is high is off the wire and another driver owns it) and it is
    /// an approximation for the case where nothing else drives the net at all,
    /// where silicon would float to the MCU's internal pull and this model
    /// stays put.
    ///
    /// Set `false` for the rare part whose disabled output is actively pulled
    /// to 0 rather than released; the engine then drives a LOW.
    #[serde(default = "yes")]
    pub hiz_when_disabled: bool,
}

fn yes() -> bool {
    true
}

impl Default for LogicDrive {
    fn default() -> Self {
        Self {
            hiz_when_disabled: true,
        }
    }
}

/// The `logic:` block of a `logic_gate` descriptor.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogicSpec {
    /// Input roles — pads the MCU drives and this part observes.
    ///
    /// Enable, direction and select pads are NOT listed here; they are named by
    /// their own blocks, and listing one twice is a load error.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Output roles — pads this part drives and the MCU samples.
    pub outputs: Vec<String>,
    /// Per-output enables (see [`LogicEnable`]).
    #[serde(default)]
    pub enables: Vec<LogicEnable>,
    /// Transceiver direction control (see [`LogicDirection`]).
    #[serde(default)]
    pub direction: Option<LogicDirection>,
    /// Bus-switch select (see [`LogicSelect`]).
    #[serde(default)]
    pub select: Option<LogicSelect>,
    /// Output role → boolean expression over the input roles.
    #[serde(default)]
    pub table: BTreeMap<String, String>,
    /// Propagation delay, input transition to output transition.
    ///
    /// Converted to simulated CYCLES at attach, with a **floor of one cycle**:
    /// a gate is never instantaneous, and a zero-cycle model would let firmware
    /// write an input and read the answer in the same store, which no real part
    /// does. At 80 MHz one cycle is 12.5 ns, which is the right order for the
    /// 5–15 ns this family actually specifies.
    #[serde(default)]
    pub tprop_ns: u64,
    /// Disabled-output behaviour (see [`LogicDrive`]).
    #[serde(default)]
    pub drive: LogicDrive,
}

/// Which mechanism a descriptor uses to say what its outputs do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicKind {
    /// [`LogicSpec::table`] — a combinational gate.
    Table,
    /// [`LogicSpec::direction`] — a bidirectional transceiver.
    Transceiver,
    /// [`LogicSpec::select`] — a 1-of-2 bus switch.
    Switch,
}

impl LogicSpec {
    /// Which of the three shapes this is. Errors when zero or more than one is
    /// present — they are different parts, not alternative spellings.
    pub fn kind(&self) -> Result<LogicKind> {
        let present: Vec<(&str, LogicKind)> = [
            (!self.table.is_empty()).then_some(("table", LogicKind::Table)),
            self.direction
                .is_some()
                .then_some(("direction", LogicKind::Transceiver)),
            self.select
                .is_some()
                .then_some(("select", LogicKind::Switch)),
        ]
        .into_iter()
        .flatten()
        .collect();
        match present.as_slice() {
            [(_, kind)] => Ok(*kind),
            [] => anyhow::bail!(
                "a logic_gate declares none of `table:`, `direction:` or `select:`, so nothing \
                 says what its outputs do — it would attach, drive nothing, and look like a \
                 working part"
            ),
            many => anyhow::bail!(
                "a logic_gate declares {} at once ({}). They describe three different parts \
                 (a combinational gate, a transceiver, a bus switch); pick one",
                many.len(),
                many.iter()
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
        }
    }

    /// Every role this part binds to a pad, in a stable order: inputs, then
    /// outputs, then the control pads. Transceiver sides are included through
    /// `inputs`/`outputs`, which is where a `direction` part lists them.
    pub fn roles(&self) -> Vec<String> {
        let mut out: Vec<String> = self.inputs.clone();
        out.extend(self.outputs.iter().cloned());
        out.extend(self.enables.iter().map(|e| e.pin.clone()));
        if let Some(d) = &self.direction {
            out.push(d.pin.clone());
        }
        if let Some(s) = &self.select {
            out.push(s.pin.clone());
        }
        out
    }

    /// Control pads — enables, direction, select. Always observed, never driven.
    pub fn control_pins(&self) -> Vec<String> {
        let mut out: Vec<String> = self.enables.iter().map(|e| e.pin.clone()).collect();
        if let Some(d) = &self.direction {
            out.push(d.pin.clone());
        }
        if let Some(s) = &self.select {
            out.push(s.pin.clone());
        }
        out
    }

    /// Full static validation. Every failure names the offending role, so a
    /// manifest preflight rejection reads like a review comment.
    pub fn validate(&self, part: &str) -> Result<()> {
        let kind = self
            .kind()
            .with_context(|| format!("logic_gate '{part}'"))?;

        anyhow::ensure!(
            !self.outputs.is_empty(),
            "logic_gate '{part}' declares no `outputs:` — it drives no pad"
        );

        // ── role hygiene ───────────────────────────────────────────────────
        //
        // A role may appear in BOTH `inputs:` and `outputs:` — and only for a
        // transceiver, where that overlap is the whole point: the same pad is
        // read in one direction and driven in the other. Everywhere else a
        // repeat is a typo that would make one pad answer to two meanings in
        // one pass, so each list is checked for repeats on its own and the
        // control pads are checked against everything.
        let controls = self.control_pins();
        for (what, list) in [
            ("inputs", &self.inputs),
            ("outputs", &self.outputs),
            ("control pins", &controls),
        ] {
            let mut seen: HashSet<&str> = HashSet::new();
            for role in list {
                anyhow::ensure!(
                    is_valid_role(role),
                    "logic_gate '{part}' pin role '{role}' is not an identifier. A role name \
                     becomes `var({role})` in the shared expression grammar, so it must match \
                     [A-Za-z_][A-Za-z0-9_]* — the datasheet's `1A` is spelled `A1` here"
                );
                anyhow::ensure!(
                    seen.insert(role.as_str()),
                    "logic_gate '{part}' names pin role '{role}' twice in `{what}`. One pad \
                     is one role"
                );
            }
        }
        for role in &controls {
            anyhow::ensure!(
                !self.inputs.contains(role) && !self.outputs.contains(role),
                "logic_gate '{part}' uses '{role}' as a control pad AND as a signal pin. An \
                 enable, a direction or a select pad is observed on its own; naming it twice \
                 would read one pad for two different things in the same pass"
            );
        }
        if kind != LogicKind::Transceiver {
            for role in &self.outputs {
                anyhow::ensure!(
                    !self.inputs.contains(role),
                    "logic_gate '{part}' lists '{role}' as both an input and an output. Only a \
                     `direction:` transceiver turns a pad around; anywhere else this is a \
                     part that would drive the pad it is reading"
                );
            }
        }

        let inputs: HashSet<&str> = self.inputs.iter().map(String::as_str).collect();
        let outputs: HashSet<&str> = self.outputs.iter().map(String::as_str).collect();

        // ── enables ────────────────────────────────────────────────────────
        for en in &self.enables {
            anyhow::ensure!(
                !en.outputs.is_empty(),
                "logic_gate '{part}' enable '{}' gates no outputs",
                en.pin
            );
            for y in &en.outputs {
                anyhow::ensure!(
                    outputs.contains(y.as_str()),
                    "logic_gate '{part}' enable '{}' gates '{y}', which is not one of its \
                     `outputs:`",
                    en.pin
                );
            }
        }

        // ── the three shapes ───────────────────────────────────────────────
        match kind {
            LogicKind::Table => {
                for y in &self.outputs {
                    anyhow::ensure!(
                        self.table.contains_key(y),
                        "logic_gate '{part}' output '{y}' has no `table:` entry, so it would \
                         sit at a level nothing computed"
                    );
                }
                for (y, expr) in &self.table {
                    anyhow::ensure!(
                        outputs.contains(y.as_str()),
                        "logic_gate '{part}' has a `table:` entry for '{y}', which is not one \
                         of its `outputs:`"
                    );
                    let names = compile_table_expr(expr).with_context(|| {
                        format!("logic_gate '{part}' table entry '{y}: {expr}'")
                    })?;
                    for name in names.1 {
                        anyhow::ensure!(
                            inputs.contains(name.as_str()),
                            "logic_gate '{part}' table entry '{y}' reads '{name}', which is \
                             not one of its `inputs:`. An undeclared name would silently \
                             evaluate to 0 and the gate would look stuck"
                        );
                    }
                }
            }
            LogicKind::Transceiver => {
                let d = self.direction.as_ref().expect("kind said transceiver");
                anyhow::ensure!(
                    d.a.len() == d.b.len(),
                    "logic_gate '{part}' direction pairs {} A-side roles with {} B-side roles; \
                     they are matched by index, one pad pair per bit",
                    d.a.len(),
                    d.b.len()
                );
                anyhow::ensure!(
                    !d.a.is_empty(),
                    "logic_gate '{part}' direction lists no bits"
                );
                anyhow::ensure!(
                    d.a_to_b_when <= 1,
                    "logic_gate '{part}' direction a_to_b_when is {}, and a pad has two levels",
                    d.a_to_b_when
                );
                // BOTH sides of a transceiver are pads this part may drive AND
                // observe, so both must be declared in `inputs:` and
                // `outputs:` — that is what tells attach to bind both ends.
                for role in d.a.iter().chain(d.b.iter()) {
                    anyhow::ensure!(
                        inputs.contains(role.as_str()) && outputs.contains(role.as_str()),
                        "logic_gate '{part}' transceiver role '{role}' must appear in BOTH \
                         `inputs:` and `outputs:` — a transceiver pad is read in one \
                         direction and driven in the other, and attach binds both ends of it"
                    );
                }
            }
            LogicKind::Switch => {
                let s = self.select.as_ref().expect("kind said switch");
                anyhow::ensure!(
                    s.low.len() == self.outputs.len() && s.high.len() == self.outputs.len(),
                    "logic_gate '{part}' select lists {} low-side and {} high-side inputs for \
                     {} outputs; all three are matched by index",
                    s.low.len(),
                    s.high.len(),
                    self.outputs.len()
                );
                for role in s.low.iter().chain(s.high.iter()) {
                    anyhow::ensure!(
                        inputs.contains(role.as_str()),
                        "logic_gate '{part}' select reads '{role}', which is not one of its \
                         `inputs:`"
                    );
                }
            }
        }
        Ok(())
    }
}

/// Compile one truth-table expression, returning the tree and the pin roles it
/// reads.
///
/// The rewrite is the whole trick: a bare pin name is turned into the
/// `var(NAME)` call [`crate::expr`] already understands, so the operators,
/// precedence and error positions are the shared ones and a gate never grows
/// its own parser. Byte offsets in a returned [`ExprError`] refer to the
/// rewritten string, so the message re-states the original source instead of
/// pointing at a column the author cannot see.
pub fn compile_table_expr(src: &str) -> Result<(expr::Expr, Vec<String>)> {
    let mut rewritten = String::with_capacity(src.len() * 5);
    let mut names: Vec<String> = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_whitespace() {
            rewritten.push(' ');
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len() {
                let c = bytes[i] as char;
                if c.is_ascii_alphanumeric() || c == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let name = &src[start..i];
            // A name followed by `(` is a function call, and the only functions
            // in this grammar read a register file a gate does not have.
            if src[i..].trim_start().starts_with('(') {
                anyhow::bail!(
                    "'{name}(' is a function call. A truth table is boolean algebra over pin \
                     names only — a gate has no registers, no variables and no stimulus \
                     channels to call into"
                );
            }
            rewritten.push_str("var(");
            rewritten.push_str(name);
            rewritten.push(')');
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
            continue;
        }
        if matches!(c, '0' | '1') {
            // Constant-0 / constant-1 outputs are legitimate (a tied-off gate).
            rewritten.push(c);
            i += 1;
            continue;
        }
        if matches!(c, '!' | '&' | '|' | '^' | '~' | '(' | ')') {
            rewritten.push(c);
            i += 1;
            continue;
        }
        anyhow::bail!(
            "'{c}' is not part of the truth-table grammar. A table entry is `! & | ^ ~`, \
             parentheses, pin names and the constants 0 and 1"
        );
    }
    let tree = expr::Expr::parse(&rewritten)
        .map_err(|e| anyhow::anyhow!("{}", e.message))
        .with_context(|| format!("could not parse the boolean expression '{src}'"))?;
    Ok((tree, names))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The context a table expression is evaluated against in the tests here:
    /// pin levels and nothing else, exactly as the engine supplies.
    struct Pins(BTreeMap<&'static str, i64>);

    impl expr::EvalCtx for Pins {
        fn reg(&self, _: &str) -> i64 {
            0
        }
        fn field(&self, _: &str, _: &str) -> i64 {
            0
        }
        fn var(&self, name: &str) -> i64 {
            self.0.get(name).copied().unwrap_or(0)
        }
        /// A logic gate's pads ARE its vars here — the table expressions name
        /// them through `var()`, which is the spelling `logic:` has always
        /// used — so `pin()` resolves against the same map rather than
        /// answering 0 for a pad this fixture is holding high.
        fn pin(&self, name: &str) -> i64 {
            self.0.get(name).copied().unwrap_or(0)
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

    fn eval(src: &str, pins: &[(&'static str, i64)]) -> i64 {
        let (tree, _) = compile_table_expr(src).expect("compiles");
        let ctx = Pins(pins.iter().copied().collect());
        tree.eval(&ctx)
    }

    #[test]
    fn a_bare_pin_name_is_a_buffer() {
        assert_eq!(eval("A1", &[("A1", 1)]), 1);
        assert_eq!(eval("A1", &[("A1", 0)]), 0);
    }

    #[test]
    fn an_inverter_is_bang() {
        assert_eq!(eval("!A", &[("A", 0)]), 1);
        assert_eq!(eval("!A", &[("A", 1)]), 0);
    }

    #[test]
    fn and_or_xor_nand_follow_their_truth_tables() {
        for (a, b) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let p = [("A", a), ("B", b)];
            assert_eq!(eval("A & B", &p), a & b, "AND {a},{b}");
            assert_eq!(eval("A | B", &p), a | b, "OR {a},{b}");
            assert_eq!(eval("A ^ B", &p), a ^ b, "XOR {a},{b}");
            assert_eq!(eval("!(A & B)", &p), 1 - (a & b), "NAND {a},{b}");
            assert_eq!(eval("!(A | B)", &p), 1 - (a | b), "NOR {a},{b}");
        }
    }

    /// `!` is a LOGICAL not in this grammar (`!0 == 1`, `!1 == 0`), which is
    /// what a one-bit pad wants. `~` is the bitwise complement and would give
    /// `-1` for a low pad — still non-zero, so an engine that treated the
    /// result as "any non-zero is high" would read `~1` as HIGH. The engine
    /// normalises with `!= 0` and this pins the difference so nobody writes
    /// `~A` meaning an inverter.
    #[test]
    fn bitwise_not_is_not_an_inverter() {
        assert_eq!(eval("!A", &[("A", 1)]), 0);
        assert_eq!(eval("~A", &[("A", 1)]), -2);
    }

    #[test]
    fn a_mux_is_expressible_without_the_select_block() {
        // The algebra `select:` is sugar for, kept honest.
        for (s, a, b) in [(0, 1, 0), (1, 1, 0), (0, 0, 1), (1, 0, 1)] {
            let want = if s == 1 { b } else { a };
            assert_eq!(
                eval("(!S & A) | (S & B)", &[("S", s), ("A", a), ("B", b)]),
                want,
                "S={s} A={a} B={b}"
            );
        }
    }

    #[test]
    fn the_names_a_table_reads_are_reported() {
        let (_, names) = compile_table_expr("!(A1 & B1) ^ C").expect("compiles");
        assert_eq!(names, vec!["A1", "B1", "C"]);
    }

    #[test]
    fn arithmetic_is_not_boolean_algebra() {
        let err = compile_table_expr("A + B").unwrap_err().to_string();
        assert!(err.contains('+'), "{err}");
    }

    #[test]
    fn a_function_call_is_refused_by_name() {
        let err = compile_table_expr("reg(FOO) & A").unwrap_err().to_string();
        assert!(err.contains("reg("), "{err}");
    }

    #[test]
    fn an_unbalanced_parenthesis_is_a_parse_error() {
        let err = format!("{:#}", compile_table_expr("!(A & B").unwrap_err());
        assert!(err.contains("!(A & B"), "{err}");
    }

    // ─── LogicSpec::validate ────────────────────────────────────────────────

    fn spec(yaml: &str) -> LogicSpec {
        serde_yaml::from_str(yaml).expect("spec parses")
    }

    #[test]
    fn a_table_entry_naming_an_undeclared_input_is_refused() {
        let s = spec("inputs: [A]\noutputs: [Y]\ntable: { Y: \"A & B\" }\n");
        let err = format!("{:#}", s.validate("t").unwrap_err());
        assert!(err.contains("'B'"), "{err}");
    }

    #[test]
    fn an_output_with_no_table_entry_is_refused() {
        let s = spec("inputs: [A]\noutputs: [Y1, Y2]\ntable: { Y1: \"A\" }\n");
        let err = format!("{:#}", s.validate("t").unwrap_err());
        assert!(err.contains("Y2"), "{err}");
    }

    #[test]
    fn declaring_nothing_that_drives_an_output_is_refused() {
        let s = spec("inputs: [A]\noutputs: [Y]\n");
        let err = format!("{:#}", s.validate("t").unwrap_err());
        assert!(err.contains("table"), "{err}");
    }

    #[test]
    fn declaring_two_shapes_at_once_is_refused() {
        let s = spec(
            "inputs: [A, B]\noutputs: [Y, B]\ntable: { Y: \"A\" }\n\
             direction: { pin: DIR, a: [A], b: [B] }\n",
        );
        let err = format!("{:#}", s.validate("t").unwrap_err());
        assert!(err.contains("at once"), "{err}");
    }

    #[test]
    fn an_enable_gating_an_unknown_output_is_refused() {
        let s = spec(
            "inputs: [A]\noutputs: [Y]\ntable: { Y: \"A\" }\n\
             enables: [{ pin: OE, outputs: [Z] }]\n",
        );
        let err = format!("{:#}", s.validate("t").unwrap_err());
        assert!(err.contains("'Z'"), "{err}");
    }

    #[test]
    fn a_role_that_is_not_an_identifier_is_refused_by_name() {
        let s = spec("inputs: [\"1A\"]\noutputs: [Y]\ntable: { Y: \"1A\" }\n");
        let err = format!("{:#}", s.validate("t").unwrap_err());
        assert!(err.contains("1A") && err.contains("identifier"), "{err}");
    }

    #[test]
    fn a_role_named_twice_is_refused() {
        let s = spec(
            "inputs: [A, OE]\noutputs: [Y]\ntable: { Y: \"A\" }\n\
             enables: [{ pin: OE, outputs: [Y] }]\n",
        );
        let err = format!("{:#}", s.validate("t").unwrap_err());
        assert!(err.contains("twice"), "{err}");
    }

    #[test]
    fn a_transceiver_side_must_be_both_an_input_and_an_output() {
        let s = spec("inputs: [A]\noutputs: [B]\ndirection: { pin: DIR, a: [A], b: [B] }\n");
        let err = format!("{:#}", s.validate("t").unwrap_err());
        assert!(err.contains("BOTH"), "{err}");
    }

    #[test]
    fn a_transceiver_with_uneven_sides_is_refused() {
        let s = spec(
            "inputs: [A1, A2, B1]\noutputs: [A1, A2, B1]\n\
             direction: { pin: DIR, a: [A1, A2], b: [B1] }\n",
        );
        let err = format!("{:#}", s.validate("t").unwrap_err());
        assert!(err.contains("by index"), "{err}");
    }

    #[test]
    fn a_switch_with_the_wrong_number_of_sources_is_refused() {
        let s = spec(
            "inputs: [A1, B1, A2, B2]\noutputs: [Y1, Y2]\n\
             select: { pin: S, low: [A1], high: [B1, B2] }\n",
        );
        let err = format!("{:#}", s.validate("t").unwrap_err());
        assert!(err.contains("by index"), "{err}");
    }

    #[test]
    fn a_valid_transceiver_validates() {
        let s = spec(
            "inputs: [A1, B1]\noutputs: [A1, B1]\n\
             direction: { pin: DIR, a_to_b_when: 1, a: [A1], b: [B1] }\n\
             enables: [{ pin: OE, active: low, outputs: [A1, B1] }]\n",
        );
        s.validate("t").expect("valid");
        assert_eq!(s.kind().unwrap(), LogicKind::Transceiver);
    }

    #[test]
    fn hiz_defaults_to_true() {
        let s = spec("inputs: [A]\noutputs: [Y]\ntable: { Y: \"A\" }\n");
        assert!(s.drive.hiz_when_disabled);
        assert_eq!(s.enables.len(), 0);
    }

    #[test]
    fn an_enable_defaults_to_active_low() {
        let s = spec(
            "inputs: [A]\noutputs: [Y]\ntable: { Y: \"A\" }\n\
             enables: [{ pin: OE, outputs: [Y] }]\n",
        );
        assert_eq!(s.enables[0].active, ActiveLevel::Low);
        assert!(ActiveLevel::Low.asserted(false));
        assert!(!ActiveLevel::Low.asserted(true));
    }
}
