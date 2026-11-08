// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::pad_lines::{LineDrive, PadLines};
use crate::CycleClock;
use crate::Peripheral;
use crate::PeripheralTickResult;
use crate::SimResult;
use anyhow::Result;
use std::sync::Arc;

/// Event-token for the self-perpetuating SM step chain under `event-scheduler`.
const EVT_SM_STEP: u32 = 1;

/// `SHIFTCTRL` reset value (RP2040 datasheet §3.7): IN_SHIFTDIR and
/// OUT_SHIFTDIR set (shift right), thresholds 0 (= 32), no autopush/pull.
const SHIFTCTRL_RESET: u32 = 0x000c_0000;
/// `PINCTRL` reset value: SET_COUNT = 5, everything else 0.
const PINCTRL_RESET: u32 = 0x1400_0000;
const SHIFTCTRL_AUTOPUSH: u32 = 1 << 16;
const SHIFTCTRL_AUTOPULL: u32 = 1 << 17;
const SHIFTCTRL_IN_SHIFTDIR: u32 = 1 << 18;
const SHIFTCTRL_OUT_SHIFTDIR: u32 = 1 << 19;
const SHIFTCTRL_FJOIN_TX: u32 = 1 << 30;
const SHIFTCTRL_FJOIN_RX: u32 = 1 << 31;
const EXECCTRL_SIDE_PINDIR: u32 = 1 << 29;
const EXECCTRL_SIDE_EN: u32 = 1 << 30;

/// One wire line per GPIO the block can drive: line `n` is the PIO's output
/// to GPIOn (datasheet §3.5.6 — every state machine's pin mapping lands on
/// the same 32 outputs, and IO_BANK0 FUNCSEL picks which pads listen).
pub const LINE_NAMES: [&str; 32] = [
    "PIO_GP0", "PIO_GP1", "PIO_GP2", "PIO_GP3", "PIO_GP4", "PIO_GP5", "PIO_GP6", "PIO_GP7",
    "PIO_GP8", "PIO_GP9", "PIO_GP10", "PIO_GP11", "PIO_GP12", "PIO_GP13", "PIO_GP14", "PIO_GP15",
    "PIO_GP16", "PIO_GP17", "PIO_GP18", "PIO_GP19", "PIO_GP20", "PIO_GP21", "PIO_GP22", "PIO_GP23",
    "PIO_GP24", "PIO_GP25", "PIO_GP26", "PIO_GP27", "PIO_GP28", "PIO_GP29", "PIO_GP30", "PIO_GP31",
];

/// Write `count` bits of `data` into `word` starting at pin `base`, wrapping
/// past GPIO31 back to GPIO0 (datasheet §3.5.6).
fn write_pin_field(word: &mut u32, base: u32, count: u32, data: u32) {
    for k in 0..count.min(32) {
        let pin = (base + k) & 31;
        if data & (1 << k) != 0 {
            *word |= 1 << pin;
        } else {
            *word &= !(1 << pin);
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct StateMachine {
    pub pc: u8,
    pub x: u32,
    pub y: u32,
    pub isr: u32,
    pub osr: u32,
    pub isr_count: u8,
    pub osr_count: u8,

    pub clkdiv_int: u16,
    pub clkdiv_frac: u8,
    pub clk_counter: u32,
    pub delay_cycles: u8,

    pub wrap_top: u8,
    pub wrap_bottom: u8,

    pub exec_ctrl: u32,
    pub shift_ctrl: u32,
    pub pin_ctrl: u32,

    pub enabled: bool,
    pub stalled: bool,
}

/// `LABWIRED_PIO_PARK=0` disables the parked-chain stop (reference runs).
fn park_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LABWIRED_PIO_PARK").as_deref() != Ok("0"))
}

#[derive(Debug)]
pub struct Pio {
    pub instruction_mem: [u16; 32],
    pub sm: [StateMachine; 4],

    pub ctrl: u32,
    pub fstat: u32,
    pub fdebug: u32,
    pub flevel: u32,

    pub irq: u8,
    pub irq_force: u8,

    pub input_sync_bypass: u32,

    pub tx_fifo: [Vec<u32>; 4],
    pub rx_fifo: [Vec<u32>; 4],

    // Write accumulator: buffers byte writes until a complete 32-bit word
    write_acc: u32,
    write_acc_mask: u8, // bitmask of which bytes have been written (0b1111 = complete)
    write_acc_offset: u64, // register offset being accumulated
    /// The block's 32 pin outputs and output enables, shared by all four
    /// state machines (a later SM wins a same-cycle write, as on silicon).
    pub pin_out: u32,
    pub pin_dir: u32,
    /// Output wire to the pads ([`LINE_NAMES`]), created when the bus binds
    /// a pad to this block; `wired` starts publishing on it.
    lines: Option<Arc<PadLines>>,
    wired: bool,
    /// True while a delay-1 SM step event is live (event-scheduler).
    chain_live: bool,
    /// Bus cycle clock, handed over at `add_peripheral`/`push_peripheral` time.
    /// Its presence is the model's proof that it is attached to a bus whose
    /// owning `Machine` actually drains the event scheduler — see
    /// [`Pio::scheduler_mode`].
    clock: Option<CycleClock>,
}

impl Default for Pio {
    fn default() -> Self {
        Self::new()
    }
}

impl Pio {
    pub fn new() -> Self {
        let mut sm: [StateMachine; 4] = Default::default();
        for item in &mut sm {
            item.clkdiv_int = 1;
            item.exec_ctrl = 0x0001f000; // wrap_top=31, wrap_bottom=0
            item.shift_ctrl = SHIFTCTRL_RESET;
            item.pin_ctrl = PINCTRL_RESET;
        }
        Self {
            instruction_mem: [0; 32],
            sm,
            ctrl: 0,
            fstat: 0x0f000f00, // TXEMPTY set for all 4
            fdebug: 0,
            flevel: 0,
            irq: 0,
            irq_force: 0,
            input_sync_bypass: 0,
            tx_fifo: Default::default(),
            rx_fifo: Default::default(),
            write_acc: 0,
            write_acc_mask: 0,
            write_acc_offset: 0xFFFF_FFFF,
            pin_out: 0,
            pin_dir: 0,
            lines: None,
            wired: false,
            chain_live: false,
            clock: None,
        }
    }

    // Scheduler mode: the `event-scheduler` feature is on AND the bus handed
    // this instance its shared [`CycleClock`] at attach time.
    //
    // The clock is the only evidence a peripheral has that it went through
    // `SystemBus::add_peripheral`/`push_peripheral` — and therefore that it
    // sits on a bus whose `Machine` drains the event scheduler. PIO's
    // scheduler path is a self-perpetuating delay-1 `EVT_SM_STEP` chain
    // (`take_scheduled_events` + `on_event`): with no drain there is nothing
    // to step the state machines and every SM freezes at PC 0 forever.
    // Hand-built buses (tests, embedders that push `PeripheralEntry` directly
    // and settle with `tick_peripherals*`) therefore stay on the legacy walk
    // with exact historical semantics — the contract documented on
    // [`Peripheral::attach_cycle_clock`]. Mirrors `timer`/`dma`/`i2c`/`exti`.
    crate::cycle_clock::scheduler_mode!();

    /// True while any state machine is enabled (needs per-cycle SM steps).
    fn any_sm_enabled(&self) -> bool {
        self.sm.iter().any(|s| s.enabled)
    }

    /// True when every enabled state machine is parked on an unconditional
    /// `jmp` to its own address, with no delay pending and a 1.0 clock
    /// divider. A tick then changes nothing: the pc, the divider counter, the
    /// scratch registers and the FIFOs stay the same, so `fstat` recomputes to
    /// the same value. The scheduler step chain may stop; any PIO write
    /// re-arms it through `take_scheduled_events` at the same cycle the live
    /// delay-1 chain would have ticked.
    fn all_enabled_sms_parked(&self) -> bool {
        self.sm.iter().filter(|s| s.enabled).all(|s| {
            let instr = self.instruction_mem[(s.pc & 0x1F) as usize];
            // JMP (opcode 0), condition "always" (0), delay/side-set 0.
            instr & 0xFFE0 == 0
                && (instr & 0x1F) as u8 == s.pc
                && s.delay_cycles == 0
                && !s.stalled
                && s.clkdiv_int == 1
                && s.clkdiv_frac == 0
        })
    }

    /// TX FIFO depth of state machine `i`: 8 when RX is joined into it, 0
    /// when it is joined into RX, else 4 (datasheet §3.5.2).
    fn tx_depth(&self, i: usize) -> usize {
        let shift = self.sm[i].shift_ctrl;
        if shift & SHIFTCTRL_FJOIN_TX != 0 {
            8
        } else if shift & SHIFTCTRL_FJOIN_RX != 0 {
            0
        } else {
            4
        }
    }

    fn rx_depth(&self, i: usize) -> usize {
        let shift = self.sm[i].shift_ctrl;
        if shift & SHIFTCTRL_FJOIN_RX != 0 {
            8
        } else if shift & SHIFTCTRL_FJOIN_TX != 0 {
            0
        } else {
            4
        }
    }

    /// `FSTAT`, from the FIFOs as they are now (a TX write between two SM
    /// steps must already read back as TXFULL to `pio_sm_put_blocking`).
    fn fstat_value(&self) -> u32 {
        let mut v = 0;
        for i in 0..4 {
            let (tx, rx) = (self.tx_fifo[i].len(), self.rx_fifo[i].len());
            if tx == 0 {
                v |= 1 << (24 + i);
            }
            if tx >= self.tx_depth(i) {
                v |= 1 << (16 + i);
            }
            if rx == 0 {
                v |= 1 << (8 + i);
            }
            if rx >= self.rx_depth(i) {
                v |= 1 << i;
            }
        }
        v
    }

    /// `FLEVEL`: TX level in bits [8i+3:8i], RX level in [8i+7:8i+4].
    fn flevel_value(&self) -> u32 {
        (0..4).fold(0, |acc, i| {
            acc | ((self.tx_fifo[i].len() as u32 & 0xF) << (8 * i))
                | ((self.rx_fifo[i].len() as u32 & 0xF) << (8 * i + 4))
        })
    }

    /// Level the state machines see on the GPIO inputs. Only the pads this
    /// block drives itself are known here: an input pad reads 0 (the block
    /// has no view of the outside world yet).
    fn pins_in(&self) -> u32 {
        self.pin_out & self.pin_dir
    }

    /// Side-set value (data, bit count) and delay encoded in `instr`'s
    /// delay/side-set field under `sm`'s PINCTRL/EXECCTRL (datasheet §3.4.1).
    fn side_and_delay(sm: &StateMachine, instr: u16) -> (Option<(u32, u32)>, u8) {
        let field = u32::from((instr >> 8) & 0x1F);
        let ss_count = (sm.pin_ctrl >> 29) & 0x7; // includes the enable bit
        let delay_bits = 5u32.saturating_sub(ss_count);
        let delay = (field & ((1 << delay_bits) - 1)) as u8;
        if ss_count == 0 {
            return (None, delay);
        }
        let side = field >> delay_bits;
        let (enabled, data_bits) = if sm.exec_ctrl & EXECCTRL_SIDE_EN != 0 {
            ((side >> (ss_count - 1)) & 1 != 0, ss_count - 1)
        } else {
            (true, ss_count)
        };
        let data = side & ((1 << data_bits) - 1);
        (enabled.then_some((data, data_bits)), delay)
    }

    /// Drive a side-set value onto the side-set pins (or their directions,
    /// with `SIDE_PINDIR`). Side-set takes effect when the instruction issues,
    /// stalled or not.
    fn apply_side_set(&mut self, sm: &StateMachine, data: u32, bits: u32) {
        let base = (sm.pin_ctrl >> 10) & 0x1F;
        if sm.exec_ctrl & EXECCTRL_SIDE_PINDIR != 0 {
            write_pin_field(&mut self.pin_dir, base, bits, data);
        } else {
            write_pin_field(&mut self.pin_out, base, bits, data);
        }
    }

    fn thresh(field: u32) -> u32 {
        if field == 0 {
            32
        } else {
            field
        }
    }

    fn pull_thresh(sm: &StateMachine) -> u32 {
        Self::thresh((sm.shift_ctrl >> 25) & 0x1F)
    }

    fn push_thresh(sm: &StateMachine) -> u32 {
        Self::thresh((sm.shift_ctrl >> 20) & 0x1F)
    }

    /// Output shift count: bits shifted out of the OSR since the last refill
    /// (`osr_count` holds the bits still in it).
    fn osr_shifted(sm: &StateMachine) -> u32 {
        32 - u32::from(sm.osr_count.min(32))
    }

    fn refill_osr(&mut self, i: usize, sm: &mut StateMachine) -> bool {
        if self.tx_fifo[i].is_empty() {
            return false;
        }
        sm.osr = self.tx_fifo[i].remove(0);
        sm.osr_count = 32;
        true
    }

    fn push_isr(&mut self, i: usize, sm: &mut StateMachine) -> bool {
        if self.rx_fifo[i].len() >= self.rx_depth(i) {
            return false;
        }
        self.rx_fifo[i].push(sm.isr);
        sm.isr = 0;
        sm.isr_count = 0;
        true
    }

    /// Execute one instruction on state machine `i` (whose live copy is
    /// `sm`). Returns `(pc_overridden, stalled)`. Side-set and delay are the
    /// caller's.
    fn execute(&mut self, i: usize, sm: &mut StateMachine, instr: u16) -> (bool, bool) {
        let opcode = (instr >> 13) & 0x7;
        let mut pc_overridden = false;
        let mut stalled = false;
        let pins_in = self.pins_in();
        let in_base = (sm.pin_ctrl >> 15) & 0x1F;
        let bit_count = |instr: u16| -> u32 {
            match u32::from(instr & 0x1F) {
                0 => 32,
                n => n,
            }
        };
        let low_bits = |v: u32, n: u32| if n >= 32 { v } else { v & ((1 << n) - 1) };

        match opcode {
            0 => {
                // JMP
                let cond = (instr >> 5) & 0x7;
                let target = instr & 0x1F;
                let should_jump = match cond {
                    0 => true,
                    1 => sm.x == 0,
                    2 => {
                        let tmp = sm.x != 0;
                        sm.x = sm.x.wrapping_sub(1);
                        tmp
                    }
                    3 => sm.y == 0,
                    4 => {
                        let tmp = sm.y != 0;
                        sm.y = sm.y.wrapping_sub(1);
                        tmp
                    }
                    5 => sm.x != sm.y,
                    6 => pins_in & (1 << ((sm.exec_ctrl >> 24) & 0x1F)) != 0,
                    // !OSRE: output shift count below the pull threshold.
                    _ => Self::osr_shifted(sm) < Self::pull_thresh(sm),
                };
                if should_jump {
                    sm.pc = target as u8;
                    pc_overridden = true;
                }
            }
            1 => {
                // WAIT
                let pol = (instr >> 7) & 0x1 != 0;
                let source = (instr >> 5) & 0x3;
                let index = u32::from(instr & 0x1F);

                let val = match source {
                    0 => pins_in & (1 << index) != 0,
                    1 => pins_in & (1 << ((in_base + index) & 31)) != 0,
                    2 => (self.irq & (1 << (index & 7))) != 0,
                    _ => false,
                };

                if val != pol {
                    stalled = true;
                } else if source == 2 && pol {
                    // `wait 1 irq` clears the flag it waited for.
                    self.irq &= !(1 << (index & 7));
                }
            }
            2 => {
                // IN
                let src = (instr >> 5) & 0x7;
                let count = bit_count(instr);
                let autopush = sm.shift_ctrl & SHIFTCTRL_AUTOPUSH != 0;
                let thresh = Self::push_thresh(sm);
                if autopush && u32::from(sm.isr_count) >= thresh && !self.push_isr(i, sm) {
                    stalled = true;
                } else {
                    let val = match src {
                        0 => pins_in.rotate_right(in_base),
                        1 => sm.x,
                        2 => sm.y,
                        6 => sm.isr,
                        7 => sm.osr,
                        _ => 0,
                    };
                    let data = low_bits(val, count);
                    sm.isr = if count == 32 {
                        data
                    } else if sm.shift_ctrl & SHIFTCTRL_IN_SHIFTDIR != 0 {
                        (sm.isr >> count) | (data << (32 - count))
                    } else {
                        (sm.isr << count) | data
                    };
                    sm.isr_count = (u32::from(sm.isr_count) + count).min(32) as u8;
                    if autopush && u32::from(sm.isr_count) >= thresh {
                        self.push_isr(i, sm);
                    }
                }
            }
            3 => {
                // OUT
                let dest = (instr >> 5) & 0x7;
                let count = bit_count(instr);
                let autopull = sm.shift_ctrl & SHIFTCTRL_AUTOPULL != 0;
                let thresh = Self::pull_thresh(sm);
                if autopull && Self::osr_shifted(sm) >= thresh && !self.refill_osr(i, sm) {
                    stalled = true;
                } else {
                    let val = if sm.shift_ctrl & SHIFTCTRL_OUT_SHIFTDIR != 0 {
                        let d = low_bits(sm.osr, count);
                        sm.osr = if count == 32 { 0 } else { sm.osr >> count };
                        d
                    } else {
                        let d = if count == 32 {
                            sm.osr
                        } else {
                            sm.osr >> (32 - count)
                        };
                        sm.osr = if count == 32 { 0 } else { sm.osr << count };
                        d
                    };
                    sm.osr_count = u32::from(sm.osr_count).saturating_sub(count) as u8;
                    let out_base = sm.pin_ctrl & 0x1F;
                    let out_count = (sm.pin_ctrl >> 20) & 0x3F;
                    match dest {
                        0 => write_pin_field(&mut self.pin_out, out_base, out_count, val),
                        1 => sm.x = val,
                        2 => sm.y = val,
                        4 => write_pin_field(&mut self.pin_dir, out_base, out_count, val),
                        5 => {
                            sm.pc = (val & 0x1F) as u8;
                            pc_overridden = true;
                        }
                        6 => {
                            sm.isr = val;
                            sm.isr_count = count as u8;
                        }
                        _ => {}
                    }
                    if autopull && Self::osr_shifted(sm) >= thresh {
                        self.refill_osr(i, sm);
                    }
                }
            }
            4 => {
                // PUSH / PULL
                let block = (instr >> 5) & 0x1 != 0;
                let if_cond = (instr >> 6) & 0x1 != 0;
                let is_pull = (instr >> 7) & 0x1 != 0;
                if is_pull {
                    if if_cond && Self::osr_shifted(sm) < Self::pull_thresh(sm) {
                        // `pull ifempty` with the OSR not yet empty: no-op.
                    } else if self.refill_osr(i, sm) {
                    } else if block {
                        stalled = true;
                    } else {
                        sm.osr = sm.x;
                        sm.osr_count = 32;
                    }
                } else if if_cond && u32::from(sm.isr_count) < Self::push_thresh(sm) {
                    // `push iffull` below the threshold: no-op.
                } else if !self.push_isr(i, sm) && block {
                    stalled = true;
                }
            }
            5 => {
                // MOV
                let dest = (instr >> 5) & 0x7;
                let op = (instr >> 3) & 0x3;
                let src = instr & 0x7;
                let mut val = match src {
                    0 => pins_in.rotate_right(in_base),
                    1 => sm.x,
                    2 => sm.y,
                    5 => {
                        // STATUS: all-ones while the selected FIFO level < N.
                        let n = (sm.exec_ctrl & 0xF) as usize;
                        let level = if sm.exec_ctrl & 0x10 != 0 {
                            self.rx_fifo[i].len()
                        } else {
                            self.tx_fifo[i].len()
                        };
                        if level < n {
                            u32::MAX
                        } else {
                            0
                        }
                    }
                    6 => sm.isr,
                    7 => sm.osr,
                    _ => 0,
                };
                match op {
                    1 => val = !val,
                    2 => val = val.reverse_bits(),
                    _ => {}
                }
                match dest {
                    0 => write_pin_field(
                        &mut self.pin_out,
                        sm.pin_ctrl & 0x1F,
                        (sm.pin_ctrl >> 20) & 0x3F,
                        val,
                    ),
                    1 => sm.x = val,
                    2 => sm.y = val,
                    4 => {
                        // EXEC (MOV only): not modelled.
                    }
                    5 => {
                        sm.pc = (val & 0x1F) as u8;
                        pc_overridden = true;
                    }
                    6 => {
                        sm.isr = val;
                        sm.isr_count = 0;
                    }
                    7 => {
                        sm.osr = val;
                        sm.osr_count = 32;
                    }
                    _ => {}
                }
            }
            6 => {
                // IRQ
                let set_clear = (instr >> 6) & 0x1;
                let index = instr & 0x7;
                if set_clear == 0 {
                    self.irq |= 1 << index;
                } else {
                    self.irq &= !(1 << index);
                }
            }
            _ => {
                // SET
                let dest = (instr >> 5) & 0x7;
                let data = u32::from(instr & 0x1F);
                let set_base = (sm.pin_ctrl >> 5) & 0x1F;
                let set_count = (sm.pin_ctrl >> 26) & 0x7;
                match dest {
                    0 => write_pin_field(&mut self.pin_out, set_base, set_count, data),
                    1 => sm.x = data,
                    2 => sm.y = data,
                    4 => write_pin_field(&mut self.pin_dir, set_base, set_count, data),
                    _ => {}
                }
            }
        }
        (pc_overridden, stalled)
    }

    /// `SMx_INSTR` write: run `instr` on state machine `i` right away
    /// (pico-sdk `pio_sm_exec`, which is how `pio_sm_set_consecutive_pindirs`
    /// and `pio_sm_set_pins` reach the pins). Delay is ignored.
    fn exec_now(&mut self, i: usize, instr: u16) {
        let mut sm = self.sm[i].clone();
        if let (Some((data, bits)), _) = Self::side_and_delay(&sm, instr) {
            self.apply_side_set(&sm, data, bits);
        }
        let _ = self.execute(i, &mut sm, instr);
        self.sm[i] = sm;
        self.publish();
    }

    /// The pin-output wire ([`LINE_NAMES`]); `arm` starts publishing on it,
    /// once the bus has bound a pad that can carry it.
    pub fn pin_lines(&mut self, arm: bool) -> Arc<PadLines> {
        let lines = self
            .lines
            .get_or_insert_with(|| {
                let lines = PadLines::new(&LINE_NAMES, &[false; 32]);
                for line in 0..32 {
                    lines.set_mode(line, LineDrive::Input);
                }
                Arc::new(lines)
            })
            .clone();
        if arm && !self.wired {
            self.wired = true;
            self.publish();
        }
        lines
    }

    /// Publish the pin outputs and enables onto the wire: an enabled output
    /// drives its pad push-pull, a disabled one leaves it to the outside.
    fn publish(&self) {
        let Some(lines) = self.lines.as_deref().filter(|_| self.wired) else {
            return;
        };
        for line in 0..32 {
            let mode = if self.pin_dir & (1 << line) != 0 {
                LineDrive::PushPull
            } else {
                LineDrive::Input
            };
            lines.drive_line(line, mode, self.pin_out & (1 << line) != 0);
        }
    }

    pub fn load_program_asm(&mut self, asm: &str) -> Result<()> {
        let programs = pio_parser::Parser::<32>::parse_file(asm)
            .map_err(|e| anyhow::anyhow!("PIO Parse Error: {:?}", e))?;

        // Extract instructions from the first program
        let (name, program) = programs
            .iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No PIO programs found in assembly"))?;

        tracing::debug!("Loading PIO program: {}", name);

        for (i, &instr) in program.program.code.iter().enumerate() {
            if i < 32 {
                self.instruction_mem[i] = instr;
            }
        }

        // Apply program settings to all state machines
        for i in 0..4 {
            self.sm[i].wrap_bottom = program.program.wrap.target;
            self.sm[i].wrap_top = program.program.wrap.source;

            // Update exec_ctrl with wrap settings
            // RP2040 EXEC_CTRL bits: 11:7 is wrap_bottom, 16:12 is wrap_top
            self.sm[i].exec_ctrl &= !(0x1F << 7);
            self.sm[i].exec_ctrl |= (program.program.wrap.target as u32) << 7;
            self.sm[i].exec_ctrl &= !(0x1F << 12);
            self.sm[i].exec_ctrl |= (program.program.wrap.source as u32) << 12;
        }

        Ok(())
    }

    pub fn read_reg(&self, addr: u64) -> u32 {
        let offset = addr & 0x1FF;
        match offset {
            0x000 => self.ctrl,
            0x004 => self.fstat_value(),
            0x008 => self.fdebug,
            0x00c => self.flevel_value(),
            0x020..=0x02c => {
                let idx = ((offset - 0x020) / 4) as usize;
                if idx < 4 && !self.rx_fifo[idx].is_empty() {
                    self.rx_fifo[idx][0]
                } else {
                    0
                }
            }
            0x030 => self.irq as u32,
            0x038 => self.input_sync_bypass,
            0x03c => self.pin_out,
            0x040 => self.pin_dir,
            // DBG_CFGINFO: IMEM_SIZE 32, SM_COUNT 4, FIFO_DEPTH 4.
            0x044 => 0x0020_0404,
            0x048..=0x0c4 => {
                let idx = ((offset - 0x048) / 4) as usize;
                if idx < 32 {
                    self.instruction_mem[idx] as u32
                } else {
                    0
                }
            }
            0x0c8..=0x124 => {
                let sm_idx = ((offset - 0x0c8) / 24) as usize;
                let reg_off = (offset - 0x0c8) % 24;
                if sm_idx < 4 {
                    let sm = &self.sm[sm_idx];
                    match reg_off {
                        0 => ((sm.clkdiv_int as u32) << 16) | ((sm.clkdiv_frac as u32) << 8),
                        4 => sm.exec_ctrl,
                        8 => sm.shift_ctrl,
                        12 => sm.pc as u32,
                        16 => self.instruction_mem[(sm.pc & 0x1F) as usize] as u32,
                        20 => sm.pin_ctrl,
                        _ => 0,
                    }
                } else {
                    0
                }
            }
            _ => {
                crate::census_reg!("pio:Pio", offset, "read");
                0
            }
        }
    }

    pub fn write_reg(&mut self, addr: u64, value: u32) {
        let offset = addr & 0x1FF;
        match offset {
            0x000 => {
                // SM_RESTART [7:4] and CLKDIV_RESTART [11:8] self-clear.
                self.ctrl = value & 0xF;
                for i in 0..4 {
                    let sm = &mut self.sm[i];
                    sm.enabled = (value & (1 << i)) != 0;
                    if value & (1 << (4 + i)) != 0 {
                        sm.isr = 0;
                        sm.isr_count = 0;
                        sm.osr_count = 0;
                        sm.delay_cycles = 0;
                        sm.stalled = false;
                    }
                    if value & (1 << (8 + i)) != 0 {
                        sm.clk_counter = 0;
                    }
                }
            }
            0x008 => self.fdebug &= !value, // W1C
            0x010..=0x01c => {
                let sm_idx = ((offset - 0x010) / 4) as usize;
                if self.tx_fifo[sm_idx].len() < self.tx_depth(sm_idx) {
                    self.tx_fifo[sm_idx].push(value);
                } else {
                    self.fdebug |= 1 << (16 + sm_idx); // TXOVER
                }
            }
            0x030 => self.irq &= !(value as u8), // W1C
            0x034 => self.irq |= value as u8,    // IRQ_FORCE
            0x038 => self.input_sync_bypass = value,
            0x048..=0x0c4 => {
                let idx = ((offset - 0x048) / 4) as usize;
                if idx < 32 {
                    self.instruction_mem[idx] = value as u16;
                }
            }
            0x0c8..=0x124 => {
                let sm_idx = ((offset - 0x0c8) / 24) as usize;
                let reg_off = (offset - 0x0c8) % 24;
                if sm_idx < 4 {
                    match reg_off {
                        0 => {
                            self.sm[sm_idx].clkdiv_int = (value >> 16) as u16;
                            self.sm[sm_idx].clkdiv_frac = (value >> 8) as u8;
                        }
                        4 => self.sm[sm_idx].exec_ctrl = value,
                        8 => self.sm[sm_idx].shift_ctrl = value,
                        12 => self.sm[sm_idx].pc = (value & 0x1F) as u8,
                        16 => self.exec_now(sm_idx, value as u16),
                        _ => self.sm[sm_idx].pin_ctrl = value,
                    }
                }
            }
            _ => {
                crate::census_reg!("pio:Pio", offset, "write");
            }
        }
    }
}

impl Peripheral for Pio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        // Start new accumulation if register changed
        if reg_offset != self.write_acc_offset {
            self.write_acc = 0;
            self.write_acc_mask = 0;
            self.write_acc_offset = reg_offset;
        }

        // Accumulate byte
        let mask = 0xFFu32 << (byte_offset * 8);
        self.write_acc = (self.write_acc & !mask) | ((value as u32) << (byte_offset * 8));
        self.write_acc_mask |= 1 << byte_offset;

        // Once all 4 bytes are written, commit the full 32-bit value
        if self.write_acc_mask == 0x0F {
            self.write_reg(reg_offset, self.write_acc);
            self.write_acc = 0;
            self.write_acc_mask = 0;
            self.write_acc_offset = 0xFFFF_FFFF;
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let (out_before, dir_before) = (self.pin_out, self.pin_dir);
        for i in 0..4 {
            let mut sm = self.sm[i].clone();
            if !sm.enabled {
                continue;
            }

            // Clock divider
            let div_int = if sm.clkdiv_int == 0 {
                65536
            } else {
                sm.clkdiv_int as u32
            };
            let div_total = (div_int << 8) | (sm.clkdiv_frac as u32);

            sm.clk_counter += 256;
            if sm.clk_counter < div_total {
                self.sm[i] = sm;
                continue;
            }
            sm.clk_counter -= div_total;

            if sm.delay_cycles > 0 {
                sm.delay_cycles -= 1;
                self.sm[i] = sm;
                continue;
            }

            let instr = self.instruction_mem[sm.pc as usize];
            let (side, delay) = Self::side_and_delay(&sm, instr);
            if let Some((data, bits)) = side {
                self.apply_side_set(&sm, data, bits);
            }
            let (pc_overridden, stalled) = self.execute(i, &mut sm, instr);

            // A stalled instruction re-issues next cycle; its delay only
            // starts once it completes.
            sm.stalled = stalled;
            if !stalled {
                sm.delay_cycles = delay;
            }
            if !pc_overridden && !stalled {
                let wrap_top = ((sm.exec_ctrl >> 12) & 0x1F) as u8;
                let wrap_bottom = ((sm.exec_ctrl >> 7) & 0x1F) as u8;
                if sm.pc == wrap_top {
                    sm.pc = wrap_bottom;
                } else {
                    sm.pc = (sm.pc + 1) & 0x1F;
                }
            }
            self.sm[i] = sm;
        }

        self.fstat = self.fstat_value();
        self.flevel = self.flevel_value();
        if (self.pin_out, self.pin_dir) != (out_before, dir_before) {
            self.publish();
        }

        PeripheralTickResult {
            irq: false,
            cycles: 0,
            ..Default::default()
        }
    }

    /// The pin-output wire ([`LINE_NAMES`]), through the same capability
    /// the RP2040 PWM publishes its A/B outputs with.
    fn timer_output_lines(&mut self, arm: bool) -> Option<Arc<PadLines>> {
        Some(self.pin_lines(arm))
    }

    fn wire_lines(&self) -> Option<&PadLines> {
        self.lines.as_deref()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    /// Event-driven on a bus-attached `event-scheduler` build so PIO does not
    /// pin the walk. Feature-off — or a hand-built bus that never handed over a
    /// cycle clock, and so has no scheduler drain — still runs `tick()` via the
    /// legacy walk. See [`Pio::scheduler_mode`].
    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        // `tick()` steps every enabled state machine, so the walk is real work
        // whenever the event chain is not the one driving it.
        !self.scheduler_mode()
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        if self.any_sm_enabled() && !self.chain_live {
            self.chain_live = true;
            vec![(0, EVT_SM_STEP)]
        } else {
            Vec::new()
        }
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != EVT_SM_STEP {
            return crate::sched::EventResult::default();
        }
        // Pin edges land at the cycle this step belongs to.
        if let Some(lines) = &self.lines {
            lines.set_stamp(Some(sched.now()));
        }
        let _ = self.tick();
        if let Some(lines) = &self.lines {
            lines.set_stamp(None);
        }
        // A parked chain stops; the next PIO write re-arms it (see
        // `all_enabled_sms_parked`). `LABWIRED_PIO_PARK=0` keeps it ticking.
        let active = self.any_sm_enabled() && !(park_enabled() && self.all_enabled_sms_parked());
        self.chain_live = active;
        crate::sched::EventResult {
            reschedule_delay: active.then_some(1),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state machine parked on `jmp <own pc>` stops the scheduler step
    /// chain; a write re-arms it, and a jump elsewhere or a slower clock
    /// divider keeps it stepping. tests/pio_parked_chain.rs (labwired-cli)
    /// checks that a parked chain reports the same run as a ticking one.
    #[test]
    fn a_parked_state_machine_stops_the_step_chain() {
        if !crate::cycle_clock::event_scheduler_enabled() {
            return;
        }
        let mut pio = Pio::new();
        pio.attach_cycle_clock(CycleClock::default());
        let mut sched = crate::sched::EventScheduler::new();
        let mut bus = crate::bus::SystemBus::new();
        let mut step = |pio: &mut Pio| {
            pio.on_event(EVT_SM_STEP, &mut sched, &mut bus)
                .reschedule_delay
        };

        pio.write_reg(0x000, 1); // CTRL: SM0 on, INSTR_MEM0 = 0 = `jmp 0`
        assert_eq!(pio.take_scheduled_events(), vec![(0, EVT_SM_STEP)]);
        assert_eq!(step(&mut pio), None, "`jmp 0` at pc 0 parks");

        pio.write_reg(0x048, 0x0001); // INSTR_MEM0 = `jmp 1`
        assert_eq!(
            pio.take_scheduled_events(),
            vec![(0, EVT_SM_STEP)],
            "a write re-arms a parked chain"
        );
        assert_eq!(step(&mut pio), Some(1), "`jmp 1` at pc 0 runs");
        assert_eq!(pio.sm[0].pc, 1);
        assert_eq!(
            step(&mut pio),
            Some(1),
            "INSTR_MEM1 = `jmp 0` is not a self-jump"
        );
        assert_eq!(pio.sm[0].pc, 0);

        pio.write_reg(0x048, 0x0000);
        pio.write_reg(0x0c8, 0x0002_0000); // SM0_CLKDIV = 2.0
        let _ = pio.take_scheduled_events();
        assert_eq!(step(&mut pio), Some(1), "a divided clock keeps stepping");

        pio.write_reg(0x0c8, 0x0001_0000);
        pio.write_reg(0x048, 0x0100); // `jmp 0 [1]`: a delay is pending each loop
        let _ = pio.take_scheduled_events();
        assert_eq!(step(&mut pio), Some(1), "a delayed jmp keeps stepping");
    }

    #[test]
    fn test_pio_set_jmp() {
        let mut pio = Pio::new();
        // SET X, 10 -> 0xE02A
        pio.instruction_mem[0] = 0xE02A;
        // JMP 0
        pio.instruction_mem[1] = 0x0000;

        pio.write_reg(0, 1); // Enable SM0

        pio.tick(); // Execute SET X, 10
        assert_eq!(pio.sm[0].x, 10);
        assert_eq!(pio.sm[0].pc, 1);

        pio.tick(); // Execute JMP 0
        assert_eq!(pio.sm[0].pc, 0);
    }

    #[test]
    fn test_pio_fifo() {
        let mut pio = Pio::new();
        // PULL blocking -> 0x80A0
        pio.instruction_mem[0] = 0x80A0;
        // SET X, 31 -> 0xE03F (Dest X=1, Data=31)
        pio.instruction_mem[1] = 0xE03F;
        // PUSH blocking -> 0x8020
        pio.instruction_mem[2] = 0x8020;

        pio.write_reg(0, 1); // Enable SM0

        // Execute PULL - should stall
        pio.tick();
        assert!(pio.sm[0].stalled);

        // Feed FIFO
        pio.write_reg(0x10, 123);

        pio.tick(); // Execute PULL
        assert_eq!(pio.sm[0].osr, 123);
        assert!(!pio.sm[0].stalled);
        assert_eq!(pio.sm[0].pc, 1);

        pio.tick(); // SET X, 31
        assert_eq!(pio.sm[0].x, 31);
        assert_eq!(pio.sm[0].pc, 2);

        pio.tick(); // PUSH
        assert_eq!(pio.rx_fifo[0][0], 0);
        assert_eq!(pio.sm[0].pc, 3);
    }

    #[test]
    fn test_pio_delay() {
        let mut pio = Pio::new();
        // SET X, 10 with Delay 2 -> 0xE02A | (2 << 8) = 0xE22A
        pio.instruction_mem[0] = 0xE22A;
        // SET Y, 5
        pio.instruction_mem[1] = 0xE045;

        pio.write_reg(0, 1); // Enable SM0

        pio.tick(); // Execute SET X, 10, set delay=2
        assert_eq!(pio.sm[0].x, 10);
        assert_eq!(pio.sm[0].pc, 1);
        assert_eq!(pio.sm[0].delay_cycles, 2);

        pio.tick(); // Delay 1
        assert_eq!(pio.sm[0].delay_cycles, 1);
        assert_eq!(pio.sm[0].pc, 1);

        pio.tick(); // Delay 2
        assert_eq!(pio.sm[0].delay_cycles, 0);
        assert_eq!(pio.sm[0].pc, 1);

        pio.tick(); // Execute SET Y, 5
        assert_eq!(pio.sm[0].y, 5);
        assert_eq!(pio.sm[0].pc, 2);
    }

    #[test]
    fn test_pio_assembly() {
        let mut pio = Pio::new();
        let asm = ".program test\nset x, 10\n";
        pio.load_program_asm(asm).unwrap();

        // 0xE02A is SET X, 10
        assert_eq!(pio.instruction_mem[0], 0xE02A);
    }

    #[test]
    fn test_pio_mov() {
        let mut pio = Pio::new();
        pio.sm[0].x = 0xAAAAAAAA;
        // MOV Y, X -> 0xA041
        pio.instruction_mem[0] = 0xA041;
        // MOV X, !X -> 0xA029 (Dest X=1, Op Not=1, Src X=1)
        pio.instruction_mem[1] = 0xA029;
        // MOV Y, ::X -> 0xA051 (Dest Y=2, Op Rev=2, Src X=1)
        pio.instruction_mem[2] = 0xA051;

        pio.write_reg(0, 1); // Enable SM0

        pio.tick();
        assert_eq!(pio.sm[0].y, 0xAAAAAAAA);
        assert_eq!(pio.sm[0].pc, 1);

        pio.tick();
        assert_eq!(pio.sm[0].x, 0x55555555);
        assert_eq!(pio.sm[0].pc, 2);

        pio.sm[0].x = 0x80000000;
        pio.tick();
        assert_eq!(pio.sm[0].y, 0x00000001);
    }

    #[test]
    fn test_pio_irq() {
        let mut pio = Pio::new();
        // IRQ set 7 -> 0xC007
        pio.instruction_mem[0] = 0xC007;
        // IRQ clear 7 -> 0xC047
        pio.instruction_mem[1] = 0xC047;

        pio.write_reg(0, 1); // Enable SM0

        pio.tick();
        assert_eq!(pio.irq, 0x80);

        pio.tick();
        assert_eq!(pio.irq, 0x00);
    }

    #[test]
    fn test_pio_wait_irq() {
        let mut pio = Pio::new();
        // WAIT 1, IRQ, 3 -> 0x20C3 (Pol=1, Src=IRQ=2, Idx=3)
        // 0x2000 (WAIT) | 0x80 (Pol=1) | 0x40 (Src=2) | 0x03 (Idx=3) = 0x20C3
        pio.instruction_mem[0] = 0x20C3;

        pio.write_reg(0, 1); // Enable SM0

        pio.tick();
        assert!(pio.sm[0].stalled);
        assert_eq!(pio.sm[0].pc, 0);

        pio.irq |= 1 << 3;
        pio.tick();
        assert!(!pio.sm[0].stalled);
        assert_eq!(pio.sm[0].pc, 1);
    }

    #[test]
    fn test_pio_wrap() {
        let mut pio = Pio::new();
        // SET X, 1 -> 0xE021
        pio.instruction_mem[0] = 0xE021;
        // SET Y, 2 -> 0xE042
        pio.instruction_mem[1] = 0xE042;

        // Wrap at PC=1 back to 0
        pio.sm[0].exec_ctrl = 1 << 12;
        pio.write_reg(0, 1); // Enable SM0

        pio.tick(); // Execute PC=0
        assert_eq!(pio.sm[0].pc, 1);

        pio.tick(); // Execute PC=1, should wrap
        assert_eq!(pio.sm[0].pc, 0);
    }

    #[test]
    fn test_pio_in_out_full() {
        let mut pio = Pio::new();
        pio.sm[0].x = 0x12345678;
        // IN X, 16 -> 0x4030 (0x4000 | Src=1(0x20) | Count=16(0x10)) = 0x4030
        pio.instruction_mem[0] = 0x4030;
        // PUSH blocking -> 0x8020
        pio.instruction_mem[1] = 0x8020;
        // OUT Y, 32 -> 0x6040
        pio.instruction_mem[2] = 0x6040;

        pio.write_reg(0, 1); // Enable SM0

        pio.tick(); // IN X, 16
        assert_eq!(pio.sm[0].isr, 0x56780000); // 16 LSBs of X (0x5678) shifted into ISR MSB
        assert_eq!(pio.sm[0].isr_count, 16);

        pio.tick(); // PUSH
        assert_eq!(pio.rx_fifo[0][0], 0x56780000);

        pio.write_reg(0x10, 0xDEADBEEF); // TX FIFO
        pio.sm[0].osr = 0xDEADBEEF;
        pio.sm[0].osr_count = 32;

        pio.tick(); // OUT Y, 32
        assert_eq!(pio.sm[0].y, 0xDEADBEEF);
        assert_eq!(pio.sm[0].osr_count, 0);
    }

    /// `.side_set 1 opt`: SIDESET_COUNT 2 counts the enable bit, so a
    /// side-set only happens on instructions that set it, and the delay
    /// field shrinks to 3 bits.
    #[test]
    fn optional_side_set_only_drives_when_enabled() {
        let mut pio = Pio::new();
        pio.sm[0].pin_ctrl = (2 << 29) | (5 << 10); // SIDESET_COUNT 2, base GP5
        pio.sm[0].exec_ctrl |= EXECCTRL_SIDE_EN;
        pio.pin_dir = 1 << 5;
        // `nop side 1 [3]`: enable bit 12, data bit 11, delay 3.
        pio.instruction_mem[0] = 0xa042 | (1 << 12) | (1 << 11) | (3 << 8);
        // `nop [7]` with no side-set: the pin keeps its level.
        pio.instruction_mem[1] = 0xa042 | (7 << 8);
        pio.write_reg(0, 1);
        pio.tick();
        assert_eq!(pio.pin_out, 1 << 5);
        assert_eq!(pio.sm[0].delay_cycles, 3);
        for _ in 0..4 {
            pio.tick();
        }
        assert_eq!(pio.sm[0].pc, 2);
        assert_eq!(pio.sm[0].delay_cycles, 7, "3 delay bits, not 5");
        assert_eq!(pio.pin_out, 1 << 5);
    }

    /// OUT shifting left with autopull at 24 bits: the MSBs of the FIFO word
    /// come out first, and the 25th OUT stalls on an empty FIFO.
    #[test]
    fn out_left_autopull_takes_the_top_24_bits_then_stalls() {
        let mut pio = Pio::new();
        pio.sm[0].shift_ctrl = SHIFTCTRL_AUTOPULL | (24 << 25); // OUT_SHIFTDIR 0
        pio.instruction_mem[0] = 0x6021; // out x, 1
        pio.write_reg(0, 1);
        pio.write_reg(0x010, 0xa500_0000 | 0xff);
        let mut bits = 0u32;
        for _ in 0..24 {
            pio.tick();
            assert!(!pio.sm[0].stalled);
            bits = (bits << 1) | pio.sm[0].x;
            pio.sm[0].pc = 0;
        }
        assert_eq!(
            bits, 0xa5_0000,
            "bits 31..8, MSB first; the low byte never shifts out"
        );
        pio.tick();
        assert!(pio.sm[0].stalled, "the 25th OUT waits for the FIFO");
        pio.write_reg(0x010, 0x8000_0000);
        pio.tick();
        assert!(!pio.sm[0].stalled);
        assert_eq!(pio.sm[0].x, 1);
    }

    /// `jmp !osre` jumps while bits remain below the pull threshold.
    #[test]
    fn jmp_not_osre_follows_the_pull_threshold() {
        let mut pio = Pio::new();
        pio.sm[0].shift_ctrl = SHIFTCTRL_RESET | (8 << 25); // PULL_THRESH 8
        pio.sm[0].osr_count = 32; // freshly pulled
        pio.instruction_mem[0] = 0x00e5; // jmp !osre 5
        pio.write_reg(0, 1);
        pio.tick();
        assert_eq!(pio.sm[0].pc, 5, "0 of 8 bits shifted: not empty");
        pio.sm[0].pc = 0;
        pio.sm[0].osr_count = 24; // 8 shifted out
        pio.tick();
        assert_eq!(pio.sm[0].pc, 1, "8 of 8 shifted: empty, falls through");
    }

    /// FJOIN_TX gives the TX FIFO all 8 entries, and FSTAT reports TXFULL
    /// as soon as the write lands (no SM step in between).
    #[test]
    fn joined_tx_fifo_holds_eight_and_fstat_is_live() {
        let mut pio = Pio::new();
        pio.write_reg(0x0d0, SHIFTCTRL_RESET | SHIFTCTRL_FJOIN_TX);
        for n in 0..8 {
            assert_eq!(pio.read_reg(0x004) & (1 << 16), 0, "not full at {n}");
            pio.write_reg(0x010, n);
        }
        assert_ne!(pio.read_reg(0x004) & (1 << 16), 0, "TXFULL after 8");
        assert_eq!(pio.read_reg(0x00c) & 0xF, 8, "FLEVEL TX0");
        pio.write_reg(0x010, 99);
        assert_ne!(pio.fdebug & (1 << 16), 0, "the 9th write overflows");
    }

    /// SM_RESTART clears the shift counters and a pending delay; the PC,
    /// X and Y survive.
    #[test]
    fn sm_restart_clears_shift_state_only() {
        let mut pio = Pio::new();
        pio.sm[1].x = 7;
        pio.sm[1].pc = 3;
        pio.sm[1].isr_count = 12;
        pio.sm[1].osr_count = 20;
        pio.sm[1].delay_cycles = 4;
        pio.write_reg(0x000, 1 << 5);
        assert_eq!(pio.read_reg(0x000), 0, "restart bits self-clear");
        let sm = &pio.sm[1];
        assert_eq!((sm.isr_count, sm.osr_count, sm.delay_cycles), (0, 0, 0));
        assert_eq!((sm.x, sm.pc), (7, 3));
    }
}
