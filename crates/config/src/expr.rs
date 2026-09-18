// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **The rule expression language** — integers only, parsed once at load.
//!
//! Why this is here and not in the engine
//! ======================================
//! A Tier-2 part (`behavior.rules:`) carries guards and computed values as
//! short expression strings: `reg(PWR_MGMT_1) & 0x40 == 0`, `written >> 3`,
//! `fifo_len(samples) > 0`. Those strings are part of the *schema*: a
//! malformed one is a bad part document, and it has to be rejected where every
//! other schema error is rejected — at load, by manifest preflight, before any
//! bus exists. That is `labwired-config`. The engine never sees a string.
//!
//! The crate stays serde-only in the sense that matters: nothing in this module
//! implements `Serialize`/`Deserialize`. The YAML field is a `String`; this is a
//! compiler from that string to an [`Expr`] tree, plus a total evaluator over an
//! [`EvalCtx`] the engine supplies. No engine type appears here, and no
//! allocation happens during [`Expr::eval`].
//!
//! The grammar
//! ===========
//! ```text
//! expr    := or
//! or      := and   ( "||" and )*
//! and     := bitor ( "&&" bitor )*
//! bitor   := bitxor ( "|" bitxor )*
//! bitxor  := bitand ( "^" bitand )*
//! bitand  := equal ( "&" equal )*
//! equal   := relate ( ("==" | "!=") relate )*
//! relate  := shift ( ("<" | "<=" | ">" | ">=") shift )*
//! shift   := sum ( ("<<" | ">>") sum )*
//! sum     := product ( ("+" | "-") product )*
//! product := unary ( ("*" | "/" | "%") unary )*
//! unary   := ("!" | "~" | "-") unary | primary
//! primary := INT | "(" expr ")" | CALL | "state" ("=="|"!=") IDENT | "written"
//! CALL    := ("reg"|"reported"|"var"|"input"|"fifo_len") "(" IDENT ")"
//!          | "abs" "(" expr ")"
//!          | "field" "(" IDENT "." IDENT ")"
//! INT     := decimal | "0x" hex   (underscores allowed in both)
//! ```
//!
//! Precedence is C's, with one deliberate exception documented below: `==` binds
//! *looser* than `&`, so `reg(X) & 0x40 == 0` means `(reg(X) & 0x40) == 0` —
//! the way it reads to someone holding a datasheet, and the way the plan's own
//! example is written. In C that expression means `reg(X) & (0x40 == 0)`, which
//! is 0 for every input: a part author who wrote the datasheet-shaped thing
//! would get a guard that is always false and never know. There are no floats
//! and no user-defined functions, so native and wasm agree bit for bit.
//!
//! Totality
//! ========
//! [`Expr::eval`] cannot panic and cannot fail. Every arithmetic operation is
//! wrapping or saturating; division and modulo by zero yield `0` and are
//! reported through [`EvalCtx::note_divide_by_zero`] as a fidelity note rather
//! than a trap. An unknown name resolves through the context, which answers `0`
//! for anything it does not know — the same answer an unprogrammed register
//! gives. A guard is true when it evaluates non-zero.

use std::fmt;

// ─── the tree ──────────────────────────────────────────────────────────────

/// A parsed rule expression. Built once by [`Expr::parse`] and evaluated many
/// times; evaluation allocates nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A literal (decimal or `0x` hex).
    Int(i64),
    /// `reg(NAME)` — the register's current STORED word.
    Reg(String),
    /// `reported(NAME)` — the word the register would put on the wire RIGHT
    /// NOW: its stored value for an ordinary register, and the fully encoded
    /// measurement for one with a `source:`.
    ///
    /// The two differ for exactly the registers that matter here. A measurement
    /// register's stored word is its RESET value forever — nothing writes it,
    /// because the value is computed at read time from the stimulus channel
    /// through `scale_from`, `clamp_from`, `calendar:` and the rest. `reg()` on
    /// such a register answers 0, honestly and uselessly.
    ///
    /// `input(KEY)` is not the same thing either: it borrows only the
    /// register's `encode:`, so a part whose counts-per-unit comes from
    /// `scale_from` (the ADXL345's range bits) gets the raw engineering value
    /// instead of the count. A FIFO that packs what the data registers report,
    /// and an alarm that compares against the clock the time registers report,
    /// both need this and nothing else will do.
    Reported(String),
    /// `field(REG.FIELD)` — a named bit-field of a register, shifted down.
    Field(String, String),
    /// `var(NAME)` — a rule-machine variable.
    Var(String),
    /// `input(KEY)` — a SimInput channel, encoded to an integer.
    Input(String),
    /// `fifo_len(NAME)` — how many entries a FIFO holds.
    FifoLen(String),
    /// `pin(NAME)` — the CURRENT level of a pad this part observes or drives,
    /// as 0 or 1.
    ///
    /// ⚠️ This is the only name in the vocabulary whose value is a SNAPSHOT
    /// rather than a stored word, and that is the whole point of it. One MMIO
    /// store can move several pads at once (a BSRR write sets CLK and clears
    /// DIO in one instruction), and the engine samples EVERY observed pad
    /// before it raises a single event — so inside any rule, `pin(X)` is the
    /// level pad X holds AFTER that store, for every X, not the level it held
    /// when some earlier pad's event was raised.
    ///
    /// Without it a two-wire protocol cannot be decoded at all: a TM1637 START
    /// is "DIO fell WHILE CLK was high", a condition over two pads that a
    /// per-pad edge event can only answer with a stale level for the other one.
    Pin(String),
    /// `frame_byte(N)` — byte N of the message frame this event closed, as the
    /// part received it on the wire, MOSI order.
    ///
    /// ⚠️ The general shape for a FRAMED part, deliberately chosen over one
    /// name per byte. A MAX7219 transaction is `[address, data]`, a
    /// chained-74HC595 module's is `[segments, digit select]`, an nRF24L01+
    /// command is `[opcode, payload…]`: the same 16 bits mean something
    /// different per part, and only the part's own rules can say which. An
    /// `opcode` / `payload` pair would have named the MAX7219's two bytes and
    /// nothing else's — and `frames.opcode_byte` still records byte 0 in
    /// `var(opcode)` for the parts whose datasheet calls it that, because a
    /// rule that fires on a LATER event needs it to have been REMEMBERED.
    ///
    /// The index is a LITERAL, checked at load against the declared
    /// `frames.length`: `frame_byte(2)` of a two-byte frame is a load error,
    /// not a silent 0. An index past the bytes actually clocked — a short frame
    /// closed by the transaction boundary — reads 0.
    FrameByte(usize),
    /// `written` — the value the master just wrote (0 outside a `write:` rule).
    Written,
    /// `state == NAME` (`negated` ⇒ `state != NAME`).
    StateIs { name: String, negated: bool },
    /// A prefix operator.
    Unary(UnOp, Box<Expr>),
    /// An infix operator.
    Binary(BinOp, Box<Expr>, Box<Expr>),
}

/// Prefix operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `!x` — 1 when `x` is zero, else 0.
    Not,
    /// `~x` — bitwise complement.
    BitNot,
    /// `-x` — wrapping negation.
    Neg,
    /// `abs(x)` — magnitude.
    ///
    /// The one function over an EXPRESSION rather than a name, and it is here
    /// because a sign-magnitude wire format cannot be written without it: an
    /// NMEA position is `DDMM.mmmm` plus a separate hemisphere character, so
    /// the number and its sign are two different fields of the sentence.
    /// Spelling it out of comparisons — `(x < 0) * -x + (x >= 0) * x` — is the
    /// same value written so that nobody reading the descriptor can see what
    /// it means.
    ///
    /// `abs(i64::MIN)` saturates to `i64::MAX` rather than wrapping to a
    /// negative, which keeps the evaluator total and keeps the one value that
    /// has no positive counterpart from reading as its own negation.
    Abs,
}

/// Infix operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
}

/// What an expression may read. Every method has a total answer; the engine's
/// implementation returns `0` for a name it does not know rather than failing,
/// which is what makes [`Expr::eval`] total.
pub trait EvalCtx {
    /// Current stored word of a register, as an integer.
    fn reg(&self, name: &str) -> i64;
    /// A named bit-field of a register, shifted down to bit 0.
    fn field(&self, register: &str, field: &str) -> i64;
    /// A rule-machine variable.
    fn var(&self, name: &str) -> i64;
    /// A SimInput channel, already encoded to an integer by the register's
    /// `encode:` (or truncated when the key has none).
    fn input(&self, key: &str) -> i64;
    /// Number of entries currently in a FIFO.
    /// The word a register would put on the wire right now — see
    /// [`Expr::Reported`]. A context with no register file answers 0, the same
    /// answer it gives [`reg`](Self::reg).
    fn reported(&self, name: &str) -> i64;
    fn fifo_len(&self, name: &str) -> i64;
    /// The current level of a named pad, 0 or 1. See [`Expr::Pin`].
    ///
    /// ⚠️ REQUIRED, not defaulted. A `fn pin(&self, _: &str) -> i64 { 0 }`
    /// default would compile everywhere, pass every existing test, and answer
    /// "low" for every pad on whichever transport forgot to implement it — so a
    /// descriptor guarded on `pin(CLK)` would decode nothing and look like a
    /// part that was never clocked. A context with no pads at all answers 0 in
    /// its own impl, where the reason is written down.
    fn pin(&self, name: &str) -> i64;
    /// Byte `index` of the frame the current `frame` event closed, MOSI order.
    ///
    /// ⚠️ REQUIRED, not defaulted, for the same reason [`Self::pin`] is. A
    /// `{ 0 }` default would compile on every transport and answer "the master
    /// sent 0x00" for every byte on whichever one forgot to wire it — a
    /// MAX7219 descriptor dispatching on the address byte would then write every
    /// frame into the no-op register and paint nothing, with every unit test
    /// green.
    ///
    /// Out of range — a short frame, or an index past what was clocked — is 0.
    fn frame_byte(&self, index: usize) -> i64;
    /// The value the master just wrote, inside a `write:` rule.
    fn written(&self) -> i64;
    /// The rule machine's current state name.
    fn state(&self) -> &str;
    /// Record that a division or modulo by zero was evaluated to 0. The default
    /// drops it — an engine that keeps a fidelity census overrides it.
    fn note_divide_by_zero(&self) {}
}

impl Expr {
    /// Evaluate against `ctx`. Total: never panics, never fails.
    pub fn eval(&self, ctx: &dyn EvalCtx) -> i64 {
        match self {
            Expr::Int(v) => *v,
            Expr::Reg(name) => ctx.reg(name),
            Expr::Field(r, f) => ctx.field(r, f),
            Expr::Var(name) => ctx.var(name),
            Expr::Input(key) => ctx.input(key),
            Expr::Reported(name) => ctx.reported(name),
            Expr::FifoLen(name) => ctx.fifo_len(name),
            Expr::Pin(name) => ctx.pin(name),
            Expr::FrameByte(index) => ctx.frame_byte(*index),
            Expr::Written => ctx.written(),
            Expr::StateIs { name, negated } => {
                let same = ctx.state() == name.as_str();
                i64::from(same != *negated)
            }
            Expr::Unary(op, inner) => {
                let v = inner.eval(ctx);
                match op {
                    UnOp::Not => i64::from(v == 0),
                    UnOp::BitNot => !v,
                    UnOp::Neg => v.wrapping_neg(),
                    UnOp::Abs => v.saturating_abs(),
                }
            }
            Expr::Binary(op, lhs, rhs) => {
                // Short-circuit before touching the right-hand side, so
                // `x != 0 && 100 / x > 2` never divides by zero at all.
                match op {
                    BinOp::AndAnd => {
                        return if lhs.eval(ctx) == 0 {
                            0
                        } else {
                            i64::from(rhs.eval(ctx) != 0)
                        }
                    }
                    BinOp::OrOr => {
                        return if lhs.eval(ctx) != 0 {
                            1
                        } else {
                            i64::from(rhs.eval(ctx) != 0)
                        }
                    }
                    _ => {}
                }
                let a = lhs.eval(ctx);
                let b = rhs.eval(ctx);
                match op {
                    BinOp::Add => a.wrapping_add(b),
                    BinOp::Sub => a.wrapping_sub(b),
                    BinOp::Mul => a.wrapping_mul(b),
                    BinOp::Div => {
                        if b == 0 {
                            ctx.note_divide_by_zero();
                            0
                        } else {
                            a.wrapping_div(b)
                        }
                    }
                    BinOp::Rem => {
                        if b == 0 {
                            ctx.note_divide_by_zero();
                            0
                        } else {
                            a.wrapping_rem(b)
                        }
                    }
                    // A shift distance outside 0..63 is not UB here: it
                    // saturates to "everything shifted out", which is what a
                    // reader of `x >> 64` means and what silicon's barrel
                    // shifter would produce for a width this model does not have.
                    BinOp::Shl => {
                        if !(0..64).contains(&b) {
                            0
                        } else {
                            a.wrapping_shl(b as u32)
                        }
                    }
                    BinOp::Shr => {
                        if !(0..64).contains(&b) {
                            if a < 0 {
                                -1
                            } else {
                                0
                            }
                        } else {
                            a.wrapping_shr(b as u32)
                        }
                    }
                    BinOp::BitAnd => a & b,
                    BinOp::BitOr => a | b,
                    BinOp::BitXor => a ^ b,
                    BinOp::Eq => i64::from(a == b),
                    BinOp::Ne => i64::from(a != b),
                    BinOp::Lt => i64::from(a < b),
                    BinOp::Le => i64::from(a <= b),
                    BinOp::Gt => i64::from(a > b),
                    BinOp::Ge => i64::from(a >= b),
                    BinOp::AndAnd | BinOp::OrOr => unreachable!("handled above"),
                }
            }
        }
    }

    /// Evaluate as a guard: non-zero is true.
    pub fn is_true(&self, ctx: &dyn EvalCtx) -> bool {
        self.eval(ctx) != 0
    }

    /// Every register name this expression reads, for load-time validation.
    pub fn registers(&self, out: &mut Vec<String>) {
        match self {
            Expr::Reg(n) | Expr::Reported(n) => out.push(n.clone()),
            Expr::Field(r, _) => out.push(r.clone()),
            Expr::Unary(_, i) => i.registers(out),
            Expr::Binary(_, a, b) => {
                a.registers(out);
                b.registers(out);
            }
            _ => {}
        }
    }

    /// Every pad this expression reads through `pin()`.
    ///
    /// The twin of [`registers`](Self::registers), and it exists for the same
    /// reason: a name the part does not declare must be a LOAD ERROR, not a
    /// silent zero. `pin()` needs it more than `reg()` does — an undeclared
    /// register is usually a typo in a map the author is staring at, while an
    /// undeclared pad is a guard that reads "low" forever on a part whose whole
    /// behaviour is which line moved.
    pub fn pin_names(&self, out: &mut Vec<String>) {
        match self {
            Expr::Pin(n) => out.push(n.clone()),
            Expr::Unary(_, i) => i.pin_names(out),
            Expr::Binary(_, a, b) => {
                a.pin_names(out);
                b.pin_names(out);
            }
            _ => {}
        }
    }

    /// Every frame byte this expression reads through `frame_byte()`.
    ///
    /// The twin of [`pin_names`](Self::pin_names), and it exists for the same
    /// reason: a part that declares no `frames:` has no frame to read a byte
    /// of, and an index past the declared `frames.length` names a byte the wire
    /// never carried. Both are load errors rather than a silent 0.
    pub fn frame_byte_indices(&self, out: &mut Vec<usize>) {
        match self {
            Expr::FrameByte(i) => out.push(*i),
            Expr::Unary(_, i) => i.frame_byte_indices(out),
            Expr::Binary(_, a, b) => {
                a.frame_byte_indices(out);
                b.frame_byte_indices(out);
            }
            _ => {}
        }
    }

    /// Parse an expression. The error names the offending token and its byte
    /// offset in `src`.
    pub fn parse(src: &str) -> Result<Expr, ExprError> {
        let tokens = lex(src)?;
        let mut p = Parser { tokens, pos: 0 };
        let e = p.parse_expr()?;
        if p.pos < p.tokens.len() {
            let t = &p.tokens[p.pos];
            return Err(ExprError {
                at: t.at,
                token: t.text.clone(),
                message: "trailing input after a complete expression".into(),
            });
        }
        Ok(e)
    }
}

// ─── errors ────────────────────────────────────────────────────────────────

/// A parse failure: what was wrong, which token, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprError {
    /// Byte offset of the offending token within the source string.
    pub at: usize,
    /// The offending token's text (empty at end of input).
    pub token: String,
    /// What the parser expected.
    pub message: String,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.token.is_empty() {
            write!(
                f,
                "{} at end of expression (offset {})",
                self.message, self.at
            )
        } else {
            write!(
                f,
                "{} at '{}' (offset {})",
                self.message, self.token, self.at
            )
        }
    }
}

impl std::error::Error for ExprError {}

// ─── lexer ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    text: String,
    at: usize,
    kind: Tok,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Int(i64),
    Ident,
    Punct,
}

/// Two-character operators, longest first so `<<` never lexes as `<` `<`.
const PUNCT2: &[&str] = &["==", "!=", "<=", ">=", "<<", ">>", "&&", "||"];
const PUNCT1: &[char] = &[
    '+', '-', '*', '/', '%', '&', '|', '^', '~', '!', '<', '>', '(', ')', '.',
];

fn lex(src: &str) -> Result<Vec<Token>, ExprError> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            let (radix, mut j) = if src[i..].starts_with("0x") || src[i..].starts_with("0X") {
                (16, i + 2)
            } else {
                (10, i)
            };
            let digits_start = j;
            while j < b.len() {
                let d = b[j] as char;
                let ok = d == '_'
                    || if radix == 16 {
                        d.is_ascii_hexdigit()
                    } else {
                        d.is_ascii_digit()
                    };
                if !ok {
                    break;
                }
                j += 1;
            }
            let text = &src[start..j];
            let digits = src[digits_start..j].replace('_', "");
            let value = i64::from_str_radix(&digits, radix).map_err(|e| ExprError {
                at: start,
                token: text.to_string(),
                message: format!("not an integer literal ({e})"),
            })?;
            out.push(Token {
                text: text.to_string(),
                at: start,
                kind: Tok::Int(value),
            });
            i = j;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Token {
                text: src[start..i].to_string(),
                at: start,
                kind: Tok::Ident,
            });
            continue;
        }
        if let Some(p) = PUNCT2.iter().find(|p| src[i..].starts_with(**p)) {
            out.push(Token {
                text: (*p).to_string(),
                at: i,
                kind: Tok::Punct,
            });
            i += p.len();
            continue;
        }
        if PUNCT1.contains(&c) {
            out.push(Token {
                text: c.to_string(),
                at: i,
                kind: Tok::Punct,
            });
            i += 1;
            continue;
        }
        return Err(ExprError {
            at: i,
            token: c.to_string(),
            message: "unexpected character".into(),
        });
    }
    Ok(out)
}

// ─── parser ────────────────────────────────────────────────────────────────

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn eat_punct(&mut self, want: &str) -> bool {
        match self.peek() {
            Some(t) if t.kind == Tok::Punct && t.text == want => {
                self.pos += 1;
                true
            }
            _ => false,
        }
    }

    fn err(&self, message: &str) -> ExprError {
        match self.peek() {
            Some(t) => ExprError {
                at: t.at,
                token: t.text.clone(),
                message: message.to_string(),
            },
            None => ExprError {
                at: self.tokens.last().map(|t| t.at + t.text.len()).unwrap_or(0),
                token: String::new(),
                message: message.to_string(),
            },
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, ExprError> {
        self.parse_binary(0)
    }

    /// One table-driven precedence climb. Level 0 is the loosest.
    fn parse_binary(&mut self, level: usize) -> Result<Expr, ExprError> {
        const LEVELS: &[&[(&str, BinOp)]] = &[
            &[("||", BinOp::OrOr)],
            &[("&&", BinOp::AndAnd)],
            // `==` / `!=` / the relations sit ABOVE the bitwise operators on
            // purpose; see the module note on `reg(X) & 0x40 == 0`.
            &[("==", BinOp::Eq), ("!=", BinOp::Ne)],
            &[
                ("<=", BinOp::Le),
                (">=", BinOp::Ge),
                ("<", BinOp::Lt),
                (">", BinOp::Gt),
            ],
            &[("|", BinOp::BitOr)],
            &[("^", BinOp::BitXor)],
            &[("&", BinOp::BitAnd)],
            &[("<<", BinOp::Shl), (">>", BinOp::Shr)],
            &[("+", BinOp::Add), ("-", BinOp::Sub)],
            &[("*", BinOp::Mul), ("/", BinOp::Div), ("%", BinOp::Rem)],
        ];
        if level >= LEVELS.len() {
            return self.parse_unary();
        }
        let mut lhs = self.parse_binary(level + 1)?;
        while let Some(t) = self.peek() {
            if t.kind != Tok::Punct {
                break;
            }
            let Some(&(_, op)) = LEVELS[level].iter().find(|(s, _)| *s == t.text) else {
                break;
            };
            self.pos += 1;
            let rhs = self.parse_binary(level + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ExprError> {
        for (text, op) in [("!", UnOp::Not), ("~", UnOp::BitNot), ("-", UnOp::Neg)] {
            if self.eat_punct(text) {
                let inner = self.parse_unary()?;
                return Ok(Expr::Unary(op, Box::new(inner)));
            }
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, ExprError> {
        let Some(t) = self.peek().cloned() else {
            return Err(self.err("expected a value"));
        };
        match t.kind {
            Tok::Int(v) => {
                self.pos += 1;
                Ok(Expr::Int(v))
            }
            Tok::Punct if t.text == "(" => {
                self.pos += 1;
                let inner = self.parse_expr()?;
                if !self.eat_punct(")") {
                    return Err(self.err("expected ')'"));
                }
                Ok(inner)
            }
            Tok::Ident => {
                self.pos += 1;
                match t.text.as_str() {
                    "written" => Ok(Expr::Written),
                    "state" => {
                        let negated = if self.eat_punct("==") {
                            false
                        } else if self.eat_punct("!=") {
                            true
                        } else {
                            return Err(self.err(
                                "`state` must be compared with '==' or '!=' to a state name",
                            ));
                        };
                        let name = self.ident("a state name")?;
                        Ok(Expr::StateIs { name, negated })
                    }
                    // The one function over an EXPRESSION. See [`UnOp::Abs`].
                    "abs" => {
                        if !self.eat_punct("(") {
                            return Err(self.err("expected '(' after `abs`"));
                        }
                        let inner = self.parse_expr()?;
                        if !self.eat_punct(")") {
                            return Err(self.err("expected ')'"));
                        }
                        Ok(Expr::Unary(UnOp::Abs, Box::new(inner)))
                    }
                    // The one function over an INTEGER LITERAL: a frame byte
                    // is addressed by position, not by name.
                    "frame_byte" => {
                        if !self.eat_punct("(") {
                            return Err(self.err("expected '(' after `frame_byte`"));
                        }
                        let index =
                            match self.peek().cloned() {
                                Some(t) => match t.kind {
                                    Tok::Int(v) if v >= 0 => {
                                        self.pos += 1;
                                        v as usize
                                    }
                                    _ => return Err(self.err(
                                        "`frame_byte` takes a non-negative integer LITERAL — the \
                                         index is checked at load against the declared \
                                         `frames.length`, which a computed one could not be",
                                    )),
                                },
                                None => return Err(self.err("expected a frame byte index")),
                            };
                        if !self.eat_punct(")") {
                            return Err(self.err("expected ')'"));
                        }
                        Ok(Expr::FrameByte(index))
                    }
                    "reg" | "reported" | "var" | "input" | "fifo_len" | "pin" => {
                        if !self.eat_punct("(") {
                            return Err(self.err(&format!("expected '(' after `{}`", t.text)));
                        }
                        let arg = self.ident("a name")?;
                        if !self.eat_punct(")") {
                            return Err(self.err("expected ')'"));
                        }
                        Ok(match t.text.as_str() {
                            "reg" => Expr::Reg(arg),
                            "reported" => Expr::Reported(arg),
                            "var" => Expr::Var(arg),
                            "input" => Expr::Input(arg),
                            "pin" => Expr::Pin(arg),
                            _ => Expr::FifoLen(arg),
                        })
                    }
                    "field" => {
                        if !self.eat_punct("(") {
                            return Err(self.err("expected '(' after `field`"));
                        }
                        let reg = self.ident("a register name")?;
                        if !self.eat_punct(".") {
                            return Err(self.err("expected '.' — `field(REGISTER.FIELD)`"));
                        }
                        let field = self.ident("a field name")?;
                        if !self.eat_punct(")") {
                            return Err(self.err("expected ')'"));
                        }
                        Ok(Expr::Field(reg, field))
                    }
                    other => Err(ExprError {
                        at: t.at,
                        token: t.text.clone(),
                        message: format!(
                            "unknown name `{other}`. The vocabulary is reg(), reported(), \
                             field(), var(), input(), fifo_len(), pin(), frame_byte(), \
                             abs(), `written`, and \
                             `state == NAME` — there are no bare identifiers and no \
                             user-defined functions"
                        ),
                    }),
                }
            }
            _ => Err(self.err("expected a value")),
        }
    }

    fn ident(&mut self, what: &str) -> Result<String, ExprError> {
        match self.peek() {
            Some(t) if t.kind == Tok::Ident => {
                let s = t.text.clone();
                self.pos += 1;
                Ok(s)
            }
            _ => Err(self.err(&format!("expected {what}"))),
        }
    }
}

// ─── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Ctx {
        regs: BTreeMap<String, i64>,
        vars: BTreeMap<String, i64>,
        inputs: BTreeMap<String, i64>,
        fifos: BTreeMap<String, i64>,
        state: String,
        written: i64,
        frame: Vec<u8>,
        div0: std::cell::Cell<u32>,
    }

    impl EvalCtx for Ctx {
        fn reg(&self, name: &str) -> i64 {
            self.regs.get(name).copied().unwrap_or(0)
        }
        fn field(&self, register: &str, field: &str) -> i64 {
            self.regs
                .get(&format!("{register}.{field}"))
                .copied()
                .unwrap_or(0)
        }
        fn var(&self, name: &str) -> i64 {
            self.vars.get(name).copied().unwrap_or(0)
        }
        fn input(&self, key: &str) -> i64 {
            self.inputs.get(key).copied().unwrap_or(0)
        }
        fn reported(&self, name: &str) -> i64 {
            self.reg(name)
        }
        fn frame_byte(&self, index: usize) -> i64 {
            self.frame.get(index).map(|b| i64::from(*b)).unwrap_or(0)
        }
        fn pin(&self, name: &str) -> i64 {
            // The test context stores pads in the same map as vars, prefixed,
            // so one fixture can pose both.
            self.vars.get(&format!("pin:{name}")).copied().unwrap_or(0)
        }
        fn fifo_len(&self, name: &str) -> i64 {
            self.fifos.get(name).copied().unwrap_or(0)
        }
        fn written(&self) -> i64 {
            self.written
        }
        fn state(&self) -> &str {
            &self.state
        }
        fn note_divide_by_zero(&self) {
            self.div0.set(self.div0.get() + 1);
        }
    }

    fn ev(src: &str) -> i64 {
        Expr::parse(src)
            .unwrap_or_else(|e| panic!("parse {src:?}: {e}"))
            .eval(&Ctx::default())
    }

    #[test]
    fn every_operator_evaluates() {
        // Arithmetic.
        assert_eq!(ev("1 + 2"), 3);
        assert_eq!(ev("7 - 9"), -2);
        assert_eq!(ev("6 * 7"), 42);
        assert_eq!(ev("7 / 2"), 3);
        assert_eq!(ev("7 % 3"), 1);
        // Bitwise.
        assert_eq!(ev("0xF0 & 0x3C"), 0x30);
        assert_eq!(ev("0xF0 | 0x0F"), 0xFF);
        assert_eq!(ev("0xFF ^ 0x0F"), 0xF0);
        assert_eq!(ev("~0"), -1);
        assert_eq!(ev("1 << 8"), 256);
        assert_eq!(ev("256 >> 4"), 16);
        // Comparison.
        assert_eq!(ev("3 == 3"), 1);
        assert_eq!(ev("3 != 3"), 0);
        assert_eq!(ev("2 < 3"), 1);
        assert_eq!(ev("3 <= 3"), 1);
        assert_eq!(ev("4 > 3"), 1);
        assert_eq!(ev("3 >= 4"), 0);
        // Logical.
        assert_eq!(ev("1 && 0"), 0);
        assert_eq!(ev("1 && 2"), 1);
        assert_eq!(ev("0 || 5"), 1);
        assert_eq!(ev("0 || 0"), 0);
        assert_eq!(ev("!0"), 1);
        assert_eq!(ev("!7"), 0);
        // Unary minus and parentheses.
        assert_eq!(ev("-(3 + 4)"), -7);
    }

    #[test]
    fn precedence_is_the_documented_one() {
        assert_eq!(ev("2 + 3 * 4"), 14);
        assert_eq!(ev("(2 + 3) * 4"), 20);
        assert_eq!(ev("1 << 2 + 1"), 8, "+ binds tighter than <<");
        // The deliberate deviation from C: the datasheet-shaped guard.
        assert_eq!(ev("0x40 & 0x40 == 0"), 0);
        assert_eq!(ev("0x40 & 0x20 == 0"), 1);
        assert_eq!(ev("1 | 0 && 0"), 0, "&& binds looser than |");
        assert_eq!(ev("0 || 1 && 0"), 0, "&& binds tighter than ||");
    }

    #[test]
    fn hex_and_underscores() {
        assert_eq!(ev("0xFF"), 255);
        assert_eq!(ev("0X10"), 16);
        assert_eq!(ev("1_000_000"), 1_000_000);
        assert_eq!(ev("0xFF_FF"), 65535);
    }

    #[test]
    fn names_resolve_through_the_context() {
        let mut ctx = Ctx {
            state: "measuring".into(),
            written: 0x08,
            ..Default::default()
        };
        ctx.regs.insert("COMMAND".into(), 0x28);
        ctx.regs.insert("COMMAND.PROX_RDY".into(), 1);
        ctx.vars.insert("count".into(), 5);
        ctx.inputs.insert("weight".into(), 1234);
        ctx.fifos.insert("samples".into(), 3);

        let e = |s: &str| Expr::parse(s).unwrap().eval(&ctx);
        assert_eq!(e("reg(COMMAND)"), 0x28);
        assert_eq!(e("field(COMMAND.PROX_RDY)"), 1);
        assert_eq!(e("var(count)"), 5);
        assert_eq!(e("input(weight)"), 1234);
        assert_eq!(e("fifo_len(samples)"), 3);
        // `abs()` is the one function over an expression; see `UnOp::Abs`.
        assert_eq!(e("abs(0 - 7)"), 7);
        assert_eq!(e("abs(7)"), 7);
        assert_eq!(e("abs(reg(NOPE) - 5) * 2"), 10);
        assert_eq!(e("written"), 0x08);
        assert_eq!(e("state == measuring"), 1);
        assert_eq!(e("state == idle"), 0);
        assert_eq!(e("state != idle"), 1);
        // An unknown name is 0, not an error: totality.
        assert_eq!(e("reg(NOPE) + var(nope) + input(nope) + fifo_len(nope)"), 0);
    }

    #[test]
    fn division_by_zero_is_zero_and_noted() {
        let ctx = Ctx::default();
        assert_eq!(Expr::parse("7 / var(zero)").unwrap().eval(&ctx), 0);
        assert_eq!(Expr::parse("7 % var(zero)").unwrap().eval(&ctx), 0);
        assert_eq!(ctx.div0.get(), 2, "both are reported as fidelity notes");
    }

    #[test]
    fn short_circuit_skips_the_right_hand_side() {
        let ctx = Ctx::default();
        // `var(zero)` is 0, so the division must never be evaluated at all.
        assert_eq!(
            Expr::parse("var(zero) != 0 && 100 / var(zero) > 2")
                .unwrap()
                .eval(&ctx),
            0
        );
        assert_eq!(ctx.div0.get(), 0, "the guard short-circuited");
    }

    #[test]
    fn shifts_past_the_word_saturate_rather_than_panic() {
        assert_eq!(ev("1 << 200"), 0);
        assert_eq!(ev("1 << -1"), 0);
        assert_eq!(ev("256 >> 200"), 0);
        assert_eq!(ev("-1 >> 200"), -1);
    }

    #[test]
    fn overflow_wraps_rather_than_panicking() {
        // i64::MIN / -1 overflows in a checked build; the evaluator must not.
        // i64::MIN has no literal spelling (the lexer reads the digits before
        // the unary minus applies), so build it the way a part author would.
        let min = format!("(0 - {} - 1)", i64::MAX);
        assert_eq!(ev(&min), i64::MIN);
        assert_eq!(ev(&format!("{min} / (0 - 1)")), i64::MIN);
        assert_eq!(ev(&format!("{min} % (0 - 1)")), 0);
        assert_eq!(ev(&format!("{min} - 1")), i64::MAX);
        let src = format!("({}) * 2", i64::MAX);
        assert_eq!(ev(&src), i64::MAX.wrapping_mul(2));
        // A literal too big for i64 is a LOAD error, not a silent truncation.
        let e = Expr::parse("9223372036854775808").unwrap_err();
        assert!(e.to_string().contains("not an integer literal"), "{e}");
    }

    #[test]
    fn errors_name_the_token_and_its_offset() {
        let e = Expr::parse("reg(A) + $").unwrap_err();
        assert_eq!(e.at, 9);
        assert_eq!(e.token, "$");
        assert!(e.to_string().contains("unexpected character"), "{e}");

        let e = Expr::parse("reg(A").unwrap_err();
        assert_eq!(e.token, "", "end of input");
        assert!(e.to_string().contains("expected ')'"), "{e}");

        let e = Expr::parse("1 + ").unwrap_err();
        assert!(e.to_string().contains("expected a value"), "{e}");

        let e = Expr::parse("temperature * 2").unwrap_err();
        assert_eq!(e.at, 0);
        assert_eq!(e.token, "temperature");
        assert!(e.to_string().contains("unknown name"), "{e}");

        let e = Expr::parse("field(A_B)").unwrap_err();
        assert!(e.to_string().contains("expected '.'"), "{e}");

        let e = Expr::parse("state").unwrap_err();
        assert!(e.to_string().contains("'==' or '!='"), "{e}");

        let e = Expr::parse("1 2").unwrap_err();
        assert_eq!(e.at, 2);
        assert!(e.to_string().contains("trailing input"), "{e}");
    }

    #[test]
    fn registers_collects_every_name_read() {
        let mut names = Vec::new();
        Expr::parse("reg(A) + field(B.C) * var(d)")
            .unwrap()
            .registers(&mut names);
        assert_eq!(names, vec!["A".to_string(), "B".to_string()]);
    }

    /// Determinism and totality over a wide spread of inputs: the same tree and
    /// the same context always yield the same integer, and nothing panics.
    #[test]
    fn evaluation_is_deterministic_and_total() {
        let sources = [
            "reg(A) / var(b)",
            "reg(A) % var(b)",
            "(reg(A) << var(b)) ^ ~input(c)",
            "reg(A) & 0xFF == var(b) || fifo_len(f) > 0",
            "-reg(A) * (var(b) - input(c)) + written",
            "!(reg(A) >= var(b)) && state == idle",
        ];
        let trees: Vec<Expr> = sources.iter().map(|s| Expr::parse(s).unwrap()).collect();
        // A spread that includes both zeroes (division) and the extremes
        // (overflow), because both are where a naive evaluator traps.
        let values = [
            0i64,
            1,
            -1,
            2,
            255,
            -255,
            i64::MAX,
            i64::MIN,
            i64::MIN + 1,
            1 << 40,
        ];
        for &a in &values {
            for &b in &values {
                for &c in &values {
                    let mut ctx = Ctx {
                        state: "idle".into(),
                        written: c,
                        ..Default::default()
                    };
                    ctx.regs.insert("A".into(), a);
                    ctx.vars.insert("b".into(), b);
                    ctx.inputs.insert("c".into(), c);
                    ctx.fifos.insert("f".into(), a.rem_euclid(4));
                    for tree in &trees {
                        let first = tree.eval(&ctx);
                        let second = tree.eval(&ctx);
                        assert_eq!(first, second, "evaluation is not deterministic: {tree:?}");
                    }
                }
            }
        }
    }
}
