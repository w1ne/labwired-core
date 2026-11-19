// SPDX-License-Identifier: MIT
// Portable port of the Pomodoro Pal application logic.
//
// Ported from `firmware/pomodoro.c` of HeyPCB/pomodoro-pal at revision
// 1d6e50ff9998978e6f4521895042ae240fff7c42 (git blob
// 0ece2a82608ad5925bbc9c03fa8efd83903af878), MIT License,
// Copyright (c) 2026 HeyPCB. The state machine, the durations, the ring and
// breathing rules, the tone timing and the tap/hold rules are those of the
// original. The CH32V003 register access is replaced: pins are Flex ports,
// the SysTick millisecond count is virtual time, and the capacitive touch
// measurement is one Boolean read. This is not the original CH32V003 binary.
#![no_std]

#[link(wasm_import_module = "labwired")]
extern "C" {
    fn read(port: u32) -> f64;
    fn write(port: u32, value: f64);
    fn advance_ns(delta: u32);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

const FOCUS_MS: u32 = 25 * 60 * 1000;
const BREAK_MS: u32 = 5 * 60 * 1000;
const LONG_BREAK_MS: u32 = 15 * 60 * 1000;
const HOLD_RESET_MS: u32 = 2000;
const TOUCH_EVERY_MS: u32 = 20;

/// One soft-PWM phase step of the original: 1024 counts of the 48 MHz SysTick
/// counter, rounded down to whole nanoseconds. One `loop` call is one step.
const PHASE_STEP_NS: u32 = 21_333;

const IDLE: u8 = 0;
const FOCUS: u8 = 1;
const BREAK: u8 = 2;

struct Pal {
    ring: [u32; 12],
    leaf: u32,
    buzz: u32,
    touch: u32,
    now_ns: u64,
    state: u8,
    paused: bool,
    sessions: u8,
    phase_ms: u32,
    phase_len: u32,
    last: u32,
    last_touch: u32,
    held: u32,
    was_touched: bool,
}

static mut PAL: Pal = Pal {
    ring: [0; 12],
    leaf: 0,
    buzz: 0,
    touch: 0,
    now_ns: 0,
    state: IDLE,
    paused: false,
    sessions: 0,
    phase_ms: 0,
    phase_len: 0,
    last: 0,
    last_touch: 0,
    held: 0,
    was_touched: false,
};

impl Pal {
    fn ms(&self) -> u32 {
        (self.now_ns / 1_000_000) as u32
    }

    fn delay_ns(&mut self, delta: u32) {
        unsafe { advance_ns(delta) };
        self.now_ns += u64::from(delta);
    }

    fn pin(&self, port: u32, on: bool) {
        unsafe { write(port, if on { 1.0 } else { 0.0 }) };
    }

    /// The piezo: a square wave from the pin, blocking.
    fn tone(&mut self, hz: u32, dur_ms: u32) {
        let half_us = 500_000 / hz;
        let cycles = hz * dur_ms / 1000;
        for _ in 0..cycles {
            self.pin(self.buzz, true);
            self.delay_ns(half_us * 1000);
            self.pin(self.buzz, false);
            self.delay_ns(half_us * 1000);
        }
    }

    fn chirp(&mut self, notes: usize) {
        const UP: [u32; 3] = [2637, 3136, 3951];
        for i in 0..notes {
            self.tone(UP[i % 3], 90);
            self.delay_ns(60_000_000);
        }
    }

    fn touched(&self) -> bool {
        unsafe { read(self.touch) != 0.0 }
    }

    /// A soft-PWM phase, 0..31: about 1.5 kHz.
    fn pwm_phase(&self) -> u8 {
        ((self.now_ns / u64::from(PHASE_STEP_NS)) & 31) as u8
    }

    /// The ring empties clockwise from 12 o'clock: the `lit` LEDs still on are
    /// indices 12-lit .. 11, and `breathing_idx` breathes at `level` of 32.
    fn ring_show(&self, lit: i32, breathing_idx: i32, level: u8) {
        let pwm = self.pwm_phase();
        for k in 0..12i32 {
            let mut on = k >= 12 - lit;
            if k == breathing_idx {
                on = pwm < level;
            }
            self.pin(self.ring[k as usize], on);
        }
    }

    fn breath(&self, period_ms: u32) -> u8 {
        let t = self.ms() % period_ms;
        let half = period_ms / 2;
        let v = if t < half { t } else { period_ms - t };
        (2 + (v * 30) / half) as u8
    }

    fn start(&mut self, state: u8) {
        self.state = state;
        self.paused = false;
        self.phase_ms = 0;
        if state == FOCUS {
            self.phase_len = FOCUS_MS;
        } else if state == BREAK {
            self.phase_len = if self.sessions % 4 == 0 { LONG_BREAK_MS } else { BREAK_MS };
        }
    }

    /// One pass of the original main loop, then one PWM phase step of time.
    fn pass(&mut self) {
        let now = self.ms();
        let dt = now - self.last;
        self.last = now;
        if !self.paused && self.state != IDLE {
            self.phase_ms += dt;
        }

        // touch: a tap toggles, a 2 s hold resets
        if now - self.last_touch >= TOUCH_EVERY_MS {
            self.last_touch = now;
            let t = self.touched();
            if t {
                self.held += TOUCH_EVERY_MS;
                if self.held >= HOLD_RESET_MS && self.held < HOLD_RESET_MS + TOUCH_EVERY_MS {
                    self.sessions = 0;
                    self.start(IDLE);
                    self.pin(self.leaf, false);
                    self.chirp(1);
                }
            } else {
                if self.was_touched && self.held < HOLD_RESET_MS {
                    if self.state == IDLE {
                        self.start(FOCUS);
                    } else {
                        self.paused = !self.paused;
                    }
                }
                self.held = 0;
            }
            self.was_touched = t;
        }

        // the phase clock
        if self.state != IDLE && self.phase_ms >= self.phase_len {
            if self.state == FOCUS {
                self.sessions += 1;
                self.ring_show(0, -1, 0);
                self.chirp(3);
                self.start(BREAK);
            } else {
                self.pin(self.leaf, false);
                self.chirp(2);
                self.start(IDLE);
            }
        }

        // what the face shows
        if self.state == IDLE {
            self.ring_show(0, 0, self.breath(3000));
            self.pin(self.leaf, false);
        } else {
            let step = self.phase_len / 12;
            let lit = (12 - (self.phase_ms / step) as i32).max(1);
            if self.paused {
                self.ring_show(if (now / 600) & 1 != 0 { lit } else { 0 }, -1, 0);
            } else {
                self.ring_show(lit, 12 - lit, self.breath(1600));
            }
            if self.state == BREAK {
                self.pin(self.leaf, self.pwm_phase() < self.breath(4000));
            }
        }
        self.delay_ns(PHASE_STEP_NS);
    }
}

/// Bind the ports and run the original start-up: all outputs low, one chirp.
/// `first_ring` is the port of minute LED 0; LEDs 1..11 follow it in order.
#[no_mangle]
pub extern "C" fn setup(first_ring: u32, leaf: u32, buzz: u32, touch: u32) {
    let pal = unsafe { &mut *core::ptr::addr_of_mut!(PAL) };
    for k in 0..12 {
        pal.ring[k] = first_ring + k as u32;
        pal.pin(pal.ring[k], false);
    }
    pal.leaf = leaf;
    pal.buzz = buzz;
    pal.touch = touch;
    pal.pin(leaf, false);
    pal.pin(buzz, false);
    pal.chirp(1);
    pal.last = pal.ms();
}

/// One main-loop pass. Returns `state + 4 * paused + 16 * sessions`, where
/// state is 0 idle, 1 focus, 2 break.
#[export_name = "loop"]
pub extern "C" fn main_loop_pass() -> u32 {
    let pal = unsafe { &mut *core::ptr::addr_of_mut!(PAL) };
    pal.pass();
    u32::from(pal.state) + 4 * u32::from(pal.paused) + 16 * u32::from(pal.sessions)
}
