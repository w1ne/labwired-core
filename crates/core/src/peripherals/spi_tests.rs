/// The named accessors index `PadLines` by `SpiSignal as usize`. Reordering
/// either the enum or `SPI_LINES` alone would silently publish MOSI's level
/// on the SCK lane — a waveform that looks plausible and is wrong.
#[test]
fn spi_line_order_matches_signal_discriminants() {
    use super::{SpiSignal, SPI_LINES};
    assert_eq!(SPI_LINES[SpiSignal::Sck as usize], "SCK");
    assert_eq!(SPI_LINES[SpiSignal::Mosi as usize], "MOSI");
    assert_eq!(SPI_LINES[SpiSignal::Miso as usize], "MISO");
    assert_eq!(SPI_LINES.len(), 3);
}

use super::{Spi, SpiDevice, SpiRegisterLayout};
use crate::Peripheral;

/// SPI slave that records every byte it receives.
struct Capture {
    rx: Vec<u8>,
}
impl SpiDevice for Capture {
    fn transfer(&mut self, mosi: u8) -> u8 {
        self.rx.push(mosi);
        0
    }
    fn cs_pin(&self) -> &str {
        ""
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

fn captured(spi: &Spi) -> Vec<u8> {
    spi.attached_devices[0]
        .as_any()
        .unwrap()
        .downcast_ref::<Capture>()
        .unwrap()
        .rx
        .clone()
}

/// Clock the bit engine to completion (DR writes no longer complete
/// instantly — the frame is stretched over simulated cycles).
fn run_engine(spi: &mut Spi) {
    for _ in 0..1_000_000 {
        if !spi.transfer_active() {
            return;
        }
        spi.tick_elapsed(8);
    }
    panic!("STM32 SPI bit engine did not complete");
}

/// FIFO-family SPI: a 16-bit DR write at DS=8 packs TWO frames — the
/// silicon behaviour that broke the real Nokia 5110 panel. The second
/// frame clocks back-to-back after the first on the wire.
#[test]
fn fifo_packs_u16_dr_write_into_two_frames() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Stm32Fifo);
    spi.push_device(Box::new(Capture { rx: Vec::new() }));
    spi.write(0x00, 0x40).unwrap(); // CR1: SPE
    spi.write_u16(0x0C, 0x00AB).unwrap(); // 16-bit DR write, DS=8 (reset 0x0700)
    run_engine(&mut spi);
    assert_eq!(
        captured(&spi),
        vec![0xAB, 0x00],
        "DS≤8 + 16-bit DR ⇒ 2 frames"
    );
}

/// The correct 8-bit DR access sends exactly one frame, even on FIFO parts.
#[test]
fn fifo_u8_dr_write_is_one_frame() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Stm32Fifo);
    spi.push_device(Box::new(Capture { rx: Vec::new() }));
    spi.write(0x00, 0x40).unwrap();
    spi.write(0x0C, 0xAB).unwrap(); // 8-bit DR write
    run_engine(&mut spi);
    assert_eq!(captured(&spi), vec![0xAB], "8-bit DR ⇒ 1 frame");
}

/// Non-FIFO STM32 (F1/F4) does NOT pack: a 16-bit DR write is one frame,
/// so the F103 ILI9341 lab (which writes DR as u16) is unaffected.
#[test]
fn plain_stm32_does_not_pack() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Stm32);
    spi.push_device(Box::new(Capture { rx: Vec::new() }));
    spi.write(0x00, 0x40).unwrap();
    spi.write_u16(0x0C, 0x00AB).unwrap();
    run_engine(&mut spi);
    assert_eq!(captured(&spi), vec![0xAB], "non-FIFO ⇒ 1 frame");
}

#[test]
fn test_spi_transfer_timing() {
    let mut spi = Spi::new();
    // Enable SPI + BR=1 (f_pclk/4): (1<<6) | (1<<3) = 0x48.
    spi.write(0x00, 0x48).unwrap();

    // Reset SR has TXE set (bit 1).
    assert_ne!(spi.read(0x08).unwrap() & 0x02, 0);

    // Write DR -> start transfer.
    spi.write(0x0C, 0xAA).unwrap();
    let sr = spi.read(0x08).unwrap();
    assert_ne!(sr & 0x80, 0, "BSY set during transfer");
    assert_eq!(sr & 0x02, 0, "TXE cleared while shifting");

    // BR=1 -> divider=4 -> 8 bits * 4 = 32 ticks.
    for _ in 0..31 {
        spi.tick();
        assert_ne!(spi.read(0x08).unwrap() & 0x80, 0, "still busy mid-transfer");
    }

    spi.tick();
    let sr = spi.read(0x08).unwrap();
    assert_eq!(sr & 0x80, 0, "BSY cleared after transfer");
    assert_ne!(sr & 0x02, 0, "TXE set after transfer");
    // Full-duplex master: the receive ALWAYS completes. Silicon samples the
    // MISO line every frame and asserts RXNE when the RX buffer fills, slave
    // or no slave — with nothing driving, the captured value is simply the
    // idle line level (0x00 here), which is data, not a missing event.
    //
    // This assertion previously read `RXNE NOT set without a slave`, pinning
    // the opposite. That was wrong about the hardware and had a real cost:
    // any polling driver that writes DR then waits for RXNE — which is what
    // HAL_SPI_TransmitReceive and therefore Arduino's SPI.transfer() do —
    // hung forever on an unpopulated bus. Corrected when the Arduino
    // conformance sketch on F401 hung in SPI.transfer().
    // CLASSIC (F1/F4) port — no RX FIFO, so a completed full-duplex frame
    // always asserts RXNE, slave or no slave: silicon samples MISO every
    // frame and the captured value is simply the idle level.
    //
    // This is the opposite of the FIFO port (L4/F7/G4), where RXNE follows
    // CR2.FRXTH and a single 8-bit frame at the reset threshold (16 bit)
    // leaves RXNE clear — verified on a real NUCLEO-L476RG, SR=0x0002.
    // Both behaviours are now modelled; do not "unify" them.
    assert_ne!(sr & 0x01, 0, "classic port sets RXNE on every frame");
    assert_eq!(
        spi.read(0x0C).unwrap(),
        0x00,
        "DR holds the idle MISO level when no slave drives"
    );
}

/// Analytic wire time: a frame completes at EXACTLY `bits × 2^(BR+1)`
/// peripheral-clock cycles, for two BR settings (and 16-bit DFF frames on
/// the classic port take twice the clocks of 8-bit ones).
#[test]
fn frame_completes_at_exact_derived_cycle_for_two_br_settings() {
    // (CR1 BR bits, expected cycles for an 8-bit frame)
    for (br, expected) in [(0u16, 8 * 2u64), (4u16, 8 * 32u64)] {
        let mut spi = Spi::new();
        spi.write_u16(0x00, (1 << 6) | (br << 3)).unwrap(); // SPE | BR
        spi.write(0x0C, 0xA5).unwrap();
        let mut cycles = 0u64;
        while spi.transfer_active() {
            spi.tick_elapsed(1);
            cycles += 1;
            assert!(cycles < 1_000_000, "engine never completed");
        }
        assert_eq!(
            cycles, expected,
            "BR={br}: 8-bit frame must complete at bits × 2^(BR+1) cycles"
        );
    }
    // Classic 16-bit frames (CR1.DFF): twice the clocks at the same BR.
    let mut spi = Spi::new();
    spi.write_u16(0x00, (1 << 6) | (1 << 3) | (1 << 11))
        .unwrap(); // SPE|BR=1|DFF
    spi.write_u16(0x0C, 0xBEEF).unwrap();
    let mut cycles = 0u64;
    while spi.transfer_active() {
        spi.tick_elapsed(1);
        cycles += 1;
        assert!(cycles < 1_000_000, "engine never completed");
    }
    assert_eq!(cycles, 16 * 4, "DFF frame = 16 bits × 2^(BR+1) cycles");
}

/// Mode-3 + LSBFIRST wire shape: SCK idles HIGH (CPOL=1), data is driven
/// on the leading (falling) edge and sampled on the trailing (rising)
/// edge (CPHA=1), and the bit order is LSB first. Decoding the MOSI line
/// at every SCK rising edge must reproduce the written byte.
#[test]
fn mode3_lsbfirst_waveform_samples_on_trailing_edge() {
    let mut spi = Spi::new();
    let lines = spi.line_levels_arc();
    // CR1: SPE | CPOL | CPHA | LSBFIRST, BR=0 (half-period = 1 cycle).
    spi.write_u16(0x00, (1 << 6) | (1 << 1) | 1 | (1 << 7))
        .unwrap();
    assert!(lines.sck(), "idle SCK level must be CPOL = 1");

    spi.write(0x0C, 0xB4).unwrap();
    let mut prev = lines.sck();
    let mut bits = Vec::new();
    for _ in 0..16 {
        spi.tick_elapsed(1);
        let sck = lines.sck();
        if sck && !prev {
            bits.push(lines.mosi()); // sample on the trailing (rising) edge
        }
        prev = sck;
    }
    assert!(!spi.transfer_active(), "16 half-periods complete the frame");
    assert!(lines.sck(), "SCK returns to the CPOL idle level");
    assert_eq!(bits.len(), 8, "8 trailing edges per 8-bit frame");
    let byte = bits
        .iter()
        .enumerate()
        .fold(0u8, |acc, (i, &b)| acc | (u8::from(b) << i));
    assert_eq!(byte, 0xB4, "LSB-first decode at the mode-3 sample edges");
}

/// Full-duplex fidelity: the byte the slave answers at the frame boundary
/// is what lands in DR when the SAME frame finishes clocking — not a byte
/// from a previous frame, and not delivered before wire time.
#[test]
fn slave_answer_clocks_back_during_the_same_frame() {
    struct Sequenced {
        next: u8,
    }
    impl SpiDevice for Sequenced {
        fn transfer(&mut self, _mosi: u8) -> u8 {
            let out = self.next;
            self.next = self.next.wrapping_add(1);
            out
        }
        fn cs_pin(&self) -> &str {
            ""
        }
    }
    let mut spi = Spi::new();
    spi.push_device(Box::new(Sequenced { next: 0x51 }));
    spi.write(0x00, 0x48).unwrap(); // SPE | BR=1
    spi.write(0x0C, 0x01).unwrap();
    assert_eq!(
        spi.read(0x08).unwrap() & 0x01,
        0,
        "RXNE must not assert before the frame finishes on the wire"
    );
    run_engine(&mut spi);
    assert_eq!(spi.read(0x0C).unwrap(), 0x51, "first frame's answer");
    assert_eq!(
        spi.read(0x08).unwrap() & 0x01,
        0,
        "RXNE must clear on DR read"
    );
    spi.write(0x0C, 0x02).unwrap();
    run_engine(&mut spi);
    assert_eq!(spi.read(0x0C).unwrap(), 0x52, "second frame's answer");
}

// ── Opt-in edge (bit-level) slave sampling ────────────────────────────────

/// Slave that declares its own CPOL/CPHA and records the bytes the wire
/// actually delivered to it. Its answer is a constant, independent of what
/// it receives, so a corrupted read can only come from the wire.
struct EdgeSlave {
    mode: u8,
    answer: u8,
    rx: Vec<u8>,
    opt_in: bool,
}
impl EdgeSlave {
    fn new(mode: u8, answer: u8) -> Self {
        Self {
            mode,
            answer,
            rx: Vec::new(),
            opt_in: true,
        }
    }
    /// Same device, same declared mode, but staying on the default
    /// byte-level path — the control arm that proves the corruption below
    /// comes from the opt-in and not from the test rig.
    fn byte_level(mode: u8, answer: u8) -> Self {
        Self {
            opt_in: false,
            ..Self::new(mode, answer)
        }
    }
}
impl SpiDevice for EdgeSlave {
    fn sampling(&self) -> super::SpiSampling {
        if self.opt_in {
            super::SpiSampling::edge_mode(self.mode)
        } else {
            super::SpiSampling::Byte
        }
    }
    fn transfer(&mut self, mosi: u8) -> u8 {
        self.rx.push(mosi);
        self.answer
    }
    fn cs_pin(&self) -> &str {
        ""
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// Clock `bytes` through a master programmed for `master_mode` against
/// `slave`, returning `(bytes the master read back, bytes the slave got)`.
fn exchange(slave: EdgeSlave, master_mode: u8, bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut spi = Spi::new();
    spi.push_device(Box::new(slave));
    let cpol = u16::from(master_mode & 0b10 != 0);
    let cpha = u16::from(master_mode & 0b01 != 0);
    // SPE | BR=1 | CPOL | CPHA
    spi.write_u16(0x00, (1 << 6) | (1 << 3) | (cpol << 1) | cpha)
        .unwrap();
    let mut read = Vec::new();
    for &b in bytes {
        spi.write(0x0C, b).unwrap();
        run_engine(&mut spi);
        read.push(spi.read(0x0C).unwrap());
    }
    let rx = spi.attached_devices[0]
        .as_any()
        .unwrap()
        .downcast_ref::<EdgeSlave>()
        .unwrap()
        .rx
        .clone();
    (read, rx)
}

/// Matched modes must round-trip EXACTLY — in all four modes, both
/// directions. Without this the mismatch tests below would pass vacuously
/// (a model that corrupts everything corrupts mismatches too).
#[test]
fn edge_slave_round_trips_when_modes_match() {
    for mode in 0..=3u8 {
        let sent = [0xA5u8, 0x3C, 0xFF, 0x01];
        let (read, rx) = exchange(EdgeSlave::new(mode, 0xB3), mode, &sent);
        assert_eq!(
            rx,
            sent.to_vec(),
            "mode {mode}: slave must latch exactly what the master sent"
        );
        assert_eq!(
            read,
            vec![0xB3; 4],
            "mode {mode}: master must read exactly what the slave answered"
        );
    }
}

/// CPHA mismatch, master leading: the slave presents its first MISO bit on
/// the very edge the master latches on, so the master reads the level the
/// pad still carried plus the slave's bits shifted down one — the classic
/// off-by-one-bit symptom of a mode mismatch on real hardware.
#[test]
fn edge_slave_cpha_mismatch_shifts_the_read_back() {
    let (read, rx) = exchange(EdgeSlave::new(1, 0xB3), 0, &[0xA5, 0xA5]);
    // First frame: MISO idled low, so bit 7 is 0 and 0xB3 arrives >> 1.
    assert_eq!(read[0], 0xB3 >> 1, "0xB3 sampled half a bit period late");
    // Second frame: the pad still held the slave's last bit, which is what
    // the master latches first.
    assert_eq!(read[1], 0x80 | (0xB3 >> 1));
    assert_ne!(read[0], 0xB3, "a mode mismatch must NOT read back cleanly");
    // The MOSI direction survives this particular pairing: the slave's
    // sample edge lands on the master's bit boundary and latches the level
    // still on the pad (propagation delay), which is the outgoing bit.
    assert_eq!(rx, vec![0xA5, 0xA5]);
}

/// The mirror image: with the master at CPHA=1 and the slave at CPHA=0 the
/// corruption lands on MOSI — the slave latches one edge early.
#[test]
fn edge_slave_cpha_mismatch_shifts_what_the_slave_receives() {
    let (read, rx) = exchange(EdgeSlave::new(0, 0xB3), 1, &[0xA5, 0x3C]);
    assert_eq!(rx[0], 0xA5 >> 1, "slave latched one edge early");
    assert_eq!(
        rx[1],
        0x80 | (0x3C >> 1),
        "the bit still on the pad from the previous frame leads"
    );
    assert_ne!(rx[0], 0xA5, "a mode mismatch must NOT deliver cleanly");
    // MISO survives this pairing (mirror of the test above).
    assert_eq!(read, vec![0xB3, 0xB3]);
}

/// The control arm. The SAME mismatch across the SAME device model, with
/// the opt-in switched off, keeps exchanging clean bytes — i.e. the
/// corruption above is the opt-in doing its job, not the rig.
#[test]
fn byte_level_slave_is_untouched_by_a_mode_mismatch() {
    for master_mode in 0..=3u8 {
        for slave_mode in 0..=3u8 {
            let (read, rx) = exchange(
                EdgeSlave::byte_level(slave_mode, 0xB3),
                master_mode,
                &[0xA5],
            );
            assert_eq!(read, vec![0xB3], "byte-level read must not change");
            assert_eq!(rx, vec![0xA5], "byte-level delivery must not change");
        }
    }
}

/// The opt-in must not disturb frame timing: an edge-sampled frame takes
/// exactly the same number of peripheral-clock cycles as a byte-level one.
#[test]
fn edge_sampling_does_not_change_frame_wire_time() {
    fn cycles(opt_in: bool) -> u64 {
        let mut spi = Spi::new();
        spi.push_device(Box::new(if opt_in {
            EdgeSlave::new(0, 0xB3)
        } else {
            EdgeSlave::byte_level(0, 0xB3)
        }));
        spi.write_u16(0x00, (1 << 6) | (1 << 3)).unwrap();
        spi.write(0x0C, 0xA5).unwrap();
        let mut n = 0;
        while spi.transfer_active() {
            spi.tick_elapsed(1);
            n += 1;
        }
        n
    }
    assert_eq!(cycles(true), cycles(false), "8 bits x 2^(BR+1) either way");
}

/// Cost SHAPE, not wall clock (that lives in `tests::bench_spi_engine`):
/// neither path may consult the device more than once per frame. A
/// per-bit device call would be the obvious way to make edge sampling
/// eat CPU, and this fails the moment one appears.
#[test]
fn neither_path_consults_the_device_more_than_once_per_frame() {
    for opt_in in [false, true] {
        let mut spi = Spi::new();
        spi.push_device(Box::new(if opt_in {
            EdgeSlave::new(0, 0xB3)
        } else {
            EdgeSlave::byte_level(0, 0xB3)
        }));
        spi.write_u16(0x00, (1 << 6) | (1 << 3)).unwrap();
        for b in 0..4u8 {
            spi.write(0x0C, b).unwrap();
            run_engine(&mut spi);
        }
        let calls = spi.attached_devices[0]
            .as_any()
            .unwrap()
            .downcast_ref::<EdgeSlave>()
            .unwrap()
            .rx
            .len();
        assert_eq!(calls, 4, "opt_in={opt_in}: one transfer() per frame");
    }
}

/// The MISO pad itself carries the slave's phase: with a CPHA mismatch the
/// line transitions half a bit period away from where the byte-level path
/// would have put them.
#[test]
fn edge_sampled_miso_transitions_on_the_slave_phase() {
    fn wire(opt_in: bool) -> Vec<bool> {
        let mut spi = Spi::new();
        let lines = spi.line_levels_arc();
        spi.push_device(Box::new(if opt_in {
            EdgeSlave::new(1, 0xB3)
        } else {
            EdgeSlave::byte_level(1, 0xB3)
        }));
        // Mode 0 master, BR=0 -> one cycle per half-period.
        spi.write_u16(0x00, 1 << 6).unwrap();
        spi.write(0x0C, 0xA5).unwrap();
        let mut levels = vec![lines.miso()];
        while spi.transfer_active() {
            spi.tick_elapsed(1);
            levels.push(lines.miso());
        }
        levels
    }
    let edge = wire(true);
    let byte = wire(false);
    assert_eq!(edge.len(), byte.len(), "same frame length");
    assert_ne!(
        edge, byte,
        "the mismatched slave drives MISO on its own edges"
    );
}

/// Arduino SPI.transfer() polls RXNE after each DR write. If RXNE is not
/// clear-on-read, the next poll exits immediately and re-reads a stale DR
/// — the MAX31855 matrix residual (`0x00019016`).
#[test]
fn rxne_clears_so_multi_byte_transfer_stays_in_sync() {
    struct Seq {
        i: u8,
        bytes: [u8; 4],
    }
    impl SpiDevice for Seq {
        fn transfer(&mut self, _mosi: u8) -> u8 {
            let b = self.bytes[self.i as usize % 4];
            self.i = self.i.wrapping_add(1);
            b
        }
        fn cs_pin(&self) -> &str {
            ""
        }
    }
    let mut spi = Spi::new();
    spi.push_device(Box::new(Seq {
        i: 0,
        bytes: [0x01, 0x90, 0x16, 0x00],
    }));
    spi.write(0x00, 0x40).unwrap(); // SPE, BR=0 (fast)
    let mut frame = 0u32;
    for _ in 0..4 {
        spi.write(0x0C, 0x00).unwrap();
        run_engine(&mut spi);
        assert_ne!(spi.read(0x08).unwrap() & 0x01, 0, "RXNE after frame");
        let b = spi.read(0x0C).unwrap();
        assert_eq!(spi.read(0x08).unwrap() & 0x01, 0, "RXNE cleared by DR");
        frame = (frame << 8) | u32::from(b);
    }
    assert_eq!(frame, 0x0190_1600);
}

// ── nRF52 SPIM EasyDMA unit tests ─────────────────────────────────────────

use crate::{Bus, DmaRequest, SimulationConfig};
use std::collections::HashMap;

/// Minimal flat-RAM bus for unit tests — no peripherals, just byte array.
struct FlatRamBus {
    mem: HashMap<u64, u8>,
    gpio: HashMap<String, bool>,
    config: SimulationConfig,
}

impl FlatRamBus {
    fn new() -> Self {
        Self {
            mem: HashMap::new(),
            gpio: HashMap::new(),
            config: SimulationConfig::default(),
        }
    }

    fn write_slice(&mut self, base: u64, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            self.mem.insert(base + i as u64, b);
        }
    }

    fn read_slice(&self, base: u64, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| *self.mem.get(&(base + i as u64)).unwrap_or(&0))
            .collect()
    }
}

impl Bus for FlatRamBus {
    fn read_u8(&self, addr: u64) -> crate::SimResult<u8> {
        Ok(*self.mem.get(&addr).unwrap_or(&0))
    }
    fn write_u8(&mut self, addr: u64, value: u8) -> crate::SimResult<()> {
        self.mem.insert(addr, value);
        Ok(())
    }
    fn tick_peripherals(&mut self) -> Vec<u32> {
        Vec::new()
    }
    fn execute_dma(&mut self, _requests: &[DmaRequest]) -> crate::SimResult<()> {
        Ok(())
    }
    fn config(&self) -> &SimulationConfig {
        &self.config
    }
    fn read_gpio_output_by_label(&self, pin: &str) -> Option<bool> {
        self.gpio.get(pin).copied()
    }
}

/// Helper: write a u32 to nRF SPIM registers as a single word write
/// (matches Cortex-M STR instruction semantics used by real firmware).
fn nrf_write_u32(spi: &mut Spi, offset: u64, value: u32) {
    spi.write_u32(offset, value).unwrap();
}

/// Helper: read a u32 from nRF SPIM registers via 4x byte reads.
fn nrf_read_u32(spi: &Spi, offset: u64) -> u32 {
    let b0 = spi.read(offset).unwrap() as u32;
    let b1 = spi.read(offset + 1).unwrap() as u32;
    let b2 = spi.read(offset + 2).unwrap() as u32;
    let b3 = spi.read(offset + 3).unwrap() as u32;
    b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
}

/// Full EasyDMA transfer with no attached device and no loopback:
/// TXD bytes are read from RAM, MISO is 0 everywhere.
/// After tick_with_bus: EVENTS_END/ENDTX/ENDRX all 1,
/// TXD.AMOUNT == TXD.MAXCNT, RXD.AMOUNT == RXD.MAXCNT,
/// RXD RAM contains zeros (no device, no loopback).
#[test]
fn nrf52_spim_easydma_no_device_txd_and_rxd_amount() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    let mut bus = FlatRamBus::new();

    let tx_base: u64 = 0x2000_0000;
    let rx_base: u64 = 0x2000_0100;
    let tx_data: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
    bus.write_slice(tx_base, &tx_data);

    // Configure SPIM: ENABLE=7, TXD.PTR/MAXCNT, RXD.PTR/MAXCNT.
    nrf_write_u32(&mut spi, 0x500, 7); // ENABLE = 7
    nrf_write_u32(&mut spi, 0x544, tx_base as u32); // TXD.PTR
    nrf_write_u32(&mut spi, 0x548, 4); // TXD.MAXCNT = 4
    nrf_write_u32(&mut spi, 0x534, rx_base as u32); // RXD.PTR
    nrf_write_u32(&mut spi, 0x538, 4); // RXD.MAXCNT = 4

    // TASKS_START — must not have fired events yet.
    nrf_write_u32(&mut spi, 0x010, 1);
    assert_eq!(
        nrf_read_u32(&spi, 0x118),
        0,
        "EVENTS_END must not be set before tick"
    );
    assert!(spi.needs_bus_tick(), "pending_start must be set");

    // Run EasyDMA.
    spi.tick_with_bus(&mut bus);

    // Completion events.
    assert_eq!(
        nrf_read_u32(&spi, 0x118),
        1,
        "EVENTS_END must be 1 after transfer"
    );
    assert_eq!(nrf_read_u32(&spi, 0x120), 1, "EVENTS_ENDTX must be 1");
    assert_eq!(nrf_read_u32(&spi, 0x110), 1, "EVENTS_ENDRX must be 1");

    // AMOUNT registers.
    assert_eq!(nrf_read_u32(&spi, 0x54C), 4, "TXD.AMOUNT must be 4");
    assert_eq!(nrf_read_u32(&spi, 0x53C), 4, "RXD.AMOUNT must be 4");

    // No device/loopback → MISO is all zeros.
    let rx = bus.read_slice(rx_base, 4);
    assert_eq!(rx, vec![0, 0, 0, 0], "RXD RAM must be zeros with no device");

    // needs_bus_tick must be clear after completion.
    assert!(
        !spi.needs_bus_tick(),
        "pending_start must be cleared after tick_with_bus"
    );
}

/// Full EasyDMA transfer with loopback (MOSI → MISO mirror):
/// RXD RAM should contain the same bytes that were transmitted.
#[test]
fn nrf52_spim_easydma_loopback_rxd_mirrors_txd() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    spi.set_loopback(true);
    let mut bus = FlatRamBus::new();

    let tx_base: u64 = 0x2000_0200;
    let rx_base: u64 = 0x2000_0300;
    let tx_data: [u8; 5] = [0x11, 0x22, 0x33, 0x44, 0x55];
    bus.write_slice(tx_base, &tx_data);

    nrf_write_u32(&mut spi, 0x500, 7);
    nrf_write_u32(&mut spi, 0x544, tx_base as u32);
    nrf_write_u32(&mut spi, 0x548, 5);
    nrf_write_u32(&mut spi, 0x534, rx_base as u32);
    nrf_write_u32(&mut spi, 0x538, 5);

    nrf_write_u32(&mut spi, 0x010, 1); // TASKS_START
    spi.tick_with_bus(&mut bus);

    // With loopback, each MISO byte is the same as the MOSI byte.
    let rx = bus.read_slice(rx_base, 5);
    assert_eq!(rx, tx_data.to_vec(), "loopback: RXD == TXD");
    assert_eq!(nrf_read_u32(&spi, 0x54C), 5, "TXD.AMOUNT");
    assert_eq!(nrf_read_u32(&spi, 0x53C), 5, "RXD.AMOUNT");
    assert_eq!(nrf_read_u32(&spi, 0x118), 1, "EVENTS_END");
}

/// Attached SpiDevice (echo slave): every MOSI byte is returned as-is.
/// RXD RAM should contain the transmitted bytes.
#[test]
fn nrf52_spim_easydma_echo_device_rxd_contains_mosi() {
    struct EchoSlave;
    impl SpiDevice for EchoSlave {
        fn transfer(&mut self, mosi: u8) -> u8 {
            mosi
        }
        fn cs_pin(&self) -> &str {
            ""
        }
    }

    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    spi.push_device(Box::new(EchoSlave));
    let mut bus = FlatRamBus::new();

    let tx_base: u64 = 0x2000_0400;
    let rx_base: u64 = 0x2000_0500;
    let tx_data: [u8; 3] = [0xA1, 0xB2, 0xC3];
    bus.write_slice(tx_base, &tx_data);

    nrf_write_u32(&mut spi, 0x500, 7);
    nrf_write_u32(&mut spi, 0x544, tx_base as u32);
    nrf_write_u32(&mut spi, 0x548, 3);
    nrf_write_u32(&mut spi, 0x534, rx_base as u32);
    nrf_write_u32(&mut spi, 0x538, 3);
    nrf_write_u32(&mut spi, 0x010, 1);
    spi.tick_with_bus(&mut bus);

    let rx = bus.read_slice(rx_base, 3);
    assert_eq!(
        rx,
        tx_data.to_vec(),
        "echo device: RXD == TXD (MISO mirrors MOSI)"
    );
    assert_eq!(nrf_read_u32(&spi, 0x118), 1, "EVENTS_END");
    assert_eq!(nrf_read_u32(&spi, 0x54C), 3, "TXD.AMOUNT == 3");
    assert_eq!(nrf_read_u32(&spi, 0x53C), 3, "RXD.AMOUNT == 3");
}

#[test]
fn nrf52_spim_gpio_cs_selects_only_matching_device_and_spans_transfers() {
    use std::sync::{Arc, Mutex};

    struct TransactionSlave(&'static str, Arc<Mutex<Vec<String>>>);
    impl SpiDevice for TransactionSlave {
        fn transfer(&mut self, _mosi: u8) -> u8 {
            0
        }
        fn cs_pin(&self) -> &str {
            "P0.12"
        }
        fn cs_select(&mut self) {
            self.1.lock().unwrap().push(format!("{}:select", self.0));
        }
        fn cs_release(&mut self) {
            self.1.lock().unwrap().push(format!("{}:release", self.0));
        }
    }

    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    spi.push_device(Box::new(TransactionSlave("a", events.clone())));
    struct Other(TransactionSlave);
    impl SpiDevice for Other {
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.0.transfer(mosi)
        }
        fn cs_pin(&self) -> &str {
            "P0.13"
        }
        fn cs_select(&mut self) {
            self.0.cs_select()
        }
        fn cs_release(&mut self) {
            self.0.cs_release()
        }
    }
    spi.push_device(Box::new(Other(TransactionSlave("b", events.clone()))));
    let mut bus = FlatRamBus::new();
    bus.gpio.insert("P0.12".into(), false);
    bus.gpio.insert("P0.13".into(), true);
    bus.write_slice(0x2000_0200, &[0xC0]);
    nrf_write_u32(&mut spi, 0x500, 7);
    nrf_write_u32(&mut spi, 0x544, 0x2000_0200);
    nrf_write_u32(&mut spi, 0x548, 1);
    nrf_write_u32(&mut spi, 0x010, 1);
    spi.tick_with_bus(&mut bus);
    nrf_write_u32(&mut spi, 0x010, 1);
    spi.tick_with_bus(&mut bus);
    assert_eq!(*events.lock().unwrap(), ["a:select"]);
    bus.gpio.insert("P0.12".into(), true);
    spi.tick_with_bus(&mut bus);
    assert_eq!(*events.lock().unwrap(), ["a:select", "a:release"]);
}

#[test]
fn nrf52_spim_start_and_zero_length_do_not_invent_cs_pulse() {
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    struct Slave(std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>);
    impl SpiDevice for Slave {
        fn transfer(&mut self, _mosi: u8) -> u8 {
            0
        }
        fn cs_pin(&self) -> &str {
            "P0.12"
        }
        fn cs_select(&mut self) {
            self.0.lock().unwrap().push("select")
        }
        fn cs_release(&mut self) {
            self.0.lock().unwrap().push("release")
        }
    }
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    spi.push_device(Box::new(Slave(events.clone())));
    let mut bus = FlatRamBus::new();
    bus.gpio.insert("P0.12".into(), true);
    nrf_write_u32(&mut spi, 0x500, 7);
    nrf_write_u32(&mut spi, 0x548, 0);
    nrf_write_u32(&mut spi, 0x538, 0);
    nrf_write_u32(&mut spi, 0x010, 1);
    spi.tick_with_bus(&mut bus);
    assert!(events.lock().unwrap().is_empty());
}

/// RXD.MAXCNT < TXD.MAXCNT: RXD fills up, remaining MISO bytes are discarded.
/// TXD.AMOUNT == TXD.MAXCNT, RXD.AMOUNT == RXD.MAXCNT.
#[test]
fn nrf52_spim_easydma_rxd_maxcnt_limits_rxd_amount() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    spi.set_loopback(true);
    let mut bus = FlatRamBus::new();

    let tx_base: u64 = 0x2000_0600;
    let rx_base: u64 = 0x2000_0700;
    bus.write_slice(tx_base, &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);

    nrf_write_u32(&mut spi, 0x544, tx_base as u32);
    nrf_write_u32(&mut spi, 0x548, 6); // TXD.MAXCNT = 6
    nrf_write_u32(&mut spi, 0x534, rx_base as u32);
    nrf_write_u32(&mut spi, 0x538, 3); // RXD.MAXCNT = 3 (less)
    nrf_write_u32(&mut spi, 0x010, 1);
    spi.tick_with_bus(&mut bus);

    assert_eq!(nrf_read_u32(&spi, 0x54C), 6, "TXD.AMOUNT == 6");
    assert_eq!(nrf_read_u32(&spi, 0x53C), 3, "RXD.AMOUNT == 3 (clamped)");
    // Only first 3 bytes written to RX buffer.
    let rx = bus.read_slice(rx_base, 3);
    assert_eq!(rx, vec![0x01, 0x02, 0x03], "first 3 bytes received");
}

/// ORC (over-read character): when TXD.MAXCNT < RXD.MAXCNT, the ORC byte
/// is clocked out for the extra cycles. With loopback, those ORC bytes
/// end up in the RXD buffer.
#[test]
fn nrf52_spim_easydma_orc_pads_extra_rx_cycles() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    spi.set_loopback(true);
    let mut bus = FlatRamBus::new();

    let tx_base: u64 = 0x2000_0800;
    let rx_base: u64 = 0x2000_0900;
    bus.write_slice(tx_base, &[0xAA, 0xBB]); // 2 TX bytes

    nrf_write_u32(&mut spi, 0x5C0, 0xFF); // ORC = 0xFF
    nrf_write_u32(&mut spi, 0x544, tx_base as u32);
    nrf_write_u32(&mut spi, 0x548, 2); // TXD.MAXCNT = 2
    nrf_write_u32(&mut spi, 0x534, rx_base as u32);
    nrf_write_u32(&mut spi, 0x538, 4); // RXD.MAXCNT = 4 (2 extra)
    nrf_write_u32(&mut spi, 0x010, 1);
    spi.tick_with_bus(&mut bus);

    // TXD.AMOUNT counts actual TX bytes, not ORC clocks.
    assert_eq!(nrf_read_u32(&spi, 0x54C), 2, "TXD.AMOUNT == 2 (not 4)");
    assert_eq!(nrf_read_u32(&spi, 0x53C), 4, "RXD.AMOUNT == 4");
    let rx = bus.read_slice(rx_base, 4);
    // Loopback: first 2 = TXD bytes, last 2 = ORC (0xFF).
    assert_eq!(rx, vec![0xAA, 0xBB, 0xFF, 0xFF], "ORC fills extra RX slots");
}

/// EVENTS write semantics: SW writing 1 to an EVENTS register must NOT set it.
/// Only SW writing 0 clears it.
#[test]
fn nrf52_spim_events_write_1_ignored() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    let mut bus = FlatRamBus::new();

    nrf_write_u32(&mut spi, 0x500, 7);
    nrf_write_u32(&mut spi, 0x548, 2);
    nrf_write_u32(&mut spi, 0x544, 0x2000_0000_u32);
    nrf_write_u32(&mut spi, 0x538, 2);
    nrf_write_u32(&mut spi, 0x534, 0x2000_0100_u32);

    // Arm and run transfer.
    nrf_write_u32(&mut spi, 0x010, 1);
    spi.tick_with_bus(&mut bus);
    assert_eq!(nrf_read_u32(&spi, 0x118), 1, "EVENTS_END set by HW");
    assert_eq!(nrf_read_u32(&spi, 0x120), 1, "EVENTS_ENDTX set by HW");
    assert_eq!(nrf_read_u32(&spi, 0x110), 1, "EVENTS_ENDRX set by HW");

    // SW write of 1 must be ignored (silicon-verified rule).
    nrf_write_u32(&mut spi, 0x118, 1); // attempt to SET EVENTS_END — must be ignored
    assert_eq!(
        nrf_read_u32(&spi, 0x118),
        1,
        "EVENTS_END unchanged by SW write of 1"
    );

    // SW write of 0 clears it.
    nrf_write_u32(&mut spi, 0x118, 0);
    assert_eq!(
        nrf_read_u32(&spi, 0x118),
        0,
        "EVENTS_END cleared by SW write of 0"
    );
    nrf_write_u32(&mut spi, 0x120, 0);
    assert_eq!(
        nrf_read_u32(&spi, 0x120),
        0,
        "EVENTS_ENDTX cleared by SW write of 0"
    );
    nrf_write_u32(&mut spi, 0x110, 0);
    assert_eq!(
        nrf_read_u32(&spi, 0x110),
        0,
        "EVENTS_ENDRX cleared by SW write of 0"
    );
}

/// TASKS_START before tick_with_bus: EVENTS must not be set immediately.
/// They should only appear after tick_with_bus runs.
#[test]
fn nrf52_spim_events_not_set_before_tick_with_bus() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);

    nrf_write_u32(&mut spi, 0x500, 7);
    nrf_write_u32(&mut spi, 0x548, 1);
    nrf_write_u32(&mut spi, 0x544, 0x2000_0000_u32);

    // Before TASKS_START: no events.
    assert_eq!(nrf_read_u32(&spi, 0x118), 0, "EVENTS_END initially 0");
    assert_eq!(nrf_read_u32(&spi, 0x120), 0, "EVENTS_ENDTX initially 0");
    assert_eq!(nrf_read_u32(&spi, 0x110), 0, "EVENTS_ENDRX initially 0");

    // After TASKS_START but BEFORE tick_with_bus: still 0.
    nrf_write_u32(&mut spi, 0x010, 1);
    assert_eq!(
        nrf_read_u32(&spi, 0x118),
        0,
        "EVENTS_END must not fire before tick"
    );
    assert_eq!(nrf_read_u32(&spi, 0x120), 0, "EVENTS_ENDTX before tick");
    assert_eq!(nrf_read_u32(&spi, 0x110), 0, "EVENTS_ENDRX before tick");
}

/// INTENSET / INTENCLR round-trip.
#[test]
fn nrf52_spim_intenset_intenclr_round_trip() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);

    // INTENSET: bit 6 = INTEN_END, bit 8 = INTEN_ENDTX.
    nrf_write_u32(&mut spi, 0x304, (1 << 6) | (1 << 8));
    assert_eq!(
        nrf_read_u32(&spi, 0x304),
        (1 << 6) | (1 << 8),
        "INTENSET sets bits"
    );

    // INTENCLR: clear bit 6 only.
    nrf_write_u32(&mut spi, 0x308, 1 << 6);
    assert_eq!(nrf_read_u32(&spi, 0x308), 1 << 8, "INTENCLR clears bit 6");
}

/// ORC register stores only the low 8 bits.
#[test]
fn nrf52_spim_orc_masks_to_8_bits() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    nrf_write_u32(&mut spi, 0x5C0, 0xFFFF_FFAB);
    assert_eq!(
        nrf_read_u32(&spi, 0x5C0),
        0xAB,
        "ORC retains only low 8 bits"
    );
}

/// ENABLE register round-trip.
#[test]
fn nrf52_spim_enable_round_trip() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    nrf_write_u32(&mut spi, 0x500, 7);
    assert_eq!(nrf_read_u32(&spi, 0x500), 7, "ENABLE round-trips");
}

/// TASKS registers read back as 0 (write-only strobes on silicon).
#[test]
fn nrf52_spim_tasks_read_as_zero() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    nrf_write_u32(&mut spi, 0x010, 1); // TASKS_START
    assert_eq!(nrf_read_u32(&spi, 0x010), 0, "TASKS_START reads as 0");
}

/// Arduino nRF SPI library: ENABLE=1 + TXD write must raise EVENTS_READY
/// and put device MISO into RXD (legacy SPI, not SPIM EasyDMA).
#[test]
fn nrf52_legacy_spi_txd_raises_ready_and_returns_miso() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    struct Echo;
    impl SpiDevice for Echo {
        fn transfer(&mut self, mosi: u8) -> u8 {
            mosi ^ 0xFF
        }
        fn cs_pin(&self) -> &str {
            "P0.22"
        }
    }
    spi.push_device(Box::new(Echo));
    nrf_write_u32(&mut spi, 0x500, 1); // ENABLE = SPI (legacy)
    nrf_write_u32(&mut spi, 0x51C, 0xA5); // TXD
    assert_eq!(nrf_read_u32(&spi, 0x108), 1, "EVENTS_READY");
    assert_eq!(nrf_read_u32(&spi, 0x518) & 0xFF, 0x5A, "RXD = mosi^0xFF");
    nrf_write_u32(&mut spi, 0x108, 0); // clear READY
    assert_eq!(nrf_read_u32(&spi, 0x108), 0);
}

/// Second TASKS_START after a completed transfer re-arms the engine.
#[test]
fn nrf52_spim_easydma_second_start_reruns_transfer() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    spi.set_loopback(true);
    let mut bus = FlatRamBus::new();

    let tx_base: u64 = 0x2000_0A00;
    let rx_base: u64 = 0x2000_0B00;
    bus.write_slice(tx_base, &[0x01, 0x02]);

    nrf_write_u32(&mut spi, 0x544, tx_base as u32);
    nrf_write_u32(&mut spi, 0x548, 2);
    nrf_write_u32(&mut spi, 0x534, rx_base as u32);
    nrf_write_u32(&mut spi, 0x538, 2);
    nrf_write_u32(&mut spi, 0x010, 1);
    spi.tick_with_bus(&mut bus);
    assert_eq!(nrf_read_u32(&spi, 0x54C), 2);

    // Update TX buffer and run a second transfer.
    bus.write_slice(tx_base, &[0x55, 0x66]);
    nrf_write_u32(&mut spi, 0x118, 0); // clear EVENTS_END
    nrf_write_u32(&mut spi, 0x120, 0); // clear EVENTS_ENDTX
    nrf_write_u32(&mut spi, 0x110, 0); // clear EVENTS_ENDRX
    nrf_write_u32(&mut spi, 0x010, 1);
    spi.tick_with_bus(&mut bus);

    let rx = bus.read_slice(rx_base, 2);
    assert_eq!(rx, vec![0x55, 0x66], "second transfer sees new TX data");
    assert_eq!(
        nrf_read_u32(&spi, 0x118),
        1,
        "EVENTS_END after second transfer"
    );
}

/// tick_with_bus with TXD.MAXCNT == 0 and RXD.MAXCNT == 0: completes
/// immediately with AMOUNT == 0 and all events fired.
#[test]
fn nrf52_spim_easydma_zero_length_transfer() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    let mut bus = FlatRamBus::new();

    nrf_write_u32(&mut spi, 0x544, 0x2000_0000);
    nrf_write_u32(&mut spi, 0x548, 0); // TXD.MAXCNT = 0
    nrf_write_u32(&mut spi, 0x534, 0x2000_0100);
    nrf_write_u32(&mut spi, 0x538, 0); // RXD.MAXCNT = 0
    nrf_write_u32(&mut spi, 0x010, 1);
    spi.tick_with_bus(&mut bus);

    assert_eq!(nrf_read_u32(&spi, 0x54C), 0, "TXD.AMOUNT == 0");
    assert_eq!(nrf_read_u32(&spi, 0x53C), 0, "RXD.AMOUNT == 0");
    assert_eq!(
        nrf_read_u32(&spi, 0x118),
        1,
        "EVENTS_END fires even for zero-length"
    );
    assert_eq!(nrf_read_u32(&spi, 0x120), 1, "EVENTS_ENDTX fires");
    assert_eq!(nrf_read_u32(&spi, 0x110), 1, "EVENTS_ENDRX fires");
}

// ── STM32H5 ("SPI v3", RM0481) unit tests ────────────────────────────────
// Register-level expectations pinned by silicon capture 2026-06-11
// (NUCLEO-H563ZI), probed over SWD. The TX data engine is spec-derived
// (the bench part had no SPI kernel clock — see Stm32H5SpiRegs docs).

fn h5() -> Spi {
    Spi::new_with_layout(SpiRegisterLayout::Stm32H5)
}

fn h5_read(spi: &Spi, offset: u64) -> u32 {
    spi.read_u32(offset).unwrap()
}

fn h5_write(spi: &mut Spi, offset: u64, value: u32) {
    spi.write_u32(offset, value).unwrap();
}

/// Master-mode bring-up: CR1.SSI=1, then CFG2 = MASTER|SSM, CR2.TSIZE.
fn h5_master(tsize: u32) -> Spi {
    let mut spi = h5();
    h5_write(&mut spi, 0x00, 1 << 12); // CR1.SSI = 1 (internal SS high)
    h5_write(&mut spi, 0x0C, (1 << 22) | (1 << 26)); // CFG2 = MASTER|SSM
    h5_write(&mut spi, 0x04, tsize); // CR2.TSIZE
    spi
}

/// The chip-yaml token "stm32h5" selects the v3 layout, NOT the L4/F7
/// FIFO map it used to alias.
#[test]
fn stm32h5_from_str_selects_v3_layout() {
    assert_eq!(
        "stm32h5".parse::<SpiRegisterLayout>().unwrap(),
        SpiRegisterLayout::Stm32H5
    );
    assert_eq!(
        "stm32l4".parse::<SpiRegisterLayout>().unwrap(),
        SpiRegisterLayout::Stm32Fifo,
        "L4/F7/G4 stay on the FIFO layout"
    );
}

/// Reset values — silicon capture 2026-06-11 (NUCLEO-H563ZI).
#[test]
fn stm32h5_reset_values_match_silicon() {
    let spi = h5();
    assert_eq!(h5_read(&spi, 0x00), 0, "CR1");
    assert_eq!(h5_read(&spi, 0x04), 0, "CR2");
    assert_eq!(h5_read(&spi, 0x08), 0x0007_0007, "CFG1");
    assert_eq!(h5_read(&spi, 0x0C), 0, "CFG2");
    assert_eq!(h5_read(&spi, 0x10), 0, "IER");
    assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "SR = TXP|TXC");
    assert_eq!(h5_read(&spi, 0x18), 0, "IFCR is write-only, reads 0");
    assert_eq!(h5_read(&spi, 0x20), 0, "TXDR is write-only, reads 0");
    assert_eq!(h5_read(&spi, 0x30), 0, "RXDR");
    assert_eq!(h5_read(&spi, 0x40), 0x0000_0107, "CRCPOLY");
    assert_eq!(h5_read(&spi, 0x44), 0, "TXCRC");
    assert_eq!(h5_read(&spi, 0x48), 0, "RXCRC");
    assert_eq!(h5_read(&spi, 0x4C), 0, "UDRDR");
    assert_eq!(h5_read(&spi, 0x50), 0, "I2SCFGR");
}

/// CFG1 writable mask — all three silicon round-trip pairs.
#[test]
fn stm32h5_cfg1_reserved_bits_masked() {
    let mut spi = h5();
    h5_write(&mut spi, 0x08, 0x7000_0007);
    assert_eq!(h5_read(&spi, 0x08), 0x7000_0007);
    h5_write(&mut spi, 0x08, 0x0008_0008);
    assert_eq!(h5_read(&spi, 0x08), 0x0008_0008);
    h5_write(&mut spi, 0x08, 0x5555_AAAA);
    assert_eq!(
        h5_read(&spi, 0x08),
        0x5055_82AA,
        "reserved bits 0x05002800 read as 0"
    );
}

/// CR2.TSIZE, CRCPOLY and IER round-trip the silicon-probed values.
#[test]
fn stm32h5_config_round_trips() {
    let mut spi = h5();
    h5_write(&mut spi, 0x04, 0x10);
    assert_eq!(h5_read(&spi, 0x04), 0x10, "CR2.TSIZE");
    h5_write(&mut spi, 0x40, 0xA5A5);
    assert_eq!(h5_read(&spi, 0x40), 0xA5A5, "CRCPOLY");
    h5_write(&mut spi, 0x10, 0x209);
    assert_eq!(h5_read(&spi, 0x10), 0x209, "IER");
}

/// MASTER is accepted when the internal SS level is high (SSM=1, SSI=1).
#[test]
fn stm32h5_cfg2_master_accepted_when_ssi_high() {
    let mut spi = h5();
    h5_write(&mut spi, 0x00, 1 << 12); // CR1.SSI = 1 first
    h5_write(&mut spi, 0x0C, (1 << 22) | (1 << 26));
    assert_eq!(h5_read(&spi, 0x0C), 0x0440_0000);
    assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "no MODF");
}

/// Mode fault: MASTER requested with SSM=1 while SSI=0 → MASTER refused,
/// SR.MODF latches, SPE is refused until IFCR clears MODF.
#[test]
fn stm32h5_mode_fault_refuses_master_and_blocks_spe() {
    let mut spi = h5();
    // SSI is 0 at reset: the MASTER|SSM request mode-faults.
    h5_write(&mut spi, 0x0C, 0x0440_0000);
    assert_eq!(h5_read(&spi, 0x0C), 0x0400_0000, "MASTER stored as 0");
    assert_eq!(h5_read(&spi, 0x14), 0x0000_1202, "SR = TXP|MODF|TXC");
    // SPE refused while the fault stands.
    h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
    assert_eq!(h5_read(&spi, 0x00), 0x0000_1000, "SPE refused, SSI kept");
    // IFCR bit 9 clears MODF; MASTER and SPE then go through.
    h5_write(&mut spi, 0x18, 1 << 9);
    assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "MODF cleared via IFCR");
    h5_write(&mut spi, 0x0C, 0x0440_0000);
    assert_eq!(h5_read(&spi, 0x0C), 0x0440_0000, "MASTER accepted (SSI=1)");
    h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12));
    assert_eq!(h5_read(&spi, 0x00) & 1, 1, "SPE accepted after clear");
}

/// While SPE=1 the configuration registers are locked: CFG1/CFG2 writes
/// are ignored.
#[test]
fn stm32h5_spe_locks_cfg1_and_cfg2() {
    let mut spi = h5_master(2);
    h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
    h5_write(&mut spi, 0x0C, 0x0440_0000 | (1 << 29));
    assert_eq!(h5_read(&spi, 0x0C), 0x0440_0000, "CFG2 locked under SPE");
    h5_write(&mut spi, 0x08, 0x7000_0007);
    assert_eq!(h5_read(&spi, 0x08), 0x0007_0007, "CFG1 locked under SPE");
}

/// Setting SPE loads SR.CTSIZE from CR2.TSIZE and clears TXC (a transfer
/// is pending).
#[test]
fn stm32h5_spe_loads_ctsize_and_clears_txc() {
    let mut spi = h5_master(2);
    h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
    assert_eq!(h5_read(&spi, 0x14), 0x0002_0002, "CTSIZE=2, TXP, TXC off");
}

/// CR1.CSTART latches while a transfer is active and cannot be cleared by
/// software (HW clears it at EOT — RM0481 §41.4.10).
#[test]
fn stm32h5_cstart_latches_while_transfer_active() {
    let mut spi = h5_master(2);
    h5_write(&mut spi, 0x00, (1 << 0) | (1 << 9) | (1 << 12)); // SPE|CSTART|SSI
    assert_eq!(h5_read(&spi, 0x00), 0x0000_1201, "CSTART latched");
    h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // try to drop CSTART
    assert_eq!(h5_read(&spi, 0x00), 0x0000_1201, "CSTART not SW-clearable");
}

/// The bench TXDR/IFCR/SPE-clear sequence. CSTART is left clear so no
/// frame shifts and CTSIZE stays put — exactly the unclocked-silicon
/// behaviour captured on the bench.
#[test]
fn stm32h5_txdr_txtf_ifcr_and_spe_clear_sequence() {
    let mut spi = h5_master(2);
    h5_write(&mut spi, 0x00, (1 << 0) | (1 << 12)); // SPE|SSI
    h5_write(&mut spi, 0x20, 0xAB); // TXDR
    assert_eq!(h5_read(&spi, 0x14), 0x0002_0012, "TXP|TXTF, CTSIZE=2");
    h5_write(&mut spi, 0x18, 0xFFFF_FFFF); // IFCR: clear all clearables
    assert_eq!(h5_read(&spi, 0x14), 0x0002_0002, "TXTF cleared");
    h5_write(&mut spi, 0x00, 1 << 12); // SPE → 0
    assert_eq!(h5_read(&spi, 0x14), 0x0002_1002, "TXC set, CTSIZE kept");
}

/// Sim-side TX engine: with SPE+CSTART in master mode each TXDR write
/// transmits one frame and decrements CTSIZE; at 0 → EOT|TXC, CSTART
/// HW-cleared. RXDR stays 0 (TX-only model).
#[test]
fn stm32h5_tx_engine_transmits_and_completes() {
    let mut spi = h5_master(2);
    spi.push_device(Box::new(Capture { rx: Vec::new() }));
    h5_write(&mut spi, 0x00, (1 << 0) | (1 << 9) | (1 << 12)); // SPE|CSTART|SSI
    h5_write(&mut spi, 0x20, 0x11);
    assert_eq!(
        h5_read(&spi, 0x14),
        0x0001_0013,
        "CTSIZE 2->1, TXP|TXTF|RXP (each frame clocks one in)"
    );
    h5_write(&mut spi, 0x20, 0x22);
    assert_eq!(captured(&spi), vec![0x11, 0x22], "both frames on the bus");
    assert_eq!(h5_read(&spi, 0x14), 0x0000_101B, "EOT|TXC|RXP at CTSIZE=0");
    assert_eq!(h5_read(&spi, 0x00), 0x0000_1001, "CSTART HW-cleared");
    // Full duplex: each transmitted frame clocks one in. With no slave
    // attached the captured value is the idle line level (0), but RXP is
    // set and RXDR is readable — the receive EVENT happens regardless.
    // This previously asserted a TX-only engine, which hung every driver
    // that writes TXDR then waits on RXP (HAL_SPI_TransmitReceive, and so
    // Arduino SPI.transfer()).
    assert_eq!(h5_read(&spi, 0x30), 0, "RXDR holds the idle MISO level");
}

/// TXDR writes are inert while SPE=0: no TXTF, nothing transmitted.
#[test]
fn stm32h5_txdr_ignored_when_disabled() {
    let mut spi = h5_master(2);
    spi.push_device(Box::new(Capture { rx: Vec::new() }));
    h5_write(&mut spi, 0x20, 0xAB);
    assert_eq!(h5_read(&spi, 0x14), 0x0000_1002, "SR untouched");
    assert!(captured(&spi).is_empty(), "nothing transmitted");
}

/// TXDR byte/halfword accesses are each ONE frame (RM0481 §41.4.13:
/// access size = frame size). TSIZE=0 = endless mode: CTSIZE stays 0,
/// no EOT, CSTART stays latched.
#[test]
fn stm32h5_byte_and_halfword_txdr_access_is_one_frame() {
    let mut spi = h5_master(0); // TSIZE=0: endless
    spi.push_device(Box::new(Capture { rx: Vec::new() }));
    h5_write(&mut spi, 0x00, (1 << 0) | (1 << 9) | (1 << 12));
    spi.write(0x20, 0x5A).unwrap(); // byte access → one 8-bit frame
    spi.write_u16(0x20, 0x1234).unwrap(); // halfword access → one frame
    assert_eq!(captured(&spi), vec![0x5A, 0x34], "low byte per frame");
    assert_eq!(h5_read(&spi, 0x14) >> 16, 0, "CTSIZE stays 0");
    assert_eq!(h5_read(&spi, 0x14) & (1 << 3), 0, "no EOT in endless mode");
    assert_eq!(h5_read(&spi, 0x00), 0x0000_1201, "CSTART stays latched");
}

/// Config registers are 32-bit with byte-merge semantics on the byte path.
#[test]
fn stm32h5_byte_writes_merge_into_32bit_registers() {
    let mut spi = h5();
    spi.write(0x40, 0xA5).unwrap(); // CRCPOLY low byte (reset 0x107)
    spi.write(0x41, 0x5A).unwrap(); // CRCPOLY byte 1
    assert_eq!(h5_read(&spi, 0x40), 0x0000_5AA5, "bytes merged in place");
}

// ── nRF54L SPIM ───────────────────────────────────────────────────────────

/// Records every byte with the D/C level the controller held while it moved.
///
/// The log is shared rather than read back through a downcast: `SpiDevice`
/// is deliberately a behaviour-only seam, so the test observes the panel
/// the same way a panel observes the bus.
#[derive(Clone)]
struct DcCapture {
    cs: String,
    dc: std::sync::Arc<std::sync::Mutex<bool>>,
    seen: std::sync::Arc<std::sync::Mutex<Vec<(bool, u8)>>>,
}
impl DcCapture {
    fn new(cs: &str) -> Self {
        Self {
            cs: cs.to_string(),
            dc: std::sync::Arc::new(std::sync::Mutex::new(false)),
            seen: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
    fn log(&self) -> Vec<(bool, u8)> {
        self.seen.lock().unwrap().clone()
    }
}
impl SpiDevice for DcCapture {
    fn transfer(&mut self, mosi: u8) -> u8 {
        let dc = *self.dc.lock().unwrap();
        self.seen.lock().unwrap().push((dc, mosi));
        0
    }
    fn cs_pin(&self) -> &str {
        &self.cs
    }
    fn set_dc_level(&mut self, level: bool) {
        *self.dc.lock().unwrap() = level;
    }
}

/// Configure an nRF54L SPIM for a TX-only EasyDMA burst on the nRF54L map.
fn nrf54l_arm(spi: &mut Spi, tx_base: u64, len: u32, dcxcnt: u32) {
    nrf_write_u32(spi, 0x500, 7); // ENABLE = 7 (SPIM)
    nrf_write_u32(spi, 0x600, 0x0000_002A); // PSEL.SCK — connected, P1.10
    nrf_write_u32(spi, 0x604, 0x0000_002B); // PSEL.MOSI — connected
    nrf_write_u32(spi, 0x610, 0x0000_002C); // PSEL.CSN — hardware chip select
    nrf_write_u32(spi, 0x60C, 0x0000_002D); // PSEL.DCX — hardware D/C
    nrf_write_u32(spi, 0x5B4, dcxcnt); // DCXCNT
    nrf_write_u32(spi, 0x73C, tx_base as u32); // DMA.TX.PTR
    nrf_write_u32(spi, 0x740, len); // DMA.TX.MAXCNT
}

/// The whole point of the map: a display write lands, and the controller —
/// not a GPIO — decides which of those bytes were command and which data.
#[test]
fn nrf54l_spim_moves_bytes_and_drives_hardware_dc() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf54lSpim);
    let mut bus = FlatRamBus::new();
    let tx_base: u64 = 0x2000_0000;
    // One command byte (0x2C RAMWR) followed by four pixel bytes.
    bus.write_slice(tx_base, &[0x2C, 0xF8, 0x00, 0x07, 0xE0]);

    // A CS label that NOTHING ever drives. With PSEL.CSN connected the
    // controller asserts chip select itself, so the transfer must still
    // reach the panel — that is the behaviour being pinned.
    let panel = DcCapture::new("P1.12");
    spi.push_device(Box::new(panel.clone()));

    nrf54l_arm(&mut spi, tx_base, 5, 1);
    nrf_write_u32(&mut spi, 0x000, 1); // TASKS_START
    assert!(
        spi.needs_bus_tick(),
        "TASKS_START at 0x000 must arm the DMA"
    );
    spi.tick_with_bus(&mut bus);

    assert_eq!(
        panel.log(),
        vec![
            (false, 0x2C), // DCXCNT = 1 -> the first byte is a COMMAND
            (true, 0xF8),
            (true, 0x00),
            (true, 0x07),
            (true, 0xE0),
        ],
        "hardware DCX must hold D/C low for exactly DCXCNT bytes"
    );

    assert_eq!(nrf_read_u32(&spi, 0x108), 1, "EVENTS_END");
    assert_eq!(nrf_read_u32(&spi, 0x168), 1, "EVENTS_DMA.TX.END");
    assert_eq!(nrf_read_u32(&spi, 0x744), 5, "DMA.TX.AMOUNT");
}

/// DCXCNT = 0 means the whole transfer is data — not "no D/C at all".
#[test]
fn nrf54l_spim_dcxcnt_zero_sends_no_command_bytes() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Nrf54lSpim);
    let mut bus = FlatRamBus::new();
    let tx_base: u64 = 0x2000_0000;
    bus.write_slice(tx_base, &[0x11, 0x22]);
    let panel = DcCapture::new("");
    spi.push_device(Box::new(panel.clone()));
    nrf54l_arm(&mut spi, tx_base, 2, 0);
    nrf_write_u32(&mut spi, 0x000, 1);
    spi.tick_with_bus(&mut bus);
    let seen = panel.log();
    assert_eq!(seen.len(), 2);
    assert!(
        seen.iter().all(|(dc, _)| *dc),
        "with DCXCNT = 0 every byte is data"
    );
}

/// NEGATIVE CONTROL — the offset map is load-bearing in BOTH directions.
///
/// Without this, a model that answered on the union of both maps would pass
/// every test above while being wrong on silicon. 0x010 is TASKS_START on
/// the nRF52 map and TASKS_RESUME on the nRF54L map; 0x000 is the reverse.
/// Each must be inert on the other generation.
#[test]
fn nrf54l_and_nrf52_start_offsets_do_not_cross() {
    // nRF54L instance: the nRF52 start offset must NOT arm it.
    let mut l = Spi::new_with_layout(SpiRegisterLayout::Nrf54lSpim);
    nrf_write_u32(&mut l, 0x500, 7);
    nrf_write_u32(&mut l, 0x73C, 0x2000_0000);
    nrf_write_u32(&mut l, 0x740, 4);
    nrf_write_u32(&mut l, 0x010, 1); // TASKS_RESUME here, START on nRF52
    assert!(
        !l.needs_bus_tick(),
        "0x010 is TASKS_RESUME on the nRF54L map and must not start a transfer"
    );

    // nRF52 instance: the nRF54L start offset must NOT arm it.
    let mut c = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    nrf_write_u32(&mut c, 0x500, 7);
    nrf_write_u32(&mut c, 0x544, 0x2000_0000);
    nrf_write_u32(&mut c, 0x548, 4);
    nrf_write_u32(&mut c, 0x000, 1);
    assert!(
        !c.needs_bus_tick(),
        "0x000 is not a task on the nRF52 map and must not start a transfer"
    );
}

/// PRESCALER is a divisor with a NON-ZERO reset. Zero-filling it would make
/// the modelled bit clock infinite and the reset-state readback a lie.
#[test]
fn nrf54l_spim_prescaler_resets_to_0x40() {
    let spi = Spi::new_with_layout(SpiRegisterLayout::Nrf54lSpim);
    assert_eq!(nrf_read_u32(&spi, 0x52C), 0x40, "PRESCALER reset value");
    assert_eq!(
        nrf_read_u32(&spi, 0x600),
        0xFFFF_FFFF,
        "PSEL.SCK resets DISCONNECTED"
    );
}
