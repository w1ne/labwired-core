// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ATmega328P external interrupts (INT0/INT1) and pin-change interrupts
//! (PCINT0..2), plus the `SLEEP` instruction they wake the core from.
//!
//! Registers (data-space addresses, datasheet section 13):
//!
//! | Register | Addr | Bits |
//! |----------|------|------|
//! | `PCIFR`  | 0x3B | PCIF2..0, cleared by writing 1 or on vector entry |
//! | `EIFR`   | 0x3C | INTF1..0, cleared by writing 1 or on vector entry |
//! | `EIMSK`  | 0x3D | INT1..0 enables |
//! | `SMCR`   | 0x53 | SM2..0, SE |
//! | `PCICR`  | 0x68 | PCIE2..0 enables |
//! | `EICRA`  | 0x69 | ISC11:10 (INT1), ISC01:00 (INT0) |
//! | `PCMSK0..2` | 0x6B..0x6D | per-pin PCINT enables (port B, C, D) |
//!
//! The pads are sampled at every instruction boundary while anything is
//! configured to look at them, as `PINx` reads them: the level the bus-side
//! port model holds for an input (a `board_io` button, a `gpio_net` delivery,
//! anything that calls `set_gpio_input`) and the core's own `PORTx` latch for
//! an output, so a pad the firmware drives raises its own interrupt as on
//! silicon. An edge raised from outside between two `advance` calls is seen
//! at the first boundary after it, and its vector is entered there.
//!
//! Semantics kept from the datasheet:
//! - a flag is set on a matching edge whether or not its interrupt is enabled,
//!   and an enabled interrupt with a flag already set fires at once;
//! - writing 1 to a flag clears it; entering the vector clears it too;
//! - INT0/INT1 in low-level mode set no flag (INTFn reads 0) and request the
//!   interrupt for as long as the pad is low and the interrupt is enabled;
//! - INT0/INT1 edge detection needs the I/O clock, so while the core sleeps in
//!   power-down, power-save or (extended) standby only a low level wakes it
//!   through INT0/INT1; pin-change interrupts are asynchronous and always do.

use super::{Avr, AVR_IO_MIRROR_BASE, AVR_PINB, AVR_PINC, AVR_PIND};
use crate::Bus;

// Pending-bit numbers follow this core's convention (`VEC_TIMER0_OVF`,
// `VEC_USART_RX`): the datasheet's 1-based "Vector No." with RESET = 1, so the
// handler is at byte `(vec - 1) * 4`. avr-libc numbers the same vectors from
// 0: `INT0_vect` is `__vector_1`.

/// `INT0_vect` (`__vector_1`, byte 0x004).
pub const VEC_INT0: u32 = 2;
/// `INT1_vect` (`__vector_2`, byte 0x008).
pub const VEC_INT1: u32 = 3;
/// `PCINT0_vect` (`__vector_3`, byte 0x00C): port B.
pub const VEC_PCINT0: u32 = 4;
/// `PCINT1_vect` (`__vector_4`, byte 0x010): port C.
pub const VEC_PCINT1: u32 = 5;
/// `PCINT2_vect` (`__vector_5`, byte 0x014): port D.
pub const VEC_PCINT2: u32 = 6;

pub(super) const ADDR_PCIFR: u16 = 0x3B;
pub(super) const ADDR_EIFR: u16 = 0x3C;
pub(super) const ADDR_EIMSK: u16 = 0x3D;
pub(super) const ADDR_SMCR: u16 = 0x53;
pub(super) const ADDR_PCICR: u16 = 0x68;
pub(super) const ADDR_EICRA: u16 = 0x69;
pub(super) const ADDR_PCMSK0: u16 = 0x6B;
pub(super) const ADDR_PCMSK2: u16 = 0x6D;

/// `PINx` of port B, C, D: the index is the PCINT group.
const PORT_PIN: [u16; 3] = [AVR_PINB, AVR_PINC, AVR_PIND];
/// INT0 is PD2, INT1 is PD3.
const PORT_D: usize = 2;
const INT_PAD_SHIFT: u32 = 2;
/// `pending_irq` bits this module owns: `VEC_INT0..=VEC_PCINT2`.
const EXT_PENDING_MASK: u64 = 0b111_1100;
/// `SMCR.SE`.
const SMCR_SE: u8 = 1 << 0;

impl Avr {
    /// The pads of port `group` (0 = B, 1 = C, 2 = D) as `PINx` reads them.
    pub(super) fn pad_levels(&self, group: usize, bus: &dyn Bus) -> u8 {
        let pin = PORT_PIN[group];
        let ddr = self.io[(pin + 1 - 0x20) as usize];
        let port = self.io[(pin + 2 - 0x20) as usize];
        // No port window on the bus: no outside world to sense, the inputs
        // read low, exactly as the core's PINx read would fail to say more.
        let external = bus
            .read_u8(AVR_IO_MIRROR_BASE + u64::from(pin))
            .unwrap_or(0);
        (port & ddr) | (external & !ddr)
    }

    /// INTn sense control: 0 low level, 1 any change, 2 falling, 3 rising.
    #[inline]
    fn isc(&self, n: usize) -> u8 {
        (self.eicra >> (2 * n)) & 3
    }

    /// `SMCR.SM2..0` of the current sleep.
    #[inline]
    fn sleep_mode(&self) -> u8 {
        (self.smcr >> 1) & 7
    }

    /// Does clk_I/O run in the current state? Always when awake; asleep only in
    /// idle (0) and ADC noise reduction (1). Timer0 and INT0/INT1 edge
    /// detection stop with it.
    #[inline]
    pub(super) fn io_clock_running(&self) -> bool {
        !self.sleeping || self.sleep_mode() <= 1
    }

    /// Which ports must be sampled at every boundary: a group with any PCMSK
    /// bit set, and port D while INT0/INT1 sense an edge (flags are kept even
    /// when masked) or are enabled (a low level must be tracked).
    fn compute_ext_watch(&self) -> u8 {
        let mut watch = 0u8;
        for (group, mask) in self.pcmsk.iter().enumerate() {
            if *mask != 0 {
                watch |= 1 << group;
            }
        }
        if self.eicra & 0x0F != 0 || self.eimsk & 0x03 != 0 {
            watch |= 1 << PORT_D;
        }
        watch
    }

    /// Re-arm after a write to `EICRA`, `EIMSK`, `PCICR` or `PCMSKx`. A port
    /// that starts being watched takes its present level as the reference,
    /// so configuring an interrupt never invents an edge.
    pub(super) fn ext_config_changed(&mut self, bus: &dyn Bus) {
        let watch = self.compute_ext_watch();
        let fresh = watch & !self.ext_watch;
        for group in 0..3 {
            if fresh & (1 << group) != 0 {
                self.ext_last[group] = self.pad_levels(group, bus);
            }
        }
        self.ext_watch = watch;
        for n in 0..2 {
            if self.isc(n) == 0 {
                // "INTFn is always cleared when INTn is configured as a level
                // interrupt."
                self.eifr &= !(1 << n);
            }
        }
        self.sync_ext_pending();
    }

    /// Sample every watched port at an instruction boundary and latch flags
    /// for the edges since the last sample.
    pub(super) fn sample_ext_pins(&mut self, bus: &dyn Bus) {
        for group in 0..3 {
            if self.ext_watch & (1 << group) == 0 {
                continue;
            }
            let now = self.pad_levels(group, bus);
            let changed = now ^ self.ext_last[group];
            self.ext_last[group] = now;
            if changed == 0 {
                continue;
            }
            if changed & self.pcmsk[group] != 0 {
                self.pcifr |= 1 << group;
            }
            if group == PORT_D && self.io_clock_running() {
                for n in 0..2 {
                    let bit = 1u8 << (INT_PAD_SHIFT as usize + n);
                    if changed & bit == 0 {
                        continue;
                    }
                    let rose = now & bit != 0;
                    let hit = match self.isc(n) {
                        1 => true,
                        2 => !rose,
                        3 => rose,
                        _ => false,
                    };
                    if hit {
                        self.eifr |= 1 << n;
                    }
                }
            }
        }
        self.sync_ext_pending();
    }

    /// `true` when a watched pad moved since the last sample (an edge the core
    /// has not looked at yet).
    pub(super) fn ext_pins_moved(&self, bus: &dyn Bus) -> bool {
        (0..3).any(|group| {
            self.ext_watch & (1 << group) != 0
                && self.pad_levels(group, bus) != self.ext_last[group]
        })
    }

    /// Recompute the pending bits of `VEC_INT0..=VEC_PCINT2` from flags, enables and
    /// (for a low-level INTn) the pad level.
    pub(super) fn sync_ext_pending(&mut self) {
        let mut want = 0u64;
        for n in 0..2 {
            if self.eimsk & (1 << n) == 0 {
                continue;
            }
            let active = if self.isc(n) == 0 {
                self.ext_last[PORT_D] & (1 << (INT_PAD_SHIFT as usize + n)) == 0
            } else {
                self.eifr & (1 << n) != 0
            };
            if active {
                want |= 1u64 << (VEC_INT0 + n as u32);
            }
        }
        for group in 0..3u32 {
            if self.pcicr & self.pcifr & (1 << group) != 0 {
                want |= 1u64 << (VEC_PCINT0 + group);
            }
        }
        self.pending_irq = (self.pending_irq & !EXT_PENDING_MASK) | want;
    }

    /// Hardware clears the flag of the vector it enters.
    #[inline]
    pub(super) fn ext_vector_entered(&mut self, vec: u32) {
        match vec {
            VEC_INT0 | VEC_INT1 => self.eifr &= !(1 << (vec - VEC_INT0)),
            VEC_PCINT0..=VEC_PCINT2 => self.pcifr &= !(1 << (vec - VEC_PCINT0)),
            _ => {}
        }
    }

    /// Register read for this block, `None` for any other address.
    pub(super) fn ext_read(&self, addr: u16) -> Option<u8> {
        Some(match addr {
            ADDR_PCIFR => self.pcifr,
            ADDR_EIFR => self.eifr,
            ADDR_EIMSK => self.eimsk,
            ADDR_SMCR => self.smcr,
            ADDR_PCICR => self.pcicr,
            ADDR_EICRA => self.eicra,
            ADDR_PCMSK0..=ADDR_PCMSK2 => self.pcmsk[(addr - ADDR_PCMSK0) as usize],
            _ => return None,
        })
    }

    /// Register write for this block; `false` for any other address.
    pub(super) fn ext_write(&mut self, addr: u16, value: u8, bus: &dyn Bus) -> bool {
        match addr {
            ADDR_PCIFR => {
                self.pcifr &= !(value & 0x07);
                self.sync_ext_pending();
            }
            ADDR_EIFR => {
                self.eifr &= !(value & 0x03);
                self.sync_ext_pending();
            }
            ADDR_EIMSK => {
                self.eimsk = value & 0x03;
                self.ext_config_changed(bus);
            }
            ADDR_SMCR => self.smcr = value & 0x0F,
            ADDR_PCICR => {
                self.pcicr = value & 0x07;
                self.ext_config_changed(bus);
            }
            ADDR_EICRA => {
                self.eicra = value & 0x0F;
                self.ext_config_changed(bus);
            }
            ADDR_PCMSK0..=ADDR_PCMSK2 => {
                self.pcmsk[(addr - ADDR_PCMSK0) as usize] = value;
                self.ext_config_changed(bus);
            }
            _ => return false,
        }
        true
    }

    /// `SLEEP`: the core stops if `SMCR.SE` is set, otherwise it is a NOP.
    pub(super) fn exec_sleep(&mut self) {
        if self.smcr & SMCR_SE != 0 {
            self.sleeping = true;
        }
    }

    /// Clear this block on reset.
    pub(super) fn ext_reset(&mut self) {
        self.eicra = 0;
        self.eimsk = 0;
        self.eifr = 0;
        self.pcicr = 0;
        self.pcifr = 0;
        self.pcmsk = [0; 3];
        self.smcr = 0;
        self.sleeping = false;
        self.ext_last = [0; 3];
        self.ext_watch = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::avr::{TIMSK_TOIE0, VEC_TIMER0_OVF};
    use crate::{Cpu, DmaRequest, SimResult, SimulationConfig};
    use std::collections::HashMap;

    /// Data memory plus the bus-side `PINx` the outside world holds.
    struct PadBus {
        mem: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl Bus for PadBus {
        fn read_u8(&self, addr: u64) -> SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
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

    const MAIN: u32 = 0x68;
    const PB: u16 = AVR_PINB;
    const PC: u16 = AVR_PINC;
    const PD: u16 = AVR_PIND;

    /// Each vector slot of INT0..PCINT2 is `INC r20+k; RETI`, so a register
    /// counts the handler's entries; Timer0's is `INC r25; RETI`. Main is a
    /// `RJMP .` spin, interrupts enabled.
    fn rig() -> (Avr, PadBus) {
        let mut cpu = Avr::new();
        for k in 0..5u16 {
            let rd = 20 + k;
            cpu.load_words(4 + 4 * u32::from(k), &[0x9403 | (rd << 4), 0x9518]);
        }
        cpu.load_words((VEC_TIMER0_OVF - 1) * 4, &[0x9403 | (25 << 4), 0x9518]);
        cpu.load_words(MAIN, &[0xCFFF]);
        cpu.set_pc(MAIN);
        cpu.sreg = 0x80;
        let bus = PadBus {
            mem: HashMap::new(),
            config: SimulationConfig::default(),
        };
        (cpu, bus)
    }

    /// The outside world holds `levels` on the input pads of the port.
    fn hold(bus: &mut PadBus, pin: u16, levels: u8) {
        bus.mem.insert(AVR_IO_MIRROR_BASE + u64::from(pin), levels);
    }

    fn write(cpu: &mut Avr, bus: &mut PadBus, addr: u16, value: u8) {
        cpu.data_write(addr, value, bus).unwrap();
    }

    fn read(cpu: &Avr, bus: &PadBus, addr: u16) -> u8 {
        cpu.data_read(addr, bus).unwrap()
    }

    fn run(cpu: &mut Avr, bus: &mut PadBus, steps: usize) {
        let cfg = SimulationConfig::default();
        for _ in 0..steps {
            cpu.step(bus, &[], &cfg).unwrap();
        }
    }

    /// Handler entries of INT0, INT1, PCINT0, PCINT1, PCINT2.
    fn entries(cpu: &Avr) -> [u8; 5] {
        [cpu.r[20], cpu.r[21], cpu.r[22], cpu.r[23], cpu.r[24]]
    }

    #[test]
    fn vector_numbers_land_on_the_datasheet_addresses() {
        // Program address (words) 0x0002, 0x0004, 0x0006, 0x0008, 0x000A.
        for (vec, byte) in [
            (VEC_INT0, 0x04),
            (VEC_INT1, 0x08),
            (VEC_PCINT0, 0x0C),
            (VEC_PCINT1, 0x10),
            (VEC_PCINT2, 0x14),
        ] {
            assert_eq!((vec - 1) * 4, byte);
        }
    }

    #[test]
    fn int0_sense_modes_any_falling_rising() {
        // ISC01:00 = 01 any change, 10 falling, 11 rising.
        for (isc, expect) in [(1u8, 4u8), (2, 2), (3, 2)] {
            let (mut cpu, mut bus) = rig();
            write(&mut cpu, &mut bus, ADDR_EICRA, isc);
            write(&mut cpu, &mut bus, ADDR_EIMSK, 1);
            for level in [0x04, 0x00, 0x04, 0x00] {
                hold(&mut bus, PD, level);
                run(&mut cpu, &mut bus, 8);
            }
            assert_eq!(entries(&cpu), [expect, 0, 0, 0, 0], "ISC0={isc}");
            assert_eq!(cpu.eifr, 0, "vector entry cleared INTF0");
            assert_eq!(cpu.get_pc(), MAIN, "back in main after RETI");
        }
    }

    #[test]
    fn int1_rising_edge_from_outside_enters_the_vector_at_the_next_boundary() {
        let (mut cpu, mut bus) = rig();
        write(&mut cpu, &mut bus, ADDR_EICRA, 0b11 << 2);
        write(&mut cpu, &mut bus, ADDR_EIMSK, 0b10);
        run(&mut cpu, &mut bus, 4);
        assert_eq!(entries(&cpu), [0; 5]);
        hold(&mut bus, PD, 1 << 3);
        let before = cpu.cycles;
        run(&mut cpu, &mut bus, 1);
        assert_eq!(cpu.get_pc(), 0x08, "INT1_vect entered at once");
        assert_eq!(cpu.cycles - before, 4, "vector entry takes four cycles");
        assert_eq!(cpu.eifr, 0, "INTF1 cleared on entry");
    }

    #[test]
    fn a_flag_is_set_while_masked_and_fires_when_enabled() {
        let (mut cpu, mut bus) = rig();
        write(&mut cpu, &mut bus, ADDR_EICRA, 0b10); // INT0 falling
        hold(&mut bus, PD, 0x04);
        run(&mut cpu, &mut bus, 2);
        hold(&mut bus, PD, 0x00);
        run(&mut cpu, &mut bus, 4);
        assert_eq!(read(&cpu, &bus, ADDR_EIFR), 0x01, "INTF0 set though masked");
        assert_eq!(entries(&cpu), [0; 5]);

        write(&mut cpu, &mut bus, ADDR_EIMSK, 1);
        run(&mut cpu, &mut bus, 4);
        assert_eq!(entries(&cpu), [1, 0, 0, 0, 0], "a stale flag fires at once");
        assert_eq!(read(&cpu, &bus, ADDR_EIFR), 0);

        // Writing 1 clears the flag: enabling afterwards fires nothing.
        write(&mut cpu, &mut bus, ADDR_EIMSK, 0);
        hold(&mut bus, PD, 0x04);
        run(&mut cpu, &mut bus, 2);
        hold(&mut bus, PD, 0x00);
        run(&mut cpu, &mut bus, 2);
        assert_eq!(read(&cpu, &bus, ADDR_EIFR), 0x01);
        write(&mut cpu, &mut bus, ADDR_EIFR, 0x01);
        assert_eq!(read(&cpu, &bus, ADDR_EIFR), 0);
        write(&mut cpu, &mut bus, ADDR_EIMSK, 1);
        run(&mut cpu, &mut bus, 4);
        assert_eq!(entries(&cpu), [1, 0, 0, 0, 0]);
    }

    #[test]
    fn a_global_interrupt_disable_holds_the_flag_until_sei() {
        let (mut cpu, mut bus) = rig();
        cpu.sreg = 0;
        write(&mut cpu, &mut bus, ADDR_EICRA, 0b11);
        write(&mut cpu, &mut bus, ADDR_EIMSK, 1);
        hold(&mut bus, PD, 0x04);
        run(&mut cpu, &mut bus, 4);
        assert_eq!(entries(&cpu), [0; 5]);
        assert_eq!(cpu.eifr, 1);
        cpu.sreg = 0x80;
        run(&mut cpu, &mut bus, 1);
        assert_eq!(cpu.get_pc(), 0x04);
    }

    #[test]
    fn int1_low_level_requests_while_low_and_sets_no_flag() {
        let (mut cpu, mut bus) = rig();
        hold(&mut bus, PD, 1 << 3);
        write(&mut cpu, &mut bus, ADDR_EICRA, 0); // ISC1 = 00, low level
        write(&mut cpu, &mut bus, ADDR_EIMSK, 0b10);
        run(&mut cpu, &mut bus, 10);
        assert_eq!(entries(&cpu), [0; 5], "high: no request");
        hold(&mut bus, PD, 0);
        run(&mut cpu, &mut bus, 30);
        let held = cpu.r[21];
        assert!(held >= 5, "re-enters while the pad stays low, got {held}");
        assert_eq!(
            read(&cpu, &bus, ADDR_EIFR),
            0,
            "level mode keeps INTF1 clear"
        );
        hold(&mut bus, PD, 1 << 3);
        run(&mut cpu, &mut bus, 4);
        let settled = cpu.r[21];
        run(&mut cpu, &mut bus, 30);
        assert_eq!(cpu.r[21], settled, "released: no more requests");
    }

    #[test]
    fn pin_change_groups_mask_flag_and_clear() {
        let (mut cpu, mut bus) = rig();
        // PCINT1 (PB1) enabled; PCINT8 (PC0) flagged but its group disabled.
        write(&mut cpu, &mut bus, ADDR_PCMSK0, 1 << 1);
        write(&mut cpu, &mut bus, ADDR_PCMSK0 + 1, 1 << 0);
        write(&mut cpu, &mut bus, ADDR_PCICR, 0b001);

        hold(&mut bus, PB, 1 << 2); // PB2 is not in PCMSK0
        run(&mut cpu, &mut bus, 4);
        assert_eq!(entries(&cpu), [0; 5]);
        assert_eq!(read(&cpu, &bus, ADDR_PCIFR), 0);

        hold(&mut bus, PB, (1 << 2) | (1 << 1));
        run(&mut cpu, &mut bus, 4);
        hold(&mut bus, PB, 1 << 2);
        run(&mut cpu, &mut bus, 4);
        assert_eq!(entries(&cpu), [0, 0, 2, 0, 0], "both directions");

        hold(&mut bus, PC, 1);
        run(&mut cpu, &mut bus, 4);
        assert_eq!(entries(&cpu), [0, 0, 2, 0, 0]);
        assert_eq!(read(&cpu, &bus, ADDR_PCIFR), 0b010, "PCIF1 set, PCIE1 off");
        write(&mut cpu, &mut bus, ADDR_PCIFR, 0b010);
        assert_eq!(read(&cpu, &bus, ADDR_PCIFR), 0, "write 1 clears");
        hold(&mut bus, PC, 0);
        run(&mut cpu, &mut bus, 2);
        write(&mut cpu, &mut bus, ADDR_PCICR, 0b011);
        run(&mut cpu, &mut bus, 4);
        assert_eq!(
            entries(&cpu),
            [0, 0, 2, 1, 0],
            "pending flag fires on enable"
        );
    }

    /// A pad the firmware drives raises its own pin-change interrupt, as on
    /// silicon; the outside level under an output does not.
    #[test]
    fn a_driven_output_raises_its_own_pin_change() {
        let (mut cpu, mut bus) = rig();
        write(&mut cpu, &mut bus, ADDR_PCMSK2, 1 << 5);
        write(&mut cpu, &mut bus, ADDR_PCICR, 0b100);
        write(&mut cpu, &mut bus, PD + 1, 1 << 5); // DDRD5
        run(&mut cpu, &mut bus, 2);
        hold(&mut bus, PD, 1 << 5); // masked by the output driver
        run(&mut cpu, &mut bus, 4);
        assert_eq!(entries(&cpu), [0; 5]);
        write(&mut cpu, &mut bus, PD + 2, 1 << 5); // PORTD5 high
        run(&mut cpu, &mut bus, 4);
        assert_eq!(entries(&cpu), [0, 0, 0, 0, 1]);
    }

    #[test]
    fn configuring_an_interrupt_on_a_high_pad_invents_no_edge() {
        let (mut cpu, mut bus) = rig();
        hold(&mut bus, PD, 0xFF);
        hold(&mut bus, PB, 0xFF);
        write(&mut cpu, &mut bus, ADDR_EICRA, 0b0101);
        write(&mut cpu, &mut bus, ADDR_EIMSK, 0b11);
        write(&mut cpu, &mut bus, ADDR_PCMSK0, 0xFF);
        write(&mut cpu, &mut bus, ADDR_PCICR, 1);
        run(&mut cpu, &mut bus, 10);
        assert_eq!(entries(&cpu), [0; 5]);
    }

    #[test]
    fn sbi_on_flag_and_pin_registers_touches_only_its_bit() {
        let (mut cpu, mut bus) = rig();
        cpu.sreg = 0;
        cpu.eifr = 0b11;
        cpu.pcifr = 0b111;
        // SBI EIFR(0x1C),1 ; SBI PCIFR(0x1B),0 ; CBI EIFR,0 ; RJMP .
        cpu.load_words(0x100, &[0x9AE1, 0x9AD8, 0x98E0, 0xCFFF]);
        cpu.set_pc(0x100);
        run(&mut cpu, &mut bus, 3);
        assert_eq!(cpu.eifr, 0b01, "only INTF1 cleared");
        assert_eq!(cpu.pcifr, 0b110, "only PCIF0 cleared");

        // PB0 and PB5 outputs, both high; SBI PINB,5 toggles PB5 alone.
        write(&mut cpu, &mut bus, PB + 1, 0x21);
        write(&mut cpu, &mut bus, PB + 2, 0x21);
        cpu.load_words(0x100, &[0x9A1D, 0xCFFF]);
        cpu.set_pc(0x100);
        run(&mut cpu, &mut bus, 1);
        assert_eq!(
            bus.mem[&(AVR_IO_MIRROR_BASE + u64::from(PB))],
            1 << 5,
            "the PINB write carries bit 5 only"
        );
    }

    /// `SLEEP` with SE stops the core one idle clock per step until an enabled
    /// interrupt wakes it; the wake adds four cycles before the vector entry.
    #[test]
    fn sleep_waits_for_a_pin_change_and_wakes_into_the_vector() {
        let (mut cpu, mut bus) = rig();
        write(&mut cpu, &mut bus, ADDR_PCMSK2, 1 << 4);
        write(&mut cpu, &mut bus, ADDR_PCICR, 0b100);
        write(&mut cpu, &mut bus, ADDR_SMCR, 0b0000_0001); // idle, SE
        cpu.load_words(0x100, &[0x9588, 0xCFFF]); // SLEEP ; RJMP .
        cpu.set_pc(0x100);
        run(&mut cpu, &mut bus, 1);
        assert!(cpu.sleeping);
        let c0 = cpu.cycles;
        run(&mut cpu, &mut bus, 50);
        assert_eq!(cpu.cycles - c0, 50, "one idle clock per step");
        assert_eq!(cpu.get_pc(), 0x102, "nothing retired");
        assert_eq!(cpu.idle_fast_forward_budget(&bus), Some(u64::MAX));

        hold(&mut bus, PD, 1 << 4);
        assert_eq!(
            cpu.idle_fast_forward_budget(&bus),
            None,
            "an unsampled pad change ends the skip"
        );
        let c1 = cpu.cycles;
        run(&mut cpu, &mut bus, 1);
        assert!(!cpu.sleeping);
        assert_eq!(cpu.get_pc(), 0x14, "PCINT2_vect");
        assert_eq!(cpu.cycles - c1, 8, "4 wake-up + 4 vector-entry cycles");
        run(&mut cpu, &mut bus, 2);
        assert_eq!(cpu.get_pc(), 0x102, "resumes after SLEEP");
        assert_eq!(entries(&cpu), [0, 0, 0, 0, 1]);
    }

    #[test]
    fn sleep_without_se_or_with_interrupts_off_behaves() {
        let (mut cpu, mut bus) = rig();
        cpu.load_words(0x100, &[0x9588, 0xCFFF]);
        cpu.set_pc(0x100);
        run(&mut cpu, &mut bus, 1);
        assert!(!cpu.sleeping, "SE clear: SLEEP is a NOP");

        // I clear: an INT0 request does not wake the core.
        let (mut cpu, mut bus) = rig();
        cpu.sreg = 0;
        write(&mut cpu, &mut bus, ADDR_EICRA, 0b01);
        write(&mut cpu, &mut bus, ADDR_EIMSK, 1);
        write(&mut cpu, &mut bus, ADDR_SMCR, 1);
        cpu.load_words(0x100, &[0x9588, 0xCFFF]);
        cpu.set_pc(0x100);
        run(&mut cpu, &mut bus, 1);
        hold(&mut bus, PD, 0x04);
        run(&mut cpu, &mut bus, 10);
        assert!(cpu.sleeping);
        assert_eq!(cpu.eifr, 1, "the flag is still latched");
    }

    /// In power-down only a low level wakes the core through INT0/INT1 (edge
    /// detection needs clk_I/O); a pin change still does.
    #[test]
    fn power_down_ignores_int_edges_but_not_pin_changes() {
        let (mut cpu, mut bus) = rig();
        write(&mut cpu, &mut bus, ADDR_EICRA, 0b11);
        write(&mut cpu, &mut bus, ADDR_EIMSK, 1);
        write(&mut cpu, &mut bus, ADDR_SMCR, (0b010 << 1) | 1);
        cpu.load_words(0x100, &[0x9588, 0xCFFF]);
        cpu.set_pc(0x100);
        run(&mut cpu, &mut bus, 1);
        hold(&mut bus, PD, 0x04);
        run(&mut cpu, &mut bus, 10);
        assert!(cpu.sleeping, "rising INT0 edge does not wake power-down");

        write(&mut cpu, &mut bus, ADDR_PCMSK2, 1 << 2);
        write(&mut cpu, &mut bus, ADDR_PCICR, 0b100);
        hold(&mut bus, PD, 0);
        run(&mut cpu, &mut bus, 1);
        assert!(!cpu.sleeping);
        assert_eq!(cpu.get_pc(), 0x14);
    }

    /// The idle skip lands the Timer0 overflow on the same cycle as stepping
    /// the sleeping core one idle clock at a time.
    #[test]
    fn timer0_budget_matches_the_stepped_wake() {
        let setup = || {
            let (mut cpu, mut bus) = rig();
            cpu.tccr0b = 3; // clk/64
            cpu.timsk0 = TIMSK_TOIE0;
            cpu.tcnt0 = 200;
            cpu.t0_prescale_acc = 17;
            write(&mut cpu, &mut bus, ADDR_SMCR, 1);
            cpu.load_words(0x100, &[0x9588, 0xCFFF]);
            cpu.set_pc(0x100);
            run(&mut cpu, &mut bus, 1);
            assert!(cpu.sleeping);
            (cpu, bus)
        };
        let (mut stepped, mut sbus) = setup();
        while stepped.get_pc() != (VEC_TIMER0_OVF - 1) * 4 {
            run(&mut stepped, &mut sbus, 1);
        }
        let (mut skipped, mut kbus) = setup();
        let budget = skipped.idle_fast_forward_budget(&kbus).expect("asleep");
        skipped.fast_forward_idle_cycles(budget);
        assert_eq!(
            skipped.idle_fast_forward_budget(&kbus),
            None,
            "TOV0 pending"
        );
        run(&mut skipped, &mut kbus, 1);
        assert_eq!(skipped.get_pc(), stepped.get_pc());
        assert_eq!(skipped.cycles, stepped.cycles, "same wake cycle");
        assert_eq!(skipped.tcnt0, stepped.tcnt0);
        assert_eq!(skipped.t0_prescale_acc, stepped.t0_prescale_acc);
    }
}
