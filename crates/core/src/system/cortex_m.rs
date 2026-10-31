// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
use crate::cpu::CortexM;
use crate::peripherals::dwt::Dwt;
use crate::peripherals::itm::Itm;
use crate::peripherals::nvic::{Nvic, NvicState};
use crate::peripherals::scb::{Scb, ScbFaultState, SharedScbState};
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;

/// Process-wide default for ARMv7-M fault escalation, read once from
/// `LABWIRED_CORTEXM_FAULTS`.
///
/// **True unless the variable is explicitly set to `0` or `false`.** Fault
/// escalation is the default; the variable is the opt-*out* that keeps the
/// legacy #880 abort behaviour for a corpus that needs it, without a
/// per-call-site plumbing change to `configure_cortex_m`, which has ~150
/// callers. `CortexM::set_faults_enabled` remains the explicit per-core
/// override, and is what the in-tree guards use.
///
/// Hoisted into a `OnceLock` for the same reason `trace_insn_enabled` is: an
/// `std::env::var` call walks the environment, and `configure_cortex_m` runs on
/// every machine build.
///
/// The notice fires only on the opt-out: escalation-on is the default, so the
/// noteworthy event is a run that deliberately disabled it. Keeping the default
/// path silent also keeps lab `stderr` free of a per-run banner.
fn faults_enabled_default() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let off = matches!(
            std::env::var("LABWIRED_CORTEXM_FAULTS").as_deref(),
            Ok("0") | Ok("false")
        );
        if off {
            eprintln!("[cortex-m] ARMv7-M fault escalation DISABLED (LABWIRED_CORTEXM_FAULTS)");
        }
        !off
    })
}

pub fn configure_cortex_m(bus: &mut SystemBus) -> (CortexM, Arc<NvicState>) {
    let vtor = Arc::new(AtomicU32::new(0));
    let vectactive = Arc::new(AtomicU32::new(0));
    let shpr1 = Arc::new(AtomicU32::new(0));
    let shpr2 = Arc::new(AtomicU32::new(0));
    let shpr3 = Arc::new(AtomicU32::new(0));
    let nvic_state = Arc::new(NvicState::default());
    // Shared SYSRESETREQ latch: the SCB sets it on an AIRCR write, the CPU's
    // batch loop stops on it, and `Machine::drain_scb_reset_request` clears it.
    // Without this the plan had to pin the CPU quantum to 1 on every
    // Cortex-M bus just to keep the reset boundary exact.
    let sysreset_signal = Arc::new(AtomicBool::new(false));
    // Shared ARMv7-M fault register file (SHCSR/CFSR/HFSR/BFAR) + the master
    // switch for fault escalation. Seeded from the process default: escalation
    // is ON unless `LABWIRED_CORTEXM_FAULTS` opts out; with it off the SCB does
    // not serve those offsets at all and the core keeps #880's abort contract.
    // `CortexM::set_faults_enabled` is the per-core override.
    let faults = Arc::new(ScbFaultState::new(faults_enabled_default()));

    let mut cpu = CortexM::default();
    cpu.set_shared_vtor(vtor.clone());
    cpu.set_shared_vectactive(vectactive.clone());
    cpu.set_shared_shpr(shpr1.clone(), shpr2.clone(), shpr3.clone());
    cpu.set_shared_nvic_state(nvic_state.clone());
    cpu.set_shared_sysreset_signal(sysreset_signal.clone());
    cpu.set_shared_faults(faults.clone());

    bus.nvic = Some(nvic_state.clone());

    // Ensure SCB exists (VTOR relocation, ICSR.VECTACTIVE mirror, SHPR1/2/3).
    let mut scb = Scb::with_shared(SharedScbState {
        vtor,
        vectactive,
        shpr1,
        shpr2,
        shpr3,
        sysreset_signal,
        faults,
    });
    scb.event_state = Some(nvic_state.clone());
    // A Cortex-M7 chip that declares its L1 caches (`cortex_m7_cache` at
    // 0xE000_EF50) gets CLIDR/CTR/CCSIDR/CSSELR with that geometry; every
    // other chip keeps reading zero there.
    scb.l1_cache = bus
        .peripherals
        .iter()
        .find_map(|p| p.dev.l1_cache_geometry());
    // `replace_or_add_peripheral_at` attaches the bus cycle clock, so the SCB's
    // ICSR pend-drain rides the event scheduler (event-scheduler builds), and
    // rebuilds the routing/tick indices after the replacement.
    //
    // 0xC8 so the MPU block is served by the SCB/SCS model, not unmapped
    // space: TYPE/CTRL/RNR/RBAR/RASR at 0x90..0xA0 plus the ARMv8-M
    // (Cortex-M33) MAIR0/MAIR1 attribute registers at 0xC0/0xC4.
    bus.replace_or_add_peripheral_at("scb", 0xE000_ED00, 0xC8, None, Box::new(scb));

    // Ensure NVIC exists (shared pending/enabled state).
    //
    // Window extent: the NVIC block of the System Control Space runs from
    // NVIC_ISER0 at 0xE000_E100 to NVIC_IPR123 at 0xE000_E5EC — see the
    // ARMv7-M ARM (DDI 0403E, B3.4.3 "NVIC register support in the SCS") and
    // the ARMv8-M ARM (DDI 0553, B11.1), which give the same map: both
    // architectures allow up to 496 external interrupts, so the byte-indexed
    // priority array NVIC_IPR spans 0xE000_E400..=0xE000_E5EF.
    //
    // `size` was 0x400, which stopped at 0xE000_E4FF — i.e. it truncated the
    // priority array at interrupt 255. That is not an ARMv7-M/ARMv8-M
    // distinction (NVIC_ITNS, the genuinely v8-M-only register, sits at
    // 0xE000_E380 and was always inside the window); it is simply a window
    // one interrupt-priority page too short. Any part with more than 256
    // interrupts hit it: Zephyr's `z_arm_interrupt_init` writes a default
    // priority to every NVIC_IPR byte from 0 to CONFIG_NUM_IRQS, and on
    // nRF54L15 (CONFIG_NUM_IRQS = 0x10F) the 257th store lands at
    // 0xE000_E500, outside the window.
    //
    // This is not a read-as-zero widening: `Nvic` already backs the whole
    // priority array with real storage (`NvicState::ipr`, 240 registers) and
    // `NvicState::ipr_priority` feeds it straight into
    // `CortexM::exception_priority`, so a priority written here is read back
    // verbatim and actually orders pre-emption. The registers were modelled;
    // only the bus window hid them.
    //
    // Known remaining gap (deliberately NOT papered over here): the
    // set-enable / set-pending / active arrays are 8 registers each, so
    // interrupts >= 256 can have a priority but cannot be enabled or pended.
    // nRF54L15 declares IRQ 260 and 261. Fixing that means widening
    // `NvicState` and the `idx < 8` guards in `bus/tick.rs`, `bus/accessors.rs`
    // and `bus/construct.rs` — a separate change with its own blast radius.
    let nvic = Nvic::new(nvic_state.clone());
    const NVIC_WINDOW_SIZE: u64 = 0x4F0;
    bus.replace_or_add_peripheral_at("nvic", 0xE000_E100, NVIC_WINDOW_SIZE, None, Box::new(nvic));

    // Ensure DWT exists. Size 0x1000 covers the full CoreSight DWT register space,
    // including the CYCCNT enable bit at offset 0 and CYCCNT at offset 4, as well as
    // extended offsets accessed by some HAL dwt_init routines (e.g. offset 0xfc).
    // `replace_or_add_peripheral_at` attaches the bus cycle clock, so CYCCNT
    // can be derived lazily (walk-free plan Part 1).
    let dwt = Dwt::new();
    bus.replace_or_add_peripheral_at("dwt", 0xE000_1000, 0x1000, None, Box::new(dwt));

    // Stimulus ports sit at 0xE0000000..0xE000007F. The window ends where DWT
    // begins (0xE0001000); do not widen DWT backwards to cover ITM.
    let itm = Itm::new();
    bus.replace_or_add_peripheral_at("itm", 0xE000_0000, 0x1000, None, Box::new(itm));

    (cpu, nvic_state)
}

pub fn attach_swd_dp(bus: &mut SystemBus, cpu: &mut CortexM, idcode: u32) -> crate::debug::SwdDp {
    let state = crate::peripherals::scs_debug::DebugHaltState::new();
    cpu.set_debug_halt(state.clone());
    bus.replace_or_add_peripheral(
        "scs_debug",
        0xE000_EDF0,
        0x10,
        None,
        Box::new(crate::peripherals::scs_debug::ScsDebug::new(state)),
    );
    crate::debug::SwdDp::new(idcode)
}

#[cfg(test)]
mod swd_attach_tests {
    use super::attach_swd_dp;
    use crate::inspect::PeekByte;
    use crate::system::cortex_m::configure_cortex_m;
    use crate::Bus;
    use crate::Machine;

    fn nrf_machine() -> Machine<crate::cpu::CortexM> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chip =
            labwired_config::ChipDescriptor::from_file(root.join("configs/chips/nrf52840.yaml"))
                .expect("nrf52840 chip");
        let manifest: labwired_config::SystemManifest =
            serde_yaml::from_str("name: swd-gate\nchip: ignored\n").expect("manifest");
        let mut bus = crate::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
        let (cpu, _) = configure_cortex_m(&mut bus);
        Machine::new(cpu, bus)
    }

    #[test]
    fn dhcsr_stays_unmapped_until_attach() {
        let m = nrf_machine();
        let peek = m.peek(0xE000_EDF0, 4);
        assert!(
            peek.bytes.iter().all(|b| *b == PeekByte::Unmapped),
            "peek before attach: {:?}",
            peek.bytes
        );
    }

    #[test]
    fn attach_makes_dhcsr_read_s_regrdy() {
        let mut m = nrf_machine();
        attach_swd_dp(&mut m.bus, &mut m.cpu, 0x2BA0_1477);
        assert_eq!(m.bus.read_u32(0xE000_EDF0).unwrap(), 0x0001_0000);
        let peek = m.peek(0xE000_EDF0, 4);
        assert_eq!(
            peek.bytes,
            vec![
                PeekByte::Mapped(0x00),
                PeekByte::Mapped(0x00),
                PeekByte::Mapped(0x01),
                PeekByte::Mapped(0x00),
            ]
        );
    }
}
