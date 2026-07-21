// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Plan 3 Task 7: integration test — hand-rolled ISR validates the full
// IRQ delivery chain (SYSTIMER alarm → bus aggregation → intmatrix
// routing → CPU dispatch_irq → kernel exception vector → ISR → GPIO write
// → observer notification) without depending on real esp-hal firmware.
//
// If this test passes, the simulator's IRQ delivery is sound; any failure
// in the real-firmware demo (Plan 3 Task 10) is firmware-side.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32s3::gpio::GpioObserver;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{Bus, Cpu, Machine};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct RecordingObserver {
    events: Mutex<Vec<(u8, bool, bool, u64)>>,
}

impl GpioObserver for RecordingObserver {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64) {
        self.events.lock().unwrap().push((pin, from, to, sim_cycle));
    }
}

/// Hand-assembled ISR (4 instructions, 9 bytes), assembled with
/// `xtensa-esp32s3-elf-as` and `objdump -d`:
///
///   s32i.n  a6, a3, 0   -> 0x69 0x03    (GPIO_OUT_W1TS = bit 2 mask → pin 2 0->1)
///   s32i.n  a6, a4, 0   -> 0x69 0x04    (GPIO_OUT_W1TC = bit 2 mask → pin 2 1->0)
///   s32i.n  a7, a5, 0   -> 0x79 0x05    (SYSTIMER_INT_CLR = 1 → ack alarm 0)
///   rfe                 -> 0x00 0x30 0x00  (return from level-1 exception)
///
/// Pre-loaded by the test:
///   a3 = 0x6000_4008  GPIO_OUT_W1TS_REG
///   a4 = 0x6000_400C  GPIO_OUT_W1TC_REG
///   a5 = 0x6002_306C  SYSTIMER_INT_CLR_REG
///   a6 = 0x0000_0004  bit 2 mask
///   a7 = 0x0000_0001  alarm 0 clear bit
const ISR_BYTES: &[u8] = &[
    0x69, 0x03, // s32i.n  a6, a3, 0
    0x69, 0x04, // s32i.n  a6, a4, 0
    0x79, 0x05, // s32i.n  a7, a5, 0
    0x00, 0x30, 0x00, // rfe
];

/// `j 0` — jump-to-self spin loop, 3 bytes.
const SPIN_BYTES: &[u8] = &[0x06, 0xff, 0xff];

// Plan 3 IRQ delivery chain test: SYSTIMER -> bus aggregation ->
// intmatrix routing -> CPU dispatch -> kernel vector -> ISR -> GPIO observer.
// Requires the SystemBus `pending_cpu_irqs` aggregation that Phase D added.
#[test]
fn intmatrix_alarm_full_irq_chain() {
    const IRAM_BASE: u32 = 0x4037_0000;
    const ISR_OFFSET: u32 = 0x1000;
    const ISR_PC: u32 = IRAM_BASE + ISR_OFFSET;
    const VECBASE_VALUE: u32 = ISR_PC - 0x300;
    const SYSTIMER_BASE: u32 = 0x6002_3000;
    const INTMATRIX_BASE: u32 = 0x600C_2000;
    const SYSTIMER_TARGET0_SOURCE: u32 = 57;
    // Slot 12 is level 1 in IRQ_LEVELS (table in xtensa_lx7.rs); level 1
    // dispatches to VECBASE+0x300 where we plant the ISR.
    const CPU_IRQ_SLOT: u8 = 12;

    let mut bus = SystemBus::new();
    let opts = Esp32s3Opts::default();
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);

    let obs = Arc::new(RecordingObserver::default());
    wiring.add_gpio_observer(&mut bus, obs.clone());

    let mut cpu = wiring.cpu;

    // Plant the spin loop at IRAM_BASE (CPU spins here between interrupts).
    for (i, &b) in SPIN_BYTES.iter().enumerate() {
        bus.write_u8((IRAM_BASE + i as u32) as u64, b).unwrap();
    }

    // Plant the ISR at IRAM_BASE + 0x1000 (kernel exception vector lands here).
    for (i, &b) in ISR_BYTES.iter().enumerate() {
        bus.write_u8((ISR_PC + i as u32) as u64, b).unwrap();
    }

    // Configure VECBASE so that the kernel exception vector (VECBASE+0x300)
    // is the ISR address.
    use labwired_core::cpu::xtensa_sr::{INTENABLE, VECBASE};
    cpu.sr.write(VECBASE, VECBASE_VALUE);

    // Pre-load AR registers used by the ISR.
    cpu.regs.write_logical(3, 0x6000_4008); // GPIO_OUT_W1TS
    cpu.regs.write_logical(4, 0x6000_400C); // GPIO_OUT_W1TC
    cpu.regs.write_logical(5, SYSTIMER_BASE + 0x6C); // SYSTIMER_INT_CLR
    cpu.regs.write_logical(6, 0x0000_0004); // bit 2 mask
    cpu.regs.write_logical(7, 0x0000_0001); // alarm 0 clear bit

    // Configure intmatrix: source 79 (SYSTIMER_TARGET0) → CPU IRQ slot 15.
    let intmatrix_off = INTMATRIX_BASE + SYSTIMER_TARGET0_SOURCE * 4;
    bus.write_u32(intmatrix_off as u64, CPU_IRQ_SLOT as u32)
        .unwrap();

    // Configure SYSTIMER ALARM0 — TRM-correct sequence (TARGET_CONF has no
    // enable bit; enable lives in SYSTIMER_CONF.target0_work_en at bit 24,
    // commit handshake via COMP0_LOAD bit 0).
    //
    // PERIOD (auto-reload) mode, period = 20 SYSTIMER ticks: the alarm fires
    // every 20 ticks, exactly like the FreeRTOS systick. This exercises
    // SUSTAINED IRQ delivery — the ISR acks INT_CLR each time and the chain
    // re-fires on the next period boundary. A one-shot alarm fires exactly
    // once → a single ISR → one GPIO2 toggle pair; the only reason a one-shot
    // ever produced repeated toggles was a since-fixed latch bug (a stale
    // routed pending_cpu_irqs bit spuriously re-entered the ISR after return).
    bus.write_u32((SYSTIMER_BASE + 0x1C) as u64, 0).unwrap(); // pending TARGET0_HI
    bus.write_u32((SYSTIMER_BASE + 0x20) as u64, 20).unwrap(); // pending TARGET0_LO (moot in period mode)
    bus.write_u32((SYSTIMER_BASE + 0x34) as u64, (1u32 << 30) | 20)
        .unwrap(); // TARGET0_CONF: PERIOD_MODE (bit30) + period=20, UNIT0
    bus.write_u32((SYSTIMER_BASE + 0x50) as u64, 1).unwrap(); // COMP0_LOAD: commit
                                                              // SYSTIMER_CONF: keep clk_en + unit0/1 work-en defaults (bits 31/30/29),
                                                              // additionally set target0_work_en (bit 24).
    let conf = bus.read_u32(SYSTIMER_BASE as u64).unwrap();
    bus.write_u32(SYSTIMER_BASE as u64, conf | (1u32 << 24))
        .unwrap();
    bus.write_u32((SYSTIMER_BASE + 0x64) as u64, 1).unwrap(); // INT_ENA bit 0

    // Configure CPU INTENABLE for the bound slot.
    cpu.sr.write(INTENABLE, 1u32 << CPU_IRQ_SLOT);

    // PS.INTLEVEL = 0, EXCM = 0 so level-1 interrupts can fire.
    cpu.ps.set_intlevel(0);
    cpu.ps.set_excm(false);

    cpu.set_pc(IRAM_BASE);

    let mut machine = Machine::new(cpu, bus);
    const MAX_STEPS: u64 = 100_000;
    for _step in 0..MAX_STEPS {
        if let Err(e) = machine.step() {
            let events = obs.events.lock().unwrap();
            panic!(
                "CPU step failed at pc=0x{:08x}: {e}; events: {events:?}",
                machine.cpu.get_pc(),
            );
        }

        let events = obs.events.lock().unwrap();
        let pin2_count = events.iter().filter(|&&(p, _, _, _)| p == 2).count();
        if pin2_count >= 3 {
            return;
        }
    }

    let events = obs.events.lock().unwrap();
    panic!(
        "did not see 3+ transitions on GPIO2 in {MAX_STEPS} steps; \
         events: {events:?}, final PC=0x{:08x}",
        machine.cpu.get_pc(),
    );
}
