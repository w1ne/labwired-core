// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **The `uart_device` primitive's schema** — a part whose whole interface is a
//! byte stream.
//!
//! What it covers
//! ==============
//! The family a register map cannot reach because there are no registers: an
//! **AT command shell** (HC-05, SIM800L, ESP-01, every cellular modem) and an
//! **unsolicited stream** (a GPS emitting NMEA at 1 Hz, a scale printing a
//! weight). Before this, each was a hand-written Rust model with its own line
//! buffer, its own `handle_line` ladder and its own `format!` calls; three of
//! them differed only in that ladder, byte for byte, including the same 128-byte
//! line cap and the same `byte as char` cast.
//!
//! The three pieces
//! ================
//! * [`UartFrames`] — where a frame ENDS. A terminator (`"\r\n"`), a fixed
//!   length, or an opcode-led frame.
//! * [`UartResponse`] — what a completed frame is ANSWERED with. A match, a
//!   template to emit, an optional delay, and optional rule [`Action`]s so a
//!   command can change the part's state as well as answer it.
//! * [`UartUnsolicited`] — what the part says on its OWN clock: a
//!   [`crate::DeviceTimer`] name, an optional guard, and a template.
//!
//! Everything else the part does — state, variables, timers, FIFOs, pin drives
//! — is the SAME Tier-2 [`crate::Rule`] machinery every other primitive uses.
//! A `uart_device` is not a second rule engine; it is a third transport for the
//! one that exists.
//!
//! Templates
//! =========
//! A template is literal text with `{…}` placeholders over the ordinary
//! [`crate::expr`] integer language, so a GPS sentence reads its position from
//! `input(lat)` and nothing in the engine knows what a latitude is. See
//! [`Template`] for the grammar and [`TemplateFormat`] for the format specs;
//! both are parsed ONCE, at load, so a malformed placeholder is a manifest
//! error and not a surprise at the first sentence.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::expr::{EvalCtx, Expr, ExprError};
use crate::rules::Action;

// ─── framing ───────────────────────────────────────────────────────────────

/// Where a frame arriving on the part's RX line ENDS.
///
/// Defaults to the line shape every AT shell has: bytes accumulate until a CR
/// or an LF, and the frame is what came before it. That is one rule rather than
/// two because the hosts in the wild disagree — `AT\r`, `AT\n` and `AT\r\n` all
/// occur, and a part that answered only one of them would look dead against
/// half the drivers.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct UartFrames {
    /// Bytes that END a frame. Each byte of this string is a terminator, and a
    /// run of them delimits ONE frame, not several empty ones — which is what
    /// makes `"\r\n"` mean the two-byte sequence AND the single `\r` a
    /// CR-only host sends.
    #[serde(default = "crlf")]
    pub terminator: String,
    /// Fixed frame length in bytes. Present ⇒ the frame completes at that
    /// length and `terminator` is not consulted. This is the binary-protocol
    /// shape (a fixed-size command packet), not the AT shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u16>,
    /// Longest frame the part will buffer. A longer line is TRUNCATED to this
    /// and the rest of it is dropped, which is what the hand-written shells did
    /// with their 128-byte cap; stating it keeps a firmware bug from growing an
    /// unbounded allocation inside the simulator.
    #[serde(default = "default_max_frame")]
    pub max_bytes: u16,
    /// Whether matching ignores ASCII case. True is the AT-shell default —
    /// every one of those models uppercased the line before comparing.
    #[serde(default = "yes")]
    pub ignore_case: bool,
}

impl Default for UartFrames {
    fn default() -> Self {
        Self {
            terminator: crlf(),
            length: None,
            max_bytes: default_max_frame(),
            ignore_case: true,
        }
    }
}

fn crlf() -> String {
    "\r\n".to_string()
}
fn default_max_frame() -> u16 {
    128
}
fn yes() -> bool {
    true
}

// ─── matching ──────────────────────────────────────────────────────────────

/// How a completed frame is matched against a [`UartResponse`].
///
/// Deliberately three shapes and no regex. A datasheet's command table is a
/// list of literals and prefixes; a regex in a part document is a second
/// language with its own failure modes, and every command any of the ported
/// shells answered is one of these three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UartMatch {
    /// The whole frame equals this text.
    Exact(String),
    /// The frame starts with this text — the `AT+CSQ=?` / `AT+CSQ?` family.
    Prefix(String),
    /// Anything. The catch-all last entry; matching stops at the FIRST hit, so
    /// an `any` above a literal makes the literal unreachable.
    Any,
}

impl Serialize for UartMatch {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde_yaml::{Mapping, Value};
        match self {
            UartMatch::Any => s.serialize_str("any"),
            UartMatch::Exact(t) => {
                let mut m = Mapping::new();
                m.insert(Value::from("exact"), Value::from(t.clone()));
                Value::Mapping(m).serialize(s)
            }
            UartMatch::Prefix(t) => {
                let mut m = Mapping::new();
                m.insert(Value::from("prefix"), Value::from(t.clone()));
                Value::Mapping(m).serialize(s)
            }
        }
    }
}

impl<'de> Deserialize<'de> for UartMatch {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_yaml::Value::deserialize(d)?;
        if let Some(s) = v.as_str() {
            return match s {
                "any" => Ok(UartMatch::Any),
                // A bare string is the common case and means EXACT. Spelling
                // `match: AT` and getting a prefix match would make every
                // `AT+…` line answer the bare-`AT` entry.
                other => Ok(UartMatch::Exact(other.to_string())),
            };
        }
        let map = v.as_mapping().ok_or_else(|| {
            D::Error::custom(
                "`match:` must be `any`, a literal, or `{ exact: … }` / `{ prefix: … }`",
            )
        })?;
        let get = |k: &str| {
            map.get(serde_yaml::Value::from(k))
                .and_then(|x| x.as_str())
                .map(str::to_string)
        };
        if let Some(t) = get("exact") {
            return Ok(UartMatch::Exact(t));
        }
        if let Some(t) = get("prefix") {
            return Ok(UartMatch::Prefix(t));
        }
        Err(D::Error::custom(
            "unknown `match:`; expected `any`, a literal, `{ exact: … }` or `{ prefix: … }`",
        ))
    }
}

impl UartMatch {
    /// Whether `frame` (already case-folded by the caller when the part asked
    /// for it) matches. `self` is folded here so the descriptor may be written
    /// in whatever case the datasheet uses.
    pub fn matches(&self, frame: &str, ignore_case: bool) -> bool {
        let fold = |s: &str| {
            if ignore_case {
                s.to_ascii_uppercase()
            } else {
                s.to_string()
            }
        };
        match self {
            UartMatch::Any => true,
            UartMatch::Exact(t) => frame == fold(t),
            UartMatch::Prefix(t) => frame.starts_with(&fold(t)),
        }
    }
}

// ─── the response table ────────────────────────────────────────────────────

/// One entry of the part's command table.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct UartResponse {
    /// What this entry answers.
    pub r#match: UartMatch,
    /// The bytes to emit, as a [`Template`]. Absent ⇒ the entry is silent,
    /// which is how a command that only changes state is written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond: Option<Template>,
    /// How the rendered text is wrapped on the wire (checksums, sentinels).
    #[serde(default, skip_serializing_if = "TemplateWrap::is_none")]
    pub wrap: TemplateWrap,
    /// Microseconds between the frame completing and the FIRST answer byte
    /// becoming available. A modem that takes 90 ms to answer `AT+CSQ` is the
    /// reason a driver's timeout is worth testing at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_us: Option<u64>,
    /// Tier-2 actions run when this entry matches, BEFORE the response is
    /// rendered — so a command that switches the part's mode answers from the
    /// new mode. Same [`Action`] vocabulary every rule uses.
    #[serde(rename = "do", default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
}

/// One thing the part says on its own clock.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct UartUnsolicited {
    /// Name of the [`crate::DeviceTimer`] whose firing emits this.
    pub timer: String,
    /// Integer guard, evaluated against the part's state as it stands when the
    /// timer fires and BEFORE any `timer:` rule runs. Absent ⇒ always.
    ///
    /// ⚠️ The pre-rule evaluation is what makes an alternating stream writable.
    /// Two entries guarded `var(n) % 2 == 0` and `== 1` are mutually exclusive
    /// because both see the SAME `n`; if the rule that increments `n` ran
    /// first, the second guard would see the incremented value and both
    /// entries would fire on every tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// The bytes to emit.
    pub template: Template,
    /// How the rendered text is wrapped on the wire.
    #[serde(default, skip_serializing_if = "TemplateWrap::is_none")]
    pub wrap: TemplateWrap,
}

/// How a rendered template becomes bytes on the wire.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TemplateWrap {
    /// The rendered text IS the bytes. The default.
    #[default]
    None,
    /// **NMEA 0183 sentence framing**: `$` + the rendered payload + `*` + the
    /// two uppercase hex digits of the XOR over every payload byte + CRLF.
    ///
    /// This is named rather than left to the template because the checksum is
    /// over the template's own OUTPUT — a template cannot contain it, and
    /// spelling it as a general "checksum the rest of this string" key would be
    /// the same algorithm with more rope. Every NMEA part reuses this one key.
    Nmea,
}

impl TemplateWrap {
    fn is_none(&self) -> bool {
        matches!(self, TemplateWrap::None)
    }

    /// Apply the framing to a rendered payload.
    pub fn apply(&self, payload: &str) -> String {
        match self {
            TemplateWrap::None => payload.to_string(),
            TemplateWrap::Nmea => {
                let checksum = payload.bytes().fold(0u8, |acc, b| acc ^ b);
                format!("${payload}*{checksum:02X}\r\n")
            }
        }
    }
}

// ─── the uart spec ─────────────────────────────────────────────────────────

/// The `uart:` block of a `uart_device` descriptor.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct UartSpec {
    /// Line rate the part's own datasheet gives, in bit/s. DIAGNOSTIC ONLY
    /// today: the host UART paces the stream by
    /// [`UartStreamDevice::max_bytes_per_tick`](https://docs.rs/) — one byte
    /// per millisecond, which is about 9600 baud — and a part that declares a
    /// different number does not yet change that. It is recorded because a
    /// descriptor that does not say its baud rate cannot later be paced by it,
    /// and because the number belongs with the part rather than in a comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baud: Option<u32>,
    /// Where a frame ends.
    #[serde(default)]
    pub frames: UartFrames,
    /// The command table, tried IN ORDER; the first match wins and the rest are
    /// not considered. Order is therefore load-bearing exactly as `rules:`
    /// order is, and for the same reason: a datasheet's table is ordered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub responses: Vec<UartResponse>,
    /// What the part says unprompted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsolicited: Vec<UartUnsolicited>,
}

// ─── templates ─────────────────────────────────────────────────────────────

/// A text template: literal text plus `{…}` placeholders.
///
/// ## Grammar
///
/// ```text
/// template    := ( LITERAL | "{{" | "}}" | placeholder )*
/// placeholder := "{" expr [ ":" format ] "}"
/// format      := "d"                       integer, as written
///              | [ "0" ] WIDTH "." PREC     fixed point (see below)
///              | "." PREC                   fixed point, no padding
///              | [ "0" ] WIDTH "X"          uppercase hex
///              | "char(" CH "," CH ")"      one character: first if non-zero
/// ```
///
/// `expr` is the ordinary [`crate::expr`] integer language — `input(lat)`,
/// `var(n)`, `reg(R)`, arithmetic, comparisons. There are no floats anywhere,
/// which is what makes a rendered sentence bit-identical on native and wasm.
///
/// ## Fixed point without floats
///
/// A `W.P` format treats the integer as a count of `10^-P` units and prints it
/// with `P` decimals, zero-padded to a TOTAL width of `W` (the point and a
/// minus sign count toward `W`, as they do in `format!("{:0W$.P$}")`). So a
/// latitude held as `37464940` in units of 1e-4 minutes renders `{…:09.4}` as
/// `3746.4940`. The part chooses the unit through
/// [`InputSpec::expr_scale`](crate::InputSpec::expr_scale) and the arithmetic
/// that follows it, so no float ever crosses the boundary and no rounding mode
/// has to be agreed between two platforms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// The original text, kept so the descriptor round-trips byte for byte.
    source: String,
    parts: Vec<TemplatePart>,
}

/// One piece of a compiled [`Template`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum TemplatePart {
    Literal(String),
    Value { expr: Expr, format: TemplateFormat },
}

/// How one placeholder's integer becomes text. See [`Template`] for the
/// spelling of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateFormat {
    /// Plain decimal.
    Int,
    /// Fixed point: `prec` decimals, zero-padded to `width` total characters.
    Fixed { width: usize, prec: u32 },
    /// Uppercase hex, zero-padded to `width`.
    Hex { width: usize },
    /// One character: `high` when the value is non-zero, else `low`.
    Char { high: char, low: char },
}

impl Serialize for Template {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.source)
    }
}

impl<'de> Deserialize<'de> for Template {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Template::parse(&raw).map_err(D::Error::custom)
    }
}

/// A template that would not compile.
#[derive(Debug, Clone)]
pub struct TemplateError {
    /// Human-readable reason, naming the offending placeholder.
    pub message: String,
    /// The expression error, when the placeholder's expression is what failed.
    pub expr: Option<ExprError>,
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.expr {
            Some(e) => write!(f, "{}: {e}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for TemplateError {}

impl Template {
    /// The text the descriptor was written with.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Compile a template. Called once, at load.
    pub fn parse(text: &str) -> Result<Self, TemplateError> {
        let err = |message: String| TemplateError {
            message,
            expr: None,
        };
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                // `{{` and `}}` are the literal braces, the way `format!`
                // spells them — a part whose datasheet text contains a brace
                // should not have to avoid the character.
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    literal.push('}');
                }
                '}' => {
                    return Err(err(format!(
                        "template has a `}}` with no `{{` before it, in `{text}` — write `}}}}` \
                         for a literal brace"
                    )))
                }
                '{' => {
                    if !literal.is_empty() {
                        parts.push(TemplatePart::Literal(std::mem::take(&mut literal)));
                    }
                    let mut body = String::new();
                    let mut depth = 0usize;
                    let mut closed = false;
                    for c in chars.by_ref() {
                        match c {
                            // `char(A,B)` has no braces, but an expression may
                            // hold parentheses; brace depth is tracked so a
                            // nested `{` cannot silently end the placeholder.
                            '{' => {
                                depth += 1;
                                body.push(c);
                            }
                            '}' if depth == 0 => {
                                closed = true;
                                break;
                            }
                            '}' => {
                                depth -= 1;
                                body.push(c);
                            }
                            _ => body.push(c),
                        }
                    }
                    if !closed {
                        return Err(err(format!("template has an unclosed `{{` in `{text}`")));
                    }
                    parts.push(Self::placeholder(&body, text)?);
                }
                _ => literal.push(c),
            }
        }
        if !literal.is_empty() {
            parts.push(TemplatePart::Literal(literal));
        }
        Ok(Self {
            source: text.to_string(),
            parts,
        })
    }

    fn placeholder(body: &str, whole: &str) -> Result<TemplatePart, TemplateError> {
        let err = |message: String| TemplateError {
            message,
            expr: None,
        };
        // The format spec is after the LAST `:` that is not inside parentheses,
        // so `{input(a):09.4}` splits but a colon inside a `char(:,;)` does not.
        let (expr_src, format) = match split_format(body) {
            Some((e, f)) => (e, Some(f)),
            None => (body, None),
        };
        let expr = Expr::parse(expr_src.trim()).map_err(|e| TemplateError {
            message: format!("template placeholder `{{{body}}}` in `{whole}` has a bad expression"),
            expr: Some(e),
        })?;
        let format = match format {
            None => TemplateFormat::Int,
            Some(spec) => parse_format(spec.trim()).ok_or_else(|| {
                err(format!(
                    "template placeholder `{{{body}}}` in `{whole}` has an unknown format `{spec}`; \
                     expected `d`, `WIDTH.PREC`, `.PREC`, `WIDTHX` or `char(A,B)`"
                ))
            })?,
        };
        Ok(TemplatePart::Value { expr, format })
    }

    /// Render against a rule-machine environment. Total: every expression
    /// evaluates, so a template cannot fail at run time.
    pub fn render(&self, ctx: &dyn EvalCtx) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                TemplatePart::Literal(t) => out.push_str(t),
                TemplatePart::Value { expr, format } => {
                    format.write(expr.eval(ctx), &mut out);
                }
            }
        }
        out
    }

    /// Every register name any placeholder reads, so a caller can check them
    /// against the declared map exactly as it does for rule expressions.
    pub fn registers(&self, out: &mut Vec<String>) {
        for part in &self.parts {
            if let TemplatePart::Value { expr, .. } = part {
                expr.registers(out);
            }
        }
    }

    /// Every `input(KEY)` a placeholder reads.
    pub fn inputs(&self, out: &mut Vec<String>) {
        for part in &self.parts {
            if let TemplatePart::Value { expr, .. } = part {
                collect_inputs(expr, out);
            }
        }
    }
}

/// Split a placeholder body into `(expression, format)` at the last top-level
/// `:`. Parenthesis depth is tracked so a colon inside `char(…)` stays put.
fn split_format(body: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut at = None;
    for (i, c) in body.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ':' if depth == 0 => at = Some(i),
            _ => {}
        }
    }
    at.map(|i| (&body[..i], &body[i + 1..]))
}

fn parse_format(spec: &str) -> Option<TemplateFormat> {
    if spec == "d" {
        return Some(TemplateFormat::Int);
    }
    if let Some(rest) = spec.strip_prefix("char(") {
        let inner = rest.strip_suffix(')')?;
        let (high, low) = inner.split_once(',')?;
        let mut h = high.chars();
        let mut l = low.chars();
        let (high, low) = (h.next()?, l.next()?);
        // One character each: `char(NN,S)` is a typo, not a two-character
        // hemisphere, and silently taking the first would hide it.
        if h.next().is_some() || l.next().is_some() {
            return None;
        }
        return Some(TemplateFormat::Char { high, low });
    }
    if let Some(width) = spec.strip_suffix('X') {
        // A leading `0` is the zero-PAD flag, as it is in `format!`, so it is
        // stripped before the width is read; `0` alone leaves nothing, which
        // is a width of zero.
        let digits = width.trim_start_matches('0');
        let width: usize = if digits.is_empty() {
            0
        } else {
            digits.parse().ok()?
        };
        return Some(TemplateFormat::Hex { width });
    }
    if let Some((w, p)) = spec.split_once('.') {
        let prec: u32 = p.parse().ok()?;
        if prec > 9 {
            return None; // beyond an i64's headroom for the 10^P divisor
        }
        let width: usize = if w.is_empty() { 0 } else { w.parse().ok()? };
        return Some(TemplateFormat::Fixed { width, prec });
    }
    None
}

impl TemplateFormat {
    fn write(&self, value: i64, out: &mut String) {
        use std::fmt::Write as _;
        match self {
            TemplateFormat::Int => {
                let _ = write!(out, "{value}");
            }
            TemplateFormat::Char { high, low } => {
                out.push(if value != 0 { *high } else { *low });
            }
            TemplateFormat::Hex { width } => {
                let _ = write!(out, "{:0width$X}", value as u64, width = *width);
            }
            TemplateFormat::Fixed { width, prec } => {
                let scale = 10i64.pow(*prec);
                let negative = value < 0;
                // `unsigned_abs` rather than `-value`: i64::MIN has no positive
                // counterpart, and a part document can hold any integer.
                let magnitude = value.unsigned_abs();
                let whole = magnitude / scale as u64;
                let frac = magnitude % scale as u64;
                let body = if *prec == 0 {
                    format!("{whole}")
                } else {
                    format!("{whole}.{frac:0prec$}", prec = *prec as usize)
                };
                let sign = if negative { 1 } else { 0 };
                let pad = width.saturating_sub(body.len() + sign);
                if negative {
                    out.push('-');
                }
                for _ in 0..pad {
                    out.push('0');
                }
                out.push_str(&body);
            }
        }
    }
}

/// Walk an expression for `input(KEY)` names. Mirrors
/// [`Expr::registers`](crate::expr::Expr::registers), which does the same for
/// `reg()`; both exist so load-time validation can reject a name the part does
/// not declare.
fn collect_inputs(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Input(k) if !out.contains(k) => out.push(k.clone()),
        Expr::Unary(_, a) => collect_inputs(a, out),
        Expr::Binary(_, a, b) => {
            collect_inputs(a, out);
            collect_inputs(b, out);
        }
        _ => {}
    }
}

// ─── validation ────────────────────────────────────────────────────────────

/// Static checks for a `uart:` block: every template compiles (it already did,
/// at deserialise time), every `unsolicited:` entry names a declared timer, and
/// every name a template reads is declared.
pub fn validate_uart(
    uart: &UartSpec,
    timers: &[String],
    registers: &[String],
    inputs: &[String],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !uart.frames.terminator.is_empty() || uart.frames.length.is_some(),
        "`uart.frames:` has neither a `terminator:` nor a `length:` — no frame would ever complete, \
         so the part would never answer anything"
    );
    anyhow::ensure!(
        uart.frames.max_bytes > 0,
        "`uart.frames.max_bytes: 0` buffers nothing, so every frame is empty"
    );
    let check = |t: &Template, what: &str| -> anyhow::Result<()> {
        let mut names = Vec::new();
        t.registers(&mut names);
        for name in &names {
            anyhow::ensure!(
                registers.iter().any(|r| r == name),
                "{what} reads `reg({name})`, which this part does not declare"
            );
        }
        let mut keys = Vec::new();
        t.inputs(&mut keys);
        for key in &keys {
            anyhow::ensure!(
                inputs.iter().any(|i| i == key),
                "{what} reads `input({key})`, which this part declares no channel for"
            );
        }
        Ok(())
    };
    for (i, r) in uart.responses.iter().enumerate() {
        if let Some(t) = &r.respond {
            check(t, &format!("uart.responses[{i}].respond"))?;
        }
    }
    for (i, u) in uart.unsolicited.iter().enumerate() {
        anyhow::ensure!(
            timers.contains(&u.timer),
            "uart.unsolicited[{i}] fires on timer '{}', which this part does not declare in \
             `timers:`",
            u.timer
        );
        check(&u.template, &format!("uart.unsolicited[{i}].template"))?;
        if let Some(src) = &u.when {
            crate::expr::Expr::parse(src)
                .map_err(|e| anyhow::anyhow!("uart.unsolicited[{i}].when: {e} — in `{src}`"))?;
        }
    }
    anyhow::ensure!(
        !uart.responses.is_empty() || !uart.unsolicited.is_empty(),
        "a `uart_device` with neither `responses:` nor `unsolicited:` answers nothing and says \
         nothing — it would attach and look like a working part"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Env {
        inputs: BTreeMap<String, i64>,
        vars: BTreeMap<String, i64>,
    }

    impl EvalCtx for Env {
        fn reg(&self, _: &str) -> i64 {
            0
        }
        fn reported(&self, _: &str) -> i64 {
            0
        }
        fn field(&self, _: &str, _: &str) -> i64 {
            0
        }
        fn var(&self, name: &str) -> i64 {
            self.vars.get(name).copied().unwrap_or(0)
        }
        fn input(&self, key: &str) -> i64 {
            self.inputs.get(key).copied().unwrap_or(0)
        }
        /// A UART part is a STREAM: it has no observed pads at all, so there is
        /// no pad whose level this could report. 0 here is the truth, not a
        /// stub — and `validate_rule_names` refuses a `pin()` naming a pad the
        /// part does not declare, so no shipped descriptor can reach it.
        fn pin(&self, _: &str) -> i64 {
            0
        }
        fn fifo_len(&self, _: &str) -> i64 {
            0
        }
        /// A UART part is a byte STREAM with no `frames:` block; see `pin`
        /// above for why 0 here is the truth rather than a stub.
        fn frame_byte(&self, _: usize) -> i64 {
            0
        }
        fn written(&self) -> i64 {
            0
        }
        fn state(&self) -> &str {
            ""
        }
        fn note_divide_by_zero(&self) {}
    }

    #[test]
    fn a_plain_template_is_literal_text() {
        let t = Template::parse("OK\r\n").unwrap();
        assert_eq!(t.render(&Env::default()), "OK\r\n");
        assert_eq!(t.source(), "OK\r\n");
    }

    /// The whole point of the fixed-point format: DDMM.mmmm with no float
    /// anywhere. 37.7749° is 3746.4940 minutes-of-arc in DDMM form, held as
    /// 37464940 counts of 1e-4.
    #[test]
    fn fixed_point_renders_a_latitude_without_a_float() {
        let t = Template::parse("{input(lat_dm):09.4}").unwrap();
        let mut env = Env::default();
        env.inputs.insert("lat_dm".into(), 37_464_940);
        assert_eq!(t.render(&env), "3746.4940");
        // A single-digit degree pads to the full nine characters, which is what
        // NMEA requires and what `{:09.4}` did in the hand-written model.
        env.inputs.insert("lat_dm".into(), 5_304_440);
        assert_eq!(t.render(&env), "0530.4440");
    }

    #[test]
    fn fixed_point_pads_a_longitude_to_ten() {
        let t = Template::parse("{input(lon_dm):010.4}").unwrap();
        let mut env = Env::default();
        env.inputs.insert("lon_dm".into(), 122_251_640);
        assert_eq!(t.render(&env), "12225.1640");
        env.inputs.insert("lon_dm".into(), 76_680);
        assert_eq!(t.render(&env), "00007.6680");
    }

    #[test]
    fn a_negative_fixed_point_keeps_its_sign_inside_the_width() {
        let t = Template::parse("{0 - input(x):08.3}").unwrap();
        let mut env = Env::default();
        env.inputs.insert("x".into(), 1_500);
        // Eight characters TOTAL, the minus sign and the point included —
        // which is exactly what `format!("{:08.3}", -1.5)` produces.
        assert_eq!(t.render(&env), "-001.500");
    }

    #[test]
    fn char_picks_by_sign() {
        let t = Template::parse("{input(lat) >= 0:char(N,S)}").unwrap();
        let mut env = Env::default();
        env.inputs.insert("lat".into(), 377_749_000);
        assert_eq!(t.render(&env), "N");
        env.inputs.insert("lat".into(), -377_749_000);
        assert_eq!(t.render(&env), "S");
    }

    #[test]
    fn hex_and_int_formats() {
        let t = Template::parse("{var(n):02X}/{var(n)}/{var(n):d}").unwrap();
        let mut env = Env::default();
        env.vars.insert("n".into(), 0x7F);
        assert_eq!(t.render(&env), "7F/127/127");
    }

    #[test]
    fn literal_braces_round_trip() {
        let t = Template::parse("{{ok}}").unwrap();
        assert_eq!(t.render(&Env::default()), "{ok}");
    }

    #[test]
    fn an_unclosed_placeholder_is_a_load_error() {
        let e = Template::parse("a{input(x)").unwrap_err();
        assert!(e.to_string().contains("unclosed"), "{e}");
    }

    #[test]
    fn a_bad_expression_names_the_placeholder() {
        let e = Template::parse("{input(x) $}").unwrap_err();
        let text = e.to_string();
        assert!(text.contains("input(x) $"), "{text}");
    }

    #[test]
    fn an_unknown_format_is_a_load_error() {
        let e = Template::parse("{var(n):zzz}").unwrap_err();
        assert!(e.to_string().contains("unknown format"), "{e}");
    }

    /// The NMEA wrap is the one checksum the engine knows, and it is checked
    /// against a sentence whose bytes came out of the hand-written model.
    #[test]
    fn the_nmea_wrap_reproduces_a_real_sentence() {
        let payload = "GPGGA,120000.00,3746.4940,N,12225.1640,W,1,08,1.0,10.0,M,0.0,M,,";
        assert_eq!(
            TemplateWrap::Nmea.apply(payload),
            "$GPGGA,120000.00,3746.4940,N,12225.1640,W,1,08,1.0,10.0,M,0.0,M,,*7F\r\n"
        );
    }

    #[test]
    fn a_match_is_exact_by_default_and_folds_case() {
        let m: UartMatch = serde_yaml::from_str("AT").unwrap();
        assert_eq!(m, UartMatch::Exact("AT".into()));
        assert!(m.matches("AT", true));
        assert!(!m.matches("AT+CSQ", true), "an exact match is not a prefix");
        let p: UartMatch = serde_yaml::from_str("{ prefix: at+csq }").unwrap();
        assert!(
            p.matches("AT+CSQ=?", true),
            "the descriptor's case folds too"
        );
        assert!(!p.matches("AT+CSQ=?", false));
    }

    #[test]
    fn the_response_table_round_trips() {
        let spec: UartSpec = serde_yaml::from_str(
            r#"
baud: 9600
frames: { terminator: "\r\n" }
responses:
  - { match: { prefix: "AT+VERSION" }, respond: "+VERSION:x\r\nOK\r\n" }
  - { match: any, respond: "ERROR\r\n" }
unsolicited:
  - { timer: sentence, when: "var(n) % 2 == 0", template: "GPGGA,{input(lat):d}", wrap: nmea }
"#,
        )
        .unwrap();
        assert_eq!(spec.baud, Some(9600));
        assert_eq!(spec.responses.len(), 2);
        assert_eq!(spec.unsolicited[0].wrap, TemplateWrap::Nmea);
        let back = serde_yaml::to_string(&spec).unwrap();
        let again: UartSpec = serde_yaml::from_str(&back).unwrap();
        assert_eq!(spec, again, "the block round-trips through YAML");
    }

    #[test]
    fn an_unsolicited_entry_naming_no_timer_is_a_load_error() {
        let spec: UartSpec =
            serde_yaml::from_str("unsolicited:\n  - { timer: nope, template: \"x\" }\n").unwrap();
        let e = validate_uart(&spec, &["sentence".into()], &[], &[]).unwrap_err();
        assert!(e.to_string().contains("nope"), "{e}");
    }

    #[test]
    fn a_template_reading_an_undeclared_channel_is_a_load_error() {
        let spec: UartSpec =
            serde_yaml::from_str("responses:\n  - { match: any, respond: \"{input(nope)}\" }\n")
                .unwrap();
        let e = validate_uart(&spec, &[], &[], &["lat".into()]).unwrap_err();
        assert!(e.to_string().contains("nope"), "{e}");
    }

    #[test]
    fn a_uart_device_that_says_nothing_is_refused() {
        let spec = UartSpec::default();
        let e = validate_uart(&spec, &[], &[], &[]).unwrap_err();
        assert!(e.to_string().contains("answers nothing"), "{e}");
    }
}
