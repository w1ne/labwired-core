// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! dsPIC33FJ32MC202 × real XC16 M2 firmware: MCPWM1 + QEI1.
//!
//! `examples/dspic33fj32mc202/firmware/dspic33fj32mc202-xc16-m2-pwm-qei.elf`
//! is built by Microchip XC16 v1.70 from `xc16-m2-pwm-qei.c` (recipe in the
//! example README). The program configures an edge-aligned complementary
//! PWM1 pair with a 2 TCY dead time and a x4 index-reset QEI1, writes a
//! completion marker to 0x0E00 and loops.
//!
//! The test checks the register image, the PWM pads over a period, and then
//! injects quadrature edges on RB0/RB1/RB2 to exercise the decoder, the
//! direction status and the direction-aware index reset.
//!
//! The bus is the PIC24 pad model pair (`peripherals::pic24_gpio`) at the
//! same mirror addresses the session bus uses, so the test drives real pad
//! levels rather than stubbing the QEI inputs.
//!
//! Scope and limitations are documented in
//! `examples/dspic33fj32mc202/VALIDATION.md` and `docs/pic24-m0-reference.md`
//! §3.3.

use labwired_core::cpu::pic24::{PIC24_IO_MIRROR_BASE, SFR_TRISA};
use labwired_core::cpu::Pic24;
use labwired_core::peripherals::pic24_gpio::{self, Pic24GpioPort};
use labwired_core::system::node::parse_elf_image;
use labwired_core::{Bus, Cpu, DmaRequest, Peripheral, SimResult, SimulationConfig};
use std::collections::HashMap;
use std::path::PathBuf;

// SFR addresses (DS70283H Table 4-8 / 4-10, Table 7-2).
const P1TCON: u16 = 0x01C0;
const P1TMR: u16 = 0x01C2;
const P1TPER: u16 = 0x01C4;
const PWM1CON1: u16 = 0x01C8;
const P1DTCON1: u16 = 0x01CC;
const P1DC1: u16 = 0x01D6;
const QEI1CON: u16 = 0x01E0;
const POS1CNT: u16 = 0x01E4;
const MAX1CNT: u16 = 0x01E6;
const IFS0: u16 = 0x0084;
const T1CON: u16 = 0x0104;
const PR1: u16 = 0x0102;
const IFS3: u16 = 0x008A;
const IRQ_T1_BIT: u16 = 1 << 3;
const RPINR14: u16 = 0x069C;
const RPINR15: u16 = 0x069E;

const QEICON_UPDN: u16 = 0x0800;
const QEICON_INDEX: u16 = 0x1000;
const IRQ_QEI_BIT: u16 = 1 << 9;
const IRQ_PWM1_BIT: u16 = 1 << 8;

/// A bus with the three PIC24 port pad models at the session mirror
/// addresses, enough for the PWM outputs and the QEI/fault inputs.
struct PadBus {
    ports: [Pic24GpioPort; 3],
    mem: HashMap<u64, u8>,
    config: SimulationConfig,
}

impl PadBus {
    fn new() -> Self {
        Self {
            ports: [
                Pic24GpioPort::new(0x001F),
                Pic24GpioPort::new(0xFFFF),
                Pic24GpioPort::new(0x0000),
            ],
            mem: HashMap::new(),
            config: SimulationConfig::default(),
        }
    }

    fn route(addr: u64) -> Option<(usize, u64)> {
        for port in 0..3 {
            let base = PIC24_IO_MIRROR_BASE + u64::from(SFR_TRISA + 0x10 * port as u16);
            if (base..base + pic24_gpio::WINDOW_BYTES).contains(&addr) {
                return Some((port, addr - base));
            }
        }
        None
    }

    fn set_input(&mut self, port: usize, pin: u8, level: bool) {
        use labwired_core::pins::{External, PinPort};
        self.ports[port].set_external(pin, External::Level(level));
    }

    /// The live pad level (the same value `PORTx` reads).
    fn pad(&self, port: usize, pin: u8) -> bool {
        use labwired_core::pins::PinPort;
        self.ports[port].input(pin).unwrap_or(false)
    }
}

impl Bus for PadBus {
    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        if let Some((port, off)) = Self::route(addr) {
            return self.ports[port].read(off);
        }
        Ok(*self.mem.get(&addr).unwrap_or(&0))
    }

    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
        if let Some((port, off)) = Self::route(addr) {
            return self.ports[port].write(off, value);
        }
        self.mem.insert(addr, value);
        Ok(())
    }

    fn tick_peripherals(&mut self) -> Vec<u32> {
        Vec::new()
    }

    fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
        Ok(())
    }

    fn config(&self) -> &SimulationConfig {
        &self.config
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_until_marker() -> (Pic24, PadBus) {
    let root = repo_root();
    let elf = std::fs::read(
        root.join("examples/dspic33fj32mc202/firmware/dspic33fj32mc202-xc16-m2-pwm-qei.elf"),
    )
    .expect("read XC16 M2 ELF");
    let image = parse_elf_image(&elf).expect("parse ELF");
    assert_eq!(image.arch, labwired_core::Arch::Pic24);

    let mut bus = PadBus::new();
    let mut cpu = Pic24::new();
    cpu.set_port_masks([0x001F, 0xFFFF, 0x0000]);
    cpu.load_program_image(&image);
    cpu.reset(&mut bus).expect("reset");
    cpu.set_pc(0);

    for _ in 0..4000 {
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .expect("M2 firmware instruction must decode and execute");
        if cpu.data_read(0x0E00).unwrap() == 0xBEEF {
            return (cpu, bus);
        }
    }
    panic!("the M2 firmware never reached its completion marker");
}

fn step(cpu: &mut Pic24, bus: &mut PadBus) {
    cpu.step(bus, &[], &SimulationConfig::default()).unwrap();
}

fn pwm_count(cpu: &mut Pic24) -> u16 {
    cpu.data_read(P1TMR).unwrap() & 0x7FFF
}

#[test]
fn xc16_m2_firmware_configures_pwm_and_qei() {
    let (mut cpu, mut bus) = run_until_marker();

    // T1: the shared timer fabric runs on the 28-pin part.
    assert_eq!(cpu.data_read(T1CON).unwrap(), 0x8000, "T1 on, 1:1");
    assert_eq!(cpu.data_read(PR1).unwrap(), 3, "T1 period register");
    assert_ne!(
        cpu.data_read(IFS0).unwrap() & IRQ_T1_BIT,
        0,
        "T1IF after the period match"
    );

    // Register image written by the firmware.
    assert_eq!(
        cpu.data_read(P1TCON).unwrap(),
        0x8000,
        "PTEN, free run, 1:1"
    );
    assert_eq!(cpu.data_read(P1TPER).unwrap(), 15);
    assert_eq!(cpu.data_read(P1DC1).unwrap(), 5);
    assert_eq!(
        cpu.data_read(PWM1CON1).unwrap(),
        0x0011,
        "PEN1H|PEN1L, complementary"
    );
    assert_eq!(cpu.data_read(P1DTCON1).unwrap(), 2, "dead time A = 2 TCY");
    assert_eq!(cpu.data_read(RPINR14).unwrap(), 0x0100, "QEA=RP0, QEB=RP1");
    assert_eq!(cpu.data_read(RPINR15).unwrap(), 2, "INDX=RP2");
    assert_eq!(cpu.data_read(MAX1CNT).unwrap(), 199);
    assert_eq!(
        cpu.data_read(QEI1CON).unwrap() & 0x0704,
        0x0604,
        "QEIM=110 (x4 index reset), POSRES"
    );

    // PWM: restart the time base deterministically and watch pair 1.
    cpu.data_write(P1TCON, 0).unwrap();
    cpu.data_write(P1TMR, 0).unwrap();
    cpu.data_write(P1TCON, 0x8000).unwrap();
    for _ in 0..8 {
        if pwm_count(&mut cpu) >= 2 {
            break;
        }
        step(&mut cpu, &mut bus);
    }
    assert!(pwm_count(&mut cpu) >= 2, "time base advances");
    assert!(bus.pad(1, 14), "PWM1H1 active after the dead time");
    assert!(!bus.pad(1, 15), "complementary low side is off");

    // Clear the flag raised while the firmware was initialising, so the loop
    // below observes the upcoming low phase first.
    cpu.data_write(IFS3, 0).unwrap();

    // Run through the duty match and the period match; the low side must go
    // active at some point and the PWM1IF must be raised at TMR = 0.
    let mut saw_low = false;
    let mut saw_flag = false;
    for _ in 0..40 {
        step(&mut cpu, &mut bus);
        saw_low |= bus.pad(1, 15);
        if cpu.data_read(IFS3).unwrap() & IRQ_PWM1_BIT != 0 {
            saw_flag = true;
            break;
        }
    }
    assert!(saw_low, "low side active during the low phase");
    assert!(saw_flag, "PWM1IF on the period match");
    // A new period drives the high side active again (after the dead time).
    for _ in 0..8 {
        if pwm_count(&mut cpu) >= 2 {
            break;
        }
        step(&mut cpu, &mut bus);
    }
    assert!(bus.pad(1, 14), "next period's high side");

    // QEI: a full forward quadrature cycle counts four edges.
    cpu.data_write(IFS3, 0).unwrap();
    for (a, b) in [(true, false), (true, true), (false, true), (false, false)] {
        bus.set_input(1, 0, a);
        bus.set_input(1, 1, b);
        step(&mut cpu, &mut bus);
    }
    assert_eq!(cpu.data_read(POS1CNT).unwrap(), 4, "x4 forward");
    assert_ne!(
        cpu.data_read(QEI1CON).unwrap() & QEICON_UPDN,
        0,
        "UPDN = forward"
    );

    // Index rising edge at state 00 with IMV = 00 resets forward to 0.
    bus.set_input(1, 2, true);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.data_read(POS1CNT).unwrap(), 0, "forward index reset");
    assert_ne!(
        cpu.data_read(IFS3).unwrap() & IRQ_QEI_BIT,
        0,
        "QEIIF on the index"
    );
    assert_ne!(
        cpu.data_read(QEI1CON).unwrap() & QEICON_INDEX,
        0,
        "INDEX status bit"
    );

    // Reverse two edges: 0 -> 9 -> 8 (POSCNT was preset) and UPDN clears.
    cpu.data_write(IFS3, 0).unwrap();
    cpu.data_write(POS1CNT, 10).unwrap();
    for (a, b) in [(false, true), (true, true)] {
        bus.set_input(1, 0, a);
        bus.set_input(1, 1, b);
        step(&mut cpu, &mut bus);
    }
    assert_eq!(cpu.data_read(POS1CNT).unwrap(), 8, "x4 reverse");
    assert_eq!(
        cpu.data_read(QEI1CON).unwrap() & QEICON_UPDN,
        0,
        "UPDN = reverse"
    );

    // An index pulse at state 11 does not match IMV = 00: no reset, no flag.
    bus.set_input(1, 2, false);
    step(&mut cpu, &mut bus);
    bus.set_input(1, 2, true);
    step(&mut cpu, &mut bus);
    assert_eq!(
        cpu.data_read(POS1CNT).unwrap(),
        8,
        "index at a non-matching state is ignored"
    );
    assert_eq!(cpu.data_read(IFS3).unwrap() & IRQ_QEI_BIT, 0);

    // Back to state 00, then a reverse index loads MAXCNT (199).
    for (a, b) in [(true, false), (false, false)] {
        bus.set_input(1, 0, a);
        bus.set_input(1, 1, b);
        step(&mut cpu, &mut bus);
    }
    assert_eq!(cpu.data_read(POS1CNT).unwrap(), 6);
    bus.set_input(1, 2, false);
    step(&mut cpu, &mut bus);
    bus.set_input(1, 2, true);
    step(&mut cpu, &mut bus);
    assert_eq!(
        cpu.data_read(POS1CNT).unwrap(),
        199,
        "reverse index reset loads MAXCNT"
    );
    assert_ne!(cpu.data_read(IFS3).unwrap() & IRQ_QEI_BIT, 0);
}
