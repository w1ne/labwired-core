// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Behavioural sources: ngspice's `B` element and the `E`/`G` `VALUE={...}`
//! forms, whose value is an expression of node voltages and branch currents.
//!
//! An expression is parsed once into a tree and evaluated at each Newton
//! iteration together with its exact partial derivatives (forward-mode
//! automatic differentiation), so the solver stamps the true tangent, as
//! ngspice does with its symbolic derivatives.
//!
//! The grammar and every function below follow ngspice-42, checked against
//! it value by value:
//!
//! * `V(node)`, `V(node1, node2)` and `I(<element>)`, where the element carries
//!   a branch current (`V`, `L`, `E`, `H` or a voltage-form `B`); `time`
//!   (seconds, 0 at the operating point), `pi` and `e`.
//! * Lowest to highest: `c ? a : b` (right-associative), `||`, `&&`,
//!   `== != < > <= >=`, `+ -`, `* /`, unary `- + !`, then `^`/`**`. As in
//!   ngspice, `^` is left-associative and binds tighter than a unary sign:
//!   `-2^2` is -4 and `2^3^2` is 64. A comparison or logical operator gives
//!   1 or 0 and has no derivative.
//! * `x^y` and `pow(x, y)` are `|x|^y`; `pwr(x, y)` is `sign(x)·|x|^y`.
//! * `u(x)` is 1, 0.5 or 0 for positive, zero or negative `x`; `u2(x)`
//!   clamps `x` to `[0, 1]`; `uramp(x)` is `max(x, 0)`; `sgn(x)` is the sign;
//!   `nint(x)` rounds half to even. None of these steps has a derivative.
//! * `abs sqrt exp ln log log10 sin cos tan asin acos atan sinh cosh tanh
//!   asinh acosh atanh floor ceil min max`. `log` is the natural logarithm.
//! * `pwl(x, x1, y1, x2, y2, ...)` interpolates linearly between at least two
//!   points and extends the first and last segment past the ends. The points
//!   are literal numbers with strictly ascending `x`, as ngspice requires.
//!
//! `E`/`G ... TABLE {x} = (x1, y1) ...` lowers to [`Expression::table`]:
//! ngspice runs it as an XSPICE `pwl` with `input_domain=0.1` and
//! `fraction=TRUE`, so the output holds the end values outside the table
//! and each corner `x_i` is a parabola over `x_i ± d`, where `d` is a tenth
//! of the shorter segment next to it (at an end, of the one inner segment).
//! The parabola meets both segments with their slopes; at `x_i` it reads
//! `y_i + d·(s_upper − s_lower)/4`. Checked against ngspice-42 `.op`.
//!
//! ngspice clamps a value that leaves the reals (`1/0` reads 1e32,
//! `exp(800)` reads 1e99, `ln(0)` reads -1e99). Here a value that is not
//! finite is an error naming the source: a clamped value is a number no
//! circuit produces. An infinite slope at a finite value (`sqrt` at 0) is
//! taken as 0 for that Newton iterate; see [`Expression::evaluate`].

use super::netlist::NodeRef;

/// What a behavioural source sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehaviouralKind {
    /// `V=`: the voltage `v(n+) − v(n−)`. Adds a branch current.
    Voltage,
    /// `I=`: the current that flows from `n+` through the source to `n−`.
    Current,
}

/// One quantity an expression reads, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlName {
    /// `V(p)` or `V(p, n)`; names lower-cased.
    Voltage(String, Option<String>),
    /// `I(<element>)`; name lower-cased.
    Current(String),
}

/// One quantity an expression reads, bound to the circuit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// `v(p) − v(n)`.
    Voltage(NodeRef, NodeRef),
    /// The branch current with this index.
    Branch(usize),
}

/// A behavioural source.
#[derive(Debug, Clone, PartialEq)]
pub struct BehaviouralSource {
    /// Element name as written (`B1`, or `E1` for `E ... VALUE={...}`).
    pub name: String,
    /// Positive terminal.
    pub p: NodeRef,
    /// Negative terminal.
    pub n: NodeRef,
    /// Voltage or current form.
    pub kind: BehaviouralKind,
    /// The parsed expression.
    pub expression: Expression,
    /// The bound quantities, in the order of [`Expression::controls`].
    pub controls: Vec<Control>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binary {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Function {
    Abs,
    Sqrt,
    Exp,
    Ln,
    Log10,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    Floor,
    Ceil,
    Nint,
    Sgn,
    U,
    U2,
    Uramp,
    Min,
    Max,
    Pow,
    Pwr,
}

impl Function {
    fn lookup(name: &str) -> Option<(Self, usize)> {
        Some(match name {
            "abs" => (Self::Abs, 1),
            "sqrt" => (Self::Sqrt, 1),
            "exp" => (Self::Exp, 1),
            "ln" | "log" => (Self::Ln, 1),
            "log10" => (Self::Log10, 1),
            "sin" => (Self::Sin, 1),
            "cos" => (Self::Cos, 1),
            "tan" => (Self::Tan, 1),
            "asin" => (Self::Asin, 1),
            "acos" => (Self::Acos, 1),
            "atan" => (Self::Atan, 1),
            "sinh" => (Self::Sinh, 1),
            "cosh" => (Self::Cosh, 1),
            "tanh" => (Self::Tanh, 1),
            "asinh" => (Self::Asinh, 1),
            "acosh" => (Self::Acosh, 1),
            "atanh" => (Self::Atanh, 1),
            "floor" => (Self::Floor, 1),
            "ceil" => (Self::Ceil, 1),
            "nint" => (Self::Nint, 1),
            "sgn" => (Self::Sgn, 1),
            "u" => (Self::U, 1),
            "u2" => (Self::U2, 1),
            "uramp" => (Self::Uramp, 1),
            "min" => (Self::Min, 2),
            "max" => (Self::Max, 2),
            "pow" => (Self::Pow, 2),
            "pwr" => (Self::Pwr, 2),
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Number(f64),
    Control(usize),
    Time,
    Neg(Box<Node>),
    Not(Box<Node>),
    Binary(Binary, Box<Node>, Box<Node>),
    Ternary(Box<Node>, Box<Node>, Box<Node>),
    Call(Function, Vec<Node>),
    /// `pwl(...)` (`smooth` false) or a lowered `TABLE` (`smooth` true).
    Pwl {
        input: Box<Node>,
        points: Vec<(f64, f64)>,
        smooth: bool,
    },
}

/// A parsed behavioural expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Expression {
    root: Node,
    controls: Vec<ControlName>,
    uses_time: bool,
}

/// A value and its partial derivatives with respect to each control.
#[derive(Debug, Clone)]
struct Dual {
    value: f64,
    grad: Vec<f64>,
}

impl Dual {
    fn constant(value: f64, width: usize) -> Self {
        Self {
            value,
            grad: vec![0.0; width],
        }
    }

    /// `f(self)` with derivative `slope · d(self)`.
    fn chain(mut self, value: f64, slope: f64) -> Self {
        self.value = value;
        if slope == 0.0 {
            self.grad.fill(0.0);
        } else {
            self.grad.iter_mut().for_each(|g| *g *= slope);
        }
        self
    }

    /// `a·d(self) + b·d(other)`, with a zero factor skipped so an infinite
    /// slope on a branch that does not move does not turn into NaN.
    fn combine(mut self, a: f64, other: &Self, b: f64, value: f64) -> Self {
        for (g, h) in self.grad.iter_mut().zip(&other.grad) {
            let left = if *g == 0.0 { 0.0 } else { a * *g };
            let right = if *h == 0.0 { 0.0 } else { b * *h };
            *g = left + right;
        }
        self.value = value;
        self
    }
}

impl Expression {
    /// Parse the text after `V=`, `I=` or `VALUE=` (braces already removed).
    pub fn parse(text: &str) -> Result<Self, String> {
        let tokens = lex(text)?;
        let mut parser = Parser {
            tokens,
            at: 0,
            controls: Vec::new(),
            uses_time: false,
        };
        let root = parser.ternary()?;
        if let Some(token) = parser.tokens.get(parser.at) {
            return Err(format!("unexpected {} in expression", describe(token)));
        }
        Ok(Self {
            root,
            controls: parser.controls,
            uses_time: parser.uses_time,
        })
    }

    /// `E`/`G ... TABLE {input} = (x1, y1) ...`: the table looked up at
    /// `input`, held flat past both ends, with rounded corners (see the
    /// module notes).
    pub fn table(input: &str, points: Vec<(f64, f64)>) -> Result<Self, String> {
        check_points(&points)?;
        let mut expression = Self::parse(input)?;
        expression.root = Node::Pwl {
            input: Box::new(expression.root),
            points,
            smooth: true,
        };
        Ok(expression)
    }

    /// The largest move a Newton iterate may make in each control, when the
    /// expression holds a `TABLE`: a tenth of the narrowest table's span.
    ///
    /// A table is flat past both ends, so a full Newton step from a flat
    /// stretch lands anywhere and the next one jumps back: Newton cycles.
    /// Walking the input in bounded steps reaches the segment that holds the
    /// answer. Only the path changes; a limited step is never accepted as
    /// converged. `pwl()` extrapolates, so it takes the plain Newton step.
    pub fn step_limit(&self) -> Option<f64> {
        fn walk(node: &Node, limit: &mut Option<f64>) {
            match node {
                Node::Number(_) | Node::Control(_) | Node::Time => {}
                Node::Neg(inner) | Node::Not(inner) => walk(inner, limit),
                Node::Binary(_, a, b) => {
                    walk(a, limit);
                    walk(b, limit);
                }
                Node::Ternary(a, b, c) => {
                    walk(a, limit);
                    walk(b, limit);
                    walk(c, limit);
                }
                Node::Call(_, args) => args.iter().for_each(|arg| walk(arg, limit)),
                Node::Pwl {
                    input,
                    points,
                    smooth,
                } => {
                    walk(input, limit);
                    if *smooth {
                        let span = points[points.len() - 1].0 - points[0].0;
                        *limit = Some(limit.map_or(0.1 * span, |l: f64| l.min(0.1 * span)));
                    }
                }
            }
        }
        let mut limit = None;
        walk(&self.root, &mut limit);
        limit
    }

    /// The quantities the expression reads, in first-use order.
    pub fn controls(&self) -> &[ControlName] {
        &self.controls
    }

    /// Whether the expression reads `time`.
    pub fn uses_time(&self) -> bool {
        self.uses_time
    }

    /// The value at `controls` (in [`Self::controls`] order) and `time`.
    pub fn value(&self, controls: &[f64], time: f64) -> Result<f64, String> {
        self.evaluate(controls, time).map(|(value, _)| value)
    }

    /// The value and its partial derivative with respect to each control.
    pub fn evaluate(&self, controls: &[f64], time: f64) -> Result<(f64, Vec<f64>), String> {
        let mut result = eval(&self.root, controls, time)?;
        if !result.value.is_finite() {
            return Err(format!(
                "value {} is not finite at {controls:?}",
                result.value
            ));
        }
        // An infinite slope at a finite value is the edge of a domain
        // (`sqrt` at 0, `x^0.5` at 0): Newton's first iterate from 0 V lands
        // there. The slope only shapes the next Newton step, never the
        // answer, which still has to satisfy the exact equations; a zero
        // slope keeps that step finite.
        for slope in &mut result.grad {
            if !slope.is_finite() {
                *slope = 0.0;
            }
        }
        Ok((result.value, result.grad))
    }
}

/// At least two points, finite, with strictly ascending `x`.
fn check_points(points: &[(f64, f64)]) -> Result<(), String> {
    if points.len() < 2 {
        return Err(format!(
            "a table needs at least two points, got {}",
            points.len()
        ));
    }
    if points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return Err("a table point is not finite".into());
    }
    if let Some(pair) = points.windows(2).find(|pair| pair[1].0 <= pair[0].0) {
        return Err(format!(
            "table x values must be strictly ascending: {} then {}",
            pair[0].0, pair[1].0
        ));
    }
    Ok(())
}

/// The value and slope of a piecewise-linear table at `x`. Without `smooth`
/// the end segments extend; with it the ends hold flat and each corner is
/// rounded as in XSPICE `pwl` (see the module notes).
fn piecewise(points: &[(f64, f64)], x: f64, smooth: bool) -> (f64, f64) {
    let last = points.len() - 1;
    let slope = |i: usize| (points[i + 1].1 - points[i].1) / (points[i + 1].0 - points[i].0);
    if smooth {
        for (i, &(xi, yi)) in points.iter().enumerate() {
            let lower = if i == 0 { 0.0 } else { slope(i - 1) };
            let upper = if i == last { 0.0 } else { slope(i) };
            let width = match (i == 0, i == last) {
                (true, _) => points[1].0 - xi,
                (_, true) => xi - points[i - 1].0,
                _ => (xi - points[i - 1].0).min(points[i + 1].0 - xi),
            };
            let d = 0.1 * width;
            if (x - xi).abs() < d {
                let t = x - (xi - d);
                let bend = (upper - lower) / (4.0 * d);
                return (
                    yi - lower * d + lower * t + bend * t * t,
                    lower + 2.0 * bend * t,
                );
            }
        }
        if x <= points[0].0 {
            return (points[0].1, 0.0);
        }
        if x >= points[last].0 {
            return (points[last].1, 0.0);
        }
    }
    // The segment holding `x`; the first or last one past the ends.
    let i = points[1..last]
        .iter()
        .position(|&(xi, _)| x < xi)
        .unwrap_or(last - 1);
    let s = slope(i);
    (points[i].1 + s * (x - points[i].0), s)
}

fn truth(value: bool, width: usize) -> Dual {
    Dual::constant(if value { 1.0 } else { 0.0 }, width)
}

fn eval(node: &Node, controls: &[f64], time: f64) -> Result<Dual, String> {
    let width = controls.len();
    Ok(match node {
        Node::Number(value) => Dual::constant(*value, width),
        Node::Time => Dual::constant(time, width),
        Node::Control(index) => {
            let mut dual = Dual::constant(controls[*index], width);
            dual.grad[*index] = 1.0;
            dual
        }
        Node::Neg(inner) => {
            let inner = eval(inner, controls, time)?;
            let value = -inner.value;
            inner.chain(value, -1.0)
        }
        Node::Not(inner) => truth(eval(inner, controls, time)?.value == 0.0, width),
        Node::Ternary(condition, then, otherwise) => {
            if eval(condition, controls, time)?.value != 0.0 {
                eval(then, controls, time)?
            } else {
                eval(otherwise, controls, time)?
            }
        }
        Node::Binary(op, left, right) => {
            let a = eval(left, controls, time)?;
            let b = eval(right, controls, time)?;
            binary(*op, a, b)?
        }
        Node::Call(function, args) => {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(eval(arg, controls, time)?);
            }
            call(*function, values)?
        }
        Node::Pwl {
            input,
            points,
            smooth,
        } => {
            let x = eval(input, controls, time)?;
            let (value, slope) = piecewise(points, x.value, *smooth);
            x.chain(value, slope)
        }
    })
}

/// `|x|^y` with its derivative; the `ln|x|` term only when `y` moves.
fn power(x: Dual, y: Dual, signed: bool) -> Dual {
    let base = x.value.abs();
    let magnitude = base.powf(y.value);
    let sign = if signed && x.value < 0.0 { -1.0 } else { 1.0 };
    let value = sign * magnitude;
    // d|x|^y/dx = y·|x|^(y−1)·sgn(x); the outer sign of pwr cancels sgn(x).
    let dx = if signed {
        y.value * base.powf(y.value - 1.0)
    } else {
        y.value * base.powf(y.value - 1.0) * x.value.signum()
    };
    let moves = y.grad.iter().any(|g| *g != 0.0);
    let dy = if moves { value * base.ln() } else { 0.0 };
    x.combine(dx, &y, dy, value)
}

fn binary(op: Binary, a: Dual, b: Dual) -> Result<Dual, String> {
    let width = a.grad.len();
    Ok(match op {
        Binary::Add => {
            let value = a.value + b.value;
            a.combine(1.0, &b, 1.0, value)
        }
        Binary::Sub => {
            let value = a.value - b.value;
            a.combine(1.0, &b, -1.0, value)
        }
        Binary::Mul => {
            let (x, y) = (a.value, b.value);
            a.combine(y, &b, x, x * y)
        }
        Binary::Div => {
            let (x, y) = (a.value, b.value);
            a.combine(1.0 / y, &b, -x / (y * y), x / y)
        }
        Binary::Pow => power(a, b, false),
        Binary::Lt => truth(a.value < b.value, width),
        Binary::Gt => truth(a.value > b.value, width),
        Binary::Le => truth(a.value <= b.value, width),
        Binary::Ge => truth(a.value >= b.value, width),
        Binary::Eq => truth(a.value == b.value, width),
        Binary::Ne => truth(a.value != b.value, width),
        Binary::And => truth(a.value != 0.0 && b.value != 0.0, width),
        Binary::Or => truth(a.value != 0.0 || b.value != 0.0, width),
    })
}

fn call(function: Function, mut args: Vec<Dual>) -> Result<Dual, String> {
    if args.len() == 2 {
        let b = args.pop().expect("two arguments");
        let a = args.pop().expect("two arguments");
        return Ok(match function {
            Function::Min => {
                if a.value <= b.value {
                    a
                } else {
                    b
                }
            }
            Function::Max => {
                if a.value >= b.value {
                    a
                } else {
                    b
                }
            }
            Function::Pow => power(a, b, false),
            Function::Pwr => power(a, b, true),
            _ => unreachable!("arity checked at parse time"),
        });
    }
    let x = args.pop().expect("one argument");
    let v = x.value;
    let domain = |ok: bool, name: &str| {
        if ok {
            Ok(())
        } else {
            Err(format!("{v} is out of range for `{name}`"))
        }
    };
    let (value, slope) = match function {
        Function::Abs => (v.abs(), if v == 0.0 { 0.0 } else { v.signum() }),
        Function::Sqrt => {
            domain(v >= 0.0, "sqrt")?;
            let root = v.sqrt();
            (root, 0.5 / root)
        }
        Function::Exp => {
            let e = v.exp();
            (e, e)
        }
        Function::Ln => {
            domain(v > 0.0, "ln")?;
            (v.ln(), 1.0 / v)
        }
        Function::Log10 => {
            domain(v > 0.0, "log10")?;
            (v.log10(), 1.0 / (v * std::f64::consts::LN_10))
        }
        Function::Sin => (v.sin(), v.cos()),
        Function::Cos => (v.cos(), -v.sin()),
        Function::Tan => {
            let c = v.cos();
            (v.tan(), 1.0 / (c * c))
        }
        Function::Asin => {
            domain(v.abs() <= 1.0, "asin")?;
            (v.asin(), 1.0 / (1.0 - v * v).sqrt())
        }
        Function::Acos => {
            domain(v.abs() <= 1.0, "acos")?;
            (v.acos(), -1.0 / (1.0 - v * v).sqrt())
        }
        Function::Atan => (v.atan(), 1.0 / (1.0 + v * v)),
        Function::Sinh => (v.sinh(), v.cosh()),
        Function::Cosh => (v.cosh(), v.sinh()),
        Function::Tanh => {
            let t = v.tanh();
            (t, 1.0 - t * t)
        }
        Function::Asinh => (v.asinh(), 1.0 / (v * v + 1.0).sqrt()),
        Function::Acosh => {
            domain(v >= 1.0, "acosh")?;
            (v.acosh(), 1.0 / (v * v - 1.0).sqrt())
        }
        Function::Atanh => {
            domain(v.abs() < 1.0, "atanh")?;
            (v.atanh(), 1.0 / (1.0 - v * v))
        }
        Function::Floor => (v.floor(), 0.0),
        Function::Ceil => (v.ceil(), 0.0),
        Function::Nint => (v.round_ties_even(), 0.0),
        Function::Sgn => (
            if v > 0.0 {
                1.0
            } else if v < 0.0 {
                -1.0
            } else {
                0.0
            },
            0.0,
        ),
        Function::U => (
            if v > 0.0 {
                1.0
            } else if v < 0.0 {
                0.0
            } else {
                0.5
            },
            0.0,
        ),
        Function::U2 => (v.clamp(0.0, 1.0), 0.0),
        Function::Uramp => {
            if v > 0.0 {
                (v, 1.0)
            } else {
                (0.0, 0.0)
            }
        }
        Function::Min | Function::Max | Function::Pow | Function::Pwr => {
            unreachable!("arity checked at parse time")
        }
    };
    Ok(x.chain(value, slope))
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    /// `V(...)` or `I(...)`: the letter and the raw argument text.
    Probe(char, String),
    Op(&'static str),
    Open,
    Close,
    Comma,
}

fn describe(token: &Token) -> String {
    match token {
        Token::Number(value) => format!("number `{value}`"),
        Token::Name(name) => format!("name `{name}`"),
        Token::Probe(letter, args) => format!("`{letter}({args})`"),
        Token::Op(op) => format!("`{op}`"),
        Token::Open => "`(`".into(),
        Token::Close => "`)`".into(),
        Token::Comma => "`,`".into(),
    }
}

const OPERATORS: [&str; 18] = [
    "**", "<=", ">=", "==", "!=", "&&", "||", "+", "-", "*", "/", "^", "<", ">", "!", "?", ":", "=",
];

fn lex(text: &str) -> Result<Vec<Token>, String> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b' ' | b'\t' => index += 1,
            b'0'..=b'9' | b'.' => {
                let start = index;
                while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.')
                {
                    index += 1;
                }
                if index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
                    let mut probe = index + 1;
                    if probe < bytes.len() && (bytes[probe] == b'+' || bytes[probe] == b'-') {
                        probe += 1;
                    }
                    if probe < bytes.len() && bytes[probe].is_ascii_digit() {
                        index = probe;
                        while index < bytes.len() && bytes[index].is_ascii_digit() {
                            index += 1;
                        }
                    }
                }
                while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
                    index += 1;
                }
                let raw = &text[start..index];
                let value = super::netlist::parse_spice_value(raw)
                    .ok_or_else(|| format!("`{raw}` is not a SPICE value"))?;
                tokens.push(Token::Number(value));
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = index;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
                {
                    index += 1;
                }
                let name = text[start..index].to_ascii_lowercase();
                let mut after = index;
                while after < bytes.len() && (bytes[after] == b' ' || bytes[after] == b'\t') {
                    after += 1;
                }
                if (name == "v" || name == "i") && bytes.get(after) == Some(&b'(') {
                    let Some(close) = text[after..].find(')').map(|at| after + at) else {
                        return Err(format!("`{name}(` has no `)`"));
                    };
                    let letter = if name == "v" { 'V' } else { 'I' };
                    tokens.push(Token::Probe(letter, text[after + 1..close].to_string()));
                    index = close + 1;
                } else {
                    tokens.push(Token::Name(name));
                }
            }
            b'(' => {
                tokens.push(Token::Open);
                index += 1;
            }
            b')' => {
                tokens.push(Token::Close);
                index += 1;
            }
            b',' => {
                tokens.push(Token::Comma);
                index += 1;
            }
            _ => {
                let Some(op) = OPERATORS.iter().find(|op| text[index..].starts_with(**op)) else {
                    let rest: String = text[index..].chars().take(1).collect();
                    return Err(format!("`{rest}` is outside the expression subset"));
                };
                if *op == "=" {
                    return Err("`=` is not an operator; use `==` to compare".into());
                }
                tokens.push(Token::Op(op));
                index += op.len();
            }
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    controls: Vec<ControlName>,
    uses_time: bool,
}

impl Parser {
    fn peek_op(&self) -> Option<&'static str> {
        match self.tokens.get(self.at) {
            Some(Token::Op(op)) => Some(op),
            _ => None,
        }
    }

    fn eat(&mut self, op: &str) -> bool {
        if self.peek_op() == Some(op) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn ternary(&mut self) -> Result<Node, String> {
        let condition = self.or()?;
        if !self.eat("?") {
            return Ok(condition);
        }
        let then = self.ternary()?;
        if !self.eat(":") {
            return Err("`?` without its `:`".into());
        }
        let otherwise = self.ternary()?;
        Ok(Node::Ternary(
            Box::new(condition),
            Box::new(then),
            Box::new(otherwise),
        ))
    }

    fn left(
        &mut self,
        operators: &[(&str, Binary)],
        next: fn(&mut Self) -> Result<Node, String>,
    ) -> Result<Node, String> {
        let mut node = next(self)?;
        'outer: loop {
            for (text, op) in operators {
                if self.eat(text) {
                    let rhs = next(self)?;
                    node = Node::Binary(*op, Box::new(node), Box::new(rhs));
                    continue 'outer;
                }
            }
            return Ok(node);
        }
    }

    fn or(&mut self) -> Result<Node, String> {
        self.left(&[("||", Binary::Or)], Self::and)
    }

    fn and(&mut self) -> Result<Node, String> {
        self.left(&[("&&", Binary::And)], Self::comparison)
    }

    fn comparison(&mut self) -> Result<Node, String> {
        self.left(
            &[
                ("==", Binary::Eq),
                ("!=", Binary::Ne),
                ("<=", Binary::Le),
                (">=", Binary::Ge),
                ("<", Binary::Lt),
                (">", Binary::Gt),
            ],
            Self::sum,
        )
    }

    fn sum(&mut self) -> Result<Node, String> {
        self.left(&[("+", Binary::Add), ("-", Binary::Sub)], Self::product)
    }

    fn product(&mut self) -> Result<Node, String> {
        self.left(&[("*", Binary::Mul), ("/", Binary::Div)], Self::unary)
    }

    /// A sign applies to the whole power: `-2^2` is -4.
    fn unary(&mut self) -> Result<Node, String> {
        if self.eat("-") {
            return Ok(Node::Neg(Box::new(self.unary()?)));
        }
        if self.eat("+") {
            return self.unary();
        }
        if self.eat("!") {
            return Ok(Node::Not(Box::new(self.unary()?)));
        }
        self.power()
    }

    /// Left-associative, as in ngspice: `2^3^2` is `(2^3)^2`.
    fn power(&mut self) -> Result<Node, String> {
        let mut node = self.atom()?;
        while self.eat("^") || self.eat("**") {
            let exponent = self.signed_atom()?;
            node = Node::Binary(Binary::Pow, Box::new(node), Box::new(exponent));
        }
        Ok(node)
    }

    /// An exponent may carry its own sign: `10^-3`.
    fn signed_atom(&mut self) -> Result<Node, String> {
        if self.eat("-") {
            return Ok(Node::Neg(Box::new(self.signed_atom()?)));
        }
        if self.eat("+") {
            return self.signed_atom();
        }
        self.atom()
    }

    fn control(&mut self, name: ControlName) -> Node {
        let index = match self.controls.iter().position(|known| *known == name) {
            Some(index) => index,
            None => {
                self.controls.push(name);
                self.controls.len() - 1
            }
        };
        Node::Control(index)
    }

    /// The rest of `pwl(x, x1, y1, ...)` after its `(`.
    fn pwl(&mut self) -> Result<Node, String> {
        let input = self.ternary()?;
        let mut values = Vec::new();
        while self.tokens.get(self.at) == Some(&Token::Comma) {
            self.at += 1;
            let negative = self.eat("-");
            if !negative {
                self.eat("+");
            }
            match self.tokens.get(self.at) {
                Some(Token::Number(value)) => {
                    values.push(if negative { -value } else { *value });
                    self.at += 1;
                }
                _ => return Err("`pwl` points must be literal numbers".into()),
            }
        }
        match self.tokens.get(self.at) {
            Some(Token::Close) => self.at += 1,
            _ => return Err("expected `)` or a literal point in `pwl`".into()),
        }
        if values.len() % 2 != 0 {
            return Err("`pwl` takes x, y pairs after its input".into());
        }
        let points: Vec<(f64, f64)> = values.chunks(2).map(|pair| (pair[0], pair[1])).collect();
        check_points(&points)?;
        Ok(Node::Pwl {
            input: Box::new(input),
            points,
            smooth: false,
        })
    }

    fn atom(&mut self) -> Result<Node, String> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        match token {
            Some(Token::Number(value)) => Ok(Node::Number(value)),
            Some(Token::Open) => {
                let node = self.ternary()?;
                match self.tokens.get(self.at) {
                    Some(Token::Close) => {
                        self.at += 1;
                        Ok(node)
                    }
                    _ => Err("expected `)`".into()),
                }
            }
            Some(Token::Probe(letter, args)) => {
                let names: Vec<String> = args
                    .split(',')
                    .map(|name| name.trim().to_ascii_lowercase())
                    .collect();
                if names
                    .iter()
                    .any(|name| name.is_empty() || name.contains(' '))
                {
                    return Err(format!("`{letter}({args})` is not a node or element name"));
                }
                let name = match (letter, names.as_slice()) {
                    ('V', [p]) => ControlName::Voltage(p.clone(), None),
                    ('V', [p, n]) => ControlName::Voltage(p.clone(), Some(n.clone())),
                    ('I', [element]) => ControlName::Current(element.clone()),
                    _ => {
                        return Err(format!(
                            "`{letter}({args})` takes {} name(s)",
                            if letter == 'V' { "one or two" } else { "one" }
                        ))
                    }
                };
                Ok(self.control(name))
            }
            Some(Token::Name(name)) => {
                if self.tokens.get(self.at) == Some(&Token::Open) {
                    self.at += 1;
                    if name == "pwl" {
                        return self.pwl();
                    }
                    let Some((function, arity)) = Function::lookup(&name) else {
                        return Err(format!(
                            "function `{name}` is outside the behavioural subset"
                        ));
                    };
                    let mut args = vec![self.ternary()?];
                    while self.tokens.get(self.at) == Some(&Token::Comma) {
                        self.at += 1;
                        args.push(self.ternary()?);
                    }
                    match self.tokens.get(self.at) {
                        Some(Token::Close) => self.at += 1,
                        _ => return Err("expected `)`".into()),
                    }
                    if args.len() != arity {
                        return Err(format!(
                            "`{name}` takes {arity} argument(s), got {}",
                            args.len()
                        ));
                    }
                    return Ok(Node::Call(function, args));
                }
                match name.as_str() {
                    "time" => {
                        self.uses_time = true;
                        Ok(Node::Time)
                    }
                    "pi" => Ok(Node::Number(std::f64::consts::PI)),
                    "e" => Ok(Node::Number(std::f64::consts::E)),
                    _ => Err(format!(
                        "`{name}` is not a behavioural quantity; use V(node), I(element), \
                         time, or a `{{...}}` parameter"
                    )),
                }
            }
            Some(other) => Err(format!("unexpected {} in expression", describe(&other))),
            None => Err("expression ends early".into()),
        }
    }
}

/// A rational function of `s`: numerator and denominator coefficients,
/// lowest power first, each with a non-zero last entry (or a single zero).
#[derive(Debug, Clone, PartialEq)]
pub struct Transfer {
    /// `b0 + b1·s + ...`.
    pub numerator: Vec<f64>,
    /// `a0 + a1·s + ...`.
    pub denominator: Vec<f64>,
}

impl Transfer {
    /// Parse the `H(s)` of a `LAPLACE` source: numbers, `s`, `pi`, `+ - * /`,
    /// parentheses and `^`/`**` to an integer power, with the precedence of
    /// [`Expression::parse`]. Anything else (`exp(-s*T)`, `sqrt(s)`, a node
    /// voltage) is not a rational function and is refused.
    pub fn parse(text: &str) -> Result<Self, String> {
        let tokens = lex(text)?;
        let mut parser = TransferParser { tokens, at: 0 };
        let transfer = parser.sum()?;
        if let Some(token) = parser.tokens.get(parser.at) {
            return Err(format!(
                "unexpected {} in a LAPLACE transfer",
                describe(token)
            ));
        }
        let numerator = trim(transfer.numerator);
        let denominator = trim(transfer.denominator);
        if denominator == [0.0] {
            return Err("the LAPLACE transfer divides by zero".into());
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    fn constant(value: f64) -> Self {
        Self {
            numerator: vec![value],
            denominator: vec![1.0],
        }
    }

    fn add(self, other: Self, sign: f64) -> Self {
        let other_numerator: Vec<f64> = other.numerator.iter().map(|c| sign * c).collect();
        // A shared denominator (`1/(1+s) + 2/(1+s)`) stays as it is, so a
        // sum adds no pole-zero pair that would only cancel.
        if self.denominator == other.denominator {
            return Self {
                numerator: poly_add(&self.numerator, &other_numerator),
                denominator: self.denominator,
            };
        }
        Self {
            numerator: poly_add(
                &poly_mul(&self.numerator, &other.denominator),
                &poly_mul(&other_numerator, &self.denominator),
            ),
            denominator: poly_mul(&self.denominator, &other.denominator),
        }
    }

    fn mul(self, other: &Self) -> Self {
        Self {
            numerator: poly_mul(&self.numerator, &other.numerator),
            denominator: poly_mul(&self.denominator, &other.denominator),
        }
    }

    fn reciprocal(self) -> Result<Self, String> {
        if trim(self.numerator.clone()) == [0.0] {
            return Err("the LAPLACE transfer divides by zero".into());
        }
        Ok(Self {
            numerator: self.denominator,
            denominator: self.numerator,
        })
    }
}

fn poly_add(a: &[f64], b: &[f64]) -> Vec<f64> {
    (0..a.len().max(b.len()))
        .map(|k| a.get(k).unwrap_or(&0.0) + b.get(k).unwrap_or(&0.0))
        .collect()
}

fn poly_mul(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            out[i + j] += x * y;
        }
    }
    out
}

/// Drop zero leading coefficients, keeping at least one entry.
fn trim(mut poly: Vec<f64>) -> Vec<f64> {
    while poly.len() > 1 && poly.last() == Some(&0.0) {
        poly.pop();
    }
    poly
}

/// Recursive descent over the [`lex`] tokens for [`Transfer::parse`].
struct TransferParser {
    tokens: Vec<Token>,
    at: usize,
}

impl TransferParser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Token::Op(found)) if *found == op)
    }

    fn sum(&mut self) -> Result<Transfer, String> {
        let mut left = self.product()?;
        loop {
            let sign = if self.is_op("+") {
                1.0
            } else if self.is_op("-") {
                -1.0
            } else {
                return Ok(left);
            };
            self.at += 1;
            let right = self.product()?;
            left = left.add(right, sign);
        }
    }

    fn product(&mut self) -> Result<Transfer, String> {
        let mut left = self.unary()?;
        loop {
            if self.is_op("*") {
                self.at += 1;
                left = left.mul(&self.unary()?);
            } else if self.is_op("/") {
                self.at += 1;
                left = left.mul(&self.unary()?.reciprocal()?);
            } else {
                return Ok(left);
            }
        }
    }

    fn unary(&mut self) -> Result<Transfer, String> {
        if self.is_op("-") {
            self.at += 1;
            return Ok(Transfer::constant(-1.0).mul(&self.unary()?));
        }
        if self.is_op("+") {
            self.at += 1;
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<Transfer, String> {
        let mut base = self.atom()?;
        while self.is_op("^") || self.is_op("**") {
            self.at += 1;
            let negative = self.is_op("-");
            if negative {
                self.at += 1;
            }
            let exponent = match self.tokens.get(self.at) {
                Some(Token::Number(value)) if value.fract() == 0.0 && *value <= 64.0 => {
                    *value as u32
                }
                _ => return Err("a LAPLACE transfer raises only to a literal integer power".into()),
            };
            self.at += 1;
            let mut raised = Transfer::constant(1.0);
            for _ in 0..exponent {
                raised = raised.mul(&base);
            }
            base = if negative {
                raised.reciprocal()?
            } else {
                raised
            };
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Transfer, String> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        match token {
            Some(Token::Number(value)) => Ok(Transfer::constant(value)),
            Some(Token::Open) => {
                let inner = self.sum()?;
                match self.tokens.get(self.at) {
                    Some(Token::Close) => {
                        self.at += 1;
                        Ok(inner)
                    }
                    _ => Err("`(` without `)` in a LAPLACE transfer".into()),
                }
            }
            Some(Token::Name(name)) if name == "s" => Ok(Transfer {
                numerator: vec![0.0, 1.0],
                denominator: vec![1.0],
            }),
            Some(Token::Name(name)) if name == "pi" => Ok(Transfer::constant(std::f64::consts::PI)),
            Some(Token::Name(name)) => Err(format!(
                "`{name}` is not rational in `s`; a LAPLACE transfer holds numbers, `s` \
                 and `{{...}}` parameters only"
            )),
            Some(other) => Err(format!(
                "unexpected {} in a LAPLACE transfer",
                describe(&other)
            )),
            None => Err("the LAPLACE transfer ends early".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str, controls: &[f64]) -> f64 {
        Expression::parse(text)
            .unwrap_or_else(|e| panic!("{text}: {e}"))
            .value(controls, 0.0)
            .unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    /// Every value below is what ngspice-42 printed for the same `B` source.
    #[test]
    fn values_match_ngspice() {
        let cases: &[(&str, f64)] = &[
            ("-2^2", -4.0),
            ("2^3^2", 64.0),
            ("2**3**2", 64.0),
            ("2*-3", -6.0),
            ("2*3^2", 18.0),
            ("1 + 2 > 2 && 1", 1.0),
            ("1 ? 2 : 0 ? 3 : 4", 2.0),
            ("!0 + 10*!1", 1.0),
            ("u(0)+2*u(1e-9)", 2.5),
            ("u2(-1)+u2(2)", 1.0),
            ("floor(-2.5)+ceil(2.2)+nint(2.5)", 2.0),
            ("nint(3.5)+nint(-2.5)", 2.0),
            ("log(100)", 100f64.ln()),
            ("log10(1000)", 3.0),
            ("pow(-2,0.5)", 2f64.sqrt()),
            ("pwr(-2,0.5)", -(2f64.sqrt())),
            ("(-2)^3", 8.0),
            ("1 < 2 < 3", 1.0),
            ("3 != 4", 1.0),
        ];
        for (text, expected) in cases {
            let got = value(text, &[]);
            assert!(
                (got - expected).abs() <= 1e-12 * expected.abs().max(1.0),
                "{text}: ngspice {expected}, got {got}"
            );
        }
        // V(m) = -4 V and -2 V.
        assert_eq!(value("V(m)^2", &[-4.0]), 16.0);
        assert_eq!(value("V(m)^3", &[-4.0]), 64.0);
        assert_eq!(value("pwr(V(m),3)", &[-4.0]), -64.0);
        assert_eq!(value("pwr(V(m),2)", &[-2.0]), -4.0);
        assert_eq!(value("uramp(V(m))+sgn(V(m))", &[-4.0]), -1.0);
        assert_eq!(value("min(V(m),2)+max(V(m),2)", &[-4.0]), -2.0);
        assert_eq!(value("v(M, 0)", &[-2.0]), -2.0);
    }

    /// Each derivative against a central difference of the value.
    #[test]
    fn derivatives_match_finite_differences() {
        let cases = [
            "V(a)^2 * I(v1)",
            "pwr(V(a), 1.5) + pow(V(a), V(b))",
            "exp(V(a)/0.1) / (1 + V(b)^2)",
            "tanh(V(a,b)) + atan(V(b)) - sqrt(V(a))",
            "V(a) > V(b) ? ln(V(a)) : log10(V(b))",
            "min(V(a), V(b)) + max(V(a), 2*V(b)) + uramp(V(a) - 1)",
            "pwl(V(a)*V(b), -1,2, 0.5,-1e-3, 3,4)",
        ];
        let point = [1.3, 0.7, 0.4];
        for text in cases {
            let expression = Expression::parse(text).unwrap();
            let x = &point[..expression.controls().len()];
            let (_, grad) = expression.evaluate(x, 0.0).unwrap();
            for k in 0..x.len() {
                let h = 1e-6;
                let mut up = x.to_vec();
                let mut down = x.to_vec();
                up[k] += h;
                down[k] -= h;
                let numeric = (expression.value(&up, 0.0).unwrap()
                    - expression.value(&down, 0.0).unwrap())
                    / (2.0 * h);
                assert!(
                    (grad[k] - numeric).abs() <= 1e-6 * numeric.abs().max(1.0),
                    "{text} d/d{k}: exact {}, numeric {numeric}",
                    grad[k]
                );
            }
        }
    }

    #[test]
    fn table_is_smooth_and_flat_outside() {
        let points = vec![(0.0, 0.0), (1.0, 1.0), (4.0, 2.0)];
        let table = Expression::table("V(a)", points).unwrap();
        let at = |x: f64| table.evaluate(&[x], 0.0).unwrap();
        // Held at the end values, with no slope, past both ends.
        assert_eq!(at(-3.0), (0.0, vec![0.0]));
        assert_eq!(at(9.0), (2.0, vec![0.0]));
        // Value and slope are continuous at each edge of every corner.
        for (edge, slope_below, slope_above) in [
            (-0.1, 0.0, 0.0),
            (0.1, 1.0, 1.0),
            (0.9, 1.0, 1.0),
            (1.1, 1.0 / 3.0, 1.0 / 3.0),
            (3.7, 1.0 / 3.0, 1.0 / 3.0),
            (4.3, 0.0, 0.0),
        ] {
            let (below, d_below) = at(edge - 1e-12);
            let (above, d_above) = at(edge + 1e-12);
            assert!((below - above).abs() < 1e-9, "value jumps at {edge}");
            assert!(
                (d_below[0] - slope_below).abs() < 1e-6,
                "slope below {edge}"
            );
            assert!(
                (d_above[0] - slope_above).abs() < 1e-6,
                "slope above {edge}"
            );
        }
        // The corner at 1 reads y + d·(s_upper − s_lower)/4.
        let (corner, _) = at(1.0);
        assert!((corner - (1.0 + 0.1 * (1.0 / 3.0 - 1.0) / 4.0)).abs() < 1e-12);
    }

    #[test]
    fn controls_are_collected_once_in_order() {
        let expression = Expression::parse("V(a)*V(A) + I(Vx) - V(b, a) + time").unwrap();
        assert_eq!(
            expression.controls(),
            &[
                ControlName::Voltage("a".into(), None),
                ControlName::Current("vx".into()),
                ControlName::Voltage("b".into(), Some("a".into())),
            ]
        );
        assert!(expression.uses_time());
    }

    #[test]
    fn refusals_name_the_problem() {
        for (text, needle) in [
            ("temper", "not a behavioural quantity"),
            ("5 % 3", "outside the expression subset"),
            ("foo(1)", "function `foo`"),
            ("min(1)", "takes 2 argument"),
            ("V(a,b,c)", "one or two"),
            ("1 ? 2", "without its `:`"),
            ("(1 + 2", "expected `)`"),
            ("V(a) = 1", "`==`"),
        ] {
            let error = Expression::parse(text).unwrap_err();
            assert!(error.contains(needle), "{text}: {error}");
        }
        let error = Expression::parse("sqrt(V(a))")
            .unwrap()
            .value(&[-1.0], 0.0)
            .unwrap_err();
        assert!(error.contains("out of range for `sqrt`"), "{error}");
        let error = Expression::parse("1/V(a)")
            .unwrap()
            .value(&[0.0], 0.0)
            .unwrap_err();
        assert!(error.contains("not finite"), "{error}");
    }
}
