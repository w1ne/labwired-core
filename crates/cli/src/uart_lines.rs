// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Simulated-time stamps for the UART capture, one per line.
//!
//! `uart.log` is a flat byte stream, so an oracle cannot ask "what was the pad
//! doing when the firmware printed FIRE". This recorder watches the SAME sink
//! `uart.log` is written from and stamps every line with the engine cycle at
//! which its first byte, and its newline, became visible to the run loop.
//!
//! Resolution is the run loop's observation window, never better: a byte that
//! appears during one `advance` is known only to lie in `(cycle_lo, cycle]`.
//! Both bounds are reported so a consumer can widen its window honestly
//! instead of trusting a stamp finer than the evidence.

use serde::{Deserialize, Serialize};

/// Lines kept before the block stops growing (a chatty firmware must not make
/// `result.json` unbounded). Overflow is counted in `dropped_lines`, never
/// silently lost.
pub(crate) const MAX_UART_LINES: usize = 20_000;
/// Bytes of text kept per line; longer lines set `text_truncated`.
pub(crate) const MAX_UART_LINE_TEXT: usize = 512;

/// One newline-terminated (or, for the last one, still open) UART line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UartLine {
    /// Line text without its terminator (`\n`, and a `\r` before it).
    pub(crate) text: String,
    /// The first byte appeared after this cycle...
    pub(crate) cycle_lo: u64,
    /// ...and at or before this one (the line's stamp).
    pub(crate) cycle: u64,
    /// Cycle by which the newline had appeared; absent for an unterminated
    /// last line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) end_cycle: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) text_truncated: bool,
}

/// `result.json` `uart_lines` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UartLinesResult {
    pub(crate) lines: Vec<UartLine>,
    /// Lines past `MAX_UART_LINES` that were counted but not kept.
    pub(crate) dropped_lines: u64,
}

#[derive(Debug, Default)]
struct OpenLine {
    bytes: Vec<u8>,
    cycle_lo: u64,
    cycle: u64,
    truncated: bool,
}

/// Incremental recorder: feed it the accumulated sink after each advance.
#[derive(Debug, Default)]
pub(crate) struct UartLineRecorder {
    seen: usize,
    last_cycle: u64,
    open: Option<OpenLine>,
    lines: Vec<UartLine>,
    dropped: u64,
}

impl UartLineRecorder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Stamp every byte past the last observation with `(last, now]`.
    pub(crate) fn observe(&mut self, accumulated: &[u8], now: u64) {
        if accumulated.len() < self.seen {
            // The sink was cleared under us: restart from its new contents.
            self.seen = 0;
        }
        let lo = self.last_cycle;
        for &byte in &accumulated[self.seen..] {
            let open = self.open.get_or_insert_with(|| OpenLine {
                cycle_lo: lo,
                cycle: now,
                ..OpenLine::default()
            });
            if byte == b'\n' {
                let line = self.open.take().unwrap_or_default();
                self.push(line, Some(now));
                continue;
            }
            if open.bytes.len() < MAX_UART_LINE_TEXT {
                open.bytes.push(byte);
            } else {
                open.truncated = true;
            }
        }
        self.seen = accumulated.len();
        self.last_cycle = now;
    }

    fn push(&mut self, mut line: OpenLine, end_cycle: Option<u64>) {
        if self.lines.len() >= MAX_UART_LINES {
            self.dropped += 1;
            return;
        }
        if line.bytes.last() == Some(&b'\r') {
            line.bytes.pop();
        }
        self.lines.push(UartLine {
            text: String::from_utf8_lossy(&line.bytes).into_owned(),
            cycle_lo: line.cycle_lo,
            cycle: line.cycle,
            end_cycle,
            text_truncated: line.truncated,
        });
    }

    /// The block for `result.json`; `None` when the run printed nothing, so a
    /// silent run keeps a byte-identical result.
    pub(crate) fn finish(mut self) -> Option<UartLinesResult> {
        if let Some(open) = self.open.take() {
            self.push(open, None);
        }
        if self.lines.is_empty() && self.dropped == 0 {
            return None;
        }
        Some(UartLinesResult {
            lines: self.lines,
            dropped_lines: self.dropped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_run_has_no_block() {
        assert!(UartLineRecorder::new().finish().is_none());
    }

    #[test]
    fn each_line_is_stamped_with_the_window_its_first_byte_appeared_in() {
        let mut r = UartLineRecorder::new();
        let mut sink = b"REA".to_vec();
        r.observe(&sink, 100);
        sink.extend_from_slice(b"DY\r\nGO");
        r.observe(&sink, 250);
        sink.extend_from_slice(b"\n");
        r.observe(&sink, 900);
        let out = r.finish().unwrap();
        let texts: Vec<_> = out.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["READY", "GO"]);
        let ready = &out.lines[0];
        assert!(ready.cycle_lo < ready.cycle && ready.cycle <= 100);
        assert_eq!(ready.end_cycle, Some(250));
        let go = &out.lines[1];
        assert!(go.cycle_lo >= 100 && go.cycle <= 250 && go.end_cycle == Some(900));
    }

    #[test]
    fn stamps_never_go_backwards_and_cover_the_whole_capture() {
        let mut r = UartLineRecorder::new();
        let mut sink = Vec::new();
        let mut expected = String::new();
        for i in 0..50u64 {
            let line = format!("L{i}\r\n");
            expected.push_str(&line);
            sink.extend_from_slice(line.as_bytes());
            r.observe(&sink, (i + 1) * 1000);
        }
        let out = r.finish().unwrap();
        let joined: String = out
            .lines
            .iter()
            .map(|l| format!("{}\r\n", l.text))
            .collect();
        assert_eq!(joined, expected);
        for pair in out.lines.windows(2) {
            assert!(pair[0].cycle <= pair[1].cycle_lo.max(pair[1].cycle));
            assert!(pair[0].cycle <= pair[1].cycle);
        }
    }

    #[test]
    fn an_unterminated_last_line_is_kept_without_an_end() {
        let mut r = UartLineRecorder::new();
        r.observe(b"A\nPARTIAL", 10);
        let out = r.finish().unwrap();
        assert_eq!(out.lines.last().unwrap().text, "PARTIAL");
        assert_eq!(out.lines.last().unwrap().end_cycle, None);
    }

    #[test]
    fn overflow_is_counted_not_lost_silently() {
        let mut r = UartLineRecorder::new();
        let sink = "x\n".repeat(MAX_UART_LINES + 3);
        r.observe(sink.as_bytes(), 1);
        let out = r.finish().unwrap();
        assert_eq!(
            out.lines.len() as u64 + out.dropped_lines,
            (MAX_UART_LINES + 3) as u64
        );
        assert!(out.dropped_lines > 0);
    }

    #[test]
    fn an_overlong_line_is_flagged_truncated() {
        let mut r = UartLineRecorder::new();
        let long = format!("{}\n", "y".repeat(MAX_UART_LINE_TEXT * 2));
        r.observe(long.as_bytes(), 1);
        let line = &r.finish().unwrap().lines[0];
        assert!(line.text_truncated && line.text.len() <= MAX_UART_LINE_TEXT);
    }
}
