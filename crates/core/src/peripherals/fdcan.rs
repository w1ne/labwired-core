// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32H5 FDCAN — Bosch M_CAN as integrated on the H5 (RM0481 §60),
//! with the *fixed* message-RAM layout (no SIDFC/RXF0C/TXBC address
//! registers — element addresses are hardwired, matching the HAL's
//! SRAMCAN_* constants).
//!
//! Register map follows CMSIS `stm32h563xx.h`. The peripheral window
//! covers the whole FDCAN estate relative to the FDCAN1 base:
//! 0x000..0x0EB M_CAN registers, 0x100/0x304/0x3F0.. the FDCAN_CONFIG
//! block (CKDIV / OPTR / HWCFG / VERR / IPIDR / SIDR), 0x800..0xE9F the
//! shared SRAMCAN. FDCAN2's register block (offset 0x400) is not
//! modeled and reads 0.
//!
//! Behavior is pinned against bench measurements: silicon capture13
//! 2026-06-12 (NUCLEO-H563ZI), internal-loopback frame driven over SWD.
//! Key pinned facts encoded here and in the tests below:
//!
//! - reset: CREL = 0x3214_1218, ENDN = 0x8765_4321, DBTP = 0x0A33,
//!   NBTP = 0x0600_0A03, CCCR = 0x1 (INIT), PSR = 0x707,
//!   TOCC = 0xFFFF_0000, TOCV = 0xFFFF, XIDAM = 0x1FFF_FFFF,
//!   TXFQS = 0x3 (TFFL = 3 free slots);
//! - clearing CCCR.INIT also clears CCCR.CCE (write 0xA2 reads 0xA0);
//! - a completed transmission sets TXBTO and IR.TFE unconditionally,
//!   but IR.TC **only for buffers enabled in TXBTIE** (capture13:
//!   IR = 0x201 = RF0N|TFE with TXBTIE = 0 — TC bit 7 stayed clear);
//! - TXFQS after one TX reads 0x0001_0103 (TFQPI = TFGI = 1, TFFL = 3);
//! - a loopback frame lands in RX FIFO0: RXF0S = 0x0001_0001
//!   (F0PI = 1, F0FL = 1), element at SRAMCAN+0xB0 with the standard
//!   ID in R0[28:18] (low bits undefined on silicon — readers mask),
//!   DLC in R1[19:16], ANMF set, payload byte-exact;
//! - IR is rc_w1 and needs atomic word writes (the byte-decomposed
//!   default would read-modify-write still-set bits back into the
//!   clear — the F103 GPIO BSRR / EXTI SWIER failure class);
//! - RXF0A acknowledge advances F0GI and drops the fill level
//!   (RXF0S reads 0x0001_0100 after acking the only element) — it does
//!   not blank the status register.
//!
//! Modeling notes (KISS, documented deviations from full silicon):
//! - standard/extended range, dual-ID and classic-mask filters route to
//!   FIFO0/FIFO1 or reject; global non-matching actions and remote-frame
//!   rejection are honored. Priority-only actions, dedicated RX buffers and
//!   the HPMS register are not modeled.
//! - bit timing is not simulated; transmission completes on the tick
//!   after TXBAR (or on the delay-0 scheduler event that stands in for
//!   that next tick under walk-free). DBTP/NBTP/CKDIV are storage with
//!   silicon reset values.
//!
//! ## Drive modes (walk-free H5 campaign)
//!
//! * **Scheduler mode** (`event-scheduler` and no `CanBus` interconnect):
//!   TXBAR-deferred completion and level IRQ re-assert ride a delay-0/1
//!   event chain. `needs_legacy_walk` is false so the single-node demo bus
//!   can flip (`max_safe=RECOMMENDED_TICK_INTERVAL`). This is the intentional
//!   green path.
//! * **Legacy / interconnect mode**: with a `CanBus` `bus_rx` attached the
//!   tick must poll mpsc (same contract as bxCAN); the walk stays on and
//!   multi-node buses correctly pin `max_safe=1`. **Do not hatch walk-free
//!   while `bus_rx` is polled only from the walk** — that starves multi-node
//!   RX. Event-driven CanBus drain (delay-0 / push-into-scheduler with a
//!   dual-lane walk@1 ≡ sched@512 fidelity gate) is a follow-up; until then
//!   multi-node is an honest interim, not a certificate.
//!   Feature-off builds also use the walk for TX + IRQ.
//!
//! ## Timed CAN bus mode
//!
//! On a world's timed bus ([`crate::network::timed_can`], `attach_timed`) a
//! TXBAR request stays pending (TXBRP) until the bus reports the attempt:
//! success completes it as above; a failed attempt leaves it pending for
//! automatic retransmission, or with CCCR.DAR cancels it (TXBRP clear,
//! TXBCF set). The bus's counters show in ECR (TEC, REC, RP, CEL) and PSR
//! (LEC, EP, EW, BO), with IR.EP / IR.EW / IR.BO on each change and IR.PEA
//! on each protocol error. Bus-off sets CCCR.INIT; once firmware clears it
//! the bus counts the recovery (RM0481: 129 occurrences of 11 recessive
//! bits). Under the event scheduler the interrupt level rides the same
//! delay-0/1 chain as single-node mode, so the walk stays deleted; without
//! the scheduler the walk holds it.
//!
//! The nominal bit time comes from NBTP and the kernel clock divided by
//! CKDIV. The kernel clock is the HSE: the RCC model does not hold
//! CCIPR5.FDCANSEL, whose reset value selects HSE, and HSE is taken as
//! 8 MHz (the NUCLEO-H563ZI, clocked from the ST-LINK MCO). With that clock
//! the reset NBTP is exactly 500 kbit/s. A PLL-sourced kernel clock is not
//! modelled. The data-phase bit rate (DBTP) is not used: an FD frame is
//! timed at the nominal rate.
//!
//! - interrupt line 1 (ILS routing, FDCAN1_IT1) is not modeled; all
//!   enabled interrupts assert the configured line (FDCAN1_IT0) when
//!   ILE.EINT0 is set.
//! - without loopback (TEST.LBCK = 0) a completed TX is queued on an
//!   internal list for a future CAN network layer; completion flags
//!   behave identically.

use crate::network::CanFrame;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};

// ---- M_CAN register offsets (CMSIS stm32h563xx.h) ----
const REG_CREL: u64 = 0x000;
const REG_ENDN: u64 = 0x004;
const REG_DBTP: u64 = 0x00C;
const REG_TEST: u64 = 0x010;
const REG_RWD: u64 = 0x014;
const REG_CCCR: u64 = 0x018;
const REG_NBTP: u64 = 0x01C;
const REG_TSCC: u64 = 0x020;
const REG_TSCV: u64 = 0x024;
const REG_TOCC: u64 = 0x028;
const REG_TOCV: u64 = 0x02C;
const REG_ECR: u64 = 0x040;
const REG_PSR: u64 = 0x044;
const REG_TDCR: u64 = 0x048;
const REG_IR: u64 = 0x050;
const REG_IE: u64 = 0x054;
const REG_ILS: u64 = 0x058;
const REG_ILE: u64 = 0x05C;
const REG_RXGFC: u64 = 0x080;
const REG_XIDAM: u64 = 0x084;
const REG_HPMS: u64 = 0x088;
const REG_RXF0S: u64 = 0x090;
const REG_RXF0A: u64 = 0x094;
const REG_RXF1S: u64 = 0x098;
const REG_RXF1A: u64 = 0x09C;
const REG_TXBC: u64 = 0x0C0;
const REG_TXFQS: u64 = 0x0C4;
const REG_TXBRP: u64 = 0x0C8;
const REG_TXBAR: u64 = 0x0CC;
const REG_TXBCR: u64 = 0x0D0;
const REG_TXBTO: u64 = 0x0D4;
const REG_TXBCF: u64 = 0x0D8;
const REG_TXBTIE: u64 = 0x0DC;
const REG_TXBCIE: u64 = 0x0E0;
const REG_TXEFS: u64 = 0x0E4;
const REG_TXEFA: u64 = 0x0E8;

// ---- FDCAN_CONFIG block (offsets relative to the FDCAN1 base) ----
const REG_CKDIV: u64 = 0x100;
const REG_OPTR: u64 = 0x304;
const REG_HWCFG: u64 = 0x3F0;
const REG_VERR: u64 = 0x3F4;
const REG_IPIDR: u64 = 0x3F8;
const REG_SIDR: u64 = 0x3FC;

// ---- Fixed SRAMCAN layout (HAL SRAMCAN_* constants, RM0481 §60.3.3) ----
/// SRAMCAN start within the peripheral window (0x4000AC00 - 0x4000A400).
const RAM_BASE: u64 = 0x800;
/// Both instances' sections: 2 x 0x350 bytes.
const RAM_WORDS: usize = 0x6A0 / 4;
/// RX FIFO0 elements: 3 x 18 words at SRAMCAN + 0xB0.
const RAM_RF0_WORDS: usize = 0xB0 / 4;
/// RX FIFO1 elements: 3 x 18 words at SRAMCAN + 0x188.
const RAM_RF1_WORDS: usize = 0x188 / 4;
/// Standard filter list: one 32-bit element each at SRAMCAN + 0x000.
const RAM_STD_FILTER_WORDS: usize = 0x000 / 4;
/// Extended filter list: two 32-bit elements each at SRAMCAN + 0x070.
const RAM_EXT_FILTER_WORDS: usize = 0x070 / 4;
/// TX buffers: 3 x 18 words at SRAMCAN + 0x278.
const RAM_TFQ_WORDS: usize = 0x278 / 4;
const ELEMENT_WORDS: usize = 18;
const FIFO_DEPTH: u32 = 3;

// ---- Bit fields ----
const CCCR_INIT: u32 = 1 << 0;
const CCCR_CCE: u32 = 1 << 1;
/// Clock stop request: the controller leaves the bus.
const CCCR_CSR: u32 = 1 << 4;
/// Bus monitoring (silent) mode.
const CCCR_MON: u32 = 1 << 5;
/// Disable automatic retransmission.
const CCCR_DAR: u32 = 1 << 6;
// CCCR.MON (bit 5) needs no dedicated handling: external loopback
// (LBCK without MON) also delivers the frame to the own receiver, so
// the model's loopback path is keyed on TEST.LBCK alone.
const CCCR_TEST: u32 = 1 << 7;

const TEST_LBCK: u32 = 1 << 4;

// IR/IE — the H5 uses the compressed M_CAN layout (no per-FIFO
// watermark bits): RF0N(0) RF0F(1) RF0L(2) RF1N(3) RF1F(4) RF1L(5)
// HPM(6) TC(7) TCF(8) TFE(9) ...
const IR_RF0N: u32 = 1 << 0;
const IR_RF0L: u32 = 1 << 2;
const IR_RF1N: u32 = 1 << 3;
const IR_RF1L: u32 = 1 << 5;
const IR_TC: u32 = 1 << 7;
const IR_TFE: u32 = 1 << 9;
const IR_EP: u32 = 1 << 17;
const IR_EW: u32 = 1 << 18;
const IR_BO: u32 = 1 << 19;
/// Protocol error in the arbitration (nominal) phase.
const IR_PEA: u32 = 1 << 21;

// PSR error-state bits (RM0481 FDCAN_PSR).
const PSR_EP: u32 = 1 << 5;
const PSR_EW: u32 = 1 << 6;
const PSR_BO: u32 = 1 << 7;

const ILE_EINT0: u32 = 1 << 0;

/// FDCAN kernel clock in timed mode: HSE, 8 MHz (see the module docs).
pub const DEFAULT_KERNEL_CLOCK_HZ: u64 = 8_000_000;
/// M_CAN leaves bus-off after 129 occurrences of 11 recessive bits
/// (RM0481 §60.4.2 "Bus-off recovery").
const MCAN_RECOVERY_OCCURRENCES: u32 = 129;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FdcanTraceFrame {
    pub seq: u64,
    pub peripheral: String,
    pub direction: String,
    pub id: u32,
    pub data: Vec<u8>,
    pub extended: bool,
    pub fd: bool,
    pub bitrate_switch: bool,
    pub remote: bool,
}

/// STM32H5 FDCAN instance (FDCAN1) plus the shared CONFIG block and
/// SRAMCAN window.
///
/// Pinned against RM0481 and silicon capture13 2026-06-12
/// (NUCLEO-H563ZI); see the module docs for the truth table and
/// modeling limits.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Fdcan {
    dbtp: u32,
    test: u32,
    rwd: u32,
    cccr: u32,
    nbtp: u32,
    tscc: u32,
    tscv: u32,
    tocc: u32,
    tocv: u32,
    ecr: u32,
    ir: u32,
    ie: u32,
    ils: u32,
    ile: u32,
    rxgfc: u32,
    xidam: u32,
    rxf0_fill: u32,
    rxf0_put: u32,
    rxf0_get: u32,
    rxf0_lost: bool,
    rxf1_fill: u32,
    rxf1_put: u32,
    rxf1_get: u32,
    rxf1_lost: bool,
    txbrp: u32,
    txbto: u32,
    txbcf: u32,
    txbtie: u32,
    txbcie: u32,
    /// TX FIFO put/get indices (mod 3). The indices advance per TX as
    /// pinned on silicon (TXFQS = 0x0001_0103 after one send).
    txfq_put: u32,
    txfq_get: u32,
    /// Frames queued by `request_tx` awaiting tick-deferred completion.
    /// Each entry carries the buffer bit-mask and the decoded frame so
    /// that completion (TXBRP→0, TXBTO, IR flags) is applied in the
    /// next `tick()` rather than synchronously on the TXBAR write.
    /// This models the real M_CAN: TXBRP stays asserted until the
    /// frame is actually placed on the bus (arbitration + bit time).
    #[serde(skip)]
    pending_tx: VecDeque<(u32, CanFrame)>,
    ckdiv: u32,
    optr: u32,
    /// Protocol has left INIT at least once — flips PSR from its reset
    /// 0x707 (LEC = DLEC = 7) to the post-traffic 0x708 (LEC = 0,
    /// ACT = idle) pinned in capture13.
    bus_active: bool,
    /// Bus-off (PSR.BO). Entered only through [`Self::enter_bus_off`] (a
    /// `can_bus_off` fault: bit errors are not simulated). Left when firmware
    /// clears CCCR.INIT, as on silicon; the 129 x 11 recessive-bit recovery
    /// wait is not modeled, recovery completes on that write.
    #[serde(default)]
    bus_off: bool,
    message_ram: Vec<u32>,
    /// Frames transmitted with loopback off and no interconnect
    /// attached. Bounded: oldest dropped past 64.
    #[serde(skip)]
    pub tx_frames: VecDeque<CanFrame>,
    /// The machine's ONE bus trace, and this controller's name in it. Private
    /// until `attach_bus_trace` hands over the shared handle at registration,
    /// so a bare `Fdcan::new()` in a unit test still records somewhere.
    #[serde(skip)]
    trace: crate::bus::bus_trace::BusTrace,
    #[serde(skip)]
    trace_name: String,
    /// `CanBus` interconnect endpoints (`new_with_bus` / `attach_bus`).
    #[serde(skip)]
    bus_tx: Option<Sender<CanFrame>>,
    #[serde(skip)]
    bus_rx: Option<Receiver<CanFrame>>,
    /// Scheduler mode: one live TX/IRQ event chain is outstanding.
    #[serde(skip)]
    chain_live: bool,
    /// The bus's cycle clock, which stamps TX requests and recovery arming.
    #[serde(skip)]
    clock: Option<crate::CycleClock>,
    /// On a world's timed CAN bus (see the module docs).
    #[serde(skip)]
    timed: bool,
    /// Timed mode: the cycle each TX buffer's request was made at.
    #[serde(skip)]
    tx_requested: [u64; 3],
    /// Timed mode: kernel clock before CKDIV, Hz.
    #[serde(skip)]
    kernel_clock_hz: u64,
    /// Timed mode: PSR.LEC, EW and EP as the bus last reported them.
    #[serde(default)]
    timed_lec: u8,
    #[serde(default)]
    error_warning: bool,
    #[serde(default)]
    error_passive: bool,
    /// Timed mode: bus-off recovery may count from this cycle.
    #[serde(skip)]
    recovery_armed: Option<u64>,
}

impl Fdcan {
    pub fn new() -> Self {
        Self {
            dbtp: 0x0000_0A33,
            test: 0,
            rwd: 0,
            cccr: CCCR_INIT,
            nbtp: 0x0600_0A03,
            tscc: 0,
            tscv: 0,
            tocc: 0xFFFF_0000,
            tocv: 0x0000_FFFF,
            ecr: 0,
            ir: 0,
            ie: 0,
            ils: 0,
            ile: 0,
            rxgfc: 0,
            xidam: 0x1FFF_FFFF,
            rxf0_fill: 0,
            rxf0_put: 0,
            rxf0_get: 0,
            rxf0_lost: false,
            rxf1_fill: 0,
            rxf1_put: 0,
            rxf1_get: 0,
            rxf1_lost: false,
            txbrp: 0,
            txbto: 0,
            txbcf: 0,
            txbtie: 0,
            txbcie: 0,
            txfq_put: 0,
            txfq_get: 0,
            ckdiv: 0,
            optr: 0,
            bus_active: false,
            bus_off: false,
            message_ram: vec![0; RAM_WORDS],
            tx_frames: VecDeque::new(),
            trace: crate::bus::bus_trace::BusTrace::new(),
            trace_name: String::new(),
            bus_tx: None,
            bus_rx: None,
            pending_tx: VecDeque::new(),
            chain_live: false,
            timed: false,
            clock: None,
            tx_requested: [0; 3],
            kernel_clock_hz: DEFAULT_KERNEL_CLOCK_HZ,
            timed_lec: 0,
            error_warning: false,
            error_passive: false,
            recovery_armed: None,
        }
    }

    #[inline]
    fn scheduler_mode(&self) -> bool {
        // Scheduler covers TX defer + level IRQ on the single-node path.
        // An attached CanBus interconnect still needs the walk to poll
        // `bus_rx` (bxCAN pattern). Do not clear this gate to chase
        // walk-free multi-node — mpsc RX is not event-driven yet.
        cfg!(feature = "event-scheduler") && self.bus_rx.is_none()
    }

    /// The scheduler chain has work: a frame to complete (not in timed mode,
    /// where the bus completes it) or a level interrupt to hold.
    fn chain_has_work(&self) -> bool {
        (!self.timed && !self.pending_tx.is_empty() && !self.bus_off) || self.irq_level_held()
    }

    #[inline]
    fn irq_level_held(&self) -> bool {
        self.ile & ILE_EINT0 != 0 && self.ir & self.ie != 0
    }

    /// One tick of TX completion + interconnect RX + level IRQ.
    fn service_once(&mut self) -> bool {
        if !self.timed {
            self.drain_pending_tx();
        }
        if let Some(rx) = self.bus_rx.take() {
            while let Ok(frame) = rx.try_recv() {
                self.receive_frame(frame);
            }
            self.bus_rx = Some(rx);
        }
        self.irq_level_held()
    }

    /// Attach to a `CanBus` interconnect: transmitted frames go out on
    /// `tx`, frames arriving on `rx` are delivered to the receiver on
    /// each tick (while the protocol is running).
    pub fn new_with_bus(tx: Sender<CanFrame>, rx: Receiver<CanFrame>) -> Self {
        let mut dev = Self::new();
        dev.attach_bus(tx, rx)
            .expect("a newly constructed FDCAN has no CAN bus attachment");
        dev
    }

    /// Bind this FDCAN to one `CanBus` endpoint after construction.
    ///
    /// `SystemBus::from_config` builds peripherals before `World` knows the
    /// environment topology, so a world needs this post-construction seam.
    /// Rebinding is rejected rather than silently dropping the original
    /// endpoint and any queued inbound frames.
    pub fn attach_bus(
        &mut self,
        tx: Sender<CanFrame>,
        rx: Receiver<CanFrame>,
    ) -> anyhow::Result<()> {
        if self.bus_tx.is_some() || self.bus_rx.is_some() {
            anyhow::bail!("FDCAN is already attached to a CAN bus");
        }
        self.bus_tx = Some(tx);
        self.bus_rx = Some(rx);
        Ok(())
    }

    /// Put this controller on a world's timed CAN bus
    /// ([`crate::network::timed_can`]).
    pub fn attach_timed(&mut self) -> anyhow::Result<()> {
        if self.timed || self.bus_tx.is_some() || self.bus_rx.is_some() {
            anyhow::bail!("FDCAN is already attached to a CAN bus");
        }
        self.timed = true;
        Ok(())
    }

    /// `true` once on a timed CAN bus.
    pub fn is_timed(&self) -> bool {
        self.timed
    }

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, |c| c.now())
    }

    /// Kernel clock after the CKDIV.PDIV divider: 0 divides by 1, n by 2n
    /// (RM0481 FDCAN_CKDIV).
    fn divided_kernel_clock_hz(&self) -> u64 {
        let pdiv = u64::from(self.ckdiv & 0xF);
        self.kernel_clock_hz / if pdiv == 0 { 1 } else { 2 * pdiv }
    }

    /// The controller as the timed bus needs it.
    ///
    /// Nominal bit time (RM0481 §60.4.1, FDCAN_NBTP): tq = (NBRP + 1) /
    /// f_kernel and one bit is 1 + (NTSEG1 + 1) + (NTSEG2 + 1) quanta.
    pub fn timed_poll(&self) -> crate::network::timed_can::CanNodePoll {
        use crate::network::timed_can::{CanNodePoll, CanTxRequest};
        let clock = self.divided_kernel_clock_hz();
        let nbrp = u128::from((self.nbtp >> 16) & 0x1FF) + 1;
        let ntseg1 = u128::from((self.nbtp >> 8) & 0xFF);
        let ntseg2 = u128::from(self.nbtp & 0x7F);
        let bit_ps = (clock > 0).then(|| {
            (nbrp * (3 + ntseg1 + ntseg2) * crate::network::timed_uart::PS_PER_SEC
                / u128::from(clock)) as u64
        });
        let tx = self.pending_tx.front().map(|(bit, frame)| CanTxRequest {
            tag: *bit,
            frame: frame.clone(),
            requested_cycle: self.tx_requested[bit.trailing_zeros() as usize % 3],
        });
        CanNodePoll {
            bit_ps,
            clock_hz: clock,
            on_bus: self.running() && !self.loopback() && self.cccr & (CCCR_MON | CCCR_CSR) == 0,
            fd_capable: true,
            tx,
            recovery_armed_cycle: self.recovery_armed,
            recovery_occurrences: MCAN_RECOVERY_OCCURRENCES,
        }
    }

    /// Apply what the timed bus decided for this controller.
    pub fn timed_apply(&mut self, update: &crate::network::timed_can::CanNodeUpdate) {
        use crate::network::timed_can::{CanErrorState, CanTxOutcome, ERROR_WARNING_LIMIT};
        let protocol_error = update.lec.is_some_and(|l| l != 0);
        // ECR (RM0481 FDCAN_ECR): CEL[23:16] counts protocol errors and
        // saturates; RP[15] with REC[14:8] (7 bits) above 127; TEC[7:0].
        let mut cel = (self.ecr >> 16) & 0xFF;
        if protocol_error {
            cel = (cel + 1).min(0xFF);
            self.ir |= IR_PEA;
        }
        let rec = u32::from(update.rec);
        self.ecr = (cel << 16)
            | (u32::from(rec > 127) << 15)
            | (rec.min(127) << 8)
            | u32::from(update.tec);
        if let Some(lec) = update.lec {
            self.timed_lec = lec & 0x7;
        }
        let warning = u16::from(update.tec) >= ERROR_WARNING_LIMIT
            || u16::from(update.rec) >= ERROR_WARNING_LIMIT;
        let passive = update.state != CanErrorState::Active;
        let off = update.state == CanErrorState::BusOff;
        // IR.EW / EP / BO flag each status change (RM0481 FDCAN_IR).
        if warning != self.error_warning {
            self.ir |= IR_EW;
        }
        if passive != self.error_passive {
            self.ir |= IR_EP;
        }
        if off != self.bus_off {
            self.ir |= IR_BO;
        }
        self.error_warning = warning;
        self.error_passive = passive;
        if off && !self.bus_off {
            // Bus-off: the controller sets CCCR.INIT and stays off the bus
            // until firmware clears it.
            self.cccr |= CCCR_INIT;
            self.recovery_armed = None;
        } else if !off {
            self.recovery_armed = None;
        }
        self.bus_off = off;
        if let Some((tag, outcome)) = update.tx {
            if self.pending_tx.front().is_some_and(|(bit, _)| *bit == tag) {
                if outcome == CanTxOutcome::Ok {
                    let (bit, frame) = self.pending_tx.pop_front().expect("checked");
                    self.complete_tx(bit, frame);
                } else if self.cccr & CCCR_DAR != 0 {
                    let (bit, _) = self.pending_tx.pop_front().expect("checked");
                    self.txbrp &= !bit;
                    self.txbcf |= bit;
                }
            }
        }
        if let Some(frame) = &update.rx {
            let _ = self.try_receive_frame(frame.clone());
        }
    }

    /// This controller's frames from the shared trace, oldest first. Derived
    /// per call — a projection of the one ring, not a second store. `seq` is
    /// therefore the GLOBAL sequence, which is what makes CAN frames orderable
    /// against I²C/SPI/UART traffic on the same machine.
    pub fn trace_snapshot(&self, peripheral: &str) -> Vec<FdcanTraceFrame> {
        crate::peripherals::can_trace_snapshot(&self.trace, &self.trace_name, peripheral)
    }

    fn push_trace(&mut self, direction: &'static str, frame: &CanFrame) {
        crate::peripherals::push_can_trace(&self.trace, &self.trace_name, direction, frame);
    }

    fn config_unlocked(&self) -> bool {
        (self.cccr & (CCCR_INIT | CCCR_CCE)) == (CCCR_INIT | CCCR_CCE)
    }

    fn running(&self) -> bool {
        self.cccr & CCCR_INIT == 0
    }

    fn loopback(&self) -> bool {
        self.cccr & CCCR_TEST != 0 && self.test & TEST_LBCK != 0
    }

    fn rxf0s(&self) -> u32 {
        (if self.rxf0_lost { 1 << 25 } else { 0 })
            | (if self.rxf0_fill == FIFO_DEPTH {
                1 << 24
            } else {
                0
            })
            | (self.rxf0_put << 16)
            | (self.rxf0_get << 8)
            | self.rxf0_fill
    }

    fn rxf1s(&self) -> u32 {
        (if self.rxf1_lost { 1 << 25 } else { 0 })
            | (if self.rxf1_fill == FIFO_DEPTH {
                1 << 24
            } else {
                0
            })
            | (self.rxf1_put << 16)
            | (self.rxf1_get << 8)
            | self.rxf1_fill
    }

    fn txfqs(&self) -> u32 {
        // TFFL reflects free slots: depth minus in-flight pending frames.
        let pending = self.pending_tx.len() as u32;
        let free = FIFO_DEPTH.saturating_sub(pending);
        (self.txfq_put << 16) | (self.txfq_get << 8) | free
    }

    fn psr(&self) -> u32 {
        // Reset: LEC = DLEC = 7 (no change). After the protocol has
        // run: LEC = 0, ACT = 01 idle — capture13 read 0x708.
        let base = if self.bus_active { 0x708 } else { 0x707 };
        if self.timed {
            let lec = if self.bus_active {
                u32::from(self.timed_lec)
            } else {
                7
            };
            return (base & !0x7)
                | lec
                | if self.bus_off { PSR_BO } else { 0 }
                | if self.error_warning { PSR_EW } else { 0 }
                | if self.error_passive { PSR_EP } else { 0 };
        }
        if self.bus_off {
            base | PSR_BO | PSR_EW | PSR_EP
        } else {
            base
        }
    }

    /// Force the controller into bus-off, as 256 transmit errors would: PSR
    /// BO/EW/EP set, ECR.TEC saturated and CEL counted, CCCR.INIT set (the
    /// controller leaves the bus), IR.BO/EW/EP raised. Pending transmissions
    /// stay pending (TXBRP set) and are sent after recovery. Firmware
    /// recovers by clearing CCCR.INIT.
    pub fn enter_bus_off(&mut self) {
        self.bus_off = true;
        let cel = ((self.ecr >> 16) & 0xFF).saturating_add(1).min(0xFF);
        self.ecr = (cel << 16) | 0xFF;
        self.cccr |= CCCR_INIT;
        self.ir |= IR_BO | IR_EW | IR_EP;
    }

    /// `true` while the controller is bus-off.
    pub fn is_bus_off(&self) -> bool {
        self.bus_off
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            REG_CREL => 0x3214_1218,
            REG_ENDN => 0x8765_4321,
            REG_DBTP => self.dbtp,
            REG_TEST => self.test,
            REG_RWD => self.rwd,
            REG_CCCR => self.cccr,
            REG_NBTP => self.nbtp,
            REG_TSCC => self.tscc,
            REG_TSCV => self.tscv,
            REG_TOCC => self.tocc,
            REG_TOCV => self.tocv,
            REG_ECR => self.ecr,
            REG_PSR => self.psr(),
            REG_TDCR => 0,
            REG_IR => self.ir,
            REG_IE => self.ie,
            REG_ILS => self.ils,
            REG_ILE => self.ile,
            REG_RXGFC => self.rxgfc,
            REG_XIDAM => self.xidam,
            REG_HPMS => 0,
            REG_RXF0S => self.rxf0s(),
            REG_RXF0A => self.rxf0_get,
            REG_RXF1S => self.rxf1s(),
            REG_RXF1A => self.rxf1_get,
            REG_TXBC => 0,
            REG_TXFQS => self.txfqs(),
            REG_TXBRP => self.txbrp,
            REG_TXBAR => 0,
            REG_TXBCR => 0,
            REG_TXBTO => self.txbto,
            REG_TXBCF => self.txbcf,
            REG_TXBTIE => self.txbtie,
            REG_TXBCIE => self.txbcie,
            REG_TXEFS => 0,
            REG_TXEFA => 0,
            REG_CKDIV => self.ckdiv,
            REG_OPTR => self.optr,
            // CONFIG identification — silicon capture13.
            REG_HWCFG => 0x0000_0022,
            REG_VERR => 0x0000_0010,
            REG_IPIDR => 0x0013_0072,
            REG_SIDR => 0xA3C5_DD01,
            _ => {
                crate::census_reg!("fdcan:Fdcan", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            REG_DBTP if self.config_unlocked() => self.dbtp = value,
            // TEST is writable only in test mode (CCCR.TEST set).
            REG_TEST if self.cccr & CCCR_TEST != 0 => self.test = value,
            REG_RWD if self.config_unlocked() => self.rwd = value,
            REG_CCCR => self.write_cccr(value),
            REG_NBTP if self.config_unlocked() => self.nbtp = value,
            REG_TSCC if self.config_unlocked() => self.tscc = value,
            REG_TSCV => self.tscv = 0, // write clears the counter
            REG_TOCC if self.config_unlocked() => self.tocc = value,
            REG_IR => self.ir &= !value,
            REG_IE => self.ie = value,
            REG_ILS => self.ils = value,
            REG_ILE => self.ile = value & 0x3,
            REG_RXGFC if self.config_unlocked() => self.rxgfc = value,
            REG_XIDAM if self.config_unlocked() => self.xidam = value,
            REG_RXF1A => self.ack_rx_fifo1(value),
            REG_RXF0A => self.ack_rx_fifo0(value),
            REG_TXBAR if self.running() => self.request_tx(value),
            REG_TXBCR => {
                // Nothing is ever pending long enough to cancel, but
                // firmware sees the cancellation acknowledged.
                self.txbrp &= !value;
                self.txbcf |= value & 0x7;
            }
            REG_TXBTIE => self.txbtie = value & 0x7,
            REG_TXBCIE => self.txbcie = value & 0x7,
            REG_CKDIV if self.config_unlocked() => self.ckdiv = value & 0xF,
            REG_OPTR => self.optr = value,
            _ => {
                crate::census_reg!("fdcan:Fdcan", offset, "write");
            }
        }
    }

    fn write_cccr(&mut self, value: u32) {
        let allowed = 0x0000_F3FF; // through NISO; reserved bits stay 0
        let mut next = value & allowed;
        // CCE can only be set while INIT is set, and clearing INIT
        // clears CCE with it (capture13: 0xA2 written, 0xA0 read).
        if next & CCCR_INIT == 0 {
            next &= !CCCR_CCE;
            self.bus_active = true;
            if self.timed {
                if self.bus_off && self.cccr & CCCR_INIT != 0 {
                    // Recovery: the bus counts 129 x 11 recessive bits from
                    // here (RM0481 §60.4.2); counters reset when it is done.
                    self.recovery_armed = Some(self.now());
                }
            } else if self.bus_off {
                // Bus-off recovery: error counters reset, BO clears.
                self.bus_off = false;
                self.ecr &= 0x00FF_0000;
            }
        } else if self.cccr & CCCR_INIT == 0 {
            // INIT being set: CCE in the same write doesn't take yet.
            next &= !CCCR_CCE;
            self.recovery_armed = None;
        }
        self.cccr = next;
    }

    fn ack_rx_fifo1(&mut self, value: u32) {
        if self.rxf1_fill == 0 {
            return;
        }
        let acked = (value & 0x3F) % FIFO_DEPTH;
        let consumed = ((acked + FIFO_DEPTH - self.rxf1_get) % FIFO_DEPTH + 1).min(self.rxf1_fill);
        self.rxf1_get = (acked + 1) % FIFO_DEPTH;
        self.rxf1_fill -= consumed;
    }

    /// Acceptance filtering (RM0456 §70.4.3 / M_CAN filter lists written by
    /// HAL_FDCAN_ConfigFilter): returns the RX FIFO and matching list index,
    /// or `None` when the frame is rejected. The filter elements live in the
    /// fixed message-RAM sections (standard 0x000, extended 0x070), and RXGFC
    /// carries the list lengths, non-matching actions and remote rejection.
    fn filter_route(&self, frame: &CanFrame) -> Option<RxFilterRoute> {
        let rxgfc = self.rxgfc;
        let reject_remote = if frame.extended {
            rxgfc & 1 != 0 // RRFE
        } else {
            rxgfc & 2 != 0 // RRFS
        };
        if frame.remote && reject_remote {
            return None;
        }
        if frame.extended {
            let count = ((rxgfc >> 24) & 0xF) as usize;
            for idx in 0..count {
                let base = RAM_EXT_FILTER_WORDS + idx * 2;
                let Some(&w0) = self.message_ram.get(base) else {
                    continue;
                };
                let Some(&w1) = self.message_ram.get(base + 1) else {
                    continue;
                };
                let config = (w0 >> 29) & 0x7;
                if config == 0 || config == 7 {
                    continue;
                }
                let id1 = w0 & 0x1FFF_FFFF;
                let filter_type = (w1 >> 30) & 0x3;
                let id2 = w1 & 0x1FFF_FFFF;
                let mask = if self.xidam == 0 {
                    0x1FFF_FFFF
                } else {
                    self.xidam
                };
                // EFT=3 is the extended range form that bypasses XIDAM.
                let id = if filter_type == 3 {
                    frame.id & 0x1FFF_FFFF
                } else {
                    frame.id & mask
                };
                let matches = match filter_type {
                    0 | 3 => id1 <= id && id <= id2,
                    1 => id == id1 || id == id2,
                    _ => (id & id2) == (id1 & id2),
                };
                if matches {
                    return accepted_filter_route(config, idx);
                }
            }
        } else {
            let count = ((rxgfc >> 16) & 0x1F) as usize;
            for idx in 0..count {
                let Some(&w) = self.message_ram.get(RAM_STD_FILTER_WORDS + idx) else {
                    continue;
                };
                let filter_type = (w >> 30) & 0x3;
                let config = (w >> 27) & 0x7;
                if config == 0 || config == 7 || filter_type == 3 {
                    continue;
                }
                let id1 = (w >> 16) & 0x7FF;
                let id2 = w & 0x7FF;
                let id = frame.id & 0x7FF;
                let matches = match filter_type {
                    0 => id1 <= id && id <= id2,
                    1 => id == id1 || id == id2,
                    _ => (id & id2) == (id1 & id2),
                };
                if matches {
                    return accepted_filter_route(config, idx);
                }
            }
        }
        // No filter matched: RXGFC.ANFS/ANFE.
        let nonmatching = if frame.extended {
            (rxgfc >> 2) & 0x3
        } else {
            (rxgfc >> 4) & 0x3
        };
        match nonmatching {
            0 => Some(RxFilterRoute {
                fifo: 0,
                filter_index: None,
            }),
            1 => Some(RxFilterRoute {
                fifo: 1,
                filter_index: None,
            }),
            _ => None,
        }
    }

    fn ack_rx_fifo0(&mut self, value: u32) {
        if self.rxf0_fill == 0 {
            return;
        }
        let acked = (value & 0x3F) % FIFO_DEPTH;
        let consumed = ((acked + FIFO_DEPTH - self.rxf0_get) % FIFO_DEPTH + 1).min(self.rxf0_fill);
        self.rxf0_get = (acked + 1) % FIFO_DEPTH;
        self.rxf0_fill -= consumed;
    }

    fn request_tx(&mut self, value: u32) {
        for idx in 0..FIFO_DEPTH {
            let bit = 1u32 << idx;
            if value & bit == 0 {
                continue;
            }
            let Some(frame) = self.decode_tx_element(idx as usize) else {
                continue;
            };
            // Assert the pending bit — it stays set until tick() delivers
            // the frame and posts the completion flags. Firmware polling
            // `while (TXBRP & 1) {}` will spin for at least one tick,
            // matching real M_CAN behavior (TXBRP clears only after the
            // frame has left the node on the physical bus).
            self.txbrp |= bit;
            self.tx_requested[idx as usize] = self.now();
            self.pending_tx.push_back((bit, frame));
        }
    }

    /// Complete all queued TX frames: deliver each frame to the bus or
    /// loopback receiver, then post the completion flags (TXBRP→0,
    /// TXBTO, IR.TFE / IR.TC). Called from `tick()` so the completion
    /// is always at least one tick after the TXBAR write.
    fn drain_pending_tx(&mut self) {
        if self.bus_off {
            // Off the bus: requests stay pending until recovery.
            return;
        }
        while let Some((bit, frame)) = self.pending_tx.pop_front() {
            self.complete_tx(bit, frame);
        }
    }

    /// One transmission done: TXBRP clears, TXBTO and IR.TFE (IR.TC for
    /// TXBTIE-enabled buffers) are set, and the frame goes to the loopback
    /// receiver, the interconnect or the local list. On a timed bus the
    /// bus has already delivered it.
    fn complete_tx(&mut self, bit: u32, frame: CanFrame) {
        self.txbrp &= !bit;
        self.txbto |= bit;
        self.txfq_put = (self.txfq_put + 1) % FIFO_DEPTH;
        self.txfq_get = self.txfq_put;
        // TFE always; TC only for TXBTIE-enabled buffers —
        // capture13 pinned TC clear with TXBTIE = 0.
        self.ir |= IR_TFE;
        if self.txbtie & bit != 0 {
            self.ir |= IR_TC;
        }
        self.push_trace("tx", &frame);
        if self.timed {
            return;
        }
        if self.loopback() {
            self.receive_frame(frame);
        } else if let Some(tx) = &self.bus_tx {
            let _ = tx.send(frame);
        } else {
            if self.tx_frames.len() >= 64 {
                self.tx_frames.pop_front();
            }
            self.tx_frames.push_back(frame);
        }
    }

    fn decode_tx_element(&self, index: usize) -> Option<CanFrame> {
        let base = RAM_TFQ_WORDS + index * ELEMENT_WORDS;
        let word0 = self.message_ram.get(base).copied()?;
        let word1 = self.message_ram.get(base + 1).copied()?;
        let extended = word0 & (1 << 30) != 0;
        let id = if extended {
            word0 & 0x1FFF_FFFF
        } else {
            (word0 >> 18) & 0x7FF
        };
        let len = dlc_to_len(((word1 >> 16) & 0xF) as u8);
        let mut data = Vec::with_capacity(len);
        for byte_idx in 0..len {
            let word = self.message_ram.get(base + 2 + byte_idx / 4).copied()?;
            data.push(((word >> ((byte_idx % 4) * 8)) & 0xFF) as u8);
        }
        Some(CanFrame {
            id,
            data,
            extended,
            fd: word1 & (1 << 21) != 0,             // T1.FDF
            bitrate_switch: word1 & (1 << 20) != 0, // T1.BRS
            remote: word0 & (1 << 29) != 0,         // T0.RTR
        })
    }

    /// Deliver a frame to the receiver — the entry point for loopback
    /// and for an external CAN network layer. Returns false when the
    /// FIFO was full and the frame was lost (RF0L).
    pub fn receive_frame(&mut self, frame: CanFrame) -> bool {
        self.try_receive_frame(frame).is_ok()
    }

    /// [`Self::receive_frame`], saying why a frame was not stored.
    pub fn try_receive_frame(
        &mut self,
        frame: CanFrame,
    ) -> Result<(), crate::network::CanRxRejection> {
        use crate::network::CanRxRejection;
        if !self.running() {
            return Err(CanRxRejection::NotRunning);
        }
        let route = self.filter_route(&frame);
        let route = match route {
            Some(route) => route,
            None => {
                self.push_trace("rx", &frame);
                return Err(CanRxRejection::NoFilterMatch);
            }
        };
        if route.fifo == 1 {
            if self.rxf1_fill >= FIFO_DEPTH {
                self.rxf1_lost = true;
                self.ir |= IR_RF1L;
                return Err(CanRxRejection::FifoFull);
            }
            self.push_trace("rx", &frame);
            let base = RAM_RF1_WORDS + self.rxf1_put as usize * ELEMENT_WORDS;
            self.encode_rx_element(base, &frame, route.filter_index);
            self.rxf1_put = (self.rxf1_put + 1) % FIFO_DEPTH;
            self.rxf1_fill += 1;
            self.ir |= IR_RF1N;
            return Ok(());
        }
        if self.rxf0_fill >= FIFO_DEPTH {
            self.rxf0_lost = true;
            self.ir |= IR_RF0L;
            return Err(CanRxRejection::FifoFull);
        }
        self.push_trace("rx", &frame);
        let base = RAM_RF0_WORDS + self.rxf0_put as usize * ELEMENT_WORDS;
        self.encode_rx_element(base, &frame, route.filter_index);
        self.rxf0_put = (self.rxf0_put + 1) % FIFO_DEPTH;
        self.rxf0_fill += 1;
        self.ir |= IR_RF0N;
        Ok(())
    }

    fn encode_rx_element(&mut self, base: usize, frame: &CanFrame, filter_index: Option<u8>) {
        if base + ELEMENT_WORDS > self.message_ram.len() {
            return;
        }
        self.message_ram[base] = if frame.extended {
            (1 << 30) | (frame.id & 0x1FFF_FFFF)
        } else {
            (frame.id & 0x7FF) << 18
        } | if frame.remote { 1 << 29 } else { 0 };
        // Preserve capture13's FIDX value for accepted non-matching frames;
        // a matched frame carries its real filter index with ANMF clear.
        let filter_metadata =
            filter_index.map_or((1 << 31) | (0x3F << 24), |index| u32::from(index) << 24);
        self.message_ram[base + 1] = filter_metadata
            | if frame.fd { 1 << 21 } else { 0 }
            | if frame.bitrate_switch { 1 << 20 } else { 0 }
            | ((len_to_dlc(frame.data.len()) as u32) << 16);
        for word in &mut self.message_ram[base + 2..base + ELEMENT_WORDS] {
            *word = 0;
        }
        for (idx, byte) in frame.data.iter().take(64).enumerate() {
            self.message_ram[base + 2 + idx / 4] |= (*byte as u32) << ((idx % 4) * 8);
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RxFilterRoute {
    fifo: u8,
    filter_index: Option<u8>,
}

fn accepted_filter_route(config: u32, index: usize) -> Option<RxFilterRoute> {
    let fifo = match config {
        1 | 5 => 0,
        2 | 6 => 1,
        _ => return None,
    };
    Some(RxFilterRoute {
        fifo,
        filter_index: Some(index as u8),
    })
}

fn dlc_to_len(dlc: u8) -> usize {
    match dlc {
        0..=8 => dlc as usize,
        9 => 12,
        10 => 16,
        11 => 20,
        12 => 24,
        13 => 32,
        14 => 48,
        _ => 64,
    }
}

fn len_to_dlc(len: usize) -> u8 {
    match len {
        0..=8 => len as u8,
        9..=12 => 9,
        13..=16 => 10,
        17..=20 => 11,
        21..=24 => 12,
        25..=32 => 13,
        33..=48 => 14,
        _ => 15,
    }
}

impl Default for Fdcan {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::network::timed_can::TimedCanController for Fdcan {
    fn attach_timed(&mut self) -> anyhow::Result<()> {
        Fdcan::attach_timed(self)
    }
    /// The FDCAN runs from its kernel clock, not the APB clock.
    fn timed_poll(&self, _apb_clock_hz: u64) -> crate::network::timed_can::CanNodePoll {
        Fdcan::timed_poll(self)
    }
    fn timed_apply(&mut self, update: &crate::network::timed_can::CanNodeUpdate) {
        Fdcan::timed_apply(self, update)
    }
}

impl Peripheral for Fdcan {
    fn timed_can(&self) -> Option<&dyn crate::network::timed_can::TimedCanController> {
        Some(self)
    }

    fn timed_can_mut(&mut self) -> Option<&mut dyn crate::network::timed_can::TimedCanController> {
        Some(self)
    }

    fn bus_trace_handle(&self) -> Option<crate::bus::bus_trace::BusTrace> {
        Some(self.trace.clone())
    }

    /// Join the machine's one bus trace; see [`crate::bus::bus_trace`].
    fn attach_bus_trace(&mut self, name: &str, trace: &crate::bus::bus_trace::BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.to_string();
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = Peripheral::read_u32(self, offset & !3)?;
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) as u32 * 8;
        if reg == REG_IR {
            // rc_w1: clear only the bits in this byte lane.
            self.ir &= !((value as u32) << shift);
            return Ok(());
        }
        let current = Peripheral::read_u32(self, reg)?;
        let next = (current & !(0xFF << shift)) | ((value as u32) << shift);
        Peripheral::write_u32(self, reg, next)
    }

    // IR is rc_w1: the default byte-decomposed 32-bit write would
    // read-modify-write the still-set bits back into the clear, wiping
    // the whole register (the F103 GPIO BSRR / EXTI SWIER failure
    // class — atomic word writes are required for w1c registers).
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let offset = offset & !3;
        if offset >= RAM_BASE {
            let idx = ((offset - RAM_BASE) / 4) as usize;
            return Ok(self.message_ram.get(idx).copied().unwrap_or(0));
        }
        Ok(self.read_reg(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let offset = offset & !3;
        if offset >= RAM_BASE {
            let idx = ((offset - RAM_BASE) / 4) as usize;
            if let Some(slot) = self.message_ram.get_mut(idx) {
                *slot = value;
            }
            return Ok(());
        }
        self.write_reg(offset, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Complete any pending TX frames queued by request_tx(). This
        // is the earliest point at which TXBRP can be cleared and
        // IR.TC/TFE posted — one tick after the TXBAR write, so firmware
        // polling `while (TXBRP & 1) {}` actually blocks.
        //
        // The walk skips `uses_scheduler()` peripherals, so production
        // scheduler mode rides `on_event` instead. Direct `tick()` calls
        // (unit tests, force-walk oracle path) still need this body.
        // Level interrupt on the configured line (FDCAN1_IT0); line 1
        // routing via ILS is not modeled.
        PeripheralTickResult::with_irq(self.service_once())
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.clock = Some(clock);
    }

    fn needs_legacy_walk(&self) -> bool {
        // Interconnect forces the walk (RX poll) — intentional multi-node
        // interim until bus_rx is event-driven. Feature-off / non-scheduler
        // builds also walk for TX completion + level IRQ. Scheduler mode
        // covers those without a walk when no CanBus is attached (single-node
        // green path for the H563 demo bus).
        !self.scheduler_mode()
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() || self.chain_live {
            return Vec::new();
        }
        // Arm delay-0 when TX is pending or a level IRQ is held — bus
        // converts to deadline `current_cycle + 1` (one tick after TXBAR).
        if self.chain_has_work() {
            self.chain_live = true;
            vec![(0, 0)]
        } else {
            Vec::new()
        }
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() {
            self.chain_live = false;
            return crate::sched::EventResult::default();
        }
        let irq = self.service_once();
        let reschedule = self.chain_has_work();
        self.chain_live = reschedule;
        crate::sched::EventResult {
            raise_own_irq: irq,
            reschedule_delay: reschedule.then_some(1),
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go through the trait's word accessors — the bus routes CPU word
    // stores to Peripheral::write_u32, and IR (rc_w1) depends on the
    // write arriving atomically rather than byte-decomposed.
    fn rd(dev: &Fdcan, offset: u64) -> u32 {
        Peripheral::read_u32(dev, offset).unwrap()
    }

    fn wr(dev: &mut Fdcan, offset: u64, value: u32) {
        Peripheral::write_u32(dev, offset, value).unwrap()
    }

    /// Drive the exact capture13 loopback sequence up to TXBAR.
    fn enter_loopback(dev: &mut Fdcan) {
        wr(dev, REG_CCCR, 0x3); // INIT | CCE
        wr(dev, REG_CCCR, 0xA3); // + TEST | MON
        wr(dev, REG_TEST, TEST_LBCK);
        // TX element 0 at SRAMCAN+0x278: std ID 0x123, DLC 8.
        wr(dev, RAM_BASE + 0x278, 0x123 << 18);
        wr(dev, RAM_BASE + 0x27C, 8 << 16);
        wr(dev, RAM_BASE + 0x280, 0xDEAD_BEEF);
        wr(dev, RAM_BASE + 0x284, 0xCAFE_BABE);
        wr(dev, REG_CCCR, 0xA2); // leave INIT, keep TEST | MON
    }

    #[test]
    fn bus_off_stops_traffic_until_firmware_clears_init() {
        let mut dev = Fdcan::new();
        enter_loopback(&mut dev);
        dev.enter_bus_off();
        assert_eq!(rd(&dev, REG_PSR) & 0xE0, 0xE0, "BO | EW | EP");
        assert_eq!(rd(&dev, REG_ECR) & 0xFF, 0xFF, "TEC saturated");
        assert_eq!(rd(&dev, REG_CCCR) & CCCR_INIT, CCCR_INIT);
        assert_ne!(rd(&dev, REG_IR) & IR_BO, 0);
        assert!(
            !dev.receive_frame(CanFrame::classic(0x1, vec![1])),
            "a bus-off controller takes no frames"
        );
        // A request queued before bus-off stays pending while off the bus.
        dev.pending_tx
            .push_back((1, CanFrame::classic(0x2, vec![2])));
        dev.txbrp |= 1;
        dev.tick();
        assert_eq!(rd(&dev, REG_TXBRP), 1, "no transmission while bus-off");
        // Recovery: firmware clears INIT.
        wr(&mut dev, REG_CCCR, 0xA2);
        assert!(!dev.is_bus_off());
        assert_eq!(rd(&dev, REG_PSR) & PSR_BO, 0);
        assert_eq!(rd(&dev, REG_ECR) & 0xFF, 0, "error counters reset");
        dev.tick();
        assert_eq!(
            rd(&dev, REG_TXBRP),
            0,
            "the pending frame goes out after recovery"
        );
    }

    #[test]
    fn reset_values_match_capture13() {
        let dev = Fdcan::new();
        assert_eq!(rd(&dev, REG_CREL), 0x3214_1218);
        assert_eq!(rd(&dev, REG_ENDN), 0x8765_4321);
        assert_eq!(rd(&dev, REG_DBTP), 0x0000_0A33);
        assert_eq!(rd(&dev, REG_CCCR), 0x0000_0001);
        assert_eq!(rd(&dev, REG_NBTP), 0x0600_0A03);
        assert_eq!(rd(&dev, REG_TOCC), 0xFFFF_0000);
        assert_eq!(rd(&dev, REG_TOCV), 0x0000_FFFF);
        assert_eq!(rd(&dev, REG_PSR), 0x0000_0707);
        assert_eq!(rd(&dev, REG_XIDAM), 0x1FFF_FFFF);
        assert_eq!(rd(&dev, REG_RXF0S), 0);
        assert_eq!(rd(&dev, REG_TXFQS), 0x0000_0003);
        assert_eq!(rd(&dev, REG_HWCFG), 0x0000_0022);
        assert_eq!(rd(&dev, REG_VERR), 0x0000_0010);
        assert_eq!(rd(&dev, REG_IPIDR), 0x0013_0072);
        assert_eq!(rd(&dev, REG_SIDR), 0xA3C5_DD01);
        assert_eq!(rd(&dev, REG_CKDIV), 0);
    }

    #[test]
    fn cce_follows_init_as_on_silicon() {
        let mut dev = Fdcan::new();
        // CCE cannot be set in the same write that sets INIT from 0...
        wr(&mut dev, REG_CCCR, 0x1);
        wr(&mut dev, REG_CCCR, 0x3);
        assert_eq!(rd(&dev, REG_CCCR), 0x3);
        wr(&mut dev, REG_CCCR, 0xA3);
        assert_eq!(rd(&dev, REG_CCCR), 0xA3);
        // ...and clearing INIT clears CCE with it (0xA2 -> 0xA0).
        wr(&mut dev, REG_CCCR, 0xA2);
        assert_eq!(rd(&dev, REG_CCCR), 0xA0);
    }

    #[test]
    fn config_registers_lock_outside_init_cce() {
        let mut dev = Fdcan::new();
        // INIT set but CCE clear: protected writes must not stick.
        wr(&mut dev, REG_NBTP, 0x1234_5678);
        assert_eq!(rd(&dev, REG_NBTP), 0x0600_0A03);
        wr(&mut dev, REG_RXGFC, 0xFFFF_FFFF);
        assert_eq!(rd(&dev, REG_RXGFC), 0);
        wr(&mut dev, REG_CCCR, 0x3);
        wr(&mut dev, REG_NBTP, 0x1234_5678);
        assert_eq!(rd(&dev, REG_NBTP), 0x1234_5678);
    }

    #[test]
    fn loopback_frame_lands_in_rx_fifo0_as_on_silicon() {
        let mut dev = Fdcan::new();
        enter_loopback(&mut dev);
        wr(&mut dev, REG_TXBAR, 0x1);
        // TX is asynchronous: TXBRP stays set until the next tick.
        assert_ne!(rd(&dev, REG_TXBRP), 0, "pending before tick");
        assert_eq!(rd(&dev, REG_TXBTO), 0, "not complete before tick");
        dev.tick();
        // Every value below is the capture13 post-TX read.
        assert_eq!(rd(&dev, REG_TXBRP), 0);
        assert_eq!(rd(&dev, REG_TXBTO), 0x1);
        assert_eq!(rd(&dev, REG_TXFQS), 0x0001_0103);
        assert_eq!(
            rd(&dev, REG_IR),
            0x0000_0201,
            "RF0N | TFE, TC gated by TXBTIE"
        );
        assert_eq!(rd(&dev, REG_RXF0S), 0x0001_0001);
        assert_eq!(rd(&dev, REG_PSR), 0x0000_0708);
        // RX element 0 at SRAMCAN+0xB0. Silicon leaves R0[17:0]
        // undefined for standard IDs — compare the masked field only.
        assert_eq!((rd(&dev, RAM_BASE + 0xB0) >> 18) & 0x7FF, 0x123);
        let r1 = rd(&dev, RAM_BASE + 0xB4);
        assert_eq!((r1 >> 16) & 0xF, 8, "DLC");
        assert_ne!(r1 & (1 << 31), 0, "ANMF");
        assert_eq!(rd(&dev, RAM_BASE + 0xB8), 0xDEAD_BEEF);
        assert_eq!(rd(&dev, RAM_BASE + 0xBC), 0xCAFE_BABE);
    }

    #[test]
    fn loopback_frames_are_exposed_as_can_trace_events() {
        let mut dev = Fdcan::new();
        enter_loopback(&mut dev);
        wr(&mut dev, REG_TXBAR, 0x1);
        // Trace events are posted by drain_pending_tx inside tick().
        dev.tick();

        let trace = dev.trace_snapshot("fdcan1");
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].peripheral, "fdcan1");
        assert_eq!(trace[0].direction, "tx");
        assert_eq!(trace[0].id, 0x123);
        assert_eq!(
            trace[0].data,
            vec![0xEF, 0xBE, 0xAD, 0xDE, 0xBE, 0xBA, 0xFE, 0xCA]
        );
        assert_eq!(trace[1].direction, "rx");
        assert_eq!(trace[1].id, 0x123);
    }

    #[test]
    fn ir_tc_set_when_txbtie_enables_the_buffer() {
        let mut dev = Fdcan::new();
        enter_loopback(&mut dev);
        wr(&mut dev, REG_TXBTIE, 0x1);
        wr(&mut dev, REG_TXBAR, 0x1);
        dev.tick();
        assert_ne!(rd(&dev, REG_IR) & IR_TC, 0);
    }

    #[test]
    fn ir_is_w1c_through_atomic_word_writes() {
        let mut dev = Fdcan::new();
        enter_loopback(&mut dev);
        wr(&mut dev, REG_TXBAR, 0x1);
        dev.tick();
        let ir = rd(&dev, REG_IR);
        assert_ne!(ir, 0);
        wr(&mut dev, REG_IR, ir);
        assert_eq!(rd(&dev, REG_IR), 0, "write-1-to-clear wiped by RMW");
    }

    #[test]
    fn rxf0a_ack_advances_get_index_as_on_silicon() {
        let mut dev = Fdcan::new();
        enter_loopback(&mut dev);
        wr(&mut dev, REG_TXBAR, 0x1);
        dev.tick();
        wr(&mut dev, REG_RXF0A, 0x0);
        // capture13: F0PI = 1, F0GI = 1, F0FL = 0.
        assert_eq!(rd(&dev, REG_RXF0S), 0x0001_0100);
    }

    #[test]
    fn rx_fifo0_overflow_sets_rf0l() {
        let mut dev = Fdcan::new();
        wr(&mut dev, REG_CCCR, 0x0);
        for i in 0..3 {
            assert!(dev.receive_frame(CanFrame::classic(0x100 + i, vec![i as u8])));
        }
        assert!(!dev.receive_frame(CanFrame::classic(0x200, vec![])));
        let s = rd(&dev, REG_RXF0S);
        assert_ne!(s & (1 << 25), 0, "RF0L");
        assert_eq!(s & 0x7F, 3);
        assert_ne!(rd(&dev, REG_IR) & IR_RF0L, 0);
    }

    #[test]
    fn extended_id_round_trips_through_loopback() {
        let mut dev = Fdcan::new();
        wr(&mut dev, REG_CCCR, 0x3);
        wr(&mut dev, REG_CCCR, 0xA3);
        wr(&mut dev, REG_TEST, TEST_LBCK);
        wr(
            &mut dev,
            RAM_BASE + 0x278,
            (1 << 30) | 0x1ABC_DEF0 & 0x1FFF_FFFF,
        );
        wr(&mut dev, RAM_BASE + 0x27C, 4 << 16);
        wr(&mut dev, RAM_BASE + 0x280, 0x0102_0304);
        wr(&mut dev, REG_CCCR, 0xA2);
        wr(&mut dev, REG_TXBAR, 0x1);
        dev.tick();
        let r0 = rd(&dev, RAM_BASE + 0xB0);
        assert_ne!(r0 & (1 << 30), 0, "XTD");
        assert_eq!(r0 & 0x1FFF_FFFF, 0x1ABC_DEF0 & 0x1FFF_FFFF);
        assert_eq!(rd(&dev, RAM_BASE + 0xB8), 0x0102_0304);
    }

    #[test]
    fn second_tx_advances_indices_and_second_rx_element() {
        let mut dev = Fdcan::new();
        enter_loopback(&mut dev);
        wr(&mut dev, REG_TXBAR, 0x1);
        dev.tick();
        // Reload buffer 0 with a different payload, send again.
        wr(&mut dev, RAM_BASE + 0x280, 0x1111_2222);
        wr(&mut dev, REG_TXBAR, 0x1);
        dev.tick();
        assert_eq!(rd(&dev, REG_TXFQS), 0x0002_0203);
        assert_eq!(rd(&dev, REG_RXF0S) & 0x7F, 2);
        // Second element at SRAMCAN + 0xB0 + 72.
        assert_eq!(rd(&dev, RAM_BASE + 0xB0 + 72 + 8), 0x1111_2222);
    }

    #[test]
    fn txbar_ignored_while_init_set() {
        let mut dev = Fdcan::new();
        wr(&mut dev, REG_TXBAR, 0x1);
        assert_eq!(rd(&dev, REG_TXBTO), 0);
        assert_eq!(rd(&dev, REG_IR), 0);
    }

    #[test]
    fn tick_raises_irq_only_with_ile_and_enabled_source() {
        let mut dev = Fdcan::new();
        enter_loopback(&mut dev);
        wr(&mut dev, REG_TXBAR, 0x1);
        assert!(!dev.tick().irq, "IR set but IE/ILE clear");
        wr(&mut dev, REG_IE, IR_RF0N);
        assert!(!dev.tick().irq, "line not enabled");
        wr(&mut dev, REG_ILE, 0x1);
        assert!(dev.tick().irq);
        let ir = rd(&dev, REG_IR);
        wr(&mut dev, REG_IR, ir);
        assert!(!dev.tick().irq, "cleared IR drops the level");
    }

    #[test]
    fn byte_reads_decompose_word_registers() {
        let dev = Fdcan::new();
        // ENDN = 0x87654321 read byte-wise — the classic endianness probe.
        assert_eq!(Peripheral::read(&dev, REG_ENDN).unwrap(), 0x21);
        assert_eq!(Peripheral::read(&dev, REG_ENDN + 3).unwrap(), 0x87);
    }

    #[test]
    fn message_ram_words_round_trip_across_both_sections() {
        let mut dev = Fdcan::new();
        // capture13 pattern: first/last word of each instance section.
        wr(&mut dev, RAM_BASE, 0xA5A5_A5A5);
        wr(&mut dev, RAM_BASE + 0x34C, 0x5A5A_5A5A);
        wr(&mut dev, RAM_BASE + 0x350, 0x1122_3344);
        wr(&mut dev, RAM_BASE + 0x69C, 0x5566_7788);
        assert_eq!(rd(&dev, RAM_BASE), 0xA5A5_A5A5);
        assert_eq!(rd(&dev, RAM_BASE + 0x34C), 0x5A5A_5A5A);
        assert_eq!(rd(&dev, RAM_BASE + 0x350), 0x1122_3344);
        assert_eq!(rd(&dev, RAM_BASE + 0x69C), 0x5566_7788);
        // Out of window: reads 0, write doesn't panic.
        wr(&mut dev, RAM_BASE + 0x6A0, 0xFFFF_FFFF);
        assert_eq!(rd(&dev, RAM_BASE + 0x6A0), 0);
    }

    /// Single-node H5 demo path: no CanBus → scheduler owns TX/IRQ under
    /// `event-scheduler`, so the peripheral is not a walk forcer.
    #[test]
    fn single_node_is_walk_free_under_event_scheduler() {
        let dev = Fdcan::new();
        #[cfg(feature = "event-scheduler")]
        {
            assert!(
                dev.uses_scheduler(),
                "single-node FDCAN must ride the scheduler"
            );
            assert!(
                !dev.needs_legacy_walk(),
                "single-node FDCAN must not force the legacy walk"
            );
        }
        #[cfg(not(feature = "event-scheduler"))]
        {
            assert!(!dev.uses_scheduler());
            assert!(
                dev.needs_legacy_walk(),
                "feature-off builds keep TX/IRQ on the walk"
            );
        }
    }

    /// Multi-node CanBus attach forces the walk so `bus_rx` keeps draining.
    /// Do not clear this without an event-driven drain + dual-lane fidelity
    /// gate — silent walk-free multi-node starves RX.
    #[test]
    fn canbus_attach_forces_legacy_walk_for_rx_poll() {
        use std::sync::mpsc::channel;
        let (out_tx, _out_rx) = channel::<CanFrame>();
        let (_in_tx, in_rx) = channel::<CanFrame>();
        let dev = Fdcan::new_with_bus(out_tx, in_rx);
        assert!(
            !dev.uses_scheduler(),
            "attached CanBus must leave scheduler mode (RX is walk-polled)"
        );
        assert!(
            dev.needs_legacy_walk(),
            "attached CanBus must force legacy walk so bus_rx is not starved"
        );
    }

    /// End-to-end CanBus mpsc path still works under the walk (TX A → RX B
    /// via tick). Proves multi-node fidelity for the interim walk mode.
    #[test]
    fn canbus_path_tx_sends_and_rx_drains_on_tick() {
        use std::sync::mpsc::channel;
        let (out_tx, out_rx) = channel::<CanFrame>();
        let (in_tx, in_rx) = channel::<CanFrame>();
        let mut dev = Fdcan::new_with_bus(out_tx, in_rx);

        // Leave INIT; no loopback — frames go to bus_tx / bus_rx.
        wr(&mut dev, REG_CCCR, 0x3);
        wr(&mut dev, REG_CCCR, 0x0);
        wr(&mut dev, RAM_BASE + 0x278, 0x123 << 18);
        wr(&mut dev, RAM_BASE + 0x27C, 2 << 16);
        wr(&mut dev, RAM_BASE + 0x280, 0x0000_BEEF);
        wr(&mut dev, REG_TXBAR, 0x1);
        assert_ne!(rd(&dev, REG_TXBRP), 0, "pending before tick");
        dev.tick();
        assert_eq!(rd(&dev, REG_TXBRP), 0);
        let sent = out_rx.try_recv().expect("frame on CanBus TX");
        assert_eq!(sent.id, 0x123);
        assert_eq!(sent.data, vec![0xEF, 0xBE]);

        // Peer injects a frame; walk tick drains bus_rx into RX FIFO0.
        in_tx
            .send(CanFrame::classic(0x456, vec![0xCA, 0xFE]))
            .unwrap();
        assert_eq!(rd(&dev, REG_RXF0S) & 0x7F, 0, "not delivered before tick");
        dev.tick();
        assert_eq!(rd(&dev, REG_RXF0S) & 0x7F, 1);
        assert_eq!((rd(&dev, RAM_BASE + 0xB0) >> 18) & 0x7FF, 0x456);
        assert_eq!(rd(&dev, RAM_BASE + 0xB8) & 0xFFFF, 0xFECA);
        assert_ne!(rd(&dev, REG_IR) & IR_RF0N, 0);
    }

    // ---- Timed CAN bus mode ----

    use crate::network::timed_can::{lec, CanErrorState, CanNodeUpdate, CanTxOutcome};

    const IR_PEA: u32 = 1 << 21;
    const CCCR_DAR: u32 = 1 << 6;

    fn timed() -> Fdcan {
        let mut dev = Fdcan::new();
        dev.attach_timed().unwrap();
        dev.attach_cycle_clock(crate::CycleClock::default());
        wr(&mut dev, REG_CCCR, 0x0);
        dev
    }

    fn queue(dev: &mut Fdcan, id: u32) {
        wr(dev, RAM_BASE + 0x278, id << 18);
        wr(dev, RAM_BASE + 0x27C, 1 << 16);
        wr(dev, RAM_BASE + 0x280, 0x5A);
        wr(dev, REG_TXBAR, 1);
    }

    fn update(
        tx: Option<(u32, CanTxOutcome)>,
        tec: u8,
        rec: u8,
        state: CanErrorState,
    ) -> CanNodeUpdate {
        CanNodeUpdate {
            tx,
            rx: None,
            tec,
            rec,
            state,
            lec: None,
        }
    }

    #[test]
    fn timed_bit_time_comes_from_nbtp_and_the_kernel_clock() {
        // RM0481 §60.4.1: tq = (NBRP+1) / f_fdcan_ker_ck, bit = tq x
        // (1 + (NTSEG1+1) + (NTSEG2+1)). Reset NBTP (NBRP 0, NTSEG1 10,
        // NTSEG2 3) is 16 tq: 500 kbit/s from the 8 MHz HSE kernel clock.
        let dev = timed();
        let poll = dev.timed_poll();
        assert_eq!(poll.bit_ps, Some(2_000_000));
        assert_eq!(poll.clock_hz, 8_000_000);
        assert!(poll.on_bus && poll.fd_capable);
        assert_eq!(poll.recovery_occurrences, 129, "M_CAN waits 129 x 11 bits");
        // CKDIV.PDIV = 1 divides the kernel clock by 2.
        let mut dev = Fdcan::new();
        dev.attach_timed().unwrap();
        wr(&mut dev, REG_CCCR, 0x3);
        wr(&mut dev, REG_CKDIV, 1);
        assert!(!dev.timed_poll().on_bus, "INIT: off the bus");
        wr(&mut dev, REG_CCCR, 0x0);
        assert_eq!(dev.timed_poll().bit_ps, Some(4_000_000));
    }

    #[test]
    fn timed_tx_completes_when_the_bus_says_so() {
        let mut dev = timed();
        dev.clock.as_ref().unwrap().publish(77);
        queue(&mut dev, 0x321);
        dev.tick();
        assert_eq!(rd(&dev, REG_TXBRP), 1, "still pending after a tick");
        let tx = dev.timed_poll().tx.expect("request");
        assert_eq!((tx.tag, tx.frame.id, tx.requested_cycle), (1, 0x321, 77));
        dev.timed_apply(&update(
            Some((1, CanTxOutcome::Error)),
            8,
            0,
            CanErrorState::Active,
        ));
        assert_eq!(rd(&dev, REG_TXBRP), 1, "retransmitted");
        dev.timed_apply(&update(
            Some((1, CanTxOutcome::Ok)),
            7,
            0,
            CanErrorState::Active,
        ));
        assert_eq!(rd(&dev, REG_TXBRP), 0);
        assert_eq!(rd(&dev, REG_TXBTO), 1);
        assert_ne!(rd(&dev, REG_IR) & IR_TFE, 0);
        assert!(dev.tx_frames.is_empty());
        // DAR: one attempt; a failed one is cancelled (TXBCF).
        let mut dev = timed();
        wr(&mut dev, REG_CCCR, 0x3);
        wr(&mut dev, REG_CCCR, CCCR_DAR);
        queue(&mut dev, 0x321);
        dev.timed_apply(&update(
            Some((1, CanTxOutcome::NoAck)),
            8,
            0,
            CanErrorState::Active,
        ));
        assert_eq!(rd(&dev, REG_TXBRP), 0);
        assert_eq!(rd(&dev, REG_TXBCF), 1);
        assert_eq!(rd(&dev, REG_TXBTO), 0);
    }

    #[test]
    fn timed_ecr_psr_and_ir_mirror_the_bus_counters() {
        let mut dev = timed();
        let mut u = update(None, 136, 5, CanErrorState::Passive);
        u.lec = Some(lec::BIT_RECESSIVE);
        dev.timed_apply(&u);
        // RM0481 FDCAN_ECR: CEL[23:16], RP[15], REC[14:8], TEC[7:0].
        assert_eq!(rd(&dev, REG_ECR), (1 << 16) | (5 << 8) | 136);
        // FDCAN_PSR: BO[7], EW[6], EP[5], ACT[4:3], LEC[2:0].
        let psr = rd(&dev, REG_PSR);
        assert_eq!(psr & 0xE7, PSR_EP | PSR_EW | 4);
        assert_eq!(
            rd(&dev, REG_IR) & (IR_EP | IR_EW | IR_PEA),
            IR_EP | IR_EW | IR_PEA
        );
        // REC above 127 shows as RP with REC saturated at 127.
        dev.timed_apply(&update(None, 0, 200, CanErrorState::Passive));
        assert_eq!(rd(&dev, REG_ECR) & 0xFF00, (1 << 15) | (127 << 8));
        // Bus-off: PSR.BO, IR.BO, and the controller sets CCCR.INIT.
        wr(&mut dev, REG_IR, 0xFFFF_FFFF);
        dev.timed_apply(&update(None, 248, 0, CanErrorState::BusOff));
        assert_ne!(rd(&dev, REG_PSR) & PSR_BO, 0);
        assert_ne!(rd(&dev, REG_IR) & IR_BO, 0);
        assert_eq!(rd(&dev, REG_CCCR) & CCCR_INIT, CCCR_INIT);
        assert_eq!(dev.timed_poll().recovery_armed_cycle, None);
        // Firmware clears INIT: recovery counts from then (and only then).
        dev.clock.as_ref().unwrap().publish(4242);
        wr(&mut dev, REG_CCCR, 0x0);
        assert!(dev.is_bus_off(), "the 129 x 11 bit wait is the bus's");
        assert_eq!(dev.timed_poll().recovery_armed_cycle, Some(4242));
        dev.timed_apply(&update(None, 0, 0, CanErrorState::Active));
        assert!(!dev.is_bus_off());
        assert_eq!(rd(&dev, REG_PSR) & (PSR_BO | PSR_EP | PSR_EW), 0);
        assert_eq!(rd(&dev, REG_ECR) & 0xFFFF, 0);
    }

    #[test]
    fn timed_rx_goes_to_fifo0() {
        let mut dev = timed();
        let mut u = update(None, 0, 0, CanErrorState::Active);
        u.rx = Some(CanFrame::classic(0x7E8, vec![1, 2]));
        u.lec = Some(lec::NO_ERROR);
        dev.timed_apply(&u);
        assert_eq!(rd(&dev, REG_RXF0S) & 0x7F, 1);
        assert_eq!((rd(&dev, RAM_BASE + 0xB0) >> 18) & 0x7FF, 0x7E8);
    }

    #[test]
    fn timed_mode_needs_the_walk_only_without_the_scheduler() {
        let mut dev = Fdcan::new();
        dev.attach_timed().unwrap();
        assert!(dev.is_timed());
        // Exactly one of the event scheduler (where the build has it) and
        // the walk holds the interrupt level.
        assert_eq!(dev.needs_legacy_walk(), !dev.uses_scheduler());
        assert!(dev.attach_timed().is_err());
        // A pending frame is the bus's to complete: it arms no chain.
        wr(&mut dev, REG_CCCR, 0x0);
        queue(&mut dev, 0x123);
        assert!(dev.take_scheduled_events().is_empty());
    }
}
