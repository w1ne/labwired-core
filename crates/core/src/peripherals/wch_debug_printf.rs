// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! WCH single-wire debug "printf" channel: the debug-module data registers a
//! CH32V003 core can reach at 0xE000_00F4, with a WCH-LinkE probe attached
//! and polling them the way `minichlink -T` does.
//!
//! ch32fun (cnlohr/ch32fun) prints through this channel by default
//! (`FUNCONF_USE_DEBUGPRINTF`), so most of its examples produce no UART
//! output at all. Register map (ch32fun `ch32v003hw.h`):
//!
//! | Offset | Address | Name | Meaning |
//! | --- | --- | --- | --- |
//! | 0x0 | 0xE00000F4 | `DMDATA0` | status byte + up to 3 characters |
//! | 0x4 | 0xE00000F8 | `DMDATA1` | characters 4..7 |
//! | 0x8 | 0xE00000FC | sentinel | reads 0 while a debugger is attached |
//!
//! Protocol (ch32fun `_write`/`putchar`, minichlink `DefaultPollTerminal`):
//! the firmware waits for `DMDATA0` bit 7 to clear, stores characters 4..7
//! into `DMDATA1`, then stores `0x80 | (n + 4)` plus characters 1..3 into
//! `DMDATA0`. The probe sees bit 7, takes `(DMDATA0 & 0xF) - 4` characters
//! (0 for the `0x80` that `SetupDebugPrintf` writes) and acknowledges by
//! writing 0 to `DMDATA0`.
//!
//! Here the probe takes each request on the store that raises bit 7, so the
//! firmware never waits on it and never reaches its timeout. A real probe
//! polls over USB, which makes silicon slower but prints the same text.
//! Requests made with byte stores are taken when the byte holding bit 7
//! lands. Not modeled: host-to-target input (minichlink's `leaveflagA`, which
//! ch32fun hands to `handle_debug_input`), a detached probe (leave this block
//! out of the chip instead), and the rest of the debug module.
//!
//! The printed text is the `terminal` log, one entry per line. A line not yet
//! ended by `\n` is the last entry.

use crate::peripheral_log::PeripheralLog;
use crate::SimResult;

const DMDATA0: u64 = 0x0;
const DMDATA1: u64 = 0x4;
const SENTINEL: u64 = 0x8;

/// `DMDATA0` bit 7: a request is waiting for the probe.
const PENDING: u32 = 0x80;
/// Longest output the protocol carries in one request.
const MAX_CHARS: usize = 7;
/// Bound on the retained transcript so a firmware printing forever cannot
/// grow memory without limit; the oldest lines go first.
const MAX_LINES: usize = 4096;

/// Name of the transcript log.
pub const TERMINAL_LOG: &str = "terminal";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct WchDebugPrintf {
    dmdata0: u32,
    dmdata1: u32,
    /// Completed lines, oldest first.
    lines: std::collections::VecDeque<String>,
    /// Characters after the last `\n`.
    partial: String,
    /// Lines dropped from the front once `MAX_LINES` was reached.
    dropped_lines: u64,
}

impl WchDebugPrintf {
    pub fn new() -> Self {
        Self::default()
    }

    /// The probe's side of one poll: take a waiting request, if any.
    fn poll_probe(&mut self) {
        if self.dmdata0 & PENDING == 0 {
            return;
        }
        let count = (self.dmdata0 & 0xF) as i32 - 4;
        if count > 0 && count as usize <= MAX_CHARS {
            let count = count as usize;
            let mut chars = [0u8; MAX_CHARS];
            chars[..3].copy_from_slice(&self.dmdata0.to_le_bytes()[1..4]);
            chars[3..].copy_from_slice(&self.dmdata1.to_le_bytes());
            for &c in &chars[..count] {
                self.push_char(c);
            }
        }
        // minichlink acknowledges with leaveflagA = 0.
        self.dmdata0 = 0;
    }

    fn push_char(&mut self, c: u8) {
        match c {
            b'\n' => {
                let line = std::mem::take(&mut self.partial);
                if self.lines.len() == MAX_LINES {
                    self.lines.pop_front();
                    self.dropped_lines += 1;
                }
                self.lines.push_back(line);
            }
            b'\r' => {}
            _ => self.partial.push(char::from(c)),
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            DMDATA0 => self.dmdata0,
            DMDATA1 => self.dmdata1,
            // Attached: ch32fun's DidDebuggerAttach() tests for 0.
            SENTINEL => 0,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            DMDATA0 => {
                self.dmdata0 = value;
                self.poll_probe();
            }
            DMDATA1 => self.dmdata1 = value,
            _ => {}
        }
    }

    /// Everything printed so far, lines joined with `\n`.
    pub fn transcript(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&self.partial);
        out
    }
}

impl crate::Peripheral for WchDebugPrintf {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) as u32 * 8;
        let merged = (self.read_reg(reg) & !(0xFF << shift)) | (u32::from(value) << shift);
        if reg == DMDATA0 && shift != 0 {
            // Only the byte that carries the status bit hands the request to
            // the probe; the character bytes before it are just stored.
            self.dmdata0 = merged;
        } else {
            self.write_reg(reg, merged);
        }
        Ok(())
    }

    /// One word store is one request, never three partial ones.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset % 4 == 0 {
            self.write_reg(offset, value);
            return Ok(());
        }
        for (i, byte) in value.to_le_bytes().iter().enumerate() {
            self.write(offset + i as u64, *byte)?;
        }
        Ok(())
    }

    fn logs(&self) -> Vec<PeripheralLog> {
        let mut lines: Vec<String> = self.lines.iter().cloned().collect();
        if !self.partial.is_empty() {
            lines.push(self.partial.clone());
        }
        vec![PeripheralLog::new(TERMINAL_LOG, lines)]
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        *self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "WchDebugPrintf snapshot decode: {error}"
            ))
        })?;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize WchDebugPrintf")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        *self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "WchDebugPrintf runtime snapshot decode: {error}"
            ))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    /// ch32fun `_write` for one chunk of up to 7 characters.
    fn ch32fun_write_chunk(p: &mut WchDebugPrintf, chunk: &[u8]) {
        assert!(chunk.len() <= MAX_CHARS);
        assert_eq!(
            p.read_u32(DMDATA0).unwrap() & PENDING,
            0,
            "probe has not acked"
        );
        let mut hi = [0u8; 4];
        for (i, &c) in chunk.iter().skip(3).enumerate() {
            hi[i] = c;
        }
        p.write_u32(DMDATA1, u32::from_le_bytes(hi)).unwrap();
        let mut lo = [0u8; 4];
        lo[0] = 0x80 | (chunk.len() as u8 + 4);
        for (i, &c) in chunk.iter().take(3).enumerate() {
            lo[i + 1] = c;
        }
        p.write_u32(DMDATA0, u32::from_le_bytes(lo)).unwrap();
    }

    #[test]
    fn sentinel_reads_attached() {
        let p = WchDebugPrintf::new();
        assert_eq!(p.read_u32(SENTINEL).unwrap(), 0);
    }

    #[test]
    fn setup_marker_is_acked_without_output() {
        let mut p = WchDebugPrintf::new();
        // SetupDebugPrintf: DMDATA1 = 0, DMDATA0 = 0x80.
        p.write_u32(DMDATA1, 0).unwrap();
        p.write_u32(DMDATA0, 0x80).unwrap();
        assert_eq!(p.read_u32(DMDATA0).unwrap(), 0);
        assert_eq!(p.transcript(), "");
    }

    #[test]
    fn write_chunks_reassemble_lines() {
        let mut p = WchDebugPrintf::new();
        let text = b"systick_irq example\nBeginning Loop...\n";
        for chunk in text.chunks(MAX_CHARS) {
            ch32fun_write_chunk(&mut p, chunk);
        }
        assert_eq!(p.transcript(), "systick_irq example\nBeginning Loop...\n");
        let logs = p.logs();
        assert_eq!(logs[0].name, TERMINAL_LOG);
        assert_eq!(
            logs[0].lines(),
            vec!["systick_irq example", "Beginning Loop..."]
        );
    }

    #[test]
    fn putchar_and_partial_line() {
        let mut p = WchDebugPrintf::new();
        // ch32fun putchar: DMDATA0 = 0x85 | (c << 8).
        for &c in b"ok" {
            p.write_u32(DMDATA0, 0x85 | (u32::from(c) << 8)).unwrap();
        }
        assert_eq!(p.logs()[0].lines(), vec!["ok"]);
    }

    #[test]
    fn timeout_marker_is_not_a_request_once_acked() {
        let mut p = WchDebugPrintf::new();
        // `*DMDATA0 |= 0xc0` after a timeout; the probe acks it like any
        // other count-0 request, so printing resumes.
        p.write_u32(DMDATA0, 0xC0).unwrap();
        assert_eq!(p.read_u32(DMDATA0).unwrap(), 0);
        ch32fun_write_chunk(&mut p, b"x\n");
        assert_eq!(p.transcript(), "x\n");
    }

    #[test]
    fn byte_stores_hand_over_on_the_status_byte() {
        let mut p = WchDebugPrintf::new();
        p.write(DMDATA0 + 1, b'h').unwrap();
        p.write(DMDATA0 + 2, b'i').unwrap();
        assert_eq!(p.transcript(), "");
        p.write(DMDATA0, 0x86).unwrap();
        assert_eq!(p.transcript(), "hi");
    }

    #[test]
    fn transcript_is_bounded() {
        let mut p = WchDebugPrintf::new();
        for _ in 0..MAX_LINES + 3 {
            p.write_u32(DMDATA0, 0x85 | (u32::from(b'\n') << 8))
                .unwrap();
        }
        assert_eq!(p.lines.len(), MAX_LINES);
        assert_eq!(p.dropped_lines, 3);
    }

    #[test]
    fn snapshot_round_trip_keeps_transcript() {
        let mut p = WchDebugPrintf::new();
        ch32fun_write_chunk(&mut p, b"abc\nde");
        let mut q = WchDebugPrintf::new();
        q.restore_runtime_snapshot(&p.runtime_snapshot()).unwrap();
        assert_eq!(q.transcript(), "abc\nde");
    }
}
