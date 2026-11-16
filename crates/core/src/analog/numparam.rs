// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ngspice's numparam: `.param` values and `{expression}` substitution.
//!
//! Expressions are evaluated once, at parse time, into numbers, so the solver
//! never sees a parameter. The grammar is the part of ngspice's numparam that
//! vendor model text uses: numbers with SPICE scale suffixes, parameter names
//! (case-insensitive), `+ - * /`, `^` and `**` (left-associative, binding
//! tighter than a unary sign, as in ngspice: `-2^2` is -4 and `2^3^2` is 64),
//! parentheses, and the functions `sqrt exp ln log log10 abs min max pow
//! floor ceil sin cos tan atan`. `log` is the natural logarithm, as in
//! ngspice. Anything else (comparisons, the ternary operator, node voltages)
//! is an error, never a guess.

use std::collections::BTreeMap;

/// Parameter values in scope, keyed by lower-cased name.
pub(super) type Scope = BTreeMap<String, f64>;

/// Why an expression did not evaluate.
#[derive(Debug, PartialEq)]
pub(super) enum EvalError {
    /// The expression names a parameter that is not in scope.
    Unknown(String),
    /// Any other failure, as a message.
    Invalid(String),
}

impl EvalError {
    pub(super) fn message(&self) -> String {
        match self {
            Self::Unknown(name) => format!("parameter `{name}` is not defined"),
            Self::Invalid(message) => message.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Op(char),
    Power,
    Open,
    Close,
    Comma,
}

fn lex(text: &str) -> Result<Vec<Token>, EvalError> {
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
                    .ok_or_else(|| EvalError::Invalid(format!("`{raw}` is not a SPICE value")))?;
                tokens.push(Token::Number(value));
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = index;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
                {
                    index += 1;
                }
                tokens.push(Token::Name(text[start..index].to_ascii_lowercase()));
            }
            b'*' if bytes.get(index + 1) == Some(&b'*') => {
                tokens.push(Token::Power);
                index += 2;
            }
            b'^' => {
                tokens.push(Token::Power);
                index += 1;
            }
            b'+' | b'-' | b'*' | b'/' => {
                tokens.push(Token::Op(byte as char));
                index += 1;
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
                let rest: String = text[index..].chars().take(1).collect();
                return Err(EvalError::Invalid(format!(
                    "`{rest}` is outside the expression subset"
                )));
            }
        }
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: Vec<Token>,
    at: usize,
    scope: &'a Scope,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    fn sum(&mut self) -> Result<f64, EvalError> {
        let mut value = self.product()?;
        while let Some(Token::Op(op @ ('+' | '-'))) = self.peek().cloned() {
            self.at += 1;
            let rhs = self.product()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Ok(value)
    }

    fn product(&mut self) -> Result<f64, EvalError> {
        let mut value = self.unary()?;
        while let Some(Token::Op(op @ ('*' | '/'))) = self.peek().cloned() {
            self.at += 1;
            let rhs = self.unary()?;
            value = if op == '*' { value * rhs } else { value / rhs };
        }
        Ok(value)
    }

    /// A sign applies to the whole power: `-2^2` is -4.
    fn unary(&mut self) -> Result<f64, EvalError> {
        match self.peek() {
            Some(Token::Op('-')) => {
                self.at += 1;
                Ok(-self.unary()?)
            }
            Some(Token::Op('+')) => {
                self.at += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    /// Left-associative, as in ngspice: `2^3^2` is `(2^3)^2`.
    fn power(&mut self) -> Result<f64, EvalError> {
        let mut value = self.atom()?;
        while self.peek() == Some(&Token::Power) {
            self.at += 1;
            let exponent = self.signed_atom()?;
            value = value.powf(exponent);
        }
        Ok(value)
    }

    /// An exponent may carry its own sign: `10^-3`.
    fn signed_atom(&mut self) -> Result<f64, EvalError> {
        match self.peek() {
            Some(Token::Op('-')) => {
                self.at += 1;
                Ok(-self.signed_atom()?)
            }
            Some(Token::Op('+')) => {
                self.at += 1;
                self.signed_atom()
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Result<f64, EvalError> {
        match self.next() {
            Some(Token::Number(value)) => Ok(value),
            Some(Token::Open) => {
                let value = self.sum()?;
                self.expect_close()?;
                Ok(value)
            }
            Some(Token::Name(name)) => {
                if self.peek() == Some(&Token::Open) {
                    self.at += 1;
                    let mut args = vec![self.sum()?];
                    while self.peek() == Some(&Token::Comma) {
                        self.at += 1;
                        args.push(self.sum()?);
                    }
                    self.expect_close()?;
                    call(&name, &args)
                } else {
                    self.scope
                        .get(&name)
                        .copied()
                        .ok_or(EvalError::Unknown(name))
                }
            }
            Some(other) => Err(EvalError::Invalid(format!(
                "unexpected {} in expression",
                describe(&other)
            ))),
            None => Err(EvalError::Invalid("expression ends early".into())),
        }
    }

    fn expect_close(&mut self) -> Result<(), EvalError> {
        match self.next() {
            Some(Token::Close) => Ok(()),
            _ => Err(EvalError::Invalid("expected `)`".into())),
        }
    }
}

fn describe(token: &Token) -> String {
    match token {
        Token::Number(value) => format!("number `{value}`"),
        Token::Name(name) => format!("name `{name}`"),
        Token::Op(op) => format!("`{op}`"),
        Token::Power => "`^`".into(),
        Token::Open => "`(`".into(),
        Token::Close => "`)`".into(),
        Token::Comma => "`,`".into(),
    }
}

fn call(name: &str, args: &[f64]) -> Result<f64, EvalError> {
    let unary = |f: fn(f64) -> f64| -> Result<f64, EvalError> {
        match args {
            [x] => Ok(f(*x)),
            _ => Err(EvalError::Invalid(format!(
                "`{name}` takes 1 argument, got {}",
                args.len()
            ))),
        }
    };
    let binary = |f: fn(f64, f64) -> f64| -> Result<f64, EvalError> {
        match args {
            [x, y] => Ok(f(*x, *y)),
            _ => Err(EvalError::Invalid(format!(
                "`{name}` takes 2 arguments, got {}",
                args.len()
            ))),
        }
    };
    match name {
        "sqrt" => unary(f64::sqrt),
        "exp" => unary(f64::exp),
        "ln" | "log" => unary(f64::ln),
        "log10" => unary(f64::log10),
        "abs" => unary(f64::abs),
        "floor" => unary(f64::floor),
        "ceil" => unary(f64::ceil),
        "sin" => unary(f64::sin),
        "cos" => unary(f64::cos),
        "tan" => unary(f64::tan),
        "atan" => unary(f64::atan),
        "min" => binary(f64::min),
        "max" => binary(f64::max),
        "pow" => binary(f64::powf),
        _ => Err(EvalError::Invalid(format!(
            "function `{name}` is outside the expression subset"
        ))),
    }
}

/// Evaluate one expression (without its braces) against `scope`.
pub(super) fn evaluate(text: &str, scope: &Scope) -> Result<f64, EvalError> {
    let mut parser = Parser {
        tokens: lex(text)?,
        at: 0,
        scope,
    };
    let value = parser.sum()?;
    if let Some(token) = parser.peek() {
        return Err(EvalError::Invalid(format!(
            "unexpected {} in expression",
            describe(token)
        )));
    }
    if !value.is_finite() {
        return Err(EvalError::Invalid(format!(
            "`{}` is not finite",
            text.trim()
        )));
    }
    Ok(value)
}

/// Evaluate a parameter value as written: `{expr}`, or a bare expression.
pub(super) fn evaluate_value(text: &str, scope: &Scope) -> Result<f64, EvalError> {
    let text = text.trim();
    let inner = text
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or(text);
    evaluate(inner, scope)
}

/// Print a value so `parse_spice_value` reads back the same double.
pub(super) fn format_value(value: f64) -> String {
    format!("{value:e}")
}

/// Replace every `{expr}` in `text` with its value.
///
/// A brace group right after `value=`, `table` or `laplace=` is left alone:
/// that is a behavioural source, which the element parser refuses by name.
pub(super) fn substitute(text: &str, scope: &Scope) -> Result<String, EvalError> {
    if !text.contains(['{', '}']) {
        return Ok(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find(['{', '}']) {
        if rest[open..].starts_with('}') {
            return Err(EvalError::Invalid("`}` without `{`".into()));
        }
        let Some(close) = rest[open..].find('}').map(|at| open + at) else {
            return Err(EvalError::Invalid("`{` without `}`".into()));
        };
        let inner = &rest[open + 1..close];
        if inner.contains('{') {
            return Err(EvalError::Invalid("nested `{` in an expression".into()));
        }
        out.push_str(&rest[..open]);
        if is_behavioural(&out) {
            out.push('{');
            out.push_str(&substitute_names(inner, scope));
            out.push('}');
        } else {
            out.push_str(&format_value(evaluate(inner, scope)?));
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// A `.func` definition: argument names (lower-cased) and its body text.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Function {
    pub(super) args: Vec<String>,
    pub(super) body: String,
}

/// `.func` definitions in scope, keyed by lower-cased name.
pub(super) type Functions = BTreeMap<String, Function>;

/// Names a `.func` may not take: the built-in functions, and the source
/// keywords a line writes as `NAME(`.
const RESERVED: &[&str] = &[
    "sqrt", "exp", "ln", "log", "log10", "abs", "floor", "ceil", "sin", "cos", "tan", "atan",
    "min", "max", "pow", "pulse", "pwl", "sffm", "am", "dc", "ac",
];

fn is_name(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Read the text after `.func`: `name(a, b) {body}`, `name(a,b) = {body}`
/// or `name(a) body`, as ngspice accepts.
pub(super) fn parse_function(text: &str) -> Result<(String, Function), String> {
    let usage = "expected `.func <name>(<arg>, ...) {<expression>}`";
    let (name, rest) = text.split_once('(').ok_or(usage)?;
    let name = name.trim().to_ascii_lowercase();
    if !is_name(&name) {
        return Err(usage.into());
    }
    if RESERVED.contains(&name.as_str()) {
        return Err(format!("`.func {name}` would shadow a built-in name"));
    }
    let (args, body) = rest.split_once(')').ok_or(usage)?;
    let args: Vec<String> = if args.trim().is_empty() {
        Vec::new()
    } else {
        args.split(',')
            .map(|arg| arg.trim().to_ascii_lowercase())
            .collect()
    };
    for (index, arg) in args.iter().enumerate() {
        if !is_name(arg) {
            return Err(format!("`.func {name}`: argument `{arg}` is not a name"));
        }
        if args[..index].contains(arg) {
            return Err(format!("`.func {name}`: argument `{arg}` is listed twice"));
        }
    }
    let body = body.trim();
    let body = body.strip_prefix('=').unwrap_or(body).trim();
    let body = body
        .strip_prefix('{')
        .and_then(|inner| inner.strip_suffix('}'))
        .unwrap_or(body)
        .trim();
    if body.is_empty() || body.contains(['{', '}']) {
        return Err(usage.into());
    }
    Ok((
        name,
        Function {
            args,
            body: body.to_string(),
        },
    ))
}

/// Replace every call of a `.func` in the expression `text` by its body,
/// arguments bound by name, as ngspice's front end does: `f(a+1)` with
/// `.func f(x) {x*x}` becomes `((a+1)*(a+1))`. The parentheses keep the
/// written precedence; names the body leaves free (a `.param`) are resolved
/// later, in the scope of the line that made the call.
pub(super) fn expand_calls(text: &str, functions: &Functions) -> Result<String, EvalError> {
    expand_within(text, functions, &mut Vec::new())
}

/// `stack` holds the functions being expanded, for the cycle check.
fn expand_within(
    text: &str,
    functions: &Functions,
    stack: &mut Vec<String>,
) -> Result<String, EvalError> {
    if functions.is_empty() {
        return Ok(text.to_string());
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        // A digit run carries its suffix (`2meg`), which is not a name.
        if byte.is_ascii_digit() || byte == b'.' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'.')
            {
                index += 1;
            }
            out.push_str(&text[start..index]);
            continue;
        }
        if !(byte.is_ascii_alphabetic() || byte == b'_') {
            out.push(byte as char);
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
        {
            index += 1;
        }
        let name = &text[start..index];
        let mut open = index;
        while open < bytes.len() && (bytes[open] == b' ' || bytes[open] == b'\t') {
            open += 1;
        }
        let function = functions.get(&name.to_ascii_lowercase());
        let Some(function) = function.filter(|_| bytes.get(open) == Some(&b'(')) else {
            out.push_str(name);
            continue;
        };
        // Split the arguments on top-level commas.
        let mut args = Vec::new();
        let mut level = 0usize;
        let mut from = open + 1;
        let mut close = None;
        for (at, &b) in bytes.iter().enumerate().skip(open + 1) {
            match b {
                b'(' => level += 1,
                b')' if level == 0 => {
                    args.push(&text[from..at]);
                    close = Some(at);
                    break;
                }
                b')' => level -= 1,
                b',' if level == 0 => {
                    args.push(&text[from..at]);
                    from = at + 1;
                }
                _ => {}
            }
        }
        let close = close.ok_or_else(|| EvalError::Invalid("expected `)`".into()))?;
        if args.len() == 1 && args[0].trim().is_empty() {
            args.clear();
        }
        let lower = name.to_ascii_lowercase();
        if stack.contains(&lower) {
            return Err(EvalError::Invalid(format!("`.func {lower}` calls itself")));
        }
        if args.len() != function.args.len() {
            return Err(EvalError::Invalid(format!(
                "`{lower}` takes {} argument{}, got {}",
                function.args.len(),
                if function.args.len() == 1 { "" } else { "s" },
                args.len()
            )));
        }
        let mut bound = Vec::with_capacity(args.len());
        for arg in &args {
            bound.push(format!(
                "({})",
                expand_within(arg.trim(), functions, stack)?
            ));
        }
        let body = bind(&function.body, &function.args, &bound);
        out.push('(');
        stack.push(lower);
        let expanded = expand_within(&body, functions, stack)?;
        stack.pop();
        // No spaces, so a bare `name=f(x)` value stays one token.
        out.extend(expanded.chars().filter(|c| !c.is_whitespace()));
        out.push(')');
        index = close + 1;
    }
    Ok(out)
}

/// Replace each whole-word, case-insensitive use of `names[i]` in `body`
/// with `values[i]`.
fn bind(body: &str, names: &[String], values: &[String]) -> String {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_digit() || byte == b'.' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'.')
            {
                index += 1;
            }
            out.push_str(&body[start..index]);
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let word = &body[start..index];
            match names.iter().position(|n| n.eq_ignore_ascii_case(word)) {
                Some(at) => out.push_str(&values[at]),
                None => out.push_str(word),
            }
        } else {
            out.push(byte as char);
            index += 1;
        }
    }
    out
}

/// Expand `.func` calls inside every `{...}` group of a netlist line, and in
/// the whole expression after `v=`/`i=` on a `B` line, braced or not. A
/// behavioural group keeps its braces and any `V(...)`/`I(...)` in it.
pub(super) fn expand_braced_calls(text: &str, functions: &Functions) -> Result<String, EvalError> {
    if functions.is_empty() {
        return Ok(text.to_string());
    }
    if text.trim_start().starts_with(['b', 'B']) {
        if let Some(at) = text.find('=') {
            let (head, expression) = text.split_at(at + 1);
            return Ok(format!("{head}{}", expand_calls(expression, functions)?));
        }
    }
    if !text.contains('{') {
        return Ok(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|at| open + at) else {
            // `substitute` reports the unbalanced brace with its own words.
            out.push_str(rest);
            return Ok(out);
        };
        out.push_str(&rest[..open]);
        out.push('{');
        out.push_str(&expand_calls(&rest[open + 1..close], functions)?);
        out.push('}');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// True when the text before a `{` ends in `value=`, `table` or `laplace=`,
/// or in `v=`/`i=` on a `B` line: the group is a behavioural expression.
fn is_behavioural(before: &str) -> bool {
    let before = before.trim_end().trim_end_matches('=').trim_end();
    let keyword = before
        .rsplit(|c: char| c.is_whitespace())
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let b_line = before.trim_start().starts_with(['b', 'B']);
    matches!(keyword.as_str(), "value" | "table" | "laplace")
        || (b_line && matches!(keyword.as_str(), "v" | "i"))
}

/// Replace each parameter name in a behavioural expression with its value,
/// leaving everything the solver reads at run time alone: function names,
/// `V(...)`/`I(...)` with their node and element names, `time`, and a scale
/// suffix such as the `k` of `2k`.
pub(super) fn substitute_names(text: &str, scope: &Scope) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    // Text before `index` not yet copied starts here; copying whole slices
    // keeps any non-ASCII character intact.
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let starts_name = (byte.is_ascii_alphabetic() || byte == b'_')
            && !(index > 0
                && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'.'));
        if !starts_name {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
        {
            index += 1;
        }
        let name = &text[start..index];
        let mut after = index;
        while after < bytes.len() && (bytes[after] == b' ' || bytes[after] == b'\t') {
            after += 1;
        }
        let call = bytes.get(after) == Some(&b'(');
        let lower = name.to_ascii_lowercase();
        if call && (lower == "v" || lower == "i") {
            // Node and element names inside the probe are not parameters.
            index = text[after..]
                .find(')')
                .map_or(text.len(), |at| after + at + 1);
        } else if let (false, Some(value)) = (call, scope.get(&lower)) {
            out.push_str(&text[copied..start]);
            out.push_str(&format_value(*value));
            copied = index;
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Split `a=1 b = {x * 2} c=3k` into `(name, value)` pairs. Spaces around
/// `=` and inside braces are allowed, as in ngspice.
pub(super) fn assignments(text: &str) -> Result<Vec<(String, String)>, String> {
    // Fold spaces around `=` outside braces, then split on whitespace
    // outside braces.
    let mut items: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '{' => {
                depth += 1;
                current.push(ch);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    items.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        items.push(current);
    }
    let mut joined: Vec<String> = Vec::new();
    let mut index = 0;
    while index < items.len() {
        let mut item = items[index].clone();
        index += 1;
        while (item.ends_with('=') || items.get(index).is_some_and(|next| next.starts_with('=')))
            && index < items.len()
        {
            item.push_str(&items[index]);
            index += 1;
        }
        joined.push(item);
    }
    joined
        .into_iter()
        .map(|item| match item.split_once('=') {
            Some((name, value))
                if !name.is_empty()
                    && !value.is_empty()
                    && name
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                Ok((name.to_ascii_lowercase(), value.to_string()))
            }
            _ => Err(format!("expected `<name>=<value>`, got `{item}`")),
        })
        .collect()
}

/// A parameter value not yet evaluated: an expression, or a number already
/// fixed by the caller.
pub(super) enum Pending {
    Expression(String),
    Fixed(f64),
}

/// Evaluate `pending` on top of `base`, in dependency order rather than the
/// order written: ngspice sorts `.param` lines the same way, so
/// `.param b={a*2}` may precede `.param a=3`. A name defined nowhere or in a
/// cycle is an error. So is a name defined twice in one scope: ngspice keeps
/// the last value, which makes the meaning of anything evaluated in between
/// depend on line order.
pub(super) fn resolve(base: &Scope, pending: Vec<(String, Pending)>) -> Result<Scope, String> {
    let mut scope = base.clone();
    let mut left: Vec<(String, String)> = Vec::new();
    for (index, (name, _)) in pending.iter().enumerate() {
        if pending[..index].iter().any(|(other, _)| other == name) {
            return Err(format!("parameter `{name}` is defined twice"));
        }
    }
    for (name, value) in pending {
        match value {
            Pending::Fixed(value) => {
                scope.insert(name, value);
            }
            Pending::Expression(text) => left.push((name, text)),
        }
    }
    let pending_names: Vec<String> = left.iter().map(|(name, _)| name.clone()).collect();
    // Shadowed names must not resolve to the outer value before the inner
    // one is known.
    for name in &pending_names {
        scope.remove(name);
    }
    while !left.is_empty() {
        let mut progressed = false;
        let mut waiting = Vec::new();
        let mut blocker = None;
        for (name, text) in left {
            match evaluate_value(&text, &scope) {
                Ok(value) => {
                    scope.insert(name, value);
                    progressed = true;
                }
                Err(EvalError::Unknown(missing)) if pending_names.contains(&missing) => {
                    blocker.get_or_insert_with(|| (name.clone(), missing.clone()));
                    waiting.push((name, text));
                }
                Err(error) => return Err(format!("`{name}`: {}", error.message())),
            }
        }
        if !progressed {
            let (name, missing) = blocker.expect("waiting on a pending name");
            return Err(format!(
                "parameter `{name}` depends on `{missing}`, which depends back on it"
            ));
        }
        left = waiting;
    }
    Ok(scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(text: &str) -> f64 {
        evaluate(text, &Scope::new()).unwrap()
    }

    #[test]
    fn precedence_matches_ngspice() {
        // ngspice-45: `{-2^2}` = -4, `{2^3^2}` = 64, `{log(100)}` = ln 100.
        assert_eq!(eval("-2^2"), -4.0);
        assert_eq!(eval("2^3^2"), 64.0);
        assert_eq!(eval("2**3"), 8.0);
        assert_eq!(eval("log(100)"), 100f64.ln());
        assert_eq!(eval("10^-3"), 1e-3);
        assert_eq!(eval("1 + 2 * 3 - 4 / 2"), 5.0);
        assert_eq!(eval("2*1k"), 2000.0);
        assert_eq!(eval("1meg/2.2u"), 1e6 / 2.2e-6);
        assert_eq!(eval("max(1, min(3, 2))"), 2.0);
    }

    #[test]
    fn formatted_values_read_back_bit_identical() {
        for value in [1e3, 2.2e-6, -0.1, 1.0 / 3.0, 4.605_170_185_988_09] {
            let read = crate::analog::netlist::parse_spice_value(&format_value(value)).unwrap();
            assert_eq!(read.to_bits(), value.to_bits());
        }
    }

    #[test]
    fn substitution_keeps_behavioural_expressions_for_the_solver() {
        let scope = Scope::from([("g".to_string(), 2.0), ("b".to_string(), 7.0)]);
        assert_eq!(
            substitute("E1 a 0 b 0 {g*5}", &scope).unwrap(),
            "E1 a 0 b 0 1e1"
        );
        // Parameter names become numbers; node names inside `v(...)`, the
        // `g` suffix of `1g` and function names stay as written.
        assert_eq!(
            substitute("E1 a 0 VALUE={v(b)*g + 1g*exp(b)}", &scope).unwrap(),
            "E1 a 0 VALUE={v(b)*2e0 + 1g*exp(7e0)}"
        );
        assert_eq!(
            substitute("B1 a 0 V={i(vb) * g + time}", &scope).unwrap(),
            "B1 a 0 V={i(vb) * 2e0 + time}"
        );
        // `v=` outside a `B` line is an ordinary value.
        assert_eq!(
            substitute("X1 a b sub v={g}", &scope).unwrap(),
            "X1 a b sub v=2e0"
        );
        assert_eq!(
            substitute("H1 a 0 TABLE {x}", &scope).unwrap(),
            "H1 a 0 TABLE {x}"
        );
    }

    #[test]
    fn resolve_orders_by_dependency_and_refuses_cycles() {
        let pending = |items: &[(&str, &str)]| {
            items
                .iter()
                .map(|(n, v)| (n.to_string(), Pending::Expression(v.to_string())))
                .collect::<Vec<_>>()
        };
        let scope = resolve(&Scope::new(), pending(&[("b", "{a*2}"), ("a", "3")])).unwrap();
        assert_eq!(scope["b"], 6.0);
        let err = resolve(&Scope::new(), pending(&[("a", "{b}"), ("b", "{a}")])).unwrap_err();
        assert!(err.contains("depends back"), "{err}");
        let err = resolve(&Scope::new(), pending(&[("a", "{c}")])).unwrap_err();
        assert!(err.contains("`c` is not defined"), "{err}");
    }

    fn functions(defs: &[&str]) -> Functions {
        defs.iter().map(|d| parse_function(d).unwrap()).collect()
    }

    #[test]
    fn function_calls_keep_the_written_precedence() {
        let table = functions(&["sq(x) {x*x}", "neg(x) = -x", "two() {2}"]);
        let eval = |text: &str| evaluate(&expand_calls(text, &table).unwrap(), &Scope::new());
        // Unparenthesised binding would give 1+1*1+1 = 3 and -2^2 = -4.
        assert_eq!(eval("sq(1+1)").unwrap(), 4.0);
        assert_eq!(eval("SQ (neg(2))").unwrap(), 4.0);
        assert_eq!(eval("2^sq(two())").unwrap(), 16.0);
        // A name that is not called, and a suffix, are left alone.
        assert_eq!(expand_calls("sq + 2sq", &table).unwrap(), "sq + 2sq");
    }

    #[test]
    fn function_definitions_parse_like_ngspice() {
        let (name, f) = parse_function("Par(A, b) = { a*b/(a+b) }").unwrap();
        assert_eq!(name, "par");
        assert_eq!(f.args, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(f.body, "a*b/(a+b)");
        assert!(parse_function("pwl(x) {x}")
            .unwrap_err()
            .contains("built-in"));
        assert!(parse_function("f(x) {}").is_err());
    }

    #[test]
    fn assignments_fold_spaces_and_keep_braces_whole() {
        let pairs = assignments("a=1 b = {x * 2} C =3k").unwrap();
        assert_eq!(
            pairs,
            vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "{x * 2}".to_string()),
                ("c".to_string(), "3k".to_string()),
            ]
        );
        assert!(assignments("a").is_err());
    }
}
