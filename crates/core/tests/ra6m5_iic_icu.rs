// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RA6M5 IIC1 talks to a BME280 and its interrupt requests reach the core
//! through the ICU. FSP `r_iic_master` sequences every transfer from the
//! RXI/TXI/TEI/ERI events (ELC_EVENT_IIC1_RXI..ERI 0x078-0x07B, FSP ra6m5
//! `bsp_elc.h`), which only `IELSRn` links to NVIC lines (Zephyr's ek_ra6m5.dts
//! puts them on 91-94). Built from the chip YAML and
//! examples/ek-ra6m5/bme280-system.yaml, so the `ra_iic` wiring and the
//! BME280 attachment are under test: the BME280 chip-ID read below is the
//! one Zephyr's driver makes first (register 0xD0, then a restart and a
//! one-byte read: 0x60).

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

const IIC1: u64 = 0x4009_F100;
const ICCR1: u64 = IIC1;
const ICCR2: u64 = IIC1 + 0x01;
const ICMR3: u64 = IIC1 + 0x04;
const ICIER: u64 = IIC1 + 0x07;
const ICSR2: u64 = IIC1 + 0x09;
const ICBRL: u64 = IIC1 + 0x10;
const ICBRH: u64 = IIC1 + 0x11;
const ICDRT: u64 = IIC1 + 0x12;
const ICDRR: u64 = IIC1 + 0x13;

const ST: u8 = 1 << 1;
const RS: u8 = 1 << 2;
const SP: u8 = 1 << 3;
const BBSY: u8 = 1 << 7;
const TEND: u8 = 1 << 6;
const NACKF: u8 = 1 << 4;
const STOP: u8 = 1 << 3;
const ACKWP: u8 = 1 << 4;
const ACKBT: u8 = 1 << 3;
/// FSP IIC_MASTER_INTERRUPT_ENABLE_INIT_MASK (TIE RIE NAKIE ALIE TMOIE)
/// plus STIE and SPIE, what `iic_master_run_hw_master` enables.
const RUN_IER: u8 = 0xB3 | 0x04 | 0x08;
const TEIE: u8 = 1 << 6;
const TIE: u8 = 1 << 7;

const IELSR0: u64 = 0x4000_6300;
const IR: u32 = 1 << 16;
const RXI: u32 = 0x078;
/// ek_ra6m5.dts `iic1` lines: rxi 91, txi 92, tei 93, eri 94.
const LINE_RXI: u32 = 91;
const TABLE: u32 = 0x2000_0400;
const IDLE: u32 = 0x2000_2000;
const HANDLER: u32 = 0x2000_2100;
const NVIC_ISER: u64 = 0xE000_E100;
const SCB_VTOR: u64 = 0xE000_ED08;

const BME280: u8 = 0x76;

fn machine(system: &str) -> Machine<CortexM> {
    let chip_path = root("configs/chips/r7fa6m5bh.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("r7fa6m5bh chip");
    let mut manifest = SystemManifest::from_file(root(system)).expect("ek-ra6m5 system");
    manifest.chip = chip_path.to_str().expect("utf8").to_string();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.bus.write_u16(u64::from(IDLE), 0xE7FE).unwrap();
    m.bus.write_u16(u64::from(HANDLER), 0xE7FE).unwrap();
    for n in 0..4 {
        m.bus
            .write_u32(u64::from(TABLE + 4 * (16 + LINE_RXI + n)), HANDLER | 1)
            .unwrap();
    }
    m.bus.write_u32(SCB_VTOR, TABLE).unwrap();
    m.cpu.set_pc(IDLE);
    m.cpu.set_sp(0x2000_7F00);
    // Link IELSR91-94 to IIC1 RXI/TXI/TEI/ERI, as Zephyr's driver does.
    for n in 0..4 {
        m.bus
            .write_u32(IELSR0 + 4 * u64::from(LINE_RXI + n), RXI + n)
            .unwrap();
    }
    // FSP iic_master_open_hw_master: off, reset, configure under reset,
    // release. BRL = BRH = 2 at CKS 0: a 10-count SCL period.
    m.bus.write_u8(ICCR1, 0x1F).unwrap();
    m.bus.write_u8(ICCR1, 0x5F).unwrap();
    m.bus.write_u8(ICCR1, 0xDF).unwrap();
    m.bus.write_u8(ICBRL, 0xE2).unwrap();
    m.bus.write_u8(ICBRH, 0xE2).unwrap();
    m.bus.write_u8(ICIER, 0xB3).unwrap();
    m.bus.write_u8(ICCR1, 0x9F).unwrap();
    m
}

/// `IELSRn.IR` of `event`'s line (RXI + 0..3), cleared on the way out like
/// `R_BSP_IrqStatusClear`.
fn take_ir(m: &mut Machine<CortexM>, event: u32) -> bool {
    let reg = IELSR0 + 4 * u64::from(LINE_RXI + (event - RXI));
    let set = m.bus.read_u32(reg).unwrap() & IR != 0;
    if set {
        m.bus.write_u32(reg, event).unwrap();
    }
    set
}

/// Step until `event` latches its IR (and clear it).
fn wait_event(m: &mut Machine<CortexM>, event: u32) -> bool {
    for _ in 0..20_000 {
        if take_ir(m, event) {
            return true;
        }
        m.step().unwrap();
    }
    false
}

fn rmw(m: &mut Machine<CortexM>, reg: u64, set: u8, clear: u8) {
    let v = m.bus.read_u8(reg).unwrap();
    m.bus.write_u8(reg, (v | set) & !clear).unwrap();
}

#[test]
fn start_vectors_the_core_through_the_txi_line() {
    let mut m = machine("examples/ek-ra6m5/bme280-system.yaml");
    let txi_line = LINE_RXI + 1;
    m.bus
        .write_u32(
            NVIC_ISER + 4 * u64::from(txi_line / 32),
            1 << (txi_line % 32),
        )
        .unwrap();
    m.bus.write_u8(ICIER, RUN_IER).unwrap();
    m.bus.write_u8(ICCR2, ST).unwrap();
    let mut vectored = false;
    for _ in 0..20_000 {
        if m.cpu.get_pc() == HANDLER {
            vectored = true;
            break;
        }
        m.step().unwrap();
    }
    assert!(
        vectored,
        "the start condition's TXI never reached IELSR{txi_line}"
    );
    assert_eq!(
        m.bus.read_u32(IELSR0 + 4 * u64::from(txi_line)).unwrap(),
        IR | (RXI + 1)
    );
    assert_ne!(m.bus.read_u8(ICCR2).unwrap() & BBSY, 0, "bus owned");
}

/// The FSP event sequence of Zephyr's first BME280 access
/// (`i2c_burst_read(dev, 0x76, 0xD0, &id, 1)`): write 0xD0 with a restart,
/// read one byte with a stop.
#[test]
fn bme280_chip_id_read_through_the_icu_events() {
    let (txi, tei, eri) = (RXI + 1, RXI + 2, RXI + 3);
    let mut m = machine("examples/ek-ra6m5/bme280-system.yaml");
    m.bus.write_u8(ICIER, RUN_IER).unwrap();
    m.bus.write_u8(ICCR2, ST).unwrap();
    assert!(wait_event(&mut m, txi), "start: TXI");
    assert!(wait_event(&mut m, eri), "start: ERI (START)");
    m.bus.write_u8(ICSR2, 0xE0).unwrap();
    m.bus.write_u8(ICDRT, BME280 << 1).unwrap();
    assert!(wait_event(&mut m, txi), "address in the shifter: TXI");
    m.bus.write_u8(ICDRT, 0xD0).unwrap();
    rmw(&mut m, ICIER, TEIE, TIE);
    assert!(wait_event(&mut m, tei), "register byte sent: TEI");
    assert_ne!(m.bus.read_u8(ICSR2).unwrap() & TEND, 0);
    // Restart: TIE back on, RS.
    rmw(&mut m, ICIER, TIE, TEIE);
    m.bus.write_u8(ICCR2, RS).unwrap();
    assert!(wait_event(&mut m, eri), "restart: ERI (START)");
    take_ir(&mut m, txi);
    m.bus.write_u8(ICSR2, 0xE0).unwrap();
    m.bus.write_u8(ICDRT, (BME280 << 1) | 1).unwrap();
    assert!(wait_event(&mut m, RXI), "read address ACKed: RXI");
    // One-byte read: NACK it, dummy read.
    rmw(&mut m, ICMR3, ACKWP, 0);
    rmw(&mut m, ICMR3, ACKBT, 0);
    rmw(&mut m, ICMR3, 0, ACKWP);
    let _ = m.bus.read_u8(ICDRR).unwrap();
    assert!(wait_event(&mut m, RXI), "chip ID received: RXI");
    rmw(&mut m, ICSR2, 0, STOP);
    m.bus.write_u8(ICCR2, SP).unwrap();
    assert_eq!(m.bus.read_u8(ICDRR).unwrap(), 0x60, "BME280 chip ID");
    assert!(wait_event(&mut m, eri), "stop: ERI (STOP)");
    assert_ne!(m.bus.read_u8(ICSR2).unwrap() & STOP, 0);
    assert_eq!(m.bus.read_u8(ICCR2).unwrap() & BBSY, 0, "bus released");
}

#[test]
fn an_absent_device_is_nacked_through_eri() {
    let eri = RXI + 3;
    let mut m = machine("examples/ek-ra6m5/system.yaml");
    m.bus.write_u8(ICIER, RUN_IER).unwrap();
    m.bus.write_u8(ICCR2, ST).unwrap();
    assert!(wait_event(&mut m, eri), "start: ERI (START)");
    m.bus.write_u8(ICSR2, 0xE0).unwrap();
    m.bus.write_u8(ICDRT, BME280 << 1).unwrap();
    assert!(wait_event(&mut m, eri), "no device at 0x76: ERI (NACKF)");
    assert_ne!(m.bus.read_u8(ICSR2).unwrap() & NACKF, 0);
}
