use super::*;
use crate::Peripheral;

/// A slave that answers with the complement of what it was sent, so a test
/// can tell a real exchange from a zero.
#[derive(Debug, Default)]
struct EchoSlave {
    seen: Vec<u8>,
}

impl SpiDevice for EchoSlave {
    fn cs_pin(&self) -> &str {
        // No pad routing in these tests: the controller broadcasts to
        // every attached device, exactly as the Kinetis DSPI path does.
        ""
    }
    fn transfer(&mut self, byte: u8) -> u8 {
        self.seen.push(byte);
        !byte
    }
}

fn controller() -> Spi {
    Spi::new_with_layout(SpiRegisterLayout::Efr32s2Usart)
}

/// The `emlib` bring-up: enable, SYNC, master, TX/RX on.
fn ready() -> Spi {
    let mut spi = controller();
    spi.write_u32(EFR_USART_EN, EFR_USART_EN_EN).unwrap();
    spi.write_u32(EFR_USART_CTRL, EFR_USART_CTRL_SYNC).unwrap();
    spi.write_u32(
        EFR_USART_CMD,
        EFR_USART_CMD_MASTEREN | EFR_USART_CMD_TXEN | EFR_USART_CMD_RXEN,
    )
    .unwrap();
    spi
}

fn status(spi: &Spi) -> u32 {
    spi.read_u32(EFR_USART_STATUS).unwrap()
}

// ── I2S mode (RM section 20.3.3.8 / 20.5.22) ────────────────────────────

/// `I2SCTRL` reads back its reset value before anything configures it.
/// RM section 20.5.22 p.669 states every field as 0x0.
#[test]
fn i2sctrl_is_zero_out_of_reset() {
    assert_eq!(controller().read_u32(EFR_USART_I2SCTRL).unwrap(), 0);
}

/// A USART is a UART until told otherwise, and I2S is one MORE thing it
/// must be told to be. Without `I2SCTRL.EN` a TXDATA write is an ordinary
/// byte transfer, so a driver that programmed SYNC but forgot I2SCTRL gets
/// SPI behaviour here and on the bench.
#[test]
fn without_i2sctrl_en_a_write_is_still_a_byte_transfer() {
    let mut spi = ready();
    spi.i2s_device = Some(Box::new(
        crate::peripherals::components::inmp441::Inmp441::new(
            "mic",
            crate::peripherals::components::inmp441::MicChannel::Left,
        ),
    ));
    spi.write_u32(EFR_USART_TXDATA, 0x00).unwrap();
    // No SPI device attached and no loopback: a byte transfer reads 0, and
    // crucially NOT a 32-bit audio slot.
    assert_eq!(spi.read_u32(EFR_USART_RXDATA).unwrap(), 0);
}

fn i2s_ready(channel: crate::peripherals::components::inmp441::MicChannel) -> Spi {
    let mut spi = ready();
    spi.i2s_device = Some(Box::new(
        crate::peripherals::components::inmp441::Inmp441::new("mic", channel),
    ));
    // FORMAT = 2 (W32D24): 32-bit word, 24-bit data — the INMP441's own
    // wire format, per its datasheet and RM section 20.5.22 p.669.
    spi.write_u32(
        EFR_USART_I2SCTRL,
        EFR_USART_I2SCTRL_EN | (2 << EFR_USART_I2SCTRL_FORMAT_SHIFT),
    )
    .unwrap();
    spi
}

/// The whole point: clock the bus and a left-channel mic's samples arrive
/// in RXDATA. On this block receiving requires clocking (RM section
/// 20.3.3.7), so a TXDATA write is how firmware advances the frame.
#[test]
fn a_left_mic_lands_samples_in_rxdata() {
    use crate::peripherals::components::inmp441::MicChannel;
    let mut spi = i2s_ready(MicChannel::Left);
    // First word after enable is the LEFT channel (RM section 20.3.3.11).
    spi.write_u32(EFR_USART_TXDATA, 0).unwrap();
    let left = spi.read_u32(EFR_USART_RXDATA).unwrap();
    // Second word is the RIGHT channel, which this mic does not drive.
    spi.write_u32(EFR_USART_TXDATA, 0).unwrap();
    let right = spi.read_u32(EFR_USART_RXDATA).unwrap();
    assert_ne!(left, 0, "the left slot must carry the mic's sample");
    assert_eq!(right, 0, "the mic tri-states outside its own channel");
}

/// A mic strapped to the RIGHT channel is the mirror image. This is the
/// wiring mistake that looks like a dead microphone: the bus clocks, the
/// part answers, and every word the firmware reads is zero.
#[test]
fn a_right_strapped_mic_is_silent_on_the_left_slot() {
    use crate::peripherals::components::inmp441::MicChannel;
    let mut spi = i2s_ready(MicChannel::Right);
    spi.write_u32(EFR_USART_TXDATA, 0).unwrap();
    assert_eq!(
        spi.read_u32(EFR_USART_RXDATA).unwrap(),
        0,
        "left slot: silent"
    );
    spi.write_u32(EFR_USART_TXDATA, 0).unwrap();
    assert_ne!(
        spi.read_u32(EFR_USART_RXDATA).unwrap(),
        0,
        "right slot: audio"
    );
}

/// FORMAT decides how many MSBs come back. W32D16 must truncate a 24-bit
/// sample to its top 16 bits — RM section 20.3.3.9 p.629.
#[test]
fn format_w32d16_hands_back_only_the_top_sixteen_bits() {
    use crate::peripherals::components::inmp441::MicChannel;
    let mut spi = ready();
    spi.i2s_device = Some(Box::new(
        crate::peripherals::components::inmp441::Inmp441::new("mic", MicChannel::Left),
    ));
    spi.write_u32(
        EFR_USART_I2SCTRL,
        EFR_USART_I2SCTRL_EN | (3 << EFR_USART_I2SCTRL_FORMAT_SHIFT),
    )
    .unwrap();
    spi.write_u32(EFR_USART_TXDATA, 0).unwrap();
    let w = spi.read_u32(EFR_USART_RXDATA).unwrap();
    assert_eq!(w & 0x0000_FFFF, 0, "the low 16 bits must not be readable");
}

/// MONO pulses the word clock per word instead of toggling it, so the
/// stream never leaves the left channel (RM section 20.3.3.8 p.629).
#[test]
fn mono_mode_never_advances_to_the_right_channel() {
    use crate::peripherals::components::inmp441::MicChannel;
    let mut spi = ready();
    spi.i2s_device = Some(Box::new(
        crate::peripherals::components::inmp441::Inmp441::new("mic", MicChannel::Left),
    ));
    spi.write_u32(
        EFR_USART_I2SCTRL,
        EFR_USART_I2SCTRL_EN | EFR_USART_I2SCTRL_MONO | (2 << EFR_USART_I2SCTRL_FORMAT_SHIFT),
    )
    .unwrap();
    for _ in 0..8 {
        spi.write_u32(EFR_USART_TXDATA, 0).unwrap();
        assert_ne!(
            spi.read_u32(EFR_USART_RXDATA).unwrap(),
            0,
            "every mono word stays on the left channel",
        );
    }
}

/// Re-enabling I2S restarts the frame on the LEFT channel, per RM section
/// 20.3.3.11 p.632. Without this a re-enable resumes mid-frame and every
/// later sample is attributed to the wrong side.
#[test]
fn re_enabling_i2s_restarts_on_the_left_channel() {
    use crate::peripherals::components::inmp441::MicChannel;
    let mut spi = i2s_ready(MicChannel::Left);
    // Consume the left word, leaving the frame pointing at right.
    spi.write_u32(EFR_USART_TXDATA, 0).unwrap();
    // Disable, then re-enable.
    spi.write_u32(EFR_USART_I2SCTRL, 0).unwrap();
    spi.write_u32(
        EFR_USART_I2SCTRL,
        EFR_USART_I2SCTRL_EN | (2 << EFR_USART_I2SCTRL_FORMAT_SHIFT),
    )
    .unwrap();
    spi.write_u32(EFR_USART_TXDATA, 0).unwrap();
    assert_ne!(
        spi.read_u32(EFR_USART_RXDATA).unwrap(),
        0,
        "after re-enable the first word is LEFT again",
    );
}

/// ⚠️ A DISPLAY-SIZED SPI BURST MUST NOT COST O(n^2).
///
/// `efr32_wire_bytes` is the wire-narration hold buffer, and it was the
/// only one in this file with no ceiling: the nRF52 path refuses a burst
/// over `NRF52_WIRE_BYTE_CAP` and the H5 path force-publishes at
/// `H5_WIRE_BURST_CAP`. A run is HELD whenever `emit_between` cannot fit it
/// in the `cursor..now` window, and with no logic-capture tap installed —
/// the default for every lab whose analyzer is closed —
/// `PadLines::tap_clock()` is `None`, so `efr32_wire_flush` reads `now` as
/// 0 and the window is empty FOREVER. Every byte was therefore held, and
/// every push (and every one-cycle `on_event` retry) re-narrated the whole
/// held run: shifting n bytes cost O(n^2).
///
/// Measured in the browser on BRD2709A driving a wired ST7789: a
/// full-screen fill (170x320 px = 108 800 SPI bytes) took the engine from
/// 4 000 000 cycles/s to 1 400, `spi0` was 93% of engine time at one call
/// per cycle, and the playground's first 4 000 000-cycle frame never
/// returned — the lab showed "Running" with no cycle counter and an empty
/// serial monitor forever.
///
/// This gates the HOLD BUFFER, not a wall time: a timing assertion would
/// pass on a fast machine with the quadratic still in place.
#[test]
fn a_display_sized_burst_does_not_hold_an_unbounded_narration_buffer() {
    let mut spi = ready();
    // MSBF is what makes `efr32_framing()` answer, i.e. what puts this
    // controller on the narration path at all. `Spi::new_with_layout`
    // already created the line cell eagerly for this layout, so there is
    // no routing step to perform — which is exactly why the `lines.is_none()`
    // early-out in `efr32_wire_push` never fires on this family.
    spi.write_u32(EFR_USART_CTRL, EFR_USART_CTRL_SYNC | EFR_USART_CTRL_MSBF)
        .unwrap();
    assert!(spi.lines.is_some(), "the EFR32 line cell is eager");
    assert!(spi.efr32_framing().is_some(), "on the narration path");

    // One row of a 170-wide RGB565 fill: 340 bytes, already past the cap.
    for i in 0..2_000u32 {
        spi.write_u32(EFR_USART_TXDATA, i & 0xFF).unwrap();
    }
    assert!(
        spi.efr32_wire_bytes.len() <= EFR32_WIRE_BYTE_CAP,
        "held {} bytes with no ceiling: every later push re-narrates all of \
             them, which is the quadratic that froze the ST7789 lab",
        spi.efr32_wire_bytes.len(),
    );
}

#[test]
fn the_layout_resolves_by_name() {
    assert_eq!(
        "efr32s2".parse::<SpiRegisterLayout>().unwrap(),
        SpiRegisterLayout::Efr32s2Usart
    );
}

#[test]
fn status_reads_the_header_reset_value_before_any_command() {
    let spi = controller();
    // TXBL | TXIDLE | TXC — the transmit side is idle and ready.
    assert_eq!(status(&spi) & 0x2040, 0x2040);
    assert_eq!(status(&spi) & EFR_USART_STATUS_MASTER, 0, "not master yet");
    assert_eq!(status(&spi) & EFR_USART_STATUS_RXDATAV, 0);
}

#[test]
fn cmd_latches_master_and_the_enables_into_status() {
    let spi = ready();
    let s = status(&spi);
    assert_eq!(s & EFR_USART_STATUS_MASTER, EFR_USART_STATUS_MASTER);
    assert_eq!(s & EFR_USART_STATUS_TXENS, EFR_USART_STATUS_TXENS);
    assert_eq!(s & EFR_USART_STATUS_RXENS, EFR_USART_STATUS_RXENS);
}

#[test]
fn a_txdata_write_clocks_one_frame_and_the_answer_lands_in_rxdata() {
    let mut spi = ready();
    spi.push_device(Box::new(EchoSlave::default()));

    spi.write_u32(EFR_USART_TXDATA, 0xA5).unwrap();
    assert_eq!(
        status(&spi) & EFR_USART_STATUS_RXDATAV,
        EFR_USART_STATUS_RXDATAV
    );
    assert_eq!(spi.read_u32(EFR_USART_RXDATA).unwrap(), 0x5A, "!0xA5");
}

/// ⚠️ The single most common EFR32 SPI bring-up failure: this block is a
/// UART until `CTRL.SYNC` says otherwise. Clocking a frame anyway would
/// hide it.
#[test]
fn nothing_is_clocked_without_sync_mode() {
    let mut spi = controller();
    spi.write_u32(EFR_USART_EN, EFR_USART_EN_EN).unwrap();
    spi.write_u32(EFR_USART_CMD, EFR_USART_CMD_MASTEREN | EFR_USART_CMD_TXEN)
        .unwrap();
    spi.push_device(Box::new(EchoSlave::default()));

    spi.write_u32(EFR_USART_TXDATA, 0xA5).unwrap();
    assert_eq!(status(&spi) & EFR_USART_STATUS_RXDATAV, 0);
    assert_eq!(spi.read_u32(EFR_USART_RXDATA).unwrap(), 0);
}

#[test]
fn nothing_is_clocked_without_txen() {
    let mut spi = controller();
    spi.write_u32(EFR_USART_EN, EFR_USART_EN_EN).unwrap();
    spi.write_u32(EFR_USART_CTRL, EFR_USART_CTRL_SYNC).unwrap();
    spi.write_u32(EFR_USART_CMD, EFR_USART_CMD_MASTEREN)
        .unwrap();
    spi.push_device(Box::new(EchoSlave::default()));

    spi.write_u32(EFR_USART_TXDATA, 0xA5).unwrap();
    assert_eq!(status(&spi) & EFR_USART_STATUS_RXDATAV, 0);
}

#[test]
fn nothing_is_clocked_while_the_usart_is_disabled() {
    let mut spi = controller();
    spi.write_u32(EFR_USART_CTRL, EFR_USART_CTRL_SYNC).unwrap();
    spi.write_u32(EFR_USART_CMD, EFR_USART_CMD_MASTEREN | EFR_USART_CMD_TXEN)
        .unwrap();
    spi.push_device(Box::new(EchoSlave::default()));

    spi.write_u32(EFR_USART_TXDATA, 0xA5).unwrap();
    assert_eq!(status(&spi) & EFR_USART_STATUS_RXDATAV, 0);
}

/// A word write is ONE frame. Byte-splitting TXDATA would clock four, and a
/// display would see three stray bytes per pixel word.
#[test]
fn a_word_write_of_txdata_is_one_frame_not_four() {
    let mut spi = ready();
    spi.push_device(Box::new(EchoSlave::default()));
    spi.write_u32(EFR_USART_TXDATA, 0x0000_00A5).unwrap();
    assert_eq!(spi.read_u32(EFR_USART_RXDATA).unwrap(), 0x5A);
}

/// ...and so is a byte write, which is how an 8-bit driver spells it.
#[test]
fn a_byte_write_of_txdata_is_also_one_frame() {
    let mut spi = ready();
    spi.push_device(Box::new(EchoSlave::default()));
    spi.write(EFR_USART_TXDATA, 0xA5).unwrap();
    assert_eq!(spi.read_u32(EFR_USART_RXDATA).unwrap(), 0x5A);
}

#[test]
fn a_stream_of_frames_reaches_the_slave_in_order() {
    let mut spi = ready();
    spi.push_device(Box::new(EchoSlave::default()));
    for b in [0x01u8, 0x02, 0x03, 0xFF] {
        spi.write_u32(EFR_USART_TXDATA, b as u32).unwrap();
        assert_eq!(spi.read_u32(EFR_USART_RXDATA).unwrap(), (!b) as u32);
    }
}

#[test]
fn the_flag_register_is_write_one_to_clear() {
    let mut spi = ready();
    spi.push_device(Box::new(EchoSlave::default()));
    spi.write_u32(EFR_USART_TXDATA, 0x11).unwrap();

    let f = spi.read_u32(EFR_USART_IF).unwrap();
    assert_eq!(f & EFR_USART_IF_TXC, EFR_USART_IF_TXC);
    assert_eq!(f & EFR_USART_IF_RXDATAV, EFR_USART_IF_RXDATAV);

    spi.write_u32(EFR_USART_IF, 0xFFFF_FFFF).unwrap();
    assert_eq!(spi.read_u32(EFR_USART_IF).unwrap(), 0);
}

#[test]
fn clearrx_drops_a_pending_byte() {
    let mut spi = ready();
    spi.push_device(Box::new(EchoSlave::default()));
    spi.write_u32(EFR_USART_TXDATA, 0x11).unwrap();
    assert_ne!(status(&spi) & EFR_USART_STATUS_RXDATAV, 0);

    spi.write_u32(EFR_USART_CMD, EFR_USART_CMD_CLEARRX).unwrap();
    assert_eq!(status(&spi) & EFR_USART_STATUS_RXDATAV, 0);
    assert_eq!(spi.read_u32(EFR_USART_RXDATA).unwrap(), 0);
}

#[test]
fn disabling_the_usart_drops_master_and_the_enables() {
    let mut spi = ready();
    spi.write_u32(EFR_USART_EN, 0).unwrap();
    let s = status(&spi);
    assert_eq!(s & EFR_USART_STATUS_MASTER, 0);
    assert_eq!(s & EFR_USART_STATUS_TXENS, 0);
    assert_eq!(s & EFR_USART_STATUS_RXENS, 0);
}
