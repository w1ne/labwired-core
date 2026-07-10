// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// I2C is one struct PER FAMILY behind the `I2c` enum:
//   * `F1I2c` — the legacy peripheral (CR1/CR2/OAR/DR/SR1/SR2/CCR/TRISE) AND
//     the full transaction state machine. START/STOP live in CR1.
//   * `L4I2c` — the modern peripheral (CR1/CR2/OAR/TIMINGR/ISR/ICR/RXDR/TXDR),
//     register-fidelity latching PLUS a minimal master transaction engine
//     (START/STOP/AUTOEND in CR2; address phase → ISR.NACKF when no slave acks).
// Each variant owns ALL of its own registers and state — an F1 I2C cannot
// carry TIMINGR/ISR, an L4 I2C cannot carry SR1/DR. CR1/CR2/OAR and the
// attached-device list exist on both because both families genuinely have
// them. The chip-yaml `profile` selects the variant.

use crate::SimResult;
use std::cell::{Cell, RefCell};
use std::str::FromStr;

pub trait I2cDevice: Send {
    fn address(&self) -> u8;
    fn read(&mut self) -> u8;
    fn write(&mut self, data: u8);
    fn start(&mut self) {}
    fn stop(&mut self) {}
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }
    /// Runtime-drivable view of this device, if it accepts simulated input.
    /// Overridden by input devices (accelerometers, …) so the generic
    /// [`crate::Machine::set_input`] resolver can reach them without a
    /// downcast. Default `None` = not an input device.
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        None
    }
}

/// I2C register layout selector. STM32F1/F2/F4 share the legacy I2C
/// peripheral; STM32L4/F7/H5/G0 share the modern peripheral. The config-facing
/// value maps 1:1 to a dedicated family struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum I2cRegisterLayout {
    #[default]
    Stm32F1,
    /// STM32L4 family (also F7/H5/G0). Verified against real NUCLEO-L476RG
    /// silicon via SWD register dump.
    Stm32L4,
    /// NXP Kinetis classic I2C (KW41Z / K series): byte-oriented A1/F/C1/S/D,
    /// interrupt-driven master matching the fsl_i2c HAL.
    Kinetis,
}

impl FromStr for I2cRegisterLayout {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32l4" | "l4" | "stm32f7" | "f7" | "stm32h5" | "h5" | "stm32g0" | "g0" => {
                Ok(Self::Stm32L4)
            }
            "kinetis" | "nxp" | "nxp_i2c" | "kw41z" | "mkw41z4" => Ok(Self::Kinetis),
            _ => Err(format!(
                "unsupported I2C register layout '{}'; supported: stm32f1, stm32l4, kinetis",
                value
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Default)]
enum I2cState {
    #[default]
    Idle,
    StartPending,
    AddressPending,
    DataPending,
}

// ── STM32F1 legacy I2C (registers + transaction state machine) ───────────────
#[derive(serde::Serialize)]
pub struct F1I2c {
    cr1: u32,
    cr2: u32,
    oar1: u32,
    oar2: u32,
    dr: u32,
    sr1: u32,
    sr2: u32,
    ccr: u32,
    trise: u32,

    state: I2cState,
    cycles_remaining: u32,

    #[serde(skip)]
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
    #[serde(skip)]
    current_target: Option<usize>,
    #[serde(skip)]
    is_reading: bool,
    #[serde(skip)]
    stop_requested: bool,
    #[serde(skip)]
    rxne_consumed: Cell<bool>,
    #[serde(skip)]
    read_dr_consumed: Cell<bool>,
}

impl Default for F1I2c {
    fn default() -> Self {
        Self {
            cr1: 0,
            cr2: 0,
            oar1: 0,
            oar2: 0,
            dr: 0,
            sr1: 0,
            sr2: 0,
            ccr: 0,
            // TRISE reset value is 0x0002 (RM0008 §26.6.9) — silicon-confirmed
            // on STM32F103 over SWD (reads 0x00000002 after RCC clock enable,
            // before any write).
            trise: 0x0002,
            state: I2cState::Idle,
            cycles_remaining: 0,
            attached_devices: Vec::new(),
            current_target: None,
            is_reading: false,
            stop_requested: false,
            rxne_consumed: Cell::new(false),
            read_dr_consumed: Cell::new(true),
        }
    }
}

impl F1I2c {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.oar1,
            0x0C => self.oar2,
            0x10 => self.dr,
            0x14 => self.sr1,
            0x18 => self.sr2,
            0x1C => self.ccr,
            0x20 => self.trise,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u16) {
        match offset {
            0x00 => {
                // CR1 writable mask 0xBFFB (bits 2,14 reserved) — silicon-
                // confirmed on F103. SWRST (bit 15) resets the peripheral on
                // real silicon; that side effect is not modelled here.
                self.cr1 = (value as u32) & 0xBFFB;
                if (value & 0x0100) != 0 && self.state == I2cState::Idle {
                    self.state = I2cState::StartPending;
                    self.cycles_remaining = 1;
                }
                if (value & 0x0200) != 0 {
                    // STOP requested. Defer if a data phase is in flight so
                    // RXNE/BTF latch first (HAL "NACK+STOP → poll RXNE → read
                    // DR" ordering); otherwise complete synchronously.
                    if matches!(self.state, I2cState::DataPending | I2cState::AddressPending) {
                        self.stop_requested = true;
                    } else {
                        self.cr1 &= !0x0200;
                        self.sr2 &= !0x0003;
                        if let Some(idx) = self.current_target {
                            self.attached_devices[idx].borrow_mut().stop();
                        }
                        self.current_target = None;
                        self.state = I2cState::Idle;
                    }
                }
            }
            // Writable masks silicon-confirmed on F103 (RM0008 §26.6):
            // CR2 0x1F3F, OAR1 0xC3FF, OAR2 0x00FF.
            0x04 => self.cr2 = (value as u32) & 0x1F3F,
            0x08 => self.oar1 = (value as u32) & 0xC3FF,
            0x0C => self.oar2 = (value as u32) & 0x00FF,
            0x10 => {
                self.dr = (value & 0xFF) as u32;
                if self.state == I2cState::Idle {
                    if (self.sr1 & 0x01) != 0 {
                        self.state = I2cState::AddressPending;
                        self.cycles_remaining = 20;
                        let addr = (self.dr >> 1) as u8;
                        self.is_reading = (self.dr & 1) != 0;
                        self.current_target = self
                            .attached_devices
                            .iter()
                            .position(|d| d.borrow().address() == addr);
                        if let Some(idx) = self.current_target {
                            self.attached_devices[idx].borrow_mut().start();
                        }
                    } else {
                        self.state = I2cState::DataPending;
                        self.cycles_remaining = 20;
                        self.sr1 &= !0x80;
                        self.sr1 &= !0x04;
                        if !self.is_reading {
                            if let Some(idx) = self.current_target {
                                self.attached_devices[idx].borrow_mut().write(self.dr as u8);
                            }
                        }
                    }
                }
            }
            0x14 => self.sr1 = value as u32,
            0x18 => self.sr2 = value as u32,
            // CCR 0xCFFF (12-bit divider + DUTY + F/S), TRISE 0x3F (6-bit) —
            // silicon-confirmed on F103.
            0x1C => self.ccr = (value as u32) & 0xCFFF,
            0x20 => self.trise = (value as u32) & 0x3F,
            _ => {}
        }
    }

    fn read(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        if reg_offset == 0x10 && byte_offset == 0 && self.is_reading && (self.sr1 & 0x0040) != 0 {
            if !self.read_dr_consumed.replace(true) {
                return (self.dr & 0xFF) as u8;
            }
            if let Some(idx) = self.current_target {
                return self.attached_devices[idx].borrow_mut().read();
            }
        }

        let reg_val = self.read_reg(reg_offset);
        // Silicon clears RXNE when firmware reads DR; mark for next tick.
        if reg_offset == 0x10 && byte_offset == 0 && (self.sr1 & 0x40) != 0 {
            self.rxne_consumed.set(true);
        }
        ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
    }

    fn write(&mut self, offset: u64, value: u8) {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);
        let mask: u32 = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);
        self.write_reg(reg_offset, reg_val as u16);
    }

    fn peek(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        if byte_offset < 2 {
            ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
        } else {
            0
        }
    }

    /// One tick of the transaction state machine. Returns whether an IRQ
    /// should be raised. Logic relocated verbatim from the pre-split model.
    fn tick(&mut self) -> bool {
        let mut irq = false;

        // "RXNE clears on DR read" mirror, fires even when Idle.
        if self.rxne_consumed.replace(false) {
            self.sr1 &= !0x0040;
            self.sr1 &= !0x0004; // BTF tied to the same shift register
            if self.is_reading && self.current_target.is_some() {
                self.state = I2cState::DataPending;
                self.cycles_remaining = 1;
            }
        }

        if self.state != I2cState::Idle {
            self.cycles_remaining = self.cycles_remaining.saturating_sub(1);
            if self.cycles_remaining == 0 {
                match self.state {
                    I2cState::StartPending => {
                        self.sr1 = 0x0001; // Only SB set
                        self.cr1 &= !0x0100; // auto-clear START request
                        self.state = I2cState::Idle;
                    }
                    I2cState::AddressPending => {
                        self.sr1 &= !0x0001; // Clear SB

                        // No slave at this address → NACK (SR1.AF), bus stays
                        // master+BUSY until firmware STOPs (matches F407 silicon).
                        if self.current_target.is_none() {
                            self.sr1 |= 0x0400; // AF
                            self.sr2 |= 0x0001; // MSL
                            self.sr2 |= 0x0002; // BUSY
                            self.state = I2cState::Idle;
                            if (self.cr2 & (1 << 8)) != 0 {
                                irq = true; // ITERR
                            }
                            return irq;
                        }

                        self.sr1 |= 0x0002; // ADDR
                        self.sr2 |= 0x0001; // MSL
                        self.sr2 |= 0x0002; // BUSY

                        if self.is_reading {
                            self.state = I2cState::DataPending;
                            self.cycles_remaining = 20;
                        } else {
                            self.sr1 |= 0x0080; // TXE
                            self.state = I2cState::Idle;
                        }
                    }
                    I2cState::DataPending => {
                        if self.is_reading {
                            self.sr1 |= 0x0040; // RXNE
                            if let Some(idx) = self.current_target {
                                self.dr = self.attached_devices[idx].borrow_mut().read() as u32;
                                self.read_dr_consumed.set(false);
                            }
                            self.state = I2cState::Idle;
                        } else {
                            self.sr1 |= 0x0080; // TXE
                            self.sr1 |= 0x0004; // BTF
                            self.state = I2cState::Idle;
                        }
                        if self.stop_requested {
                            self.stop_requested = false;
                            self.cr1 &= !0x0200;
                            self.sr2 &= !0x0003;
                            if let Some(idx) = self.current_target {
                                self.attached_devices[idx].borrow_mut().stop();
                            }
                            self.current_target = None;
                        }
                    }
                    I2cState::Idle => {}
                }

                if (self.cr2 & (1 << 9)) != 0 || (self.cr2 & (1 << 10)) != 0 {
                    irq = true; // ITEVTEN or ITBUFEN
                }
            }
        }

        irq
    }
}

// ── STM32L4 modern I2C (register-fidelity latching + minimal master engine) ──
#[derive(serde::Serialize)]
pub struct L4I2c {
    cr1: u32,
    cr2: u32,
    oar1: u32,
    oar2: u32,
    timingr: u32,
    timeoutr: u32,
    isr: u32,
    icr: u32,
    pecr: u32,
    rxdr: u32,
    txdr: u32,

    // Minimal master transaction engine (mirrors F1I2c, modern-register flavour).
    state: I2cState,
    cycles_remaining: u32,

    #[serde(skip)]
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
    /// Index of the addressed slave for the armed/in-flight transfer (None when
    /// no attached device matches SADD — the tier-1 no-device case).
    #[serde(skip)]
    current_target: Option<usize>,
    #[serde(skip)]
    is_reading: bool,
    #[serde(skip)]
    autoend: bool,
    /// CR2.START has latched a transfer; the address phase fires once the first
    /// data byte is loaded into TXDR (write) — mirrors F1's START→DR ordering.
    #[serde(skip)]
    start_armed: bool,
}

impl Default for L4I2c {
    fn default() -> Self {
        Self {
            cr1: 0,
            cr2: 0,
            oar1: 0,
            oar2: 0,
            timingr: 0,
            timeoutr: 0,
            isr: 0x0000_0001, // TXE=1 at reset
            icr: 0,
            pecr: 0,
            rxdr: 0,
            txdr: 0,
            state: I2cState::Idle,
            cycles_remaining: 0,
            attached_devices: Vec::new(),
            current_target: None,
            is_reading: false,
            autoend: false,
            start_armed: false,
        }
    }
}

impl L4I2c {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.oar1,
            0x0C => self.oar2,
            0x10 => self.timingr,
            0x14 => self.timeoutr,
            0x18 => self.isr,
            0x1C => self.icr,
            0x20 => self.pecr,
            0x24 => self.rxdr,
            0x28 => self.txdr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr1 = value & 0x00FF_E1FF,
            0x04 => {
                self.cr2 = value;
                if (value & (1 << 13)) != 0 {
                    // START: latch BUSY and arm a master transfer. Capture the
                    // addressed slave (SADD[7:1] in 7-bit mode), direction
                    // (RD_WRN), NBYTES and AUTOEND. The address phase runs once
                    // the first byte reaches TXDR (write) or immediately for a
                    // read — mirrors the F1 START→DR handshake.
                    self.isr |= 1 << 15; // BUSY
                    let addr = ((value >> 1) & 0x7F) as u8;
                    self.is_reading = (value & (1 << 10)) != 0; // RD_WRN
                    self.autoend = (value & (1 << 25)) != 0;
                    self.current_target = self
                        .attached_devices
                        .iter()
                        .position(|d| d.borrow().address() == addr);
                    if let Some(idx) = self.current_target {
                        self.attached_devices[idx].borrow_mut().start();
                    }
                    self.start_armed = true;
                    if self.is_reading {
                        // Read needs no TXDR write to begin; kick the engine now.
                        self.state = I2cState::AddressPending;
                        self.cycles_remaining = 20;
                        self.start_armed = false;
                    }
                    // For a write, TXIS is NOT asserted here: on real STM32 L4
                    // silicon TXIS only sets once the address phase has been
                    // clocked out and ACKed (hardware then requests the first
                    // byte). Firmware reading ISR immediately after setting
                    // CR2.START sees only BUSY|TXE (0x8001), not TXIS. The byte
                    // request is modelled by the engine consuming TXDR after the
                    // address ACK (see `tick`), so no premature TXIS is needed.
                }
                if (value & (1 << 14)) != 0 {
                    // STOP: release the bus and tear down any armed/in-flight
                    // transfer.
                    self.isr &= !(1 << 15); // clear BUSY
                    if let Some(idx) = self.current_target {
                        self.attached_devices[idx].borrow_mut().stop();
                    }
                    self.current_target = None;
                    self.state = I2cState::Idle;
                    self.start_armed = false;
                }
            }
            0x08 => self.oar1 = value,
            0x0C => self.oar2 = value,
            0x10 => self.timingr = value,
            0x14 => self.timeoutr = value,
            0x18 => {
                let rw_mask: u32 = 0x0000_0001; // TXE is RW
                self.isr = (self.isr & !rw_mask) | (value & rw_mask);
            }
            0x1C => {
                let clearable: u32 = 0x0000_3F38;
                self.isr &= !(value & clearable);
                self.icr = 0;
            }
            0x20 => self.pecr = value,
            0x24 => self.rxdr = value & 0xFF,
            0x28 => {
                self.txdr = value & 0xFF;
                self.isr &= !0x0000_0003; // writing TXDR clears TXE+TXIS
                                          // Loading the first byte while a write transfer is armed starts
                                          // the address phase.
                if self.start_armed && self.state == I2cState::Idle {
                    self.state = I2cState::AddressPending;
                    self.cycles_remaining = 20;
                    self.start_armed = false;
                }
            }
            _ => {}
        }
    }

    fn read(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
    }

    fn write(&mut self, offset: u64, value: u8) {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);
        let mask: u32 = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);
        self.write_reg(reg_offset, reg_val);
    }

    fn peek(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        if byte_offset < 2 {
            ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
        } else {
            0
        }
    }

    /// One tick of the minimal master transaction engine. Returns whether an
    /// IRQ should be raised. Structure mirrors `F1I2c::tick` but uses the modern
    /// ISR/ICR/CR2 register set (NACKF/STOPF/TC, START/STOP/AUTOEND in CR2).
    fn tick(&mut self) -> bool {
        let mut irq = false;
        if self.state == I2cState::Idle {
            return irq;
        }
        self.cycles_remaining = self.cycles_remaining.saturating_sub(1);
        if self.cycles_remaining != 0 {
            return irq;
        }
        if self.state == I2cState::AddressPending {
            match self.current_target {
                None => {
                    // No slave ACKed the address → NACKF (matches L476 silicon:
                    // a write to an absent device sets ISR.NACKF, and AUTOEND
                    // auto-generates STOP, clearing BUSY and setting STOPF).
                    self.isr |= 1 << 4; // NACKF
                    self.isr &= !(1 << 1); // no further byte requested (TXIS off)
                    if self.autoend {
                        self.isr |= 1 << 5; // STOPF
                        self.isr &= !(1 << 15); // BUSY released
                    }
                    if (self.cr1 & (1 << 4)) != 0 {
                        irq = true; // NACKIE
                    }
                }
                Some(idx) => {
                    // Slave ACKed. Deliver the one armed byte (write) or fetch
                    // one (read), then complete (TC) and auto-STOP if requested.
                    if self.is_reading {
                        self.rxdr = self.attached_devices[idx].borrow_mut().read() as u32;
                        self.isr |= 1 << 2; // RXNE
                    } else {
                        self.attached_devices[idx]
                            .borrow_mut()
                            .write(self.txdr as u8);
                        self.isr |= 1 << 0; // TXE
                    }
                    self.isr |= 1 << 6; // TC (NBYTES transferred)
                    if self.autoend {
                        self.isr |= 1 << 5; // STOPF
                        self.isr &= !(1 << 15); // BUSY released
                        if let Some(i) = self.current_target {
                            self.attached_devices[i].borrow_mut().stop();
                        }
                        self.current_target = None;
                    }
                    if (self.cr1 & (1 << 6)) != 0 {
                        irq = true; // TCIE
                    }
                }
            }
            self.state = I2cState::Idle;
        }
        irq
    }
}

// ── NXP Kinetis I2C (classic Freescale module: A1/F/C1/S/D/C2/FLT, byte-oriented,
//    interrupt-driven master) ──────────────────────────────────────────────────
//
// 1-byte registers: A1=0x00, F=0x01, C1=0x02, S=0x03, D=0x04, C2=0x05, FLT=0x06,
// RA=0x07, SMB=0x08, A2=0x09, SLTH=0x0A, SLTL=0x0B, S2=0x0C.
//   C1 bits: IICEN 0x80, IICIE 0x40, MST 0x20, TX 0x10, TXAK 0x08, RSTA 0x04.
//   S  bits: TCF 0x80, IAAS 0x40, BUSY 0x20, ARBL 0x10, SRW 0x04, IICIF 0x02, RXAK 0x01.
//
// The NXP fsl_i2c HAL drives each transfer byte-by-byte from the I2C ISR
// (I2C_MasterTransferHandleIRQ): START is C1.MST 0→1 then the slave address is
// written to D; a repeated START is C1.RSTA then the new address to D; entering
// master-receive clears C1.TX and the HAL dummy-reads D once to release the bus;
// STOP is C1.MST 1→0. Every byte the firmware moves through D "completes"
// synchronously here — we raise S.TCF|S.IICIF and set S.RXAK from whether a
// slave answered the address. The interrupt is LEVEL-driven: tick() asserts the
// IRQ while (S.IICIF & C1.IICIE), because the HAL enables IICIE only AFTER the
// opening address byte is already on the wire (I2C_MasterTransferNonBlocking),
// so an edge model would drop the first interrupt and hang the transfer.
const KI_C1_IICIE: u8 = 0x40;
const KI_C1_MST: u8 = 0x20;
const KI_C1_TX: u8 = 0x10;
const KI_C1_RSTA: u8 = 0x04;
const KI_S_TCF: u8 = 0x80;
const KI_S_BUSY: u8 = 0x20;
const KI_S_ARBL: u8 = 0x10;
const KI_S_IICIF: u8 = 0x02;
const KI_S_RXAK: u8 = 0x01;

#[derive(serde::Serialize)]
pub struct KinetisI2c {
    a1: u8,
    f: u8,
    c1: u8,
    s: Cell<u8>,
    d: Cell<u8>,
    c2: u8,
    flt: u8,
    ra: u8,
    smb: u8,
    a2: u8,
    slth: u8,
    sltl: u8,

    /// Next byte written to D is a slave address (after START / repeated START).
    expect_address: bool,
    /// Next read of D is the HAL bus-release dummy (return junk, no device byte).
    rx_dummy_pending: Cell<bool>,
    /// Current transfer is a master read (set from the address R/W bit).
    is_reading: bool,

    #[serde(skip)]
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
    #[serde(skip)]
    current_target: Option<usize>,
}

impl Default for KinetisI2c {
    fn default() -> Self {
        Self {
            a1: 0,
            f: 0,
            c1: 0,
            // TCF=1 (idle, transfer complete), everything else clear (RM §49.3.4).
            s: Cell::new(KI_S_TCF),
            d: Cell::new(0),
            c2: 0,
            flt: 0,
            ra: 0,
            smb: 0,
            a2: 0,
            slth: 0,
            sltl: 0,
            expect_address: false,
            rx_dummy_pending: Cell::new(false),
            is_reading: false,
            attached_devices: Vec::new(),
            current_target: None,
        }
    }
}

impl KinetisI2c {
    /// Mark a byte transfer complete: TCF + IICIF latch; RXAK mirrors the slave ack.
    fn byte_complete(&self, acked: bool) {
        let mut s = self.s.get() | KI_S_TCF | KI_S_IICIF;
        if acked {
            s &= !KI_S_RXAK;
        } else {
            s |= KI_S_RXAK;
        }
        self.s.set(s);
    }

    fn read_reg(&self, offset: u64) -> u8 {
        match offset {
            0x00 => self.a1,
            0x01 => self.f,
            0x02 => self.c1,
            0x03 => self.s.get(),
            0x04 => {
                // Bus-release dummy read after entering RX: HAL discards it.
                if self.rx_dummy_pending.replace(false) {
                    self.byte_complete(true);
                    return 0xFF;
                }
                if self.is_reading {
                    let byte = match self.current_target {
                        Some(idx) => self.attached_devices[idx].borrow_mut().read(),
                        None => 0xFF,
                    };
                    self.d.set(byte);
                    self.byte_complete(true);
                    return byte;
                }
                self.d.get()
            }
            0x05 => self.c2,
            0x06 => self.flt,
            0x07 => self.ra,
            0x08 => self.smb,
            0x09 => self.a2,
            0x0A => self.slth,
            0x0B => self.sltl,
            // S2: EMPTY=1 always (double-buffer TX FIFO empty) — the HAL polls
            // this before every D write on parts with double buffering.
            0x0C => 0x01,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u8) {
        match offset {
            0x00 => self.a1 = value,
            0x01 => self.f = value,
            0x02 => {
                let old = self.c1;
                self.c1 = value;
                let mst_old = old & KI_C1_MST != 0;
                let mst_new = value & KI_C1_MST != 0;
                let tx_old = old & KI_C1_TX != 0;
                let tx_new = value & KI_C1_TX != 0;

                if !mst_old && mst_new {
                    // START: the next D write is the slave address.
                    self.expect_address = true;
                    self.s.set(self.s.get() | KI_S_BUSY);
                } else if mst_old && !mst_new {
                    // STOP. Keep current_target so a trailing last-byte D read
                    // (the HAL issues STOP just before reading it) still resolves.
                    if let Some(idx) = self.current_target {
                        self.attached_devices[idx].borrow_mut().stop();
                    }
                    self.s.set(self.s.get() & !KI_S_BUSY);
                }
                if value & KI_C1_RSTA != 0 && mst_new {
                    // Repeated START: next D write is a fresh address; RSTA self-clears.
                    self.expect_address = true;
                    self.c1 &= !KI_C1_RSTA;
                }
                if tx_old && !tx_new && mst_new {
                    // Entering master-receive: HAL dummy-reads D next to release the bus.
                    self.rx_dummy_pending.set(true);
                }
            }
            0x03 => {
                // S: IICIF and ARBL are write-1-to-clear.
                let mut s = self.s.get();
                if value & KI_S_IICIF != 0 {
                    s &= !KI_S_IICIF;
                }
                if value & KI_S_ARBL != 0 {
                    s &= !KI_S_ARBL;
                }
                self.s.set(s);
            }
            0x04 => {
                self.d.set(value);
                if self.expect_address {
                    let addr = value >> 1;
                    self.is_reading = (value & 1) != 0;
                    self.current_target = self
                        .attached_devices
                        .iter()
                        .position(|dev| dev.borrow().address() == addr);
                    if let Some(idx) = self.current_target {
                        self.attached_devices[idx].borrow_mut().start();
                        self.byte_complete(true);
                    } else {
                        self.byte_complete(false); // address NAK
                    }
                    self.expect_address = false;
                } else {
                    if let Some(idx) = self.current_target {
                        self.attached_devices[idx].borrow_mut().write(value);
                    }
                    self.byte_complete(true);
                }
            }
            0x05 => self.c2 = value,
            0x06 => self.flt = value,
            0x07 => self.ra = value,
            0x08 => self.smb = value,
            0x09 => self.a2 = value,
            0x0A => self.slth = value,
            0x0B => self.sltl = value,
            _ => {}
        }
    }

    /// LEVEL interrupt: asserted while a byte is pending (IICIF) and IICIE is set.
    fn tick(&mut self) -> bool {
        (self.s.get() & KI_S_IICIF) != 0 && (self.c1 & KI_C1_IICIE) != 0
    }
}

/// I2C peripheral — one variant per chip family. Register sets fully isolated.
#[derive(serde::Serialize)]
pub enum I2c {
    Stm32F1(F1I2c),
    Stm32L4(L4I2c),
    Kinetis(KinetisI2c),
}

impl core::fmt::Debug for I2c {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            I2c::Stm32F1(i) => f.debug_struct("I2c::F1").field("state", &i.state).finish(),
            I2c::Stm32L4(_) => f.debug_struct("I2c::L4").finish(),
            I2c::Kinetis(i) => f
                .debug_struct("I2c::Kinetis")
                .field("c1", &i.c1)
                .field("s", &i.s.get())
                .finish(),
        }
    }
}

impl Default for I2c {
    fn default() -> Self {
        Self::Stm32F1(F1I2c::default())
    }
}

impl I2c {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_with_layout(layout: I2cRegisterLayout) -> Self {
        match layout {
            I2cRegisterLayout::Stm32F1 => Self::Stm32F1(F1I2c::default()),
            I2cRegisterLayout::Stm32L4 => Self::Stm32L4(L4I2c::default()),
            I2cRegisterLayout::Kinetis => Self::Kinetis(KinetisI2c::default()),
        }
    }

    /// Attach a slave to a bare (off-bus) controller, wrapping it into `trace`.
    /// The trace handle is mandatory, so there is no untraced attach — this is
    /// the off-bus counterpart of the on-bus choke point
    /// [`crate::bus::SystemBus::attach_i2c_slave`], and both funnel through the
    /// one wrap helper `bus_trace::wrap_i2c`. Used by standalone tests that
    /// drive an `I2c` directly (no `SystemBus`).
    pub fn attach_traced(
        &mut self,
        bus_name: &str,
        trace: &crate::bus::bus_trace::BusTrace,
        device: Box<dyn I2cDevice>,
    ) {
        self.push_slave(crate::bus::bus_trace::wrap_i2c(bus_name, trace, device));
    }

    /// Raw slave push — does NOT wrap for tracing. The only caller is the bus
    /// choke point [`crate::bus::SystemBus::attach_i2c_slave`], which wraps the
    /// device first; nothing else should attach directly (that would bypass the
    /// universal bus trace).
    pub(crate) fn push_slave(&mut self, device: Box<dyn I2cDevice>) {
        match self {
            Self::Stm32F1(i) => i.attached_devices.push(RefCell::new(device)),
            Self::Stm32L4(i) => i.attached_devices.push(RefCell::new(device)),
            Self::Kinetis(i) => i.attached_devices.push(RefCell::new(device)),
        }
    }

    /// Attached I2C devices (used by config/bus validation + tests).
    pub fn attached_devices(&self) -> &[RefCell<Box<dyn I2cDevice>>] {
        match self {
            Self::Stm32F1(i) => &i.attached_devices,
            Self::Stm32L4(i) => &i.attached_devices,
            Self::Kinetis(i) => &i.attached_devices,
        }
    }
}

impl crate::Peripheral for I2c {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(match self {
            Self::Stm32F1(i) => i.read(offset),
            Self::Stm32L4(i) => i.read(offset),
            Self::Kinetis(i) => i.read_reg(offset),
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match self {
            Self::Stm32F1(i) => i.write(offset, value),
            Self::Stm32L4(i) => i.write(offset, value),
            Self::Kinetis(i) => i.write_reg(offset, value),
        }
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        let irq = match self {
            Self::Stm32F1(i) => i.tick(),
            Self::Stm32L4(i) => i.tick(),
            Self::Kinetis(i) => i.tick(),
        };
        crate::PeripheralTickResult {
            irq,
            cycles: 0,
            ..Default::default()
        }
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some(match self {
            Self::Stm32F1(i) => i.peek(offset),
            Self::Stm32L4(i) => i.peek(offset),
            // Kinetis registers are side-effect-free to read except D; peek D
            // without consuming a device byte.
            Self::Kinetis(i) => {
                if offset == 0x04 {
                    i.d.get()
                } else {
                    i.read_reg(offset)
                }
            }
        })
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    /// Custom inspection: the generic register decode plus a `framebuffer`
    /// artifact for any attached SSD1306 OLED. This is the pattern the ~10
    /// bespoke `get_*_framebuffer` wasm accessors generalize into — the
    /// controller walks its own attached devices and emits panel artifacts, one
    /// code path instead of a bespoke accessor per panel. Summary mode omits the
    /// bytes and carries a cheap `generation` hash so callers skip unchanged
    /// buffers.
    fn inspect(
        &self,
        base: u64,
        name: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::PeripheralInspect {
        let mut pi = crate::inspect::default_inspect(self, base, name, opts);
        pi.kind = "i2c".to_string();
        for dev_cell in self.attached_devices() {
            let dev = dev_cell.borrow();
            let addr = dev.address();
            if let Some(oled) = dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::components::Ssd1306>())
            {
                let fb = oled.framebuffer();
                pi.artifacts.push(crate::inspect::Artifact {
                    kind: "framebuffer".to_string(),
                    id: format!("i2c@0x{:02x}", addr),
                    meta: serde_json::json!({
                        "w": oled.width(),
                        "h": oled.height(),
                        "format": "ssd1306_page",
                        "generation": crate::inspect::artifact_generation(fb),
                        "ink_bytes": oled.ink_bytes(),
                        "lit_pixels": oled.lit_pixels(),
                    }),
                    bytes: if opts.include_bytes {
                        Some(fb.to_vec())
                    } else {
                        None
                    },
                });
            }
        }
        pi
    }

    fn snapshot(&self) -> serde_json::Value {
        match self {
            Self::Stm32F1(i) => serde_json::to_value(i),
            Self::Stm32L4(i) => serde_json::to_value(i),
            Self::Kinetis(i) => serde_json::to_value(i),
        }
        .unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::{I2c, I2cDevice, KinetisI2c, KI_C1_MST, KI_C1_TX};
    use crate::Peripheral;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    /// The I2C controller's custom `inspect()` emits a `framebuffer` artifact
    /// for an attached SSD1306 OLED: metadata always present; the (large) byte
    /// payload only when `include_bytes` is requested. This is the pattern that
    /// generalizes the bespoke `get_*_framebuffer` accessors.
    #[test]
    fn inspect_emits_ssd1306_framebuffer_artifact() {
        use crate::inspect::InspectOpts;
        use crate::peripherals::components::Ssd1306;

        let mut i2c = I2c::new();
        i2c.push_slave(Box::new(Ssd1306::new(0x3C)));

        // Summary mode: metadata present, bytes omitted.
        let summary = i2c.inspect(0x4000_5400, "i2c1", &InspectOpts::default());
        assert_eq!(summary.kind, "i2c");
        let fb = summary
            .artifacts
            .iter()
            .find(|a| a.kind == "framebuffer")
            .expect("framebuffer artifact present");
        assert_eq!(fb.id, "i2c@0x3c");
        assert_eq!(fb.meta["w"], 128);
        assert_eq!(fb.meta["h"], 64);
        assert_eq!(fb.meta["format"], "ssd1306_page");
        assert!(
            fb.meta["generation"].is_u64(),
            "cheap change-detection hash"
        );
        assert!(fb.bytes.is_none(), "bytes omitted in summary mode");

        // include_bytes: full GDDRAM payload attached.
        let full = i2c.inspect(
            0x4000_5400,
            "i2c1",
            &InspectOpts {
                include_bytes: true,
                peripheral: None,
            },
        );
        let fb = full
            .artifacts
            .iter()
            .find(|a| a.kind == "framebuffer")
            .expect("framebuffer artifact present");
        assert_eq!(
            fb.bytes.as_ref().map(|b| b.len()),
            Some(128 * 8),
            "1024-byte page-major GDDRAM"
        );
    }

    struct CountingDevice {
        address: u8,
        reads: Arc<AtomicUsize>,
    }

    impl CountingDevice {
        fn new(address: u8, reads: Arc<AtomicUsize>) -> Self {
            Self { address, reads }
        }
    }

    impl I2cDevice for CountingDevice {
        fn address(&self) -> u8 {
            self.address
        }
        fn read(&mut self) -> u8 {
            self.reads.fetch_add(1, Ordering::SeqCst) as u8
        }
        fn write(&mut self, _data: u8) {}
    }

    #[test]
    fn test_i2c_reset_values() {
        let i2c = I2c::new();
        assert_eq!(i2c.read(0x00).unwrap(), 0); // CR1
        assert_eq!(i2c.read(0x04).unwrap(), 0); // CR2
    }

    #[test]
    fn test_i2c_start_bit() {
        let mut i2c = I2c::new();
        i2c.write(0x01, 0x01).unwrap(); // CR1 SB (bit 8)
        assert_eq!(i2c.peek(0x14).unwrap() & 0x01, 0); // not immediate
        for _ in 0..10 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0); // SB set after ticks
    }

    #[test]
    fn test_i2c_full_transfer_flow() {
        use crate::peripherals::components::Mpu6050;
        let mut i2c = I2c::new();
        i2c.push_slave(Box::new(Mpu6050::new(0x50)));

        i2c.write(0x01, 0x01).unwrap(); // START
        for _ in 0..10 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0); // SB

        i2c.write(0x10, 0xA0).unwrap(); // addr 0x50<<1 | W
        for _ in 0..20 {
            i2c.tick();
        }
        assert_eq!(i2c.peek(0x14).unwrap() & 0x01, 0); // SB cleared
        assert_ne!(i2c.peek(0x14).unwrap() & 0x02, 0); // ADDR
        assert_ne!(i2c.peek(0x18).unwrap() & 0x01, 0); // MSL

        i2c.write(0x10, 0x42).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x80, 0); // TXE
        assert_ne!(i2c.peek(0x14).unwrap() & 0x04, 0); // BTF

        i2c.write(0x01, 0x02).unwrap(); // STOP (bit 9)
        for _ in 0..10 {
            i2c.tick();
        }
        assert_eq!(
            i2c.peek(0x18).unwrap() & 0x03,
            0,
            "STOP must clear MSL+BUSY"
        );
    }

    #[test]
    fn test_adxl345_devid_and_axis_read() {
        use crate::peripherals::components::Adxl345;

        let mut i2c = I2c::new();
        let mut sensor = Adxl345::new(0x53);
        sensor.set_sample(256, -128, 64);
        i2c.push_slave(Box::new(sensor));

        i2c.write(0x00, 0x01).unwrap();
        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0);

        i2c.write(0x10, 0xA6).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x02, 0);

        i2c.write(0x10, 0x00).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }

        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        i2c.write(0x10, 0xA7).unwrap();
        for _ in 0..40 {
            i2c.tick();
        }
        assert_eq!(i2c.read(0x10).unwrap(), 0xE5);

        i2c.write(0x01, 0x02).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }

        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        i2c.write(0x10, 0xA6).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        i2c.write(0x10, 0x32).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        i2c.write(0x10, 0xA7).unwrap();
        for _ in 0..40 {
            i2c.tick();
        }

        assert_eq!(i2c.read(0x10).unwrap(), 0x00);
        assert_eq!(i2c.read(0x10).unwrap(), 0x01);
        assert_eq!(i2c.read(0x10).unwrap(), 0x80);
        assert_eq!(i2c.read(0x10).unwrap(), 0xFF);
        assert_eq!(i2c.read(0x10).unwrap(), 0x40);
        assert_eq!(i2c.read(0x10).unwrap(), 0x00);
    }

    #[test]
    fn test_i2c_single_byte_read_advances_device_once() {
        let reads = Arc::new(AtomicUsize::new(0));
        let mut i2c = I2c::new();
        i2c.push_slave(Box::new(CountingDevice::new(0x42, reads.clone())));

        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }

        i2c.write(0x10, 0x85).unwrap();
        for _ in 0..40 {
            i2c.tick();
        }

        assert_ne!(i2c.peek(0x14).unwrap() & 0x40, 0);
        assert_eq!(i2c.read(0x10).unwrap(), 0);
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    // ── STM32L4 (modern) transaction engine ──────────────────────────────────

    /// Configure CR2 for a 1-byte 7-bit master write to `addr` with AUTOEND,
    /// then load TXDR — the no-device case the tier-1 fixtures exercise.
    fn l4_write_xfer(i2c: &mut I2c, addr: u8, byte: u8) {
        use crate::Peripheral;
        i2c.write(0x00, 1).unwrap(); // CR1.PE
                                     // CR2 = SADD(addr<<1) | NBYTES=1<<16 | AUTOEND<<25 | START<<13
        let cr2: u32 = ((addr as u32) << 1) | (1 << 16) | (1 << 25) | (1 << 13);
        for b in 0..4 {
            i2c.write(0x04 + b, ((cr2 >> (b * 8)) & 0xFF) as u8)
                .unwrap();
        }
        i2c.write(0x28, byte).unwrap(); // TXDR: first (only) byte
    }

    #[test]
    fn test_l4_i2c_nack_on_no_device() {
        use super::I2cRegisterLayout;
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);

        // START latches BUSY (ISR bit15) up front.
        l4_write_xfer(&mut i2c, 0x52, 0xAB);
        // BUSY is ISR bit15 → byte offset 0x19, bit7.
        assert_ne!(i2c.peek(0x19).unwrap() & (1 << 7), 0, "BUSY after START");

        // No attached device → address phase NACKs after the engine ticks.
        let mut nacked = false;
        for _ in 0..40 {
            i2c.tick();
            if i2c.peek(0x18).unwrap() & (1 << 4) != 0 {
                nacked = true;
                break;
            }
        }
        assert!(nacked, "ISR.NACKF must set when no slave acknowledges");
        // AUTOEND released the bus: BUSY clear, STOPF set.
        assert_eq!(i2c.peek(0x19).unwrap() & (1 << 7), 0, "AUTOEND clears BUSY");
        assert_ne!(i2c.peek(0x18).unwrap() & (1 << 5), 0, "AUTOEND sets STOPF");

        // ICR.NACKCF (bit4) + STOPCF (bit5) clear the flags.
        i2c.write(0x1C, (1 << 4) | (1 << 5)).unwrap();
        assert_eq!(
            i2c.peek(0x18).unwrap() & (1 << 4),
            0,
            "NACKF cleared by ICR"
        );
    }

    #[test]
    fn test_l4_i2c_ack_delivers_byte_to_device() {
        use super::I2cRegisterLayout;
        use std::sync::atomic::AtomicUsize;
        let writes = Arc::new(AtomicUsize::new(0));

        struct WriteCounter {
            address: u8,
            writes: Arc<AtomicUsize>,
        }
        impl I2cDevice for WriteCounter {
            fn address(&self) -> u8 {
                self.address
            }
            fn read(&mut self) -> u8 {
                0
            }
            fn write(&mut self, _data: u8) {
                self.writes.fetch_add(1, Ordering::SeqCst);
            }
        }

        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
        i2c.push_slave(Box::new(WriteCounter {
            address: 0x3C,
            writes: writes.clone(),
        }));

        l4_write_xfer(&mut i2c, 0x3C, 0x42);
        for _ in 0..40 {
            i2c.tick();
        }
        // Attached device ACKs → no NACKF, the byte reaches the device, TC set.
        assert_eq!(
            i2c.peek(0x18).unwrap() & (1 << 4),
            0,
            "no NACKF when device present"
        );
        assert_ne!(
            i2c.peek(0x18).unwrap() & (1 << 6),
            0,
            "TC after byte transferred"
        );
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn i2c_attach_wraps_device_into_shared_log() {
        use crate::bus::bus_trace::{new_log, wrap_i2c, BusPayload};
        use crate::Peripheral;

        let log = new_log();
        let mut i2c = I2c::Kinetis(KinetisI2c::default());

        // device at 0x1E
        struct D;
        impl I2cDevice for D {
            fn address(&self) -> u8 {
                0x1E
            }
            fn read(&mut self) -> u8 {
                0
            }
            fn write(&mut self, _: u8) {}
        }
        // The bus choke point wraps before push; emulate it here.
        i2c.push_slave(wrap_i2c("i2c1", &log, Box::new(D)));

        // Drive START + addr(W) + one data byte through the Kinetis register
        // model via the public `Peripheral::write` MMIO path (the same path
        // every other Kinetis-adjacent test in this module uses to poke
        // registers — `write_reg` itself is private).
        i2c.write(0x02, KI_C1_MST | KI_C1_TX).unwrap(); // START
        i2c.write(0x04, 0x3C).unwrap(); // addr 0x1E + W -> selects device, start()
        i2c.write(0x04, 0xAF).unwrap(); // data -> device.write -> wrapper records

        let snap = log.snapshot();
        assert!(snap
            .iter()
            .any(|e| matches!(&e.payload, BusPayload::I2c { byte, .. } if *byte == 0xAF)));
    }
}
