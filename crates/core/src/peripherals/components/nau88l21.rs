// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Nuvoton NAU88L21 stereo audio codec (Linux name `nau8821`): the I²C
//! control port and the register file, as an [`I2cDevice`].
//!
//! Source: Nuvoton NAU88L21 datasheet Rev 3.3 (Oct 30, 2023), "DS" below.
//! The reset values are the `DEFAULT` rows of DS §10 "Control and Status
//! Registers". The mainline Linux `nau8821` driver (`nau8821_reg_defaults`,
//! the readable/writeable/volatile tables) was used as a cross-check; where
//! the two differ the datasheet wins (R80: DS 0x0B00, Linux 0x0000).
//!
//! ## Control port (DS §7)
//!
//! * Slave only. The device address is set by the GPIO1/CSB strap: 0x1B with
//!   CSB low, 0x54 with CSB high (DS §7.1, §7.3). No other address exists,
//!   so the kit rejects any other `i2c_address`.
//! * 16-bit register address, then 16-bit data, both MSB first (DS §7.3,
//!   Figure 22).
//! * Read: write the 16-bit address, repeated START with R/W=1, then the
//!   device sends 16-bit words. Without a STOP it auto-increments the address
//!   after each word and wraps 0xFFFF to 0x0000 (DS §7.4, Figure 23).
//! * Write: "a three-byte instruction followed by one or more Data Bytes"
//!   (DS §7.3). The datasheet states auto-increment only for reads. This
//!   model also increments after each complete data word of a write, the
//!   same as a read. Firmware that writes one word per transaction (the
//!   Linux driver, the i.MX RT SDK drivers) does not see the difference.
//! * A data word that is not complete at STOP is not written.
//! * The device ACKs every byte, also for addresses with no register. Such
//!   addresses read 0 and ignore writes (the datasheet does not say what
//!   happens; the model records them as `unmapped` in the `writes` log).
//!
//! ## Register behaviour
//!
//! * R00 HARDWARE_RESET: a write of any value resets every register to its
//!   default (DS §7.6, §10 R00).
//! * R5A SOFTWARE_RST: "write any value twice to reset all internal states
//!   without resetting the config registers" (DS §10 R5A). The model has no
//!   internal state beyond the register file, so it only counts and logs it.
//! * Read-only status registers (R10 IRQ_STATUS, R4D IMM_RMS_L, R53/R54
//!   OTPDOUT, R58 I2C_DEVICE_ID, R59 SARDOUT_RAM_STATUS, R81
//!   CHARGE_PUMP_INPUT_READ, R82 GENERAL_STATUS) ignore writes.
//! * R58 I2C_DEVICE_ID reads `X0011 01X0 0010 0000` (DS §10 R58). The model
//!   reads the X bits as 0: 0x1A20. KEYDET/MICDET: no headset is modelled.
//! * R81 reads 0x0013 and R82 0x0020 (the values DS §10 gives for these
//!   read-only registers).
//!
//! ## What is not modelled
//!
//! No audio: the codec is not connected to an I²S/SAI data line, there is no
//! ADC, DAC, FLL, DRC, biquad, jack or mic detection, and no IRQ pin. Status
//! registers do not change. The model answers the question the firmware asks
//! over I²C (is the codec there, and what did the firmware configure), and
//! records the configuration for tests.
//!
//! ## Logs (`peripheral_log`, device id as `peripheral`)
//!
//! * `writes`: one line per register write, in order:
//!   `write 0x001c = 0x000e`. A write to R00 adds ` (reset)`, to an address
//!   with no register ` (unmapped)`, to a read-only register ` (read-only)`.
//! * `reads`: one line per register word read: `read 0x0058 = 0x1a20`.
//! * `regs`: the register file now, one line per register: `0x001c = 0x000e`.
//! * `state`: the digital audio interface and path enables decoded from the
//!   registers (DS §10 R01, R1C, R1D), for example
//!   `dai slave i2s 32-bit` and `enable dac_l dac_r adc_l adc_r`.

use crate::peripheral_log::PeripheralLog;
use crate::peripherals::i2c::I2cDevice;

/// Address with the GPIO1/CSB strap low (DS §7.1).
pub const NAU88L21_ADDR_CSB_LOW: u8 = 0x1B;
/// Address with the GPIO1/CSB strap high (DS §7.1).
pub const NAU88L21_ADDR_CSB_HIGH: u8 = 0x54;

/// R00 HARDWARE_RESET (DS §7.6).
pub const REG_RESET: u16 = 0x00;
/// R01 ENA_CTRL.
pub const REG_ENA_CTRL: u16 = 0x01;
/// R1C I2S_PCM_CTRL1.
pub const REG_I2S_PCM_CTRL1: u16 = 0x1C;
/// R1D I2S_PCM_CTRL2.
pub const REG_I2S_PCM_CTRL2: u16 = 0x1D;
/// R58 I2C_DEVICE_ID (read only).
pub const REG_I2C_DEVICE_ID: u16 = 0x58;
/// R5A SOFTWARE_RST.
pub const REG_SOFTWARE_RST: u16 = 0x5A;

/// R58 as DS §10 gives it, X bits read as 0.
pub const DEVICE_ID_VALUE: u16 = 0x1A20;

/// How many lines the `writes` and `reads` logs keep. A firmware that polls
/// a register forever must not grow memory without limit; the first lines
/// (the init sequence) are the ones a test asserts, so the log stops
/// growing when it is full and counts what it dropped.
const LOG_CAP: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    ReadWrite,
    ReadOnly,
    /// R00 and R5A: a write is a command, a read returns 0.
    Command,
}

/// DS §10: every register with its default (reset) value.
/// `(address, default, access)`.
const REGISTERS: &[(u16, u16, Access)] = &[
    (0x00, 0x0000, Access::Command),           // HARDWARE_RESET
    (0x01, 0x00FF, Access::ReadWrite),         // ENA_CTRL
    (0x03, 0x0050, Access::ReadWrite),         // CLK_DIVIDER
    (0x04, 0x0000, Access::ReadWrite),         // FLL1
    (0x05, 0x00BC, Access::ReadWrite),         // FLL2
    (0x06, 0x0008, Access::ReadWrite),         // FLL3
    (0x07, 0x0010, Access::ReadWrite),         // FLL4
    (0x08, 0x4000, Access::ReadWrite),         // FLL5
    (0x09, 0x6900, Access::ReadWrite),         // FLL6
    (0x0A, 0x0031, Access::ReadWrite),         // FLL7
    (0x0B, 0x26E9, Access::ReadWrite),         // FLL8
    (0x0D, 0x0000, Access::ReadWrite),         // JACK_DET_CTRL
    (0x0F, 0x0000, Access::ReadWrite),         // INTERRUPT_MASK
    (0x10, 0x0000, Access::ReadOnly),          // IRQ_STATUS
    (0x11, 0x0000, Access::ReadWrite),         // INT_CLR_KEY_STATUS
    (0x12, 0xFFFF, Access::ReadWrite),         // INTERRUPT_DIS_CTRL
    (0x13, 0x0000, Access::ReadWrite),         // DMIC_CTRL
    (0x1A, 0x0000, Access::ReadWrite),         // GPIO12_CTRL
    (0x1B, 0x0000, Access::ReadWrite),         // TDM_CTRL
    (0x1C, 0x000A, Access::ReadWrite),         // I2S_PCM_CTRL1
    (0x1D, 0x8010, Access::ReadWrite),         // I2S_PCM_CTRL2
    (0x1E, 0x0000, Access::ReadWrite),         // LEFT_TIME_SLOT
    (0x1F, 0x0000, Access::ReadWrite),         // RIGHT_TIME_SLOT
    (0x21, 0x0000, Access::ReadWrite),         // BIQ0_COF1
    (0x22, 0x0000, Access::ReadWrite),         // BIQ0_COF2
    (0x23, 0x0000, Access::ReadWrite),         // BIQ0_COF3
    (0x24, 0x0000, Access::ReadWrite),         // BIQ0_COF4
    (0x25, 0x0000, Access::ReadWrite),         // BIQ0_COF5
    (0x26, 0x0000, Access::ReadWrite),         // BIQ0_COF6
    (0x27, 0x0000, Access::ReadWrite),         // BIQ0_COF7
    (0x28, 0x0000, Access::ReadWrite),         // BIQ0_COF8
    (0x29, 0x0000, Access::ReadWrite),         // BIQ0_COF9
    (0x2A, 0x0000, Access::ReadWrite),         // BIQ0_COF10
    (0x2B, 0x0002, Access::ReadWrite),         // ADC_RATE
    (0x2C, 0x0082, Access::ReadWrite),         // DAC_CTRL1
    (0x2D, 0x0000, Access::ReadWrite),         // DAC_CTRL2
    (0x2F, 0x0000, Access::ReadWrite),         // DAC_DGAIN_CTRL
    (0x30, 0x0000, Access::ReadWrite),         // ADC_DGAIN_CTRL
    (0x31, 0x0000, Access::ReadWrite),         // MUTE_CTRL
    (0x32, 0x0000, Access::ReadWrite),         // HSVOL_CTRL
    (0x34, 0xCFCF, Access::ReadWrite),         // DACR_CTRL (DAC volume)
    (0x35, 0xCFCF, Access::ReadWrite),         // ADC_DGAIN_CTRL1 (ADC volume)
    (0x36, 0x1486, Access::ReadWrite),         // ADC_DRC_KNEE_IP12
    (0x37, 0x0F12, Access::ReadWrite),         // ADC_DRC_KNEE_IP34
    (0x38, 0x25FF, Access::ReadWrite),         // ADC_DRC_SLOPES
    (0x39, 0x3457, Access::ReadWrite),         // ADC_DRC_ATKDCY
    (0x3A, 0x1486, Access::ReadWrite),         // DAC_DRC_KNEE_IP12
    (0x3B, 0x0F12, Access::ReadWrite),         // DAC_DRC_KNEE_IP34
    (0x3C, 0x25F9, Access::ReadWrite),         // DAC_DRC_SLOPES
    (0x3D, 0x3457, Access::ReadWrite),         // DAC_DRC_ATKDCY
    (0x41, 0x0000, Access::ReadWrite),         // BIQ1_COF1
    (0x42, 0x0000, Access::ReadWrite),         // BIQ1_COF2
    (0x43, 0x0000, Access::ReadWrite),         // BIQ1_COF3
    (0x44, 0x0000, Access::ReadWrite),         // BIQ1_COF4
    (0x45, 0x0000, Access::ReadWrite),         // BIQ1_COF5
    (0x46, 0x0000, Access::ReadWrite),         // BIQ1_COF6
    (0x47, 0x0000, Access::ReadWrite),         // BIQ1_COF7
    (0x48, 0x0000, Access::ReadWrite),         // BIQ1_COF8
    (0x49, 0x0000, Access::ReadWrite),         // BIQ1_COF9
    (0x4A, 0x0000, Access::ReadWrite),         // BIQ1_COF10
    (0x4B, 0x0000, Access::ReadWrite),         // CLASSG_CTRL
    (0x4C, 0x0000, Access::ReadWrite),         // IMM_MODE_CTRL
    (0x4D, 0x0000, Access::ReadOnly),          // IMM_RMS_L
    (0x4E, 0x0000, Access::ReadWrite),         // FUSE_CTRL2
    (0x4F, 0x0000, Access::ReadWrite),         // FUSE_CTRL3
    (0x51, 0x0400, Access::ReadWrite),         // FUSE_CTRL1
    (0x53, 0x0000, Access::ReadOnly),          // OTPDOUT_1
    (0x54, 0x0000, Access::ReadOnly),          // OTPDOUT_2
    (0x55, 0x0000, Access::ReadWrite),         // MISC_CTRL
    (0x58, DEVICE_ID_VALUE, Access::ReadOnly), // I2C_DEVICE_ID
    (0x59, 0x0000, Access::ReadOnly),          // SARDOUT_RAM_STATUS
    (0x5A, 0x0000, Access::Command),           // SOFTWARE_RST
    (0x66, 0x0000, Access::ReadWrite),         // BIAS_ADJ
    (0x68, 0x0000, Access::ReadWrite),         // TRIM_SETTINGS
    (0x69, 0x0000, Access::ReadWrite),         // ANALOG_CONTROL_1
    (0x6A, 0x0000, Access::ReadWrite),         // ANALOG_CONTROL_2
    (0x6B, 0x0000, Access::ReadWrite),         // PGA_MUTE
    (0x71, 0x0011, Access::ReadWrite),         // ANALOG_ADC_1
    (0x72, 0x0020, Access::ReadWrite),         // ANALOG_ADC_2
    (0x73, 0x0008, Access::ReadWrite),         // RDAC
    (0x74, 0x0006, Access::ReadWrite),         // MIC_BIAS
    (0x76, 0x0000, Access::ReadWrite),         // BOOST
    (0x77, 0x0000, Access::ReadWrite),         // FEPGA
    (0x7E, 0x0000, Access::ReadWrite),         // PGA_GAIN
    (0x7F, 0x0000, Access::ReadWrite),         // POWER_UP_CONTROL
    (0x80, 0x0B00, Access::ReadWrite),         // CHARGE_PUMP
    (0x81, 0x0013, Access::ReadOnly),          // CHARGE_PUMP_INPUT_READ
    (0x82, 0x0020, Access::ReadOnly),          // GENERAL_STATUS
];

fn slot_of(reg: u16) -> Option<usize> {
    REGISTERS.iter().position(|&(a, _, _)| a == reg)
}

/// Where the byte stream of the current transaction is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Next byte is the high byte of the register address.
    AddrHi,
    /// Next byte is the low byte of the register address.
    AddrLo(u8),
    /// Next byte is the high byte of a data word.
    DataHi,
    /// Next byte is the low byte of a data word.
    DataLo(u8),
}

#[derive(Debug)]
pub struct Nau88l21 {
    address: u8,
    values: Vec<u16>,
    /// Register address pointer (DS §7.3/§7.4).
    pointer: u16,
    phase: Phase,
    /// Read phase: which byte of the current word goes out next.
    read_low_next: bool,
    hardware_resets: u32,
    software_resets: u32,
    writes: Vec<String>,
    reads: Vec<String>,
    dropped_lines: u64,
}

impl Nau88l21 {
    /// A codec at `address` (0x1B or 0x54), registers at their defaults.
    pub fn new(address: u8) -> Self {
        Self {
            address,
            values: REGISTERS.iter().map(|&(_, d, _)| d).collect(),
            pointer: 0,
            phase: Phase::AddrHi,
            read_low_next: false,
            hardware_resets: 0,
            software_resets: 0,
            writes: Vec::new(),
            reads: Vec::new(),
            dropped_lines: 0,
        }
    }

    /// Current value of register `reg` as the firmware would read it.
    pub fn register(&self, reg: u16) -> u16 {
        match slot_of(reg) {
            Some(i) if REGISTERS[i].2 != Access::Command => self.values[i],
            _ => 0,
        }
    }

    /// Writes to R00 so far (each one reset the register file).
    pub fn hardware_resets(&self) -> u32 {
        self.hardware_resets
    }

    /// Writes to R5A so far.
    pub fn software_resets(&self) -> u32 {
        self.software_resets
    }

    /// The `writes` log lines so far.
    pub fn write_log(&self) -> &[String] {
        &self.writes
    }

    fn push_line(log: &mut Vec<String>, dropped: &mut u64, line: String) {
        if log.len() < LOG_CAP {
            log.push(line);
        } else {
            *dropped += 1;
        }
    }

    fn reset_registers(&mut self) {
        for (v, &(_, d, _)) in self.values.iter_mut().zip(REGISTERS) {
            *v = d;
        }
    }

    /// One complete 16-bit register write from the bus.
    fn write_register(&mut self, reg: u16, value: u16) {
        let note = match slot_of(reg) {
            None => " (unmapped)",
            Some(i) => match REGISTERS[i].2 {
                Access::ReadWrite => {
                    self.values[i] = value;
                    ""
                }
                Access::ReadOnly => " (read-only)",
                Access::Command if reg == REG_RESET => {
                    self.reset_registers();
                    self.hardware_resets += 1;
                    " (reset)"
                }
                Access::Command => {
                    self.software_resets += 1;
                    " (software reset)"
                }
            },
        };
        Self::push_line(
            &mut self.writes,
            &mut self.dropped_lines,
            format!("write 0x{reg:04x} = 0x{value:04x}{note}"),
        );
    }

    /// The `state` log: what the registers configure (DS §10).
    fn state_lines(&self) -> Vec<String> {
        let r1c = self.register(REG_I2S_PCM_CTRL1);
        let r1d = self.register(REG_I2S_PCM_CTRL2);
        let r01 = self.register(REG_ENA_CTRL);
        // R1D[3] MS0: 0 = slave, 1 = master.
        let role = if r1d & (1 << 3) != 0 {
            "master"
        } else {
            "slave"
        };
        // R1C[1:0] AIFMT0.
        let format = match r1c & 0x3 {
            0 => "right-justified",
            1 => "left-justified",
            2 => "i2s",
            _ => "pcm",
        };
        // R1C[3:2] WLEN0.
        let bits = [16, 20, 24, 32][((r1c >> 2) & 0x3) as usize];
        // R1D[15] I2S_TRI: 1 = ADCOUT high-Z.
        let adcout = if r1d & (1 << 15) != 0 {
            "hi-z"
        } else {
            "driven"
        };
        // R01[11:8] RDACEN, LDACEN, RADCEN, LADCEN.
        let mut enable = String::from("enable");
        for (bit, name) in [(10, "dac_l"), (11, "dac_r"), (8, "adc_l"), (9, "adc_r")] {
            if r01 & (1 << bit) != 0 {
                enable.push(' ');
                enable.push_str(name);
            }
        }
        if enable == "enable" {
            enable.push_str(" none");
        }
        vec![
            format!("dai {role} {format} {bits}-bit"),
            format!("adcout {adcout}"),
            enable,
            format!(
                "resets hardware {} software {}",
                self.hardware_resets, self.software_resets
            ),
        ]
    }
}

impl I2cDevice for Nau88l21 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        // A (repeated) START begins a new byte stream. A read after a
        // repeated START uses the address pointer the write set (DS §7.4).
        self.phase = Phase::AddrHi;
        self.read_low_next = false;
    }

    fn write(&mut self, data: u8) {
        match self.phase {
            Phase::AddrHi => self.phase = Phase::AddrLo(data),
            Phase::AddrLo(hi) => {
                self.pointer = u16::from_be_bytes([hi, data]);
                self.phase = Phase::DataHi;
            }
            Phase::DataHi => self.phase = Phase::DataLo(data),
            Phase::DataLo(hi) => {
                let reg = self.pointer;
                self.write_register(reg, u16::from_be_bytes([hi, data]));
                self.pointer = self.pointer.wrapping_add(1);
                self.phase = Phase::DataHi;
            }
        }
    }

    fn read(&mut self) -> u8 {
        let value = self.register(self.pointer);
        if self.read_low_next {
            self.read_low_next = false;
            self.pointer = self.pointer.wrapping_add(1); // wraps 0xFFFF -> 0 (DS §7.4)
            value as u8
        } else {
            self.read_low_next = true;
            Self::push_line(
                &mut self.reads,
                &mut self.dropped_lines,
                format!("read 0x{:04x} = 0x{value:04x}", self.pointer),
            );
            (value >> 8) as u8
        }
    }

    fn stop(&mut self) {
        // A data word not complete at STOP is not written.
        self.phase = Phase::AddrHi;
        self.read_low_next = false;
    }

    fn logs(&self) -> Vec<PeripheralLog> {
        let regs = REGISTERS
            .iter()
            .filter(|(_, _, a)| *a != Access::Command)
            .map(|&(reg, _, _)| format!("0x{reg:04x} = 0x{:04x}", self.register(reg)))
            .collect();
        let mut writes = self.writes.clone();
        if self.dropped_lines > 0 {
            writes.push(format!(
                "({} lines not logged: log full)",
                self.dropped_lines
            ));
        }
        vec![
            PeripheralLog::new("writes", writes),
            PeripheralLog::new("reads", self.reads.clone()),
            PeripheralLog::new("regs", regs),
            PeripheralLog::new("state", self.state_lines()),
        ]
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

/// The strap addresses; anything else is a wiring mistake (DS §7.1).
pub fn check_address(address: u8) -> anyhow::Result<u8> {
    if address == NAU88L21_ADDR_CSB_LOW || address == NAU88L21_ADDR_CSB_HIGH {
        Ok(address)
    } else {
        anyhow::bail!(
            "nau88l21: i2c_address 0x{address:02x} is not possible; the CSB strap \
             selects 0x1b (low) or 0x54 (high)"
        )
    }
}

pub struct Nau88l21Kit;
pub static NAU88L21_KIT: Nau88l21Kit = Nau88l21Kit;

static NAU88L21_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(&[]),
    device_type: std::borrow::Cow::Borrowed("nau88l21"),
    label: std::borrow::Cow::Borrowed("NAU88L21 Audio Codec"),
    summary: std::borrow::Cow::Borrowed(
        "Nuvoton NAU88L21 stereo audio codec: I2C control port and register file.",
    ),
    detail: std::borrow::Cow::Borrowed(
        "16-bit register address and data, auto-increment, reset values and \
         read-only status registers from the datasheet (Rev 3.3). Records the \
         register writes and the configured audio interface. No audio data path.",
    ),
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: std::borrow::Cow::Borrowed(&[ConfigKey {
        name: std::borrow::Cow::Borrowed("i2c_address"),
        ty: ConfigType::Int,
        doc: std::borrow::Cow::Borrowed(
            "7-bit slave address set by the CSB strap: 0x1B (CSB low, default) or 0x54 (CSB high).",
        ),
    }]),
    labs: std::borrow::Cow::Borrowed(&[]),
};

impl PeripheralKit for Nau88l21Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &NAU88L21_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = check_address(ctx.i2c_address_or(NAU88L21_ADDR_CSB_LOW)?)?;
        ctx.attach_i2c_device(Box::new(Nau88l21::new(address)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One register write as the firmware sends it: START, 2 address bytes,
    /// 2 data bytes, STOP.
    fn write_reg(dev: &mut Nau88l21, reg: u16, value: u16) {
        dev.start();
        for b in reg.to_be_bytes().into_iter().chain(value.to_be_bytes()) {
            dev.write(b);
        }
        dev.stop();
    }

    /// Register read: START, 2 address bytes, repeated START, `words` x 2
    /// bytes, STOP.
    fn read_regs(dev: &mut Nau88l21, reg: u16, words: usize) -> Vec<u16> {
        dev.start();
        for b in reg.to_be_bytes() {
            dev.write(b);
        }
        dev.start();
        let out = (0..words)
            .map(|_| u16::from_be_bytes([dev.read(), dev.read()]))
            .collect();
        dev.stop();
        out
    }

    fn log(logs: &[PeripheralLog], name: &str) -> Vec<String> {
        logs.iter().find(|l| l.name == name).unwrap().lines()
    }

    #[test]
    fn reset_values_match_the_datasheet_default_rows() {
        let mut dev = Nau88l21::new(NAU88L21_ADDR_CSB_HIGH);
        // DS §10 DEFAULT rows.
        for (reg, want) in [
            (0x01, 0x00FF),
            (0x03, 0x0050),
            (0x12, 0xFFFF),
            (0x1C, 0x000A),
            (0x1D, 0x8010),
            (0x2C, 0x0082),
            (0x34, 0xCFCF),
            (0x3C, 0x25F9),
            (0x51, 0x0400),
            (0x80, 0x0B00),
        ] {
            assert_eq!(read_regs(&mut dev, reg, 1), vec![want], "R{reg:02X}");
        }
    }

    #[test]
    fn device_id_register_reads_the_datasheet_value_and_ignores_writes() {
        let mut dev = Nau88l21::new(NAU88L21_ADDR_CSB_HIGH);
        assert_eq!(read_regs(&mut dev, REG_I2C_DEVICE_ID, 1), vec![0x1A20]);
        write_reg(&mut dev, REG_I2C_DEVICE_ID, 0x1234);
        assert_eq!(read_regs(&mut dev, REG_I2C_DEVICE_ID, 1), vec![0x1A20]);
        assert_eq!(
            dev.write_log().last().unwrap(),
            "write 0x0058 = 0x1234 (read-only)"
        );
    }

    #[test]
    fn write_then_read_back_is_big_endian_16_bit() {
        let mut dev = Nau88l21::new(NAU88L21_ADDR_CSB_HIGH);
        write_reg(&mut dev, 0x1C, 0x000E);
        assert_eq!(dev.register(0x1C), 0x000E);
        assert_eq!(read_regs(&mut dev, 0x1C, 1), vec![0x000E]);
        // Byte order: the high byte goes first.
        write_reg(&mut dev, 0x34, 0x12CF);
        assert_eq!(dev.register(0x34), 0x12CF);
    }

    #[test]
    fn read_auto_increments_and_wraps_at_0xffff() {
        let mut dev = Nau88l21::new(NAU88L21_ADDR_CSB_HIGH);
        // R34, R35, R36 in one read (DS §7.4).
        assert_eq!(read_regs(&mut dev, 0x34, 3), vec![0xCFCF, 0xCFCF, 0x1486]);
        // 0xFFFF (no register: 0) then 0x0000 (R00 reads 0) then R01.
        assert_eq!(read_regs(&mut dev, 0xFFFF, 3), vec![0, 0, 0x00FF]);
    }

    #[test]
    fn multi_word_write_auto_increments() {
        let mut dev = Nau88l21::new(NAU88L21_ADDR_CSB_HIGH);
        dev.start();
        for b in [0x00, 0x34, 0x11, 0x22, 0x33, 0x44] {
            dev.write(b);
        }
        dev.stop();
        assert_eq!(dev.register(0x34), 0x1122);
        assert_eq!(dev.register(0x35), 0x3344);
    }

    #[test]
    fn a_data_word_cut_by_stop_is_not_written() {
        let mut dev = Nau88l21::new(NAU88L21_ADDR_CSB_HIGH);
        dev.start();
        for b in [0x00, 0x34, 0x11] {
            dev.write(b);
        }
        dev.stop();
        assert_eq!(dev.register(0x34), 0xCFCF);
        assert!(dev.write_log().is_empty());
    }

    #[test]
    fn a_write_to_r00_resets_every_register() {
        let mut dev = Nau88l21::new(NAU88L21_ADDR_CSB_HIGH);
        write_reg(&mut dev, 0x01, 0x0FFF);
        write_reg(&mut dev, 0x80, 0x0720);
        write_reg(&mut dev, REG_RESET, 0x0000);
        assert_eq!(dev.register(0x01), 0x00FF);
        assert_eq!(dev.register(0x80), 0x0B00);
        assert_eq!(dev.hardware_resets(), 1);
        assert_eq!(
            dev.write_log().last().unwrap(),
            "write 0x0000 = 0x0000 (reset)"
        );
    }

    #[test]
    fn software_reset_keeps_the_config_registers() {
        let mut dev = Nau88l21::new(NAU88L21_ADDR_CSB_HIGH);
        write_reg(&mut dev, 0x01, 0x0FFF);
        write_reg(&mut dev, REG_SOFTWARE_RST, 0);
        write_reg(&mut dev, REG_SOFTWARE_RST, 0);
        assert_eq!(dev.register(0x01), 0x0FFF);
        assert_eq!(dev.software_resets(), 2);
    }

    #[test]
    fn unmapped_address_reads_zero_and_ignores_writes() {
        let mut dev = Nau88l21::new(NAU88L21_ADDR_CSB_HIGH);
        write_reg(&mut dev, 0x02, 0xBEEF);
        assert_eq!(read_regs(&mut dev, 0x02, 1), vec![0]);
        assert_eq!(
            dev.write_log().last().unwrap(),
            "write 0x0002 = 0xbeef (unmapped)"
        );
    }

    #[test]
    fn logs_record_writes_reads_registers_and_decoded_state() {
        let mut dev = Nau88l21::new(NAU88L21_ADDR_CSB_HIGH);
        // Reset state: DS defaults are slave, I2S, 24-bit, ADCOUT high-Z,
        // no channel enabled.
        let logs = dev.logs();
        assert_eq!(
            log(&logs, "state")[..3],
            ["dai slave i2s 24-bit", "adcout hi-z", "enable none"]
        );
        write_reg(&mut dev, 0x01, 0x0FFF);
        write_reg(&mut dev, 0x1C, 0x000E);
        write_reg(&mut dev, 0x1D, 0x0000);
        read_regs(&mut dev, 0x58, 1);
        let logs = dev.logs();
        assert_eq!(
            log(&logs, "writes"),
            [
                "write 0x0001 = 0x0fff",
                "write 0x001c = 0x000e",
                "write 0x001d = 0x0000"
            ]
        );
        assert_eq!(log(&logs, "reads"), ["read 0x0058 = 0x1a20"]);
        assert!(log(&logs, "regs").contains(&"0x001c = 0x000e".to_string()));
        assert_eq!(
            log(&logs, "state")[..3],
            [
                "dai slave i2s 32-bit",
                "adcout driven",
                "enable dac_l dac_r adc_l adc_r"
            ]
        );
    }

    #[test]
    fn master_mode_is_decoded() {
        let mut dev = Nau88l21::new(NAU88L21_ADDR_CSB_HIGH);
        write_reg(&mut dev, 0x1D, 0x0008);
        write_reg(&mut dev, 0x1C, 0x0001);
        let logs = dev.logs();
        assert_eq!(log(&logs, "state")[0], "dai master left-justified 16-bit");
    }

    #[test]
    fn kit_accepts_only_the_two_strap_addresses() {
        assert_eq!(check_address(0x1B).unwrap(), 0x1B);
        assert_eq!(check_address(0x54).unwrap(), 0x54);
        assert!(check_address(0x1A).is_err());
        assert!(check_address(0x55).is_err());
    }
}
