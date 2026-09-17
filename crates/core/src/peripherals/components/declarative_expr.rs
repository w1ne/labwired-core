// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The arithmetic expression behind [`labwired_config::DerivedChannel`].
//!
//! A derived channel is a named value a part COMPUTES from the values it
//! senses — the INA219's POWER register is `bus_mV × |I_mA| / 1000` (§8.5.4),
//! a product of two stimulus channels. This module is the whole language: a
//! recursive-descent parser over `+ - * /`, unary minus, parentheses, decimal
//! literals, names, and `abs` / `min` / `max`.
//!
//! **Parsed once, at load.** [`CompiledExpr`] is the AST; the engine builds one
//! per declared channel when the descriptor is built and evaluates it on every
//! read, so a read never touches the expression text. A name that is not a
//! declared input or an EARLIER derived channel is rejected at parse time by
//! [`compile_derived`], which is what makes a cycle unwritable — see
//! [`labwired_config::DerivedChannel`] for why that is the rule rather than a
//! dependency sort.
//!
//! **What is deliberately absent.** No rounding, no comparison, no
//! conditional. Rounding to a register count is [`labwired_config::Encode`]'s
//! single rule for every part, and a value that depends on what the part is
//! currently doing is a state machine. Either one grown into this grammar
//! would be a second place the engine decides those things.

use std::collections::HashMap;

use anyhow::{bail, Result};
use labwired_config::DerivedChannel;

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    Const(f64),
    /// Index into the evaluation's name list — resolved at compile time so a
    /// read is an array index, not a string hash.
    Name(String),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Abs(Box<Expr>),
    Min(Box<Expr>, Box<Expr>),
    Max(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// One derived channel, compiled: the name it publishes and the AST that
/// produces it.
#[derive(Debug, Clone)]
pub(crate) struct CompiledExpr {
    pub(crate) name: String,
    expr: Expr,
}

impl Expr {
    fn eval(&self, slots: &HashMap<String, f64>) -> f64 {
        match self {
            Expr::Const(v) => *v,
            // A name that reached evaluation was checked at load, so a miss
            // cannot happen; 0.0 rather than a panic keeps a read on the bus
            // from aborting the whole simulation if it ever did.
            Expr::Name(n) => slots.get(n).copied().unwrap_or(0.0),
            Expr::Neg(a) => -a.eval(slots),
            Expr::Bin(op, a, b) => {
                let (a, b) = (a.eval(slots), b.eval(slots));
                match op {
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    BinOp::Mul => a * b,
                    BinOp::Div => a / b,
                }
            }
            Expr::Abs(a) => a.eval(slots).abs(),
            Expr::Min(a, b) => a.eval(slots).min(b.eval(slots)),
            Expr::Max(a, b) => a.eval(slots).max(b.eval(slots)),
        }
    }
}

// ─── tokenizer ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Comma,
}

fn lex(src: &str) -> Result<Vec<Tok>> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                out.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                out.push(Tok::Star);
                i += 1;
            }
            '/' => {
                out.push(Tok::Slash);
                i += 1;
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            _ if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                // Exponent form (`1e3`, `2.5e-4`): consumed here so it is a
                // number rather than a number followed by an unknown name.
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    let mark = i;
                    i += 1;
                    if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                        i += 1;
                    }
                    if i < chars.len() && chars[i].is_ascii_digit() {
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    } else {
                        i = mark;
                    }
                }
                let text: String = chars[start..i].iter().collect();
                match text.parse::<f64>() {
                    Ok(v) => out.push(Tok::Num(v)),
                    Err(_) => bail!("'{text}' is not a number"),
                }
            }
            _ if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(Tok::Ident(chars[start..i].iter().collect()));
            }
            _ => bail!("unexpected character '{c}'"),
        }
    }
    Ok(out)
}

// ─── parser ────────────────────────────────────────────────────────────────

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, want: &Tok) -> bool {
        if self.peek() == Some(want) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, want: &Tok, what: &str) -> Result<()> {
        if self.eat(want) {
            Ok(())
        } else {
            bail!("expected {what}")
        }
    }

    /// `sum := product (('+' | '-') product)*`
    fn sum(&mut self) -> Result<Expr> {
        let mut lhs = self.product()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinOp::Add,
                Some(Tok::Minus) => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.product()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// `product := unary (('*' | '/') unary)*`
    fn product(&mut self) -> Result<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => BinOp::Mul,
                Some(Tok::Slash) => BinOp::Div,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.unary()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// `unary := '-' unary | atom`
    fn unary(&mut self) -> Result<Expr> {
        if self.eat(&Tok::Minus) {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.atom()
    }

    /// `atom := number | name | call | '(' sum ')'`
    fn atom(&mut self) -> Result<Expr> {
        match self.next() {
            Some(Tok::Num(v)) => Ok(Expr::Const(v)),
            Some(Tok::LParen) => {
                let inner = self.sum()?;
                self.expect(&Tok::RParen, "')'")?;
                Ok(inner)
            }
            Some(Tok::Ident(name)) => {
                if !self.eat(&Tok::LParen) {
                    return Ok(Expr::Name(name));
                }
                match name.as_str() {
                    "abs" => {
                        let a = self.sum()?;
                        self.expect(&Tok::RParen, "')' after abs(…)")?;
                        Ok(Expr::Abs(Box::new(a)))
                    }
                    "min" | "max" => {
                        let a = self.sum()?;
                        self.expect(&Tok::Comma, "',' between the two arguments")?;
                        let b = self.sum()?;
                        self.expect(&Tok::RParen, "')'")?;
                        Ok(if name == "min" {
                            Expr::Min(Box::new(a), Box::new(b))
                        } else {
                            Expr::Max(Box::new(a), Box::new(b))
                        })
                    }
                    // Named loudly: a descriptor that reaches for `round(` or
                    // `sqrt(` is reaching for a decision this language does not
                    // make, and a silent "unknown name" would look like a typo.
                    other => bail!(
                        "unknown function '{other}(' — this language has abs(), min() and max() \
                         and nothing else"
                    ),
                }
            }
            Some(t) => bail!("unexpected token {t:?}"),
            None => bail!("expression ended early"),
        }
    }
}

fn parse(src: &str) -> Result<Expr> {
    let mut p = Parser {
        toks: lex(src)?,
        pos: 0,
    };
    if p.toks.is_empty() {
        bail!("the expression is empty");
    }
    let e = p.sum()?;
    if p.pos != p.toks.len() {
        bail!("trailing input after a complete expression");
    }
    Ok(e)
}

/// Collect every name an expression reads.
fn names(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Const(_) => {}
        Expr::Name(n) => out.push(n.clone()),
        Expr::Neg(a) | Expr::Abs(a) => names(a, out),
        Expr::Bin(_, a, b) | Expr::Min(a, b) | Expr::Max(a, b) => {
            names(a, out);
            names(b, out);
        }
    }
}

// ─── the load-time contract ────────────────────────────────────────────────

/// Compile a descriptor's `behavior.derived` list against its declared input
/// channel keys.
///
/// Enforced here, so a broken descriptor fails at load with the channel and the
/// name in the message rather than reading 0.0 forever on the bus:
///   * a derived name may not collide with a stimulus channel key, or with an
///     earlier derived name — a `source:` naming both would be ambiguous;
///   * every name an expression reads must be a declared input or a derived
///     channel declared ABOVE it. That is also why no cycle can be written.
pub(crate) fn compile_derived(
    derived: &[DerivedChannel],
    input_keys: &[String],
) -> Result<Vec<CompiledExpr>> {
    let mut out: Vec<CompiledExpr> = Vec::with_capacity(derived.len());
    for d in derived {
        if d.name.trim().is_empty() {
            bail!("a derived channel has an empty name");
        }
        if input_keys.contains(&d.name) {
            bail!(
                "derived channel '{}' has the same name as a stimulus input channel — \
                 a `source:` naming it would be ambiguous",
                d.name
            );
        }
        if out.iter().any(|c| c.name == d.name) {
            bail!("derived channel '{}' is declared twice", d.name);
        }
        let expr = parse(&d.expr)
            .map_err(|e| anyhow::anyhow!("derived channel '{}': {e} (in `{}`)", d.name, d.expr))?;
        let mut read = Vec::new();
        names(&expr, &mut read);
        for n in &read {
            let known = input_keys.iter().any(|k| k == n) || out.iter().any(|c| &c.name == n);
            if !known {
                bail!(
                    "derived channel '{}' reads '{n}', which is neither a declared input \
                     channel nor a derived channel declared above it. Derived channels are \
                     evaluated in declaration order, so a name must already exist when it is \
                     read — which is also what makes a cycle impossible to write.",
                    d.name
                );
            }
        }
        out.push(CompiledExpr {
            name: d.name.clone(),
            expr,
        });
    }
    Ok(out)
}

/// Evaluate every derived channel into `slots`, in declaration order, so a
/// later expression sees the values the earlier ones just produced.
///
/// Called on the observed (noise-applied) slot view, which is what makes a
/// derived channel a function of what the part MEASURED rather than of the
/// noiseless stimulus behind it.
pub(crate) fn eval_derived(compiled: &[CompiledExpr], slots: &mut HashMap<String, f64>) {
    for c in compiled {
        let v = c.expr.eval(slots);
        slots.insert(c.name.clone(), v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(name: &str, expr: &str) -> DerivedChannel {
        DerivedChannel {
            name: name.into(),
            expr: expr.into(),
        }
    }

    fn eval(expr: &str, slots: &[(&str, f64)]) -> f64 {
        let keys: Vec<String> = slots.iter().map(|(k, _)| (*k).to_string()).collect();
        let compiled = compile_derived(&[d("out", expr)], &keys).expect("compiles");
        let mut map: HashMap<String, f64> =
            slots.iter().map(|(k, v)| ((*k).to_string(), *v)).collect();
        eval_derived(&compiled, &mut map);
        map["out"]
    }

    #[test]
    fn precedence_and_parentheses_follow_arithmetic() {
        assert_eq!(eval("2 + 3 * 4", &[]), 14.0);
        assert_eq!(eval("(2 + 3) * 4", &[]), 20.0);
        assert_eq!(eval("10 / 4", &[]), 2.5);
        assert_eq!(
            eval("1 - 2 - 3", &[]),
            -4.0,
            "subtraction is left-associative"
        );
        assert_eq!(eval("-2 * -3", &[]), 6.0);
        assert_eq!(eval("2 * 1e3", &[]), 2000.0);
    }

    #[test]
    fn the_three_functions_are_the_only_ones() {
        assert_eq!(eval("abs(0 - 4.5)", &[]), 4.5);
        assert_eq!(eval("min(3, 7)", &[]), 3.0);
        assert_eq!(eval("max(3, 7)", &[]), 7.0);
        let err = compile_derived(&[d("out", "round(1.5)")], &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown function 'round('"), "{err}");
    }

    #[test]
    fn the_ina219_power_expression_is_the_product_of_two_channels() {
        // bus_mV × |I_mA| in watts: 12 V × 1.5 A = 18 W, and the sign of the
        // current does not change the power.
        assert_eq!(
            eval(
                "bus_voltage * abs(current)",
                &[("bus_voltage", 12.0), ("current", 1.5)]
            ),
            18.0
        );
        assert_eq!(
            eval(
                "bus_voltage * abs(current)",
                &[("bus_voltage", 12.0), ("current", -1.5)]
            ),
            18.0
        );
    }

    #[test]
    fn a_later_channel_sees_an_earlier_one() {
        let compiled = compile_derived(
            &[d("half", "x / 2"), d("quarter", "half / 2")],
            &["x".to_string()],
        )
        .expect("compiles");
        let mut slots: HashMap<String, f64> = [("x".to_string(), 8.0)].into_iter().collect();
        eval_derived(&compiled, &mut slots);
        assert_eq!(slots["half"], 4.0);
        assert_eq!(slots["quarter"], 2.0);
    }

    #[test]
    fn a_forward_reference_is_a_load_error_which_is_why_a_cycle_cannot_be_written() {
        // `a` reads `b`, which is declared BELOW it. Refusing this is what makes
        // `a → b → a` unwritable: the second half could never compile either.
        let err = compile_derived(&[d("a", "b + 1"), d("b", "a + 1")], &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("derived channel 'a' reads 'b'"), "{err}");
        assert!(err.contains("makes a cycle impossible to write"), "{err}");
        // …and the self-reference, the shortest cycle of all.
        let err = compile_derived(&[d("a", "a * 2")], &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("reads 'a'"), "{err}");
    }

    #[test]
    fn a_derived_name_cannot_shadow_a_stimulus_channel() {
        let err = compile_derived(&[d("temp", "temp * 2")], &["temp".to_string()])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("same name as a stimulus input channel"),
            "{err}"
        );
    }

    #[test]
    fn a_malformed_expression_names_the_channel_and_the_text() {
        for bad in ["", "1 +", "2 3", "(1 + 2", "1 $ 2"] {
            let err = compile_derived(&[d("out", bad)], &[])
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("derived channel 'out'") && err.contains(bad.trim()),
                "`{bad}` produced an unhelpful error: {err}"
            );
        }
    }
}
