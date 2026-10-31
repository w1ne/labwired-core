// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use serde::{Deserialize, Serialize, Serializer};
use std::collections::{BTreeMap, HashMap};

/// On-disk schema version for `MachineSnapshot`. Bump whenever the wire
/// format changes in a non-backwards-compatible way (new required fields,
/// renamed variants, changed semantics). Older snapshots whose version
/// does not match are rejected by `Machine::apply_snapshot` rather than
/// silently ignored — a zero-filled field is worse than a clean refusal.
pub const SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    // Treat a missing field as v0 (pre-versioned snapshots). `apply_snapshot`
    // will reject anything that isn't the current SCHEMA_VERSION, so this
    // default means "old snapshot, please re-record".
    0
}

fn serialize_peripherals<S>(
    peripherals: &HashMap<String, serde_json::Value>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let ordered = peripherals.iter().collect::<BTreeMap<_, _>>();
    ordered.serialize(serializer)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MachineSnapshot {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub cpu: CpuSnapshot,
    #[serde(serialize_with = "serialize_peripherals")]
    pub peripherals: HashMap<String, serde_json::Value>,
    // Future: metrics
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CpuSnapshot {
    Arm(ArmCpuSnapshot),
    /// Diagnostic visible state only; native Cortex-A restore is unsupported.
    CortexA(CortexACpuSnapshot),
    RiscV(RiscVCpuSnapshot),
    XtensaLx7(XtensaLx7CpuSnapshot),
    Avr(AvrCpuSnapshot),
    Pic24(Box<Pic24CpuSnapshot>),
    Pic18(Box<Pic18CpuSnapshot>),
    Pic16(Box<Pic16CpuSnapshot>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CortexACpuSnapshot {
    pub registers: Vec<u32>,
    pub pc: u32,
    pub cpsr: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AvrCpuSnapshot {
    pub registers: Vec<u8>,
    pub pc: u32,
    pub sp: u16,
    pub sreg: u8,
}

/// PIC24/dsPIC33 visible state (word-addressed PC; W0–W15; UART1 latches;
/// M1a timers, interrupt latches and accumulators).
/// Default PIC24 package masks: the 44-pin dsPIC33FJ32MC204. Used when a
/// snapshot predates the per-chip `port_masks` field.
pub fn default_pic24_port_masks() -> [u16; 3] {
    [0x079F, 0xFFFF, 0x03FF]
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Pic24CpuSnapshot {
    pub registers: Vec<u16>,
    pub pc: u32,
    pub sr: u16,
    pub cycles: u64,
    pub u1mode: u16,
    pub u1sta: u16,
    #[serde(default)]
    pub t1con: u16,
    #[serde(default)]
    pub tmr1: u16,
    #[serde(default)]
    pub pr1: u16,
    #[serde(default)]
    pub t1_frac: u16,
    #[serde(default)]
    pub t2con: u16,
    #[serde(default)]
    pub tmr2: u16,
    #[serde(default)]
    pub pr2: u16,
    #[serde(default)]
    pub t2_frac: u16,
    #[serde(default)]
    pub t3con: u16,
    #[serde(default)]
    pub tmr3: u16,
    #[serde(default)]
    pub pr3: u16,
    #[serde(default)]
    pub t3_frac: u16,
    #[serde(default)]
    pub ifs0: u16,
    #[serde(default)]
    pub iec0: u16,
    #[serde(default)]
    pub ipc0: u16,
    #[serde(default)]
    pub ipc1: u16,
    #[serde(default)]
    pub ipc2: u16,
    #[serde(default)]
    pub intcon1: u16,
    #[serde(default)]
    pub intcon2: u16,
    #[serde(default)]
    pub corcon: u16,
    #[serde(default)]
    pub acc_a: i64,
    #[serde(default)]
    pub acc_b: i64,
    #[serde(default = "default_pic24_port_masks")]
    pub port_masks: [u16; 3],
    #[serde(default)]
    pub tris: [u16; 3],
    #[serde(default)]
    pub lat: [u16; 3],
    #[serde(default)]
    pub odc: [u16; 3],
    #[serde(default)]
    pub pad_in: [u16; 3],
    #[serde(default)]
    pub rpinr7: u16,
    #[serde(default)]
    pub rpor: Vec<u16>,
    #[serde(default)]
    pub ic_con: [u16; 2],
    #[serde(default)]
    pub ic_fifo: [Vec<u16>; 2],
    #[serde(default)]
    pub ic_prescale: [u8; 2],
    #[serde(default)]
    pub ic_ici_count: [u8; 2],
    #[serde(default)]
    pub oc_con: [u16; 2],
    #[serde(default)]
    pub oc_r: [u16; 2],
    #[serde(default)]
    pub oc_rs: [u16; 2],
    #[serde(default)]
    pub oc_level: [Option<bool>; 2],
    #[serde(default)]
    pub oc_halted: [bool; 2],
    // M2: Motor Control PWM (MCPWM1) and QEI1.
    #[serde(default)]
    pub p1tcon: u16,
    #[serde(default)]
    pub p1tmr: u16,
    #[serde(default)]
    pub p1tper: u16,
    #[serde(default)]
    pub p1tper_active: u16,
    #[serde(default)]
    pub p1secmp: u16,
    #[serde(default)]
    pub pwm1con1: u16,
    #[serde(default)]
    pub pwm1con2: u16,
    #[serde(default)]
    pub p1dtcon1: u16,
    #[serde(default)]
    pub p1dtcon2: u16,
    #[serde(default)]
    pub p1fltacon: u16,
    #[serde(default)]
    pub p1ovdcon: u16,
    #[serde(default)]
    pub p1dc: [u16; 3],
    #[serde(default)]
    pub p1dc_active: [u16; 3],
    #[serde(default)]
    pub p1_frac: u16,
    #[serde(default)]
    pub p1_post: u8,
    #[serde(default)]
    pub p1_gen: [bool; 3],
    #[serde(default)]
    pub p1_dt_hi: [u16; 3],
    #[serde(default)]
    pub p1_dt_lo: [u16; 3],
    #[serde(default)]
    pub p1_fault_force: bool,
    #[serde(default)]
    pub p1_fault_latched: bool,
    #[serde(default)]
    pub p1_fault_release: bool,
    #[serde(default)]
    pub flta_prev: bool,
    #[serde(default)]
    pub rpinr12: u16,
    #[serde(default)]
    pub rpinr14: u16,
    #[serde(default)]
    pub rpinr15: u16,
    #[serde(default)]
    pub ifs3: u16,
    #[serde(default)]
    pub iec3: u16,
    #[serde(default)]
    pub ipc14: u16,
    #[serde(default)]
    pub ipc15: u16,
    #[serde(default)]
    pub qei1con: u16,
    #[serde(default)]
    pub dflt1con: u16,
    #[serde(default)]
    pub pos1cnt: u16,
    #[serde(default)]
    pub max1cnt: u16,
    #[serde(default)]
    pub qei_prev_ab: u8,
    #[serde(default)]
    pub qei_index_prev: bool,
    #[serde(default)]
    pub qei_filt: [u8; 3],
    #[serde(default)]
    pub qei_filt_cnt: [u16; 3],
    #[serde(default)]
    pub qei_filt_out: [bool; 3],
    #[serde(default)]
    pub qei_frac: u16,
    #[serde(default)]
    pub qei_timer_prev: bool,
    #[serde(default)]
    pub qei_ext_psc: u16,
    // M3: ADC1 / SPI1 / I2C1.
    #[serde(default)]
    pub ad1con1: u16,
    #[serde(default)]
    pub ad1con2: u16,
    #[serde(default)]
    pub ad1con3: u16,
    #[serde(default)]
    pub ad1chs123: u16,
    #[serde(default)]
    pub ad1chs0: u16,
    #[serde(default)]
    pub ad1pcfgl: u16,
    #[serde(default)]
    pub ad1cssl: u16,
    #[serde(default)]
    pub adc_buf: [u16; 16],
    #[serde(default)]
    pub adc_analog: [u16; 16],
    /// 0 = off, 1 = sampling, 2 = converting.
    #[serde(default)]
    pub adc_state: u8,
    #[serde(default)]
    pub adc_conv_remaining: u32,
    #[serde(default)]
    pub adc_sample_remaining: u32,
    #[serde(default)]
    pub adc_conv_count: u16,
    #[serde(default)]
    pub adc_seq_pos: u8,
    #[serde(default)]
    pub adc_buf_pos: u8,
    #[serde(default)]
    pub adc_half: u8,
    #[serde(default)]
    pub adc_alt: bool,
    #[serde(default)]
    pub adc_scan_next: u8,
    #[serde(default)]
    pub adc_t3_trigger: bool,
    #[serde(default)]
    pub adc_pwm_trigger: bool,
    #[serde(default)]
    pub spi1stat: u16,
    #[serde(default)]
    pub spi1con1: u16,
    #[serde(default)]
    pub spi1con2: u16,
    #[serde(default)]
    pub spi1_rx: u16,
    #[serde(default)]
    pub spi1_tx: u16,
    #[serde(default)]
    pub spi1_tx_pending: Option<u16>,
    #[serde(default)]
    pub spi1_busy: u32,
    #[serde(default)]
    pub spi1_shifting: bool,
    #[serde(default)]
    pub i2c1con: u16,
    #[serde(default)]
    pub i2c1stat: u16,
    #[serde(default)]
    pub i2c1add: u16,
    #[serde(default)]
    pub i2c1msk: u16,
    #[serde(default)]
    pub i2c1brg: u16,
    #[serde(default)]
    pub i2c1rcv: u16,
    #[serde(default)]
    pub i2c1trn: u16,
    #[serde(default)]
    pub i2c_target: Option<usize>,
    #[serde(default)]
    pub i2c_reading: bool,
    #[serde(default)]
    pub i2c_addressed: bool,
    #[serde(default)]
    pub i2c_10bit_prefix: Option<u16>,
    #[serde(default)]
    pub i2c_busy: u32,
    /// 0 = start, 1 = repeated start, 2 = transmit, 3 = receive, 4 = ack,
    /// 5 = stop.
    #[serde(default)]
    pub i2c_op: Option<u8>,
    #[serde(default)]
    pub ifs1: u16,
    #[serde(default)]
    pub iec1: u16,
    #[serde(default)]
    pub ipc3: u16,
    #[serde(default)]
    pub ipc4: u16,
}

/// PIC18 visible architectural state (byte-addressed PC, bank selector,
/// FSRs and the hardware return stack). Data memory is not part of the CPU
/// snapshot: it belongs to the core's private file store for now.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Pic18CpuSnapshot {
    pub wreg: u8,
    pub status: u8,
    pub bsr: u8,
    pub pc: u32,
    pub cycles: u64,
    pub fsr: [u16; 3],
    pub tblptr: u32,
    pub tablat: u8,
    pub pclath: u8,
    pub pclatu: u8,
    #[serde(default)]
    pub prod: u16,
    pub stkptr: u8,
    pub stack: Vec<u32>,
    #[serde(default)]
    pub stk_full: bool,
    #[serde(default)]
    pub stk_unf: bool,
    #[serde(default)]
    pub asleep: bool,
}

/// PIC16 visible architectural state (word-addressed PC, W, the STATUS/FSR/
/// PCLATH core registers and the eight-level return stack). Data memory is
/// the core's private file store and is not part of the CPU snapshot.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Pic16CpuSnapshot {
    pub wreg: u8,
    pub status: u8,
    pub fsr: u8,
    pub pclath: u8,
    pub pc: u32,
    pub cycles: u64,
    pub stkptr: u8,
    pub stack: Vec<u32>,
    #[serde(default)]
    pub asleep: bool,
    /// Enhanced mid-range only: BSR, FSR0H (FSR0L is `fsr`) and FSR1.
    #[serde(default)]
    pub bsr: u8,
    #[serde(default)]
    pub fsr0h: u8,
    #[serde(default)]
    pub fsr1: u16,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ArmCpuSnapshot {
    pub registers: Vec<u32>,
    pub pc: u32,
    pub xpsr: u32,
    pub primask: bool,
    pub pending_exceptions: u64,
    /// Exceptions 64..255 (words 1..3 of the widened bitmask). Optional so
    /// pre-widening snapshots still deserialize.
    #[serde(default)]
    pub pending_exceptions_hi: Vec<u64>,
    pub vtor: u32,
    #[serde(default)]
    pub waiting_for_event: bool,
    #[serde(default)]
    pub event_register: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RiscVCpuSnapshot {
    pub registers: Vec<u32>,
    pub pc: u32,

    pub mstatus: u32,
    pub mie: u32,
    pub mip: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,

    pub mtime: u64,
    pub mtimecmp: u64,

    /// Debug trigger module (hardware breakpoints / watchpoints). Absent in
    /// snapshots written before it was captured; those still load, with the
    /// trigger module at reset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triggers: Option<crate::cpu::riscv::trigger::TriggerSnapshot>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct XtensaLx7CpuSnapshot {
    /// Logical AR registers a0..a15 in the current window.
    pub registers: Vec<u32>,
    pub pc: u32,
    /// Raw PS register value.
    pub ps: u32,
    /// WindowBase (from ArFile).
    pub window_base: u8,
    /// WindowStart (from ArFile).
    pub window_start: u16,
    /// VECBASE SR value.
    pub vecbase: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm_cpu() -> CpuSnapshot {
        CpuSnapshot::Arm(ArmCpuSnapshot {
            registers: Vec::new(),
            pc: 0,
            xpsr: 0,
            primask: false,
            pending_exceptions: 0,
            pending_exceptions_hi: Vec::new(),
            vtor: 0,
            waiting_for_event: false,
            event_register: false,
        })
    }

    #[test]
    fn snapshot_json_orders_peripherals_by_name() {
        let mut peripherals = HashMap::new();
        for name in ["zeta", "beta", "alpha", "gamma"] {
            peripherals.insert(name.to_owned(), serde_json::json!({ "name": name }));
        }

        let snapshot = MachineSnapshot {
            schema_version: SCHEMA_VERSION,
            cpu: arm_cpu(),
            peripherals,
        };

        let json = serde_json::to_string(&snapshot).expect("serialize snapshot");
        let alpha = json.find("\"alpha\"").expect("alpha key");
        let beta = json.find("\"beta\"").expect("beta key");
        let gamma = json.find("\"gamma\"").expect("gamma key");
        let zeta = json.find("\"zeta\"").expect("zeta key");
        assert!(alpha < beta && beta < gamma && gamma < zeta, "{json}");
    }
}
