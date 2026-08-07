// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The regex engine behind `uart_regex` assertions.
//!
//! ## Why this replaced a 30-line matcher
//!
//! `uart_regex` used to run a Rob Pike-style matcher supporting exactly `^`,
//! `$`, `.` and `*`. Everything else — `+`, `?`, `[a-z]`, `\d`, `|`, `{2,4}` —
//! was matched **literally**, so a pattern like `temp=\d+C` silently never
//! matched and the assertion failed with no hint that the syntax was the
//! problem. Firmware output is full of numbers and delimiters; a matcher that
//! cannot say "one or more digits" pushes every real assertion toward
//! `uart_contains`, which is weaker still.
//!
//! ## What it supports
//!
//! | Syntax | Meaning |
//! |---|---|
//! | `.` | any character |
//! | `*` `+` `?` | zero-or-more, one-or-more, optional (append `?` for lazy) |
//! | `{n}` `{n,}` `{n,m}` | counted repetition |
//! | `[abc]` `[a-z]` `[^0-9]` | character class, range, negated |
//! | `\d \D \w \W \s \S` | digit / word / space classes and their negations |
//! | `\.` `\*` `\\` … | escape any metacharacter |
//! | `\|` | alternation |
//! | `( … )` | grouping (no capture — these assertions only ask "did it match") |
//! | `^` `$` | start / end of the searched text |
//!
//! A pattern with no `^` is searched at every position, matching the old
//! engine's behaviour.
//!
//! ## Two deliberate safety properties
//!
//! - **Bounded work.** Backtracking regexes can blow up exponentially
//!   (`(a+)+b` against a long run of `a`). This engine counts steps and gives
//!   up at [`STEP_BUDGET`] rather than hanging a test run forever.
//! - **A broken pattern fails loudly.** A parse error returns `Err`, and the
//!   caller turns that into "assertion did not pass" — so a typo fails the test
//!   instead of quietly matching nothing and being mistaken for a firmware bug.

use std::fmt;

/// Maximum match steps before the engine gives up. Generous for real firmware
/// output (megabytes of UART against a sane pattern) and still far below
/// anything a user would experience as a hang.
pub const STEP_BUDGET: u64 = 2_000_000;

/// Why a pattern could not be evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegexError {
    /// The pattern is not valid syntax.
    Parse(String),
    /// Matching exceeded [`STEP_BUDGET`] — almost always catastrophic
    /// backtracking in the pattern.
    StepBudgetExceeded,
}

impl fmt::Display for RegexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(msg) => write!(f, "invalid regex: {msg}"),
            Self::StepBudgetExceeded => write!(
                f,
                "regex gave up after {STEP_BUDGET} steps (pattern backtracks too much)"
            ),
        }
    }
}

/// One entry inside `[...]`.
#[derive(Debug, Clone)]
enum ClassItem {
    Char(char),
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

impl ClassItem {
    fn matches(&self, c: char) -> bool {
        match self {
            Self::Char(x) => *x == c,
            Self::Range(lo, hi) => *lo <= c && c <= *hi,
            Self::Digit(pos) => c.is_ascii_digit() == *pos,
            Self::Word(pos) => (c.is_alphanumeric() || c == '_') == *pos,
            Self::Space(pos) => c.is_whitespace() == *pos,
        }
    }
}

#[derive(Debug, Clone)]
enum Node {
    Char(char),
    Any,
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
    Group(Alt),
    Repeat {
        node: Box<Node>,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    },
    Start,
    End,
}

/// A sequence of nodes matched in order.
type Concat = Vec<Node>;
/// Alternatives, any one of which may match.
type Alt = Vec<Concat>;

// ---------------------------------------------------------------- parsing

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_alt(&mut self) -> Result<Alt, RegexError> {
        let mut alts = vec![self.parse_concat()?];
        while self.eat('|') {
            alts.push(self.parse_concat()?);
        }
        Ok(alts)
    }

    fn parse_concat(&mut self) -> Result<Concat, RegexError> {
        let mut nodes = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            nodes.push(self.parse_repeat()?);
        }
        Ok(nodes)
    }

    fn parse_repeat(&mut self) -> Result<Node, RegexError> {
        let atom = self.parse_atom()?;
        let (min, max) = match self.peek() {
            Some('*') => {
                self.pos += 1;
                (0, None)
            }
            Some('+') => {
                self.pos += 1;
                (1, None)
            }
            Some('?') => {
                self.pos += 1;
                (0, Some(1))
            }
            Some('{') => match self.parse_counted()? {
                Some(bounds) => bounds,
                // `{` that is not a valid counted repetition stays a literal —
                // firmware prints `{` constantly (JSON, C braces), so rejecting
                // the pattern would be worse than matching what the user sees.
                None => return Ok(atom),
            },
            _ => return Ok(atom),
        };
        // A trailing `?` makes the quantifier lazy.
        let greedy = !self.eat('?');
        if let Some(max) = max {
            if max < min {
                return Err(RegexError::Parse(format!(
                    "repetition {{{min},{max}}} has max below min"
                )));
            }
        }
        Ok(Node::Repeat {
            node: Box::new(atom),
            min,
            max,
            greedy,
        })
    }

    /// `{n}` / `{n,}` / `{n,m}`. Returns `None` (rewinding) if this `{` does not
    /// begin a valid counted repetition.
    fn parse_counted(&mut self) -> Result<Option<(u32, Option<u32>)>, RegexError> {
        let start = self.pos;
        self.pos += 1; // consume '{'
        let mut lo = String::new();
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            lo.push(self.bump().unwrap());
        }
        if lo.is_empty() {
            self.pos = start;
            return Ok(None);
        }
        let min: u32 = lo
            .parse()
            .map_err(|_| RegexError::Parse(format!("repetition count {lo} is too large")))?;
        let max = if self.eat(',') {
            let mut hi = String::new();
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                hi.push(self.bump().unwrap());
            }
            if hi.is_empty() {
                None
            } else {
                Some(hi.parse::<u32>().map_err(|_| {
                    RegexError::Parse(format!("repetition count {hi} is too large"))
                })?)
            }
        } else {
            Some(min)
        };
        if !self.eat('}') {
            self.pos = start;
            return Ok(None);
        }
        Ok(Some((min, max)))
    }

    fn parse_atom(&mut self) -> Result<Node, RegexError> {
        let c = self
            .bump()
            .ok_or_else(|| RegexError::Parse("pattern ends where an atom was expected".into()))?;
        match c {
            '(' => {
                let inner = self.parse_alt()?;
                if !self.eat(')') {
                    return Err(RegexError::Parse("unclosed `(`".into()));
                }
                Ok(Node::Group(inner))
            }
            '[' => self.parse_class(),
            '.' => Ok(Node::Any),
            '^' => Ok(Node::Start),
            '$' => Ok(Node::End),
            '\\' => {
                let e = self
                    .bump()
                    .ok_or_else(|| RegexError::Parse("pattern ends with a lone `\\`".into()))?;
                Ok(escape_node(e))
            }
            // A quantifier with nothing to quantify.
            '*' | '+' => Err(RegexError::Parse(format!(
                "`{c}` has nothing to repeat — escape it as `\\{c}` to match it literally"
            ))),
            other => Ok(Node::Char(other)),
        }
    }

    fn parse_class(&mut self) -> Result<Node, RegexError> {
        let negated = self.eat('^');
        let mut items = Vec::new();
        // A `]` first is a literal `]`, as in POSIX.
        if self.eat(']') {
            items.push(ClassItem::Char(']'));
        }
        loop {
            let c = match self.bump() {
                Some(']') => break,
                Some(c) => c,
                None => return Err(RegexError::Parse("unclosed `[`".into())),
            };
            let item = if c == '\\' {
                let e = self.bump().ok_or_else(|| {
                    RegexError::Parse("pattern ends with a lone `\\` in a class".into())
                })?;
                match escape_node(e) {
                    Node::Class { items, .. } if items.len() == 1 => items[0].clone(),
                    Node::Char(ch) => ClassItem::Char(ch),
                    _ => ClassItem::Char(e),
                }
            } else {
                ClassItem::Char(c)
            };
            // Range, but only between plain characters: `[\d-z]` is a literal
            // `-`, not a range from a class.
            if let (ClassItem::Char(lo), Some('-')) = (&item, self.peek()) {
                if self.chars.get(self.pos + 1).is_some_and(|&n| n != ']') {
                    self.pos += 1; // consume '-'
                    let hi = self.bump().unwrap();
                    let hi = if hi == '\\' {
                        self.bump().ok_or_else(|| {
                            RegexError::Parse("pattern ends inside a class range".into())
                        })?
                    } else {
                        hi
                    };
                    if hi < *lo {
                        return Err(RegexError::Parse(format!(
                            "class range {lo}-{hi} runs backwards"
                        )));
                    }
                    items.push(ClassItem::Range(*lo, hi));
                    continue;
                }
            }
            items.push(item);
        }
        if items.is_empty() {
            return Err(RegexError::Parse("empty character class `[]`".into()));
        }
        Ok(Node::Class { negated, items })
    }
}

fn escape_node(e: char) -> Node {
    let single = |item| Node::Class {
        negated: false,
        items: vec![item],
    };
    match e {
        'd' => single(ClassItem::Digit(true)),
        'D' => single(ClassItem::Digit(false)),
        'w' => single(ClassItem::Word(true)),
        'W' => single(ClassItem::Word(false)),
        's' => single(ClassItem::Space(true)),
        'S' => single(ClassItem::Space(false)),
        'n' => Node::Char('\n'),
        'r' => Node::Char('\r'),
        't' => Node::Char('\t'),
        '0' => Node::Char('\0'),
        other => Node::Char(other),
    }
}

// --------------------------------------------------------------- matching

struct Matcher<'a> {
    text: &'a [char],
    steps: u64,
}

impl Matcher<'_> {
    fn step(&mut self) -> Result<(), RegexError> {
        self.steps += 1;
        if self.steps > STEP_BUDGET {
            return Err(RegexError::StepBudgetExceeded);
        }
        Ok(())
    }

    /// Match `nodes` starting at `pos`, calling `k` with each end position that
    /// works. Continuation passing is what makes backtracking through nested
    /// quantifiers straightforward.
    fn concat(
        &mut self,
        nodes: &[Node],
        pos: usize,
        k: &mut dyn FnMut(&mut Self, usize) -> Result<bool, RegexError>,
    ) -> Result<bool, RegexError> {
        self.step()?;
        let Some((first, rest)) = nodes.split_first() else {
            return k(self, pos);
        };
        self.node(first, pos, &mut |m, next| m.concat(rest, next, k))
    }

    fn alt(
        &mut self,
        alts: &Alt,
        pos: usize,
        k: &mut dyn FnMut(&mut Self, usize) -> Result<bool, RegexError>,
    ) -> Result<bool, RegexError> {
        for branch in alts {
            if self.concat(branch, pos, k)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn node(
        &mut self,
        node: &Node,
        pos: usize,
        k: &mut dyn FnMut(&mut Self, usize) -> Result<bool, RegexError>,
    ) -> Result<bool, RegexError> {
        self.step()?;
        match node {
            Node::Start => {
                if pos == 0 {
                    k(self, pos)
                } else {
                    Ok(false)
                }
            }
            Node::End => {
                if pos == self.text.len() {
                    k(self, pos)
                } else {
                    Ok(false)
                }
            }
            Node::Char(c) => {
                if self.text.get(pos) == Some(c) {
                    k(self, pos + 1)
                } else {
                    Ok(false)
                }
            }
            Node::Any => {
                if pos < self.text.len() {
                    k(self, pos + 1)
                } else {
                    Ok(false)
                }
            }
            Node::Class { negated, items } => {
                let Some(&c) = self.text.get(pos) else {
                    return Ok(false);
                };
                let hit = items.iter().any(|i| i.matches(c));
                if hit != *negated {
                    k(self, pos + 1)
                } else {
                    Ok(false)
                }
            }
            Node::Group(alts) => self.alt(alts, pos, k),
            Node::Repeat {
                node,
                min,
                max,
                greedy,
            } => self.repeat(node, *min, *max, *greedy, 0, pos, k),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat(
        &mut self,
        node: &Node,
        min: u32,
        max: Option<u32>,
        greedy: bool,
        done: u32,
        pos: usize,
        k: &mut dyn FnMut(&mut Self, usize) -> Result<bool, RegexError>,
    ) -> Result<bool, RegexError> {
        self.step()?;
        let can_more = max.is_none_or(|m| done < m);
        let may_stop = done >= min;

        // One more iteration. The `next > pos` guard stops an empty-matching
        // body (`(a*)*`) from looping forever.
        let take_more = |m: &mut Self,
                         k: &mut dyn FnMut(&mut Self, usize) -> Result<bool, RegexError>|
         -> Result<bool, RegexError> {
            if !can_more {
                return Ok(false);
            }
            m.node(node, pos, &mut |m, next| {
                if next == pos {
                    return Ok(false);
                }
                m.repeat(node, min, max, greedy, done + 1, next, k)
            })
        };

        if greedy {
            if take_more(self, k)? {
                return Ok(true);
            }
            if may_stop {
                return k(self, pos);
            }
            Ok(false)
        } else {
            if may_stop && k(self, pos)? {
                return Ok(true);
            }
            take_more(self, k)
        }
    }
}

/// Compile `pattern` and report whether it matches anywhere in `text`.
///
/// Anchor with `^` / `$` to require a whole-text match. Errors are returned
/// rather than swallowed so a bad pattern is distinguishable from a pattern
/// that simply did not match.
pub fn is_match(pattern: &str, text: &str) -> Result<bool, RegexError> {
    let pat: Vec<char> = pattern.chars().collect();
    let mut parser = Parser {
        chars: &pat,
        pos: 0,
    };
    let ast = parser.parse_alt()?;
    if parser.pos != pat.len() {
        // The only way to stop early is an unbalanced `)`.
        return Err(RegexError::Parse("unmatched `)`".into()));
    }

    let text_chars: Vec<char> = text.chars().collect();
    let mut m = Matcher {
        text: &text_chars,
        steps: 0,
    };
    for start in 0..=text_chars.len() {
        if m.alt(&ast, start, &mut |_, _| Ok(true))? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, t: &str) -> bool {
        is_match(p, t).unwrap_or_else(|e| panic!("pattern {p:?} failed: {e}"))
    }

    // ---- everything the old engine could do, unchanged --------------------

    #[test]
    fn literals_anchors_dot_and_star_behave_as_before() {
        assert!(m("MOOD=ACTIVE", "before MOOD=ACTIVE after"));
        assert!(!m("MOOD=CALM", "before MOOD=ACTIVE after"));
        assert!(m("^Hello.*$", "Hello world"));
        assert!(!m("^ThisTextWillNeverBeFound$", "Hello world"));
        assert!(m(".*", ""));
        assert!(m("a.c", "abc"));
        assert!(m("ab*c", "ac"));
        assert!(m("ab*c", "abbbc"));
        assert!(!m("^abc$", "xabc"));
    }

    // ---- what it could not do, which is the point -------------------------

    #[test]
    fn digits_and_plus_now_work() {
        // The motivating case: firmware prints a number and a test wants to
        // assert its shape. The old engine matched this literally and failed.
        assert!(m(r"temp=\d+C", "log: temp=25C ok"));
        assert!(!m(r"temp=\d+C", "log: temp=C ok"));
    }

    #[test]
    fn character_classes_and_ranges() {
        assert!(m("[abc]x", "bx"));
        assert!(!m("[abc]x", "dx"));
        assert!(m("[a-f0-9]{4}", "beef"));
        assert!(!m("^[a-f0-9]{4}$", "beeg"));
        assert!(m("[^0-9]+", "abc"));
        assert!(m("[]]", "]"));
        assert!(m("[a-]", "-"));
    }

    #[test]
    fn alternation_groups_and_optional() {
        assert!(m("^(PASS|FAIL)$", "PASS"));
        assert!(m("^(PASS|FAIL)$", "FAIL"));
        assert!(!m("^(PASS|FAIL)$", "MEH"));
        assert!(m("colou?r", "color"));
        assert!(m("colou?r", "colour"));
        assert!(m("^(ab)+$", "ababab"));
    }

    #[test]
    fn counted_repetition() {
        assert!(m("^a{3}$", "aaa"));
        assert!(!m("^a{3}$", "aa"));
        assert!(m("^a{2,}$", "aaaa"));
        assert!(m("^a{2,3}$", "aaa"));
        assert!(!m("^a{2,3}$", "aaaa"));
    }

    #[test]
    fn escapes_make_metacharacters_literal() {
        assert!(m(r"3\.14", "pi=3.14"));
        assert!(!m(r"^3\.14$", "3x14"));
        assert!(m(r"a\+b", "a+b"));
        assert!(m(r"\[ok\]", "[ok]"));
    }

    #[test]
    fn lazy_quantifiers_stop_early() {
        // Both match; the distinction matters only with anchors around them.
        assert!(m("^<.+?>$", "<a>"));
        assert!(m("^<.*?>$", "<>"));
    }

    #[test]
    fn a_brace_that_is_not_a_quantifier_stays_literal() {
        // Firmware prints JSON and C braces constantly. Rejecting these would
        // be worse than matching what the user actually sees.
        assert!(m("{ok}", "value {ok} here"));
        assert!(m(r"\{.*\}", "{a:1}"));
    }

    // ---- failure modes ----------------------------------------------------

    #[test]
    fn a_broken_pattern_is_an_error_not_a_silent_non_match() {
        assert!(matches!(
            is_match("(unclosed", "x"),
            Err(RegexError::Parse(_))
        ));
        assert!(matches!(is_match("a)", "x"), Err(RegexError::Parse(_))));
        assert!(matches!(is_match("[a-", "x"), Err(RegexError::Parse(_))));
        assert!(matches!(is_match("*abc", "x"), Err(RegexError::Parse(_))));
        assert!(matches!(is_match(r"a\", "x"), Err(RegexError::Parse(_))));
        assert!(matches!(is_match("a{3,1}", "x"), Err(RegexError::Parse(_))));
    }

    #[test]
    fn catastrophic_backtracking_terminates_instead_of_hanging() {
        // The classic exponential blowup. It must come back — with either an
        // answer or a budget error — not spin.
        let text = "a".repeat(40);
        match is_match("^(a+)+b$", &text) {
            Ok(false) => {}
            Err(RegexError::StepBudgetExceeded) => {}
            other => panic!("expected a bounded result, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_matching_body_does_not_loop_forever() {
        assert!(m("^(a*)*$", "aaa"));
        assert!(m("^(a*)*$", ""));
    }

    #[test]
    fn it_handles_realistic_firmware_output() {
        let log = "boot ok\r\nsensor bme280 t=21.4C h=48%\r\nstatus: READY\r\n";
        assert!(m(r"t=\d+\.\d+C", log));
        assert!(m(r"h=\d+%", log));
        assert!(m(r"status:\s+(READY|BUSY)", log));
        assert!(!m(r"t=\d+\.\d+F", log));
    }
}
