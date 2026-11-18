use labwired_core::network::timed_uart::{LineFormat, Parity, TimedUartNet, TimedUartPort};
use labwired_core::peripherals::uart::{Uart, UartRegisterLayout};
use labwired_core::{CycleClock, Peripheral};

fn write32(uart: &mut Uart, offset: u64, value: u32) {
    for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
        uart.write(offset + i as u64, byte).unwrap();
    }
}
fn read32(uart: &Uart, offset: u64) -> u32 {
    let mut bytes = [0; 4];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = uart.read(offset + i as u64).unwrap();
    }
    u32::from_le_bytes(bytes)
}
fn setup() -> (Uart, CycleClock, TimedUartPort, LineFormat) {
    let net = TimedUartNet::new();
    let (_, port, peer) = net.add_link(
        ("device", "uart1", 16_000_000),
        ("master", "uart1", 16_000_000),
        0,
        0,
        1,
    );
    let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32V2);
    let clock = CycleClock::default();
    uart.attach_cycle_clock(clock.clone());
    uart.attach_timed_port(port).unwrap();
    write32(&mut uart, 0x0c, 417);
    write32(
        &mut uart,
        0,
        1 | (1 << 2) | (1 << 3) | (1 << 12) | (1 << 10),
    );
    let format = LineFormat {
        bit_ps: 26_062_500,
        data_bits: 8,
        parity: Parity::Even,
        stop_half_bits: 2,
        rx_enabled: true,
        tx_enabled: true,
        tx_inverted: false,
        rx_inverted: false,
    };
    (uart, clock, peer, format)
}

#[test]
fn clear_tc_during_transmit_does_not_hide_completion() {
    let (mut uart, clock, _, _) = setup();
    write32(&mut uart, 0x28, 0x5a);
    clock.publish(1);
    uart.tick();
    write32(&mut uart, 0x20, 1 << 6);
    clock.publish(417 * 11 + 1);
    uart.tick();
    assert_ne!(read32(&uart, 0x1c) & (1 << 6), 0);
}

#[test]
fn tc_completion_survives_first_service_after_entire_frame() {
    let (mut uart, clock, _, _) = setup();
    write32(&mut uart, 0x28, 0x5a);
    write32(&mut uart, 0x20, 1 << 6);
    clock.publish(417 * 11 + 1);
    uart.tick();
    assert_ne!(read32(&uart, 0x1c) & (1 << 6), 0);
}

#[test]
fn parity_error_survives_read_and_next_clean_character_until_icr() {
    let (mut uart, clock, peer, format) = setup();
    peer.transmit(
        0,
        0x35,
        LineFormat {
            parity: Parity::Odd,
            ..format
        },
    );
    clock.publish(5000);
    uart.tick();
    assert_ne!(read32(&uart, 0x1c) & 1, 0);
    assert_eq!(uart.read(0x24).unwrap(), 0x35);
    assert_ne!(read32(&uart, 0x1c) & 1, 0);
    peer.transmit(10000, 0x45, format);
    clock.publish(15000);
    uart.tick();
    assert_ne!(read32(&uart, 0x1c) & 1, 0);
    write32(&mut uart, 0x18, 1 << 3);
    assert_eq!(read32(&uart, 0x1c) & (1 << 5), 0);
    assert_ne!(read32(&uart, 0x1c) & 1, 0);
    write32(&mut uart, 0x20, 1);
    assert_eq!(read32(&uart, 0x1c) & 1, 0);
}

#[test]
fn disabled_usart_does_not_raise_tc_interrupt() {
    let (mut uart, _, _, _) = setup();
    let cr1 = read32(&uart, 0);
    write32(&mut uart, 0, cr1 | (1 << 6));
    assert_eq!(uart.irq_line_level(), Some(true));
    write32(&mut uart, 0, (cr1 | (1 << 6)) & !1);
    assert_eq!(uart.irq_line_level(), Some(false));
    assert!(!uart.tick().irq);
}

#[test]
fn modern_8e1_registers_transmit_complete_and_receive_peek() {
    let (mut uart, clock, peer, format) = setup();
    assert_eq!(read32(&uart, 0x1c) & (3 << 21), 3 << 21);
    assert_eq!(read32(&uart, 0x0c), 417);
    write32(&mut uart, 4, 2 << 12);
    assert_eq!(read32(&uart, 4), 2 << 12);
    write32(&mut uart, 4, 0);
    write32(&mut uart, 0x28, 0xda);
    assert_eq!(read32(&uart, 0x1c) & (1 << 6), 0);
    clock.publish(1);
    uart.tick();
    clock.publish(417 * 11);
    uart.tick();
    assert_eq!(read32(&uart, 0x1c) & (1 << 6), 0);
    clock.publish(417 * 11 + 1);
    uart.tick();
    assert_ne!(read32(&uart, 0x1c) & (1 << 6), 0);
    let (got, _) = peer.receive_due(417 * 12, &format, false);
    let got = got.unwrap();
    // The shared sampler includes the parity bit in the register-width value.
    assert_eq!(got.value & 0xff, 0xda);
    assert_eq!(got.value >> 8, 1);
    assert!(!got.parity_error && !got.framing_error);
    peer.transmit(10000, 0xa5, format);
    clock.publish(15000);
    uart.tick();
    assert_ne!(read32(&uart, 0x1c) & (1 << 5), 0);
    assert_eq!(uart.peek(0x24), Some(0xa5));
    assert_ne!(read32(&uart, 0x1c) & (1 << 5), 0);
    assert_eq!(uart.read(0x24).unwrap(), 0xa5);
    assert_eq!(read32(&uart, 0x1c) & (1 << 5), 0);
}

#[test]
fn icr_clears_only_selected_error_and_rx_flush_preserves_error() {
    let (mut uart, clock, peer, format) = setup();
    let bad = LineFormat {
        parity: Parity::Odd,
        stop_half_bits: 0,
        ..format
    };
    peer.transmit(0, 0x35, bad);
    peer.transmit(417 * 10, 0, bad);
    clock.publish(5000);
    uart.tick();
    assert_eq!(read32(&uart, 0x1c) & 3, 3);
    write32(&mut uart, 0x20, 1);
    assert_eq!(read32(&uart, 0x1c) & 3, 2);
    write32(&mut uart, 0x18, 1 << 3);
    assert_eq!(read32(&uart, 0x1c) & (1 << 5), 0);
    assert_eq!(read32(&uart, 0x1c) & 3, 2);
    write32(&mut uart, 0x20, 2);
    assert_eq!(read32(&uart, 0x1c) & 3, 0);
}

// Real L6362A firmware programs both CR2 polarity bits. Ignoring them can
// make two incorrectly wired transceivers appear to exchange valid bytes.
#[test]
fn txinv_inverts_the_complete_8e1_waveform() {
    for tx_inverted in [false, true] {
        for rx_inverted in [false, true] {
            let (mut uart, clock, peer, mut format) = setup();
            write32(&mut uart, 4, u32::from(tx_inverted) << 17);
            format.rx_inverted = rx_inverted;
            write32(&mut uart, 0x28, 0x5a);
            clock.publish(1);
            uart.tick();
            let (received, _) = peer.receive_due(10_000, &format, false);
            let clean = received
                .is_some_and(|d| d.value & 0xff == 0x5a && !d.parity_error && !d.framing_error);
            assert_eq!(
                clean,
                tx_inverted == rx_inverted,
                "TXINV={tx_inverted}, receiver inversion={rx_inverted}"
            );
        }
    }
}

#[test]
fn rxinv_normalizes_start_data_parity_and_stop_before_decoding() {
    for tx_inverted in [false, true] {
        for rx_inverted in [false, true] {
            let (mut uart, clock, peer, mut format) = setup();
            write32(&mut uart, 4, u32::from(rx_inverted) << 16);
            format.tx_inverted = tx_inverted;
            peer.transmit(0, 0xa5, format);
            clock.publish(10_000);
            uart.tick();
            let status = read32(&uart, 0x1c);
            let value = uart.read(0x24).unwrap();
            let clean = status & (1 << 5) != 0 && status & 3 == 0 && value == 0xa5;
            assert_eq!(
                clean,
                tx_inverted == rx_inverted,
                "sender inversion={tx_inverted}, RXINV={rx_inverted}"
            );
        }
    }
}

#[test]
fn a_transmitted_frames_polarity_does_not_change_with_later_cr2_writes() {
    let (mut uart, clock, peer, mut format) = setup();
    write32(&mut uart, 4, 1 << 17);
    write32(&mut uart, 0x28, 0xff);
    clock.publish(1);
    uart.tick();
    // Change CR2 after the start bit. The active frame retains its format.
    write32(&mut uart, 4, 0);
    format.rx_inverted = true;
    let (received, _) = peer.receive_due(5000, &format, false);
    let received = received.expect("inverted frame");
    assert_eq!(received.value & 0xff, 0xff);
    assert!(!received.parity_error && !received.framing_error);
    clock.publish(6000);
    uart.tick();
    write32(&mut uart, 0x28, 0);
    clock.publish(6001);
    uart.tick();
    format.rx_inverted = false;
    let (received, _) = peer.receive_due(12000, &format, false);
    let received = received.expect("next non-inverted frame");
    assert_eq!(received.value & 0xff, 0);
    assert!(!received.parity_error && !received.framing_error);
}
