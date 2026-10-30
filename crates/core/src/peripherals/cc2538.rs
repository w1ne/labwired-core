// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! TI CC2538 peripherals: the PL061-style GPIO port and the SYS_CTRL block.
//! The RF core, the sleep timer and the flash controller live in [`rfcore`],
//! [`sleep_timer`] and [`flash_ctrl`].
//!
//! Source: *CC2538 System-on-Chip Solution for 2.4-GHz IEEE 802.15.4 and
//! ZigBee/ZigBee IP Applications, User's Guide* (SWRU319C, April 2012, revised
//! May 2013).
//!
//! **GPIO** (§9.2, Table 9-2 "GPIO Common Registers Mapping Summary"). One
//! instance per 8-pin port A–D. `GPIO_DATA` is address-masked: offset bits
//! \[9:2\] select which data bits a read returns or a write alters, so the
//! register spans the window `0x000..=0x3FC` (§9.2.2.1.2, Figures 9-2/9-3).
//! `GPIO_DIR` @ `0x400` picks output pins; `GPIO_AFSEL` @ `0x420` hands a pin
//! to its peripheral (the IOC selects which one).
//!
//! Modelled: masked DATA read/write, DIR, AFSEL, the pad level each pin
//! presents (latch when driven, outside world or reset pull-up otherwise), and
//! storage-only read-back of the interrupt-configuration, lock/commit and
//! power-up-interrupt registers at their Table 9-2 reset values.
//! NOT modelled: interrupt detection (`GPIO_RIS`/`GPIO_MIS` stay 0, no IRQ is
//! raised), the `GPIOLOCK`/`GPIOCR` commit protection of AFSEL (AFSEL writes
//! always land — the reset state of `GPIOCR` is all ones, §9.2.2.4, so this is
//! what silicon does until firmware locks it), the peripheral side of an AFSEL
//! pin (an AFSEL pin is not driven by this model), and `IOC_Pxx_OVER` pulls:
//! the IOC is a separate block, so the pull each undriven input follows is
//! the reset state (pull-up on every pad except the PC0–PC3 high-drive cells,
//! §9.2.3 CAUTION and the §9.2.2.5 NOTE), supplied by the chip descriptor as
//! `config.pull_up_mask`.
//!
//! **SYS_CTRL** (§7.7.1, Table 7-3 "SYS_CTRL Register Summary"). A register
//! file at the documented reset values. `CLOCK_STA` follows `CLOCK_CTRL`:
//! the oscillator selection and the SYS_DIV/IO_DIV dividers switch at once
//! (`SOURCE_CHANGE` never reads 1), and a divider faster than the selected
//! oscillator allows is clamped ("Cannot be higher than OSC setting").
//! `CLOCK_STA.SYNC_32K` (bit 26) is the 32.768-kHz clock as the system clock
//! samples it: a square wave derived from the bus cycle count at the selected
//! SYS_DIV rate, so a firmware waiting out one 32-kHz edge (Contiki-NG
//! `sys_ctrl_init`) sees it. Without an attached cycle clock it reads 0.
//! NOT modelled: oscillator start-up timing, the 32-kHz source's own accuracy
//! (RCOSC vs crystal: both are exactly 32 768 Hz here),
//! the clock gates' effect on other blocks (RCGC/SCGC/DCGC are stored and read
//! back but no peripheral is gated by them), software reset (`SRCRC`), the
//! power-mode controller (`PMCTL`) and the alternate interrupt map (`I_MAP`).

use crate::pins::{External, InputChange, PadDriver, PinPort, Pull};
use crate::{Peripheral, SimResult};

const PINS: usize = 8;
const PIN_MASK: u32 = 0xFF;

// ── GPIO register map (SWRU319C Table 9-2) ──────────────────────────────────
/// Last word of the address-masked `GPIO_DATA` window (`0x000..=0x3FC`).
const GPIO_DATA_END: u64 = 0x3FC;
const GPIO_DIR: u64 = 0x400;
const GPIO_IS: u64 = 0x404;
const GPIO_IBE: u64 = 0x408;
const GPIO_IEV: u64 = 0x40C;
const GPIO_IE: u64 = 0x410;
const GPIO_RIS: u64 = 0x414;
const GPIO_MIS: u64 = 0x418;
const GPIO_IC: u64 = 0x41C;
const GPIO_AFSEL: u64 = 0x420;
const GPIO_GPIOLOCK: u64 = 0x520;
const GPIO_GPIOCR: u64 = 0x524;
const GPIO_PMUX: u64 = 0x700;
const GPIO_P_EDGE_CTRL: u64 = 0x704;
const GPIO_PI_IEN: u64 = 0x710;
const GPIO_IRQ_DETECT_ACK: u64 = 0x718;
const GPIO_USB_IRQ_ACK: u64 = 0x71C;
const GPIO_IRQ_DETECT_UNMASK: u64 = 0x720;

/// Table 9-2 reset values that are not zero.
const GPIOLOCK_RESET: u32 = 0x0000_0001;
const GPIOCR_RESET: u32 = 0x0000_00FF;

fn bit(word: u32, index: usize) -> bool {
    word & (1 << index) != 0
}

/// One CC2538 GPIO port (PL061-style, 8 pins).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cc2538Gpio {
    /// The `GPIO_DATA` output latch.
    data: u32,
    /// `GPIO_DIR`: 1 = output.
    dir: u32,
    /// `GPIO_AFSEL`: 1 = pin controlled by its peripheral.
    afsel: u32,
    /// Interrupt configuration, stored only (no detection is modelled).
    is: u32,
    ibe: u32,
    iev: u32,
    ie: u32,
    gpiolock: u32,
    gpiocr: u32,
    pmux: u32,
    p_edge_ctrl: u32,
    pi_ien: u32,
    irq_detect_ack: u32,
    usb_irq_ack: u32,
    irq_detect_unmask: u32,
    /// Pads with the reset `IOC_Pxx_OVER` pull-up (descriptor-supplied; the
    /// IOC itself is not modelled, see the module docs).
    pull_up_mask: u32,
    /// Levels presented on the pads by the outside world.
    #[serde(default)]
    external: [Option<bool>; PINS],
}

impl Cc2538Gpio {
    /// A port whose undriven pads all carry the reset pull-up.
    pub fn new() -> Self {
        Self::with_pull_up_mask(PIN_MASK)
    }

    /// A port whose undriven pads in `mask` carry the reset pull-up. Port C
    /// passes `0xF0`: PC0–PC3 are high-drive cells with no on-die pulls
    /// (SWRU319C §9.2.2.5 NOTE).
    pub fn with_pull_up_mask(mask: u32) -> Self {
        Self {
            data: 0,
            dir: 0,
            afsel: 0,
            is: 0,
            ibe: 0,
            iev: 0,
            ie: 0,
            gpiolock: GPIOLOCK_RESET,
            gpiocr: GPIOCR_RESET,
            pmux: 0,
            p_edge_ctrl: 0,
            pi_ien: 0,
            irq_detect_ack: 0,
            usb_irq_ack: 0,
            irq_detect_unmask: 0,
            pull_up_mask: mask & PIN_MASK,
            external: [None; PINS],
        }
    }

    fn drives(&self, pin: usize) -> bool {
        bit(self.dir, pin) && !bit(self.afsel, pin)
    }

    /// The level on `pin`: the latch when software drives it, otherwise the
    /// outside world, otherwise the reset pull (an unpulled pad reads 0).
    fn pad_level(&self, pin: usize) -> bool {
        if self.drives(pin) {
            bit(self.data, pin)
        } else if let Some(level) = self.external[pin] {
            level
        } else {
            bit(self.pull_up_mask, pin)
        }
    }

    fn pad_word(&self) -> u32 {
        (0..PINS)
            .filter(|&p| self.pad_level(p))
            .fold(0u32, |word, p| word | (1 << p))
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            // Address-masked read: bits whose address line is clear read 0.
            0..=GPIO_DATA_END => self.pad_word() & ((offset as u32 >> 2) & PIN_MASK),
            GPIO_DIR => self.dir,
            GPIO_IS => self.is,
            GPIO_IBE => self.ibe,
            GPIO_IEV => self.iev,
            GPIO_IE => self.ie,
            GPIO_RIS | GPIO_MIS | GPIO_IC => 0,
            GPIO_AFSEL => self.afsel,
            GPIO_GPIOLOCK => self.gpiolock,
            GPIO_GPIOCR => self.gpiocr,
            GPIO_PMUX => self.pmux,
            GPIO_P_EDGE_CTRL => self.p_edge_ctrl,
            GPIO_PI_IEN => self.pi_ien,
            GPIO_IRQ_DETECT_ACK => self.irq_detect_ack,
            GPIO_USB_IRQ_ACK => self.usb_irq_ack,
            GPIO_IRQ_DETECT_UNMASK => self.irq_detect_unmask,
            _ => {
                crate::census_reg!("cc2538:Cc2538Gpio", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // Address-masked write: only bits whose address line is set change.
            0..=GPIO_DATA_END => {
                let mask = (offset as u32 >> 2) & PIN_MASK;
                self.data = (self.data & !mask) | (value & mask);
            }
            GPIO_DIR => self.dir = value & PIN_MASK,
            GPIO_IS => self.is = value & PIN_MASK,
            GPIO_IBE => self.ibe = value & PIN_MASK,
            GPIO_IEV => self.iev = value & PIN_MASK,
            GPIO_IE => self.ie = value & PIN_MASK,
            // RIS/MIS are read-only; IC clears latched edges, none are latched.
            GPIO_RIS | GPIO_MIS | GPIO_IC => {}
            GPIO_AFSEL => self.afsel = value & PIN_MASK,
            GPIO_GPIOLOCK => self.gpiolock = value,
            GPIO_GPIOCR => self.gpiocr = value & PIN_MASK,
            GPIO_PMUX => self.pmux = value,
            GPIO_P_EDGE_CTRL => self.p_edge_ctrl = value,
            GPIO_PI_IEN => self.pi_ien = value,
            GPIO_IRQ_DETECT_ACK => self.irq_detect_ack = value,
            GPIO_USB_IRQ_ACK => self.usb_irq_ack = value,
            GPIO_IRQ_DETECT_UNMASK => self.irq_detect_unmask = value,
            _ => {
                crate::census_reg!("cc2538:Cc2538Gpio", offset, "write");
            }
        }
    }
}

impl Default for Cc2538Gpio {
    fn default() -> Self {
        Self::new()
    }
}

/// Byte-lane read of a 32-bit register file.
fn lane_read(word: u32, offset: u64) -> u8 {
    ((word >> ((offset % 4) as u32 * 8)) & 0xFF) as u8
}

/// Byte-lane read-modify-write of a 32-bit register file.
fn lane_merge(word: u32, offset: u64, value: u8) -> u32 {
    let shift = (offset % 4) as u32 * 8;
    (word & !(0xFF << shift)) | (u32::from(value) << shift)
}

/// `keep [field, ...]` names `#[serde(skip)]` fields (the bus cycle clock)
/// that a restore must carry over instead of resetting to `None`.
macro_rules! snapshot_impl {
    ($ty:ty, $name:literal) => {
        snapshot_impl!($ty, $name, keep []);
    };
    ($ty:ty, $name:literal, keep [$($keep:ident),*]) => {
        fn snapshot(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
        }

        fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
            #[allow(unused_mut)]
            let mut restored: $ty = serde_json::from_value(state).map_err(|error| {
                crate::SimulationError::NotImplemented(format!(
                    concat!($name, " snapshot decode: {}"),
                    error
                ))
            })?;
            $(restored.$keep = self.$keep.take();)*
            *self = restored;
            Ok(())
        }

        fn runtime_snapshot(&self) -> Vec<u8> {
            bincode::serialize(self).expect(concat!("bincode serialize ", $name))
        }

        fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
            if bytes.is_empty() {
                return Ok(());
            }
            #[allow(unused_mut)]
            let mut restored: $ty = bincode::deserialize(bytes).map_err(|error| {
                crate::SimulationError::NotImplemented(format!(
                    concat!($name, " runtime snapshot decode: {}"),
                    error
                ))
            })?;
            $(restored.$keep = self.$keep.take();)*
            *self = restored;
            Ok(())
        }
    };
}

pub mod flash_ctrl;
pub mod rfcore;
pub mod sleep_timer;

impl Peripheral for Cc2538Gpio {
    /// A pure register bank: no `tick()`, no IRQ, DMA or event from the walk
    /// (pad levels are recomputed on access), so deleting the legacy walk
    /// cannot change what it does.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane_read(self.read_reg(offset & !3), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        if reg <= GPIO_DATA_END {
            // DATA holds bits 7:0 only, so a byte store to lane 0 is the
            // whole register; the upper lanes are reserved. A read-modify-
            // write here would fold pad levels back into the latch.
            if offset % 4 == 0 {
                self.write_reg(reg, u32::from(value));
            }
            return Ok(());
        }
        let merged = lane_merge(self.read_reg(reg), offset, value);
        self.write_reg(reg, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    /// A node reset clears the register state; the outside world stays
    /// connected (a held button is not register state) and so does the
    /// descriptor's pull map.
    fn on_node_reset(&mut self) {
        let external = self.external;
        *self = Self::with_pull_up_mask(self.pull_up_mask);
        self.external = external;
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.pad_level(pin))
    }

    fn read_gpio_input_word(&self) -> u32 {
        self.pad_word()
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.drives(pin) && bit(self.data, pin))
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.drives(pin))
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    snapshot_impl!(Cc2538Gpio, "Cc2538Gpio");
}

impl PinPort for Cc2538Gpio {
    fn pin_count(&self) -> u8 {
        PINS as u8
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        let pin = pin as usize;
        if pin >= PINS {
            return None;
        }
        Some(if self.drives(pin) {
            PadDriver::drive(bit(self.data, pin))
        } else if bit(self.pull_up_mask, pin) {
            PadDriver::released(Pull::Up)
        } else {
            PadDriver::OFF
        })
    }

    fn external(&self, pin: u8) -> External {
        match self.external.get(pin as usize).copied().flatten() {
            Some(level) => External::Level(level),
            None => External::Released,
        }
    }

    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        let pin = pin as usize;
        if pin >= PINS {
            return None;
        }
        let before = self.pad_level(pin);
        self.external[pin] = ext.level();
        let after = self.pad_level(pin);
        Some(InputChange { before, after })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.pad_level(pin))
    }
}

// ── SYS_CTRL register map (SWRU319C Table 7-3) ──────────────────────────────
const SYS_CTRL_CLOCK_CTRL: u64 = 0x00;
const SYS_CTRL_CLOCK_STA: u64 = 0x04;
const SYS_CTRL_RCGCGPT: u64 = 0x08;
const SYS_CTRL_RCGCUART: u64 = 0x28;
#[cfg(test)]
const SYS_CTRL_PMCTL: u64 = 0x58;
const SYS_CTRL_SRCRC: u64 = 0x5C;
const SYS_CTRL_PWRDBG: u64 = 0x74;
const SYS_CTRL_CLD: u64 = 0x80;
const SYS_CTRL_IWE: u64 = 0x94;
const SYS_CTRL_I_MAP: u64 = 0x98;
const SYS_CTRL_RCGCRFC: u64 = 0xA8;
const SYS_CTRL_EMUOVR: u64 = 0xB4;

/// Table 7-3 reset values.
const CLOCK_CTRL_RESET: u32 = 0x0103_0109;
const CLOCK_STA_RESET: u32 = 0x0101_0109;
const PWRDBG_RESET: u32 = 0x0000_0004;
const CLD_RESET: u32 = 0x0000_0100;
const IWE_RESET: u32 = 0x0000_003F;
const EMUOVR_RESET: u32 = 0x0000_00FF;

/// `CLOCK_CTRL` writable fields (§7.7.1.2): OSC32K_CALDIS(25), OSC32K(24),
/// AMP_DET(21), OSC_PD(17), OSC(16), IO_DIV(10:8), bit 5 and bits 4:3
/// (reserved RW), SYS_DIV(2:0).
const CLOCK_CTRL_RW: u32 = (1 << 25) | (1 << 24) | (1 << 21) | (1 << 17) | (1 << 16) | 0x73F;
const CLK_OSC32K_CALDIS: u32 = 1 << 25;
const CLK_OSC32K: u32 = 1 << 24;
/// `OSC`: 0 = 32-MHz crystal, 1 = 16-MHz HF-RC.
const CLK_OSC: u32 = 1 << 16;
/// `CLOCK_STA.XOSC_STB` (bit 19).
const STA_XOSC_STB: u32 = 1 << 19;
/// `CLOCK_STA.SYNC_32K` (bit 26): the 32-kHz clock synced to the system clock.
const STA_SYNC_32K: u32 = 1 << 26;
/// The 32-kHz clock (§7.2.3: 32.768-kHz crystal or 32-kHz RCOSC).
const CLK_32K_HZ: u64 = 32_768;
/// The 32-MHz crystal; SYS clock = this / 2^SYS_DIV (SYS_DIV >= 1 on HF-RC).
const XOSC_HZ: u64 = 32_000_000;
const DIV_MASK: u32 = 0x7;
const IO_DIV_SHIFT: u32 = 8;
/// `CLOCK_STA` bits that never come from `CLOCK_CTRL` in this model: the
/// reset value's low byte bit 3, kept verbatim from Table 7-3 (the bit-field
/// table calls 7:3 reserved; the summary's `0x...0109` sets bit 3).
const STA_RESET_RESIDUE: u32 = CLOCK_STA_RESET & (1 << 3);

/// Gate / software-reset banks whose documented fields are narrower than the
/// register: (offset, field mask). RCGCGPT/SCGCGPT/DCGCGPT/SRGPT: GPT3:0;
/// RCGCUART/SCGCUART/DCGCUART/SRUART: UART1:0 (§7.7.1.2).
const NARROW_FIELDS: &[(u64, u32)] = &[
    (SYS_CTRL_RCGCGPT, 0xF),
    (0x0C, 0xF),
    (0x10, 0xF),
    (0x14, 0xF),
    (SYS_CTRL_RCGCUART, 0x3),
    (0x2C, 0x3),
    (0x30, 0x3),
    (0x34, 0x3),
];

/// Number of 32-bit words in the documented register span `0x00..=0xB4`.
const SYS_CTRL_WORDS: usize = (SYS_CTRL_EMUOVR as usize / 4) + 1;

/// The documented offsets of Table 7-3 (anything else in the span is a gap).
fn sys_ctrl_documented(offset: u64) -> bool {
    matches!(
        offset,
        SYS_CTRL_CLOCK_CTRL..=SYS_CTRL_SRCRC
            | SYS_CTRL_PWRDBG
            | SYS_CTRL_CLD
            | SYS_CTRL_IWE
            | SYS_CTRL_I_MAP
            | SYS_CTRL_RCGCRFC..=SYS_CTRL_EMUOVR
    ) && offset % 4 == 0
}

/// CC2538 System Control (`0x400D_2000`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cc2538SysCtrl {
    regs: Vec<u32>,
    /// Bus cycle clock, for `CLOCK_STA.SYNC_32K`. Not chip state.
    #[serde(skip)]
    clock: Option<crate::CycleClock>,
}

impl Cc2538SysCtrl {
    pub fn new() -> Self {
        let mut regs = vec![0u32; SYS_CTRL_WORDS];
        regs[(SYS_CTRL_CLOCK_CTRL / 4) as usize] = CLOCK_CTRL_RESET;
        regs[(SYS_CTRL_PWRDBG / 4) as usize] = PWRDBG_RESET;
        regs[(SYS_CTRL_CLD / 4) as usize] = CLD_RESET;
        regs[(SYS_CTRL_IWE / 4) as usize] = IWE_RESET;
        regs[(SYS_CTRL_EMUOVR / 4) as usize] = EMUOVR_RESET;
        Self { regs, clock: None }
    }

    fn clock_ctrl(&self) -> u32 {
        self.regs[(SYS_CTRL_CLOCK_CTRL / 4) as usize]
    }

    /// `CLOCK_STA` derived from `CLOCK_CTRL`: the requested source and
    /// dividers take effect at once. A divider code faster than the selected
    /// oscillator is clamped: the 16-MHz HF-RC cannot run SYS_DIV/IO_DIV at
    /// `000` (32 MHz), so those read back `001`.
    fn clock_sta(&self) -> u32 {
        let ctrl = self.clock_ctrl();
        let on_hfrc = ctrl & CLK_OSC != 0;
        let floor = u32::from(on_hfrc);
        let sys_div = (ctrl & DIV_MASK).max(floor);
        let io_div = ((ctrl >> IO_DIV_SHIFT) & DIV_MASK).max(floor);
        // CLOCK_STA.OSC_PD is not derived: Table 7-3 resets it to 0 while
        // CLOCK_CTRL.OSC_PD resets to 1, and the power-down handshake that
        // links them is not modelled.
        let mut sta = (ctrl & (CLK_OSC32K_CALDIS | CLK_OSC32K | CLK_OSC))
            | (io_div << IO_DIV_SHIFT)
            | sys_div
            | STA_RESET_RESIDUE;
        if !on_hfrc {
            // The 32-MHz crystal is reported stable as soon as it is
            // selected; no start-up time is modelled.
            sta |= STA_XOSC_STB;
        }
        if let Some(clock) = &self.clock {
            // The core runs at SYS clock = undivided / 2^SYS_DIV, so `now`
            // cycles are now / sys_hz seconds; SYNC_32K is the level of a
            // 50 %-duty 32 768-Hz wave at that instant. A clock switch moves
            // the phase (the cycle count is not re-anchored). `sys_div` is
            // already clamped to the oscillator, so this never exceeds it.
            let sys_hz = XOSC_HZ >> sys_div;
            let half_periods =
                u128::from(clock.now()) * u128::from(2 * CLK_32K_HZ) / u128::from(sys_hz);
            if half_periods & 1 == 1 {
                sta |= STA_SYNC_32K;
            }
        }
        sta
    }

    fn read_reg(&self, offset: u64) -> u32 {
        if offset == SYS_CTRL_CLOCK_STA {
            return self.clock_sta();
        }
        if sys_ctrl_documented(offset) {
            return self.regs[(offset / 4) as usize];
        }
        crate::census_reg!("cc2538:Cc2538SysCtrl", offset, "read");
        0
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // CLOCK_STA is read-only.
            SYS_CTRL_CLOCK_STA => {}
            SYS_CTRL_CLOCK_CTRL => {
                self.regs[0] = value & CLOCK_CTRL_RW;
            }
            o if sys_ctrl_documented(o) => {
                let mask = NARROW_FIELDS
                    .iter()
                    .find(|(off, _)| *off == o)
                    .map_or(u32::MAX, |(_, m)| *m);
                self.regs[(o / 4) as usize] = value & mask;
            }
            _ => {
                crate::census_reg!("cc2538:Cc2538SysCtrl", offset, "write");
            }
        }
    }
}

impl Default for Cc2538SysCtrl {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Cc2538SysCtrl {
    /// A pure register bank: no `tick()`, no IRQ, DMA or event from the walk
    /// (pad levels are recomputed on access), so deleting the legacy walk
    /// cannot change what it does.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane_read(self.read_reg(offset & !3), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let merged = lane_merge(self.read_reg(reg), offset, value);
        self.write_reg(reg, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn on_node_reset(&mut self) {
        let clock = self.clock.take();
        *self = Self::new();
        self.clock = clock;
    }

    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.clock = Some(clock);
    }

    snapshot_impl!(Cc2538SysCtrl, "Cc2538SysCtrl", keep[clock]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GPIO_DATA` address for a pin mask (offset bits [9:2]).
    fn data(mask: u32) -> u64 {
        u64::from(mask) << 2
    }

    #[test]
    fn gpio_masked_write_touches_only_the_addressed_bits() {
        let mut port = Cc2538Gpio::new();
        port.write_u32(GPIO_DIR, 0xFF).unwrap();
        // SWRU319C Figure 9-2: 0xEB written at DATA + 0x098 (mask 0x26)
        // changes bits 5, 2 and 1 only → 1, 0, 1.
        port.write_u32(data(0xFF), 0x00).unwrap();
        port.write_u32(0x098, 0xEB).unwrap();
        assert_eq!(port.read_u32(data(0xFF)).unwrap(), 0x22);
        // A second write through a single-bit mask leaves the rest alone.
        port.write_u32(data(1 << 7), 0xFF).unwrap();
        assert_eq!(port.read_u32(data(0xFF)).unwrap(), 0xA2);
        // Writing zeros through a zero mask (DATA + 0) changes nothing.
        port.write_u32(0x000, 0x00).unwrap();
        assert_eq!(port.read_u32(data(0xFF)).unwrap(), 0xA2);
    }

    #[test]
    fn gpio_masked_read_zeroes_unaddressed_bits() {
        let mut port = Cc2538Gpio::new();
        port.write_u32(GPIO_DIR, 0xFF).unwrap();
        port.write_u32(data(0xFF), 0xFF).unwrap();
        // SWRU319C Figure 9-3: reading DATA + 0x0C4 (mask 0x31) returns
        // only bits 5, 4 and 0.
        assert_eq!(port.read_u32(0x0C4).unwrap(), 0x31);
        assert_eq!(port.read_u32(data(1 << 3)).unwrap(), 1 << 3);
        assert_eq!(port.read_u32(0x000).unwrap(), 0);
        // The byte path agrees with the word path.
        assert_eq!(port.read(0x0C4).unwrap(), 0x31);
    }

    #[test]
    fn gpio_byte_store_to_data_uses_the_address_mask() {
        let mut port = Cc2538Gpio::new();
        port.write_u32(GPIO_DIR, 0xFF).unwrap();
        port.write(data(1 << 2), 0xFF).unwrap();
        assert_eq!(port.read_u32(data(0xFF)).unwrap(), 1 << 2);
    }

    #[test]
    fn gpio_dir_selects_latch_or_pad_and_drives_the_pin_port() {
        let mut port = Cc2538Gpio::with_pull_up_mask(0xF0);
        assert_eq!(port.read_u32(GPIO_DIR).unwrap(), 0);
        // Inputs at reset: PC4..PC7 pulled up, PC0..PC3 have no pulls.
        assert_eq!(port.read_u32(data(0xFF)).unwrap(), 0xF0);
        assert_eq!(port.driver(4), Some(PadDriver::released(Pull::Up)));
        assert_eq!(port.driver(1), Some(PadDriver::OFF));

        // Latch a 1 on PC1 while it is still an input: the pad does not move.
        port.write_u32(data(1 << 1), 0xFF).unwrap();
        assert_eq!(port.read_u32(data(1 << 1)).unwrap(), 0);
        port.write_u32(GPIO_DIR, 1 << 1).unwrap();
        assert_eq!(port.read_u32(data(1 << 1)).unwrap(), 1 << 1);
        assert_eq!(port.driver(1), Some(PadDriver::drive(true)));
        assert_eq!(port.read_gpio_output(1), Some(true));
        assert_eq!(port.read_u32(GPIO_DIR).unwrap(), 1 << 1);
        // DIR is 8 bits wide.
        port.write_u32(GPIO_DIR, 0xFFFF_FF02).unwrap();
        assert_eq!(port.read_u32(GPIO_DIR).unwrap(), 0xFF02 & 0xFF);
    }

    #[test]
    fn gpio_afsel_hands_the_pin_to_its_peripheral() {
        let mut port = Cc2538Gpio::new();
        port.write_u32(GPIO_DIR, 1 << 1).unwrap();
        port.write_u32(data(1 << 1), 0).unwrap();
        port.write_u32(GPIO_AFSEL, 0x3).unwrap();
        assert_eq!(port.read_u32(GPIO_AFSEL).unwrap(), 0x3);
        assert_eq!(port.read_gpio_is_output(1), Some(false));
    }

    #[test]
    fn gpio_external_level_wins_over_the_pull_and_survives_reset() {
        let mut port = Cc2538Gpio::new();
        // Pressing an active-low key pulls the pad to ground.
        port.set_external(3, External::Level(false));
        assert_eq!(port.read_u32(data(1 << 3)).unwrap(), 0);
        port.write_u32(GPIO_DIR, 0x01).unwrap();
        port.on_node_reset();
        assert_eq!(port.read_u32(GPIO_DIR).unwrap(), 0);
        assert_eq!(port.input(3), Some(false));
        assert_eq!(port.read_u32(GPIO_GPIOLOCK).unwrap(), GPIOLOCK_RESET);
        assert_eq!(port.read_u32(GPIO_GPIOCR).unwrap(), GPIOCR_RESET);
    }

    #[test]
    fn sys_ctrl_resets_to_table_7_3() {
        let sc = Cc2538SysCtrl::new();
        assert_eq!(sc.read_u32(SYS_CTRL_CLOCK_CTRL).unwrap(), CLOCK_CTRL_RESET);
        assert_eq!(sc.read_u32(SYS_CTRL_CLOCK_STA).unwrap(), CLOCK_STA_RESET);
        assert_eq!(sc.read_u32(SYS_CTRL_RCGCUART).unwrap(), 0);
        assert_eq!(sc.read_u32(SYS_CTRL_PWRDBG).unwrap(), PWRDBG_RESET);
        assert_eq!(sc.read_u32(SYS_CTRL_CLD).unwrap(), CLD_RESET);
        assert_eq!(sc.read_u32(SYS_CTRL_IWE).unwrap(), IWE_RESET);
        assert_eq!(sc.read_u32(SYS_CTRL_I_MAP).unwrap(), 0);
        assert_eq!(sc.read_u32(SYS_CTRL_EMUOVR).unwrap(), EMUOVR_RESET);
        assert_eq!(sc.read_u32(SYS_CTRL_PMCTL).unwrap(), 0);
    }

    #[test]
    fn sys_ctrl_gates_store_their_documented_fields() {
        let mut sc = Cc2538SysCtrl::new();
        sc.write_u32(SYS_CTRL_RCGCUART, 0xFFFF_FFFF).unwrap();
        assert_eq!(sc.read_u32(SYS_CTRL_RCGCUART).unwrap(), 0x3);
        sc.write_u32(SYS_CTRL_RCGCGPT, 0xFFFF_FFFF).unwrap();
        assert_eq!(sc.read_u32(SYS_CTRL_RCGCGPT).unwrap(), 0xF);
        // Byte stores merge into the word.
        sc.write(SYS_CTRL_RCGCUART, 0x01).unwrap();
        assert_eq!(sc.read_u32(SYS_CTRL_RCGCUART).unwrap(), 0x1);
        // CLOCK_STA is read-only.
        sc.write_u32(SYS_CTRL_CLOCK_STA, 0).unwrap();
        assert_eq!(sc.read_u32(SYS_CTRL_CLOCK_STA).unwrap(), CLOCK_STA_RESET);
    }

    #[test]
    fn sys_ctrl_clock_sta_follows_clock_ctrl() {
        let mut sc = Cc2538SysCtrl::new();
        // Select the 32-MHz crystal at 32 MHz SYS/IO: CLOCK_STA reports it
        // (XOSC_STB set, OSC clear, dividers 000).
        sc.write_u32(SYS_CTRL_CLOCK_CTRL, CLOCK_CTRL_RESET & !(CLK_OSC | 0x707))
            .unwrap();
        let sta = sc.read_u32(SYS_CTRL_CLOCK_STA).unwrap();
        assert_eq!(sta & CLK_OSC, 0);
        assert_ne!(sta & STA_XOSC_STB, 0);
        assert_eq!(sta & DIV_MASK, 0);
        assert_eq!((sta >> IO_DIV_SHIFT) & DIV_MASK, 0);
        // Back on the HF-RC with a 32-MHz divider request: clamped to 16 MHz.
        sc.write_u32(SYS_CTRL_CLOCK_CTRL, (CLOCK_CTRL_RESET | CLK_OSC) & !0x707)
            .unwrap();
        let sta = sc.read_u32(SYS_CTRL_CLOCK_STA).unwrap();
        assert_ne!(sta & CLK_OSC, 0);
        assert_eq!(sta & DIV_MASK, 1);
        assert_eq!((sta >> IO_DIV_SHIFT) & DIV_MASK, 1);
        // Reserved read-only CLOCK_CTRL bits do not stick.
        sc.write_u32(SYS_CTRL_CLOCK_CTRL, 0xFFFF_FFFF).unwrap();
        assert_eq!(sc.read_u32(SYS_CTRL_CLOCK_CTRL).unwrap(), CLOCK_CTRL_RW);
    }
}
