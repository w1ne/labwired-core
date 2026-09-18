use super::*;
use crate::network::{CanBus, CanFrame, Interconnect};

fn transaction(dev: &mut Mcp2515, bytes: &[u8]) -> Vec<u8> {
    dev.cs_select();
    let result = bytes.iter().map(|byte| dev.transfer(*byte)).collect();
    dev.cs_release();
    result
}

fn read(dev: &mut Mcp2515, address: u8, count: usize) -> Vec<u8> {
    transaction(
        dev,
        &[INST_READ, address]
            .into_iter()
            .chain(std::iter::repeat_n(0, count))
            .collect::<Vec<_>>(),
    )[2..]
        .to_vec()
}

fn write(dev: &mut Mcp2515, address: u8, values: &[u8]) {
    transaction(
        dev,
        &[INST_WRITE, address]
            .into_iter()
            .chain(values.iter().copied())
            .collect::<Vec<_>>(),
    );
}

fn configure_500k_normal(dev: &mut Mcp2515) {
    write(dev, REG_CNF3, &[0x01, 0xBC, 0x00]);
    write(dev, REG_CANCTRL, &[0x00]);
}

#[test]
fn rts_sends_standard_txb0_frame_through_shared_can_bus() {
    let mut bus = CanBus::new();
    let (mcp_tx, mcp_rx) = bus.attach();
    let (_peer_tx, peer_rx) = bus.attach();
    let mut dev = Mcp2515::new("PA4");
    dev.attach_can_bus(mcp_tx, mcp_rx).unwrap();
    configure_500k_normal(&mut dev);

    transaction(
        &mut dev,
        &[0x40, 0xFB, 0xE0, 0, 0, 8, 0x02, 0x01, 0x0C, 0, 0, 0, 0, 0],
    );
    transaction(&mut dev, &[0x81]);
    dev.poll_external_bus();
    bus.tick().unwrap();

    assert_eq!(
        peer_rx.try_recv().unwrap(),
        CanFrame::classic(0x7DF, vec![0x02, 0x01, 0x0C, 0, 0, 0, 0, 0])
    );
    assert_eq!(read(&mut dev, REG_TXB0CTRL, 1)[0] & TXREQ, 0);
    assert_ne!(read(&mut dev, REG_CANINTF, 1)[0] & CANINTF_TX0IF, 0);
}

#[test]
fn standard_bus_response_populates_rxb0_and_drives_active_low_irq() {
    let mut bus = CanBus::new();
    let (mcp_tx, mcp_rx) = bus.attach();
    let (peer_tx, _peer_rx) = bus.attach();
    let mut dev = Mcp2515::new("PA4");
    dev.attach_can_bus(mcp_tx, mcp_rx).unwrap();
    configure_500k_normal(&mut dev);
    write(&mut dev, REG_RXB0CTRL, &[0x60]); // receive any valid standard frame
    write(&mut dev, REG_CANINTE, &[CANINTF_RX0IF]);

    peer_tx
        .send(CanFrame::classic(0x7E8, vec![3, 0x41, 0x0C, 0x12]))
        .unwrap();
    bus.tick().unwrap();
    dev.poll_external_bus();

    assert_eq!(
        read(&mut dev, REG_RXB0SIDH, 9),
        [0xFD, 0x00, 0, 0, 4, 3, 0x41, 0x0C, 0x12]
    );
    assert_eq!(read(&mut dev, REG_CANINTF, 1), [CANINTF_RX0IF]);
    assert_eq!(transaction(&mut dev, &[INST_READ_STATUS, 0])[1] & 1, 1);
    assert_eq!(transaction(&mut dev, &[INST_RX_STATUS, 0])[1] & 0xC0, 0x40);
    assert!(dev.irq_asserted());

    transaction(&mut dev, &[0x90, 0]); // READ RXB0 auto-clear on CS rise
    assert!(!dev.irq_asserted());
}

fn inject(dev: &mut Mcp2515, frame: CanFrame) {
    if !matches!(dev.current_mode(), OpMode::Normal | OpMode::ListenOnly) {
        configure_500k_normal(dev);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(frame).unwrap();
    // Use the supported external-bus receiver path without constructing a full world.
    dev.bus_rx = Some(rx);
    dev.poll_external_bus();
}

fn write_standard_id(dev: &mut Mcp2515, base: u8, id: u32) {
    write(dev, base, &encode_standard_id(id).unwrap());
}

#[test]
fn standard_masks_filters_rollover_and_overflow_preserve_unread_frames() {
    let mut dev = Mcp2515::new("PA4");
    write_standard_id(&mut dev, 0x20, 0x7F0);
    write_standard_id(&mut dev, 0x00, 0x7E0);
    write_standard_id(&mut dev, 0x04, 0x7E0);
    write(&mut dev, REG_RXB0CTRL, &[0x04]); // filtered + BUKT
    write(&mut dev, REG_RXB1CTRL, &[0x40]); // reject direct standard RXB1 matches

    inject(&mut dev, CanFrame::classic(0x123, vec![0xAA]));
    assert_eq!(read(&mut dev, REG_CANINTF, 1), [0], "nonmatch is dropped");
    inject(&mut dev, CanFrame::classic(0x7E8, vec![1]));
    inject(&mut dev, CanFrame::classic(0x7E9, vec![2]));
    assert_eq!(read(&mut dev, REG_CANINTF, 1)[0] & 3, 3, "BUKT fills RXB1");
    assert_eq!(read(&mut dev, REG_RXB0SIDH + 5, 1), [1]);
    assert_eq!(read(&mut dev, REG_RXB1SIDH + 5, 1), [2]);

    inject(&mut dev, CanFrame::classic(0x7EA, vec![3]));
    assert_eq!(
        read(&mut dev, REG_EFLG, 1)[0] & (EFLG_RX0OVR | EFLG_RX1OVR),
        EFLG_RX0OVR | EFLG_RX1OVR
    );
    assert_eq!(read(&mut dev, REG_RXB0SIDH + 5, 1), [1]);
    assert_eq!(read(&mut dev, REG_RXB1SIDH + 5, 1), [2]);
}

#[test]
fn standard_acceptance_uses_sid_with_mide_and_ignores_eid_fields() {
    let mut dev = Mcp2515::new("PA4");
    write(&mut dev, 0x20, &[0xFF, 0xEB, 0xA5, 0x5A]); // SID mask + MIDE, EID ignored
    write(&mut dev, 0x00, &[0xFD, 0x00, 0x12, 0x34]); // 0x7E8, EXIDE=0
    write(&mut dev, 0x04, &[0xFD, 0x00, 0x56, 0x78]);
    write(&mut dev, REG_RXB1CTRL, &[0x40]);

    inject(&mut dev, CanFrame::classic(0x7E8, vec![0x41]));

    assert_eq!(
        read(&mut dev, REG_CANINTF, 1)[0] & CANINTF_RX0IF,
        CANINTF_RX0IF
    );
    assert_eq!(read(&mut dev, REG_RXB0SIDH + 5, 1), [0x41]);
}

#[test]
fn standard_acceptance_rejects_exide_filter_when_mide_is_set() {
    let mut dev = Mcp2515::new("PA4");
    write(&mut dev, 0x20, &[0xFF, 0xEB, 0xA5, 0x5A]); // MIDE=1
    write(&mut dev, 0x00, &[0xFD, 0x08, 0x12, 0x34]); // EXIDE=1
    write(&mut dev, 0x04, &[0xFD, 0x08, 0x56, 0x78]);
    write(&mut dev, REG_RXB1CTRL, &[0x40]);

    inject(&mut dev, CanFrame::classic(0x7E8, vec![0x41]));

    assert_eq!(read(&mut dev, REG_CANINTF, 1)[0] & CANINTF_RX0IF, 0);
}

#[test]
fn unsupported_or_unsendable_tx_stays_pending_without_success_flag() {
    for (mode, header) in [
        (0x80, [0x24, 0x60, 0, 0, 1, 0xAA]),    // config mode
        (0x60, [0x24, 0x60, 0, 0, 1, 0xAA]),    // listen-only
        (0x00, [0x24, 0x60, 0, 0, 1, 0xAA]),    // absent bus channel
        (0x00, [0x24, 0x68, 0, 0, 1, 0xAA]),    // EXIDE
        (0x00, [0x24, 0x60, 0, 0, 9, 0xAA]),    // invalid DLC
        (0x00, [0x24, 0x60, 0, 0, 0x41, 0xAA]), // unsupported RTR
    ] {
        let mut dev = Mcp2515::new("PA4");
        write(&mut dev, REG_CNF3, &[0x01, 0xBC, 0x00]);
        write(&mut dev, REG_CANCTRL, &[mode]);
        transaction(
            &mut dev,
            &[0x40].into_iter().chain(header).collect::<Vec<_>>(),
        );
        transaction(&mut dev, &[0x81]);
        dev.poll_external_bus();
        assert_ne!(read(&mut dev, REG_TXB0CTRL, 1)[0] & TXREQ, 0);
        assert_eq!(read(&mut dev, REG_CANINTF, 1)[0] & CANINTF_TX0IF, 0);
    }
}

#[test]
fn inactive_modes_discard_external_frames_as_time_advances() {
    for inactive_mode in [0x80, 0x20, 0x40] {
        let mut bus = CanBus::new();
        let (mcp_tx, mcp_rx) = bus.attach();
        let (peer_tx, _peer_rx) = bus.attach();
        let mut dev = Mcp2515::new("PA4");
        dev.attach_can_bus(mcp_tx, mcp_rx).unwrap();
        write(&mut dev, REG_CNF3, &[0x01, 0xBC, 0x00]);
        write(&mut dev, REG_RXB0CTRL, &[0x60]);
        write(&mut dev, REG_CANCTRL, &[inactive_mode]);
        peer_tx
            .send(CanFrame::classic(0x321, vec![inactive_mode]))
            .unwrap();
        bus.tick().unwrap();

        dev.poll_external_bus();
        assert_eq!(read(&mut dev, REG_CANINTF, 1)[0] & CANINTF_RX0IF, 0);
        write(&mut dev, REG_CANCTRL, &[0x00]);
        dev.poll_external_bus();
        assert_eq!(read(&mut dev, REG_CANINTF, 1)[0] & CANINTF_RX0IF, 0);

        peer_tx
            .send(CanFrame::classic(0x321, vec![inactive_mode]))
            .unwrap();
        bus.tick().unwrap();
        dev.poll_external_bus();
        assert_eq!(read(&mut dev, REG_RXB0SIDH + 5, 1), [inactive_mode]);
        transaction(&mut dev, &[0x90]);
        dev.poll_external_bus();
        assert_eq!(read(&mut dev, REG_CANINTF, 1)[0] & CANINTF_RX0IF, 0);
    }
}

#[test]
fn listen_only_receives_external_frames_but_never_transmits() {
    let mut bus = CanBus::new();
    let (mcp_tx, mcp_rx) = bus.attach();
    let (peer_tx, peer_rx) = bus.attach();
    let mut dev = Mcp2515::new("PA4");
    dev.attach_can_bus(mcp_tx, mcp_rx).unwrap();
    configure_500k_normal(&mut dev);
    write(&mut dev, REG_RXB0CTRL, &[0x60]);
    write(&mut dev, REG_CANCTRL, &[0x60]);
    transaction(&mut dev, &[0x40, 0x24, 0x60, 0, 0, 1, 0xAA]);
    transaction(&mut dev, &[0x81]);
    peer_tx.send(CanFrame::classic(0x456, vec![0xBB])).unwrap();
    bus.tick().unwrap();

    dev.poll_external_bus();
    bus.tick().unwrap();
    assert_eq!(read(&mut dev, REG_RXB0SIDH + 5, 1), [0xBB]);
    assert!(peer_rx.try_recv().is_err());
    assert_ne!(read(&mut dev, REG_TXB0CTRL, 1)[0] & TXREQ, 0);
}

#[test]
fn remote_tx_is_retained_without_emitting_or_signaling_success() {
    let mut bus = CanBus::new();
    let (mcp_tx, mcp_rx) = bus.attach();
    let (_peer_tx, peer_rx) = bus.attach();
    let mut dev = Mcp2515::new("PA4");
    dev.attach_can_bus(mcp_tx, mcp_rx).unwrap();
    configure_500k_normal(&mut dev);
    transaction(&mut dev, &[0x40, 0x24, 0x60, 0, 0, 0x41, 0xAA]);
    transaction(&mut dev, &[0x81]);

    dev.poll_external_bus();
    bus.tick().unwrap();

    assert!(peer_rx.try_recv().is_err());
    assert_ne!(read(&mut dev, REG_TXB0CTRL, 1)[0] & TXREQ, 0);
    assert_eq!(read(&mut dev, REG_CANINTF, 1)[0] & CANINTF_TX0IF, 0);
}

#[test]
fn overflow_sets_errif_and_enabled_error_irq_until_flag_clear() {
    let mut dev = Mcp2515::new("PA4");
    write(&mut dev, REG_RXB0CTRL, &[0x64]);
    write(&mut dev, REG_RXB1CTRL, &[0x60]);
    write(&mut dev, REG_CANINTE, &[0x20]);
    inject(&mut dev, CanFrame::classic(0x100, vec![1]));
    inject(&mut dev, CanFrame::classic(0x101, vec![2]));
    inject(&mut dev, CanFrame::classic(0x102, vec![3]));

    assert_eq!(read(&mut dev, REG_EFLG, 1)[0] & 0xC0, 0xC0);
    assert_eq!(read(&mut dev, REG_CANINTF, 1)[0] & 0x20, 0x20);
    assert!(dev.irq_asserted());
    transaction(&mut dev, &[INST_BITMOD, REG_CANINTF, 0x20, 0]);
    assert!(!dev.irq_asserted());
    assert_eq!(read(&mut dev, REG_EFLG, 1)[0] & 0xC0, 0xC0);
}

#[test]
fn loopback_uses_receive_filters_without_emitting_to_bus() {
    let mut bus = CanBus::new();
    let (mcp_tx, mcp_rx) = bus.attach();
    let (_peer_tx, peer_rx) = bus.attach();
    let mut dev = Mcp2515::new("PA4");
    dev.attach_can_bus(mcp_tx, mcp_rx).unwrap();
    configure_500k_normal(&mut dev);
    write(&mut dev, REG_RXB0CTRL, &[0x60]);
    write(&mut dev, REG_CANCTRL, &[0x40]);
    transaction(&mut dev, &[0x40, 0x24, 0x60, 0, 0, 1, 0xAB]);
    transaction(&mut dev, &[0x81]);
    dev.poll_external_bus();
    bus.tick().unwrap();
    assert!(peer_rx.try_recv().is_err());
    assert_eq!(read(&mut dev, REG_RXB0SIDH + 5, 1), [0xAB]);
}

#[test]
fn external_poll_drains_fifo_and_ignores_unsupported_frame_kinds() {
    let mut dev = Mcp2515::new("PA4");
    configure_500k_normal(&mut dev);
    write_standard_id(&mut dev, 0x20, 0x7FF);
    write_standard_id(&mut dev, 0x24, 0x7FF);
    write_standard_id(&mut dev, 0x00, 0x101);
    write_standard_id(&mut dev, 0x04, 0x101);
    for base in [0x08, 0x10, 0x14, 0x18] {
        write_standard_id(&mut dev, base, 0x102);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    dev.bus_rx = Some(rx);
    let mut unsupported = CanFrame::classic(0x100, vec![0xEE]);
    unsupported.fd = true;
    tx.send(unsupported).unwrap();
    tx.send(CanFrame::classic(0x101, vec![1])).unwrap();
    tx.send(CanFrame::classic(0x102, vec![2])).unwrap();
    dev.poll_external_bus();
    assert_eq!(read(&mut dev, REG_RXB0SIDH + 5, 1), [1]);
    assert_eq!(read(&mut dev, REG_RXB1SIDH + 5, 1), [2]);
}

#[test]
fn bit_modify_interrupt_clear_recomputes_irq() {
    let mut dev = Mcp2515::new("PA4");
    write(&mut dev, REG_CANINTE, &[CANINTF_TX0IF]);
    write(&mut dev, REG_CANINTF, &[CANINTF_TX0IF]);
    assert!(dev.irq_asserted());
    transaction(&mut dev, &[INST_BITMOD, REG_CANINTF, CANINTF_TX0IF, 0]);
    assert!(!dev.irq_asserted());
}

#[test]
fn external_poll_demand_starts_only_after_can_attachment() {
    let mut dev = Mcp2515::new("PA4");
    assert!(!dev.needs_external_bus_poll());
    let (tx, _outbound) = std::sync::mpsc::channel();
    let (_inbound, rx) = std::sync::mpsc::channel();
    dev.attach_can_bus(tx, rx).unwrap();
    assert!(dev.needs_external_bus_poll());
}

#[test]
fn reset_and_read_canctrl() {
    let mut dev = Mcp2515::new("PA4");
    dev.cs_select();
    dev.transfer(INST_RESET);
    dev.cs_release();
    dev.cs_select();
    dev.transfer(INST_READ);
    dev.transfer(REG_CANCTRL);
    let v = dev.transfer(0x00);
    assert_eq!(v, 0x87);
    assert_eq!(read(&mut dev, REG_CANSTAT, 2), [0x80, 0x87]);
    assert_eq!(read(&mut dev, REG_CANINTF, 1), [0]);
    assert_eq!(read(&mut dev, REG_EFLG, 1), [0]);
}

#[test]
fn write_canctrl_updates_canstat_opmode() {
    let mut dev = Mcp2515::new("PA4");
    write(&mut dev, REG_CNF3, &[0x01, 0xBC, 0x00]);
    dev.cs_select();
    dev.transfer(INST_WRITE);
    dev.transfer(REG_CANCTRL);
    dev.transfer(0x00); // normal mode
    dev.cs_release();
    dev.cs_select();
    dev.transfer(INST_READ);
    dev.transfer(REG_CANSTAT);
    let st = dev.transfer(0);
    assert_eq!(st & 0xE0, 0x00);
}

#[test]
fn sequential_write_read_and_bit_modify_share_register_side_effects() {
    let mut dev = Mcp2515::new("PA4");
    write(&mut dev, REG_CNF3, &[0x01, 0xBC, 0x00]);
    assert_eq!(read(&mut dev, REG_CNF3, 3), [0x01, 0xBC, 0x00]);
    transaction(&mut dev, &[INST_BITMOD, REG_CANCTRL, 0xE0, 0x40]);
    assert_eq!(read(&mut dev, REG_CANCTRL, 1)[0] & 0xE0, 0x40);
    assert_eq!(read(&mut dev, REG_CANSTAT, 1)[0] & 0xE0, 0x40);
    for mode in [0x20, 0x60, 0x80] {
        write(&mut dev, REG_CANCTRL, &[mode]);
        assert_eq!(read(&mut dev, REG_CANSTAT, 1)[0] & 0xE0, mode);
    }
}

#[test]
fn absent_timing_rejects_active_mode_without_changing_interrupt_flags() {
    let mut dev = Mcp2515::new("PA4");
    write(&mut dev, REG_CANINTF, &[CANINTF_TX0IF]);
    write(&mut dev, REG_CANCTRL, &[0x00]);
    assert_eq!(read(&mut dev, REG_CANSTAT, 1)[0] & 0xE0, 0x80);
    assert_eq!(read(&mut dev, REG_CANCTRL, 1)[0] & 0xE0, 0x00);
    assert_eq!(read(&mut dev, REG_CANINTF, 1), [CANINTF_TX0IF]);
}

#[test]
fn structurally_invalid_timing_rejects_active_mode() {
    let mut dev = Mcp2515::new("PA4");
    // 16 TQ / 500 kbit/s, but SJW=4 TQ exceeds PHSEG2=2 TQ.
    write(&mut dev, REG_CNF3, &[0x01, 0xBC, 0xC0]);
    write(&mut dev, REG_CANCTRL, &[0x40]);
    assert_eq!(read(&mut dev, REG_CANSTAT, 1)[0] & 0xE0, 0x80);
}

#[test]
fn tseg1_shorter_than_phase2_rejects_active_mode_without_touching_interrupts() {
    let mut dev = Mcp2515::new("PA4");
    // Exact 16 TQ / 500 kbit/s, but PROPSEG(1) + PHSEG1(6) < PHSEG2(8).
    write(&mut dev, REG_CNF3, &[0x07, 0xA8, 0x00]);
    write(&mut dev, REG_CANINTF, &[CANINTF_TX0IF]);
    write(&mut dev, REG_CANCTRL, &[0x40]);
    assert_eq!(read(&mut dev, REG_CANSTAT, 1)[0] & 0xE0, 0x80);
    assert_eq!(read(&mut dev, REG_CANINTF, 1), [CANINTF_TX0IF]);
}

#[test]
fn bitrate_mismatch_over_one_percent_rejects_active_mode() {
    let mut dev = Mcp2515::new("PA4");
    // Structurally valid 16 TQ timing, but BRP=1 computes to 250 kbit/s.
    write(&mut dev, REG_CNF3, &[0x01, 0xBC, 0x01]);
    write(&mut dev, REG_CANCTRL, &[0x60]);
    assert_eq!(read(&mut dev, REG_CANSTAT, 1)[0] & 0xE0, 0x80);
}

#[test]
fn valid_16mhz_500k_timing_accepts_active_modes() {
    let mut dev = Mcp2515::new("PA4");
    write(&mut dev, REG_CNF3, &[0x01, 0xBC, 0x00]);
    for mode in [0x00, 0x40, 0x60] {
        write(&mut dev, REG_CANCTRL, &[mode]);
        assert_eq!(read(&mut dev, REG_CANSTAT, 1)[0] & 0xE0, mode);
    }
}

#[test]
fn config_and_sleep_are_allowed_without_timing() {
    let mut dev = Mcp2515::new("PA4");
    write(&mut dev, REG_CANCTRL, &[0x20]);
    assert_eq!(read(&mut dev, REG_CANSTAT, 1)[0] & 0xE0, 0x20);
    write(&mut dev, REG_CANCTRL, &[0x80]);
    assert_eq!(read(&mut dev, REG_CANSTAT, 1)[0] & 0xE0, 0x80);
}

#[test]
fn load_tx_buffer_variants_and_rts_set_txreq_and_status() {
    let mut dev = Mcp2515::new("PA4");
    let frame = [0x24, 0x60, 0, 0, 3, 0x11, 0x22, 0x33];
    for (load, rts, ctrl, base) in [
        (0x40, 0x81, REG_TXB0CTRL, REG_TXB0SIDH),
        (0x42, 0x82, REG_TXB1CTRL, REG_TXB1SIDH),
        (0x44, 0x84, REG_TXB2CTRL, REG_TXB2SIDH),
    ] {
        transaction(
            &mut dev,
            &[load].into_iter().chain(frame).collect::<Vec<_>>(),
        );
        assert_eq!(read(&mut dev, base, frame.len()), frame);
        transaction(&mut dev, &[rts]);
        assert_ne!(read(&mut dev, ctrl, 1)[0] & TXREQ, 0);
    }
    assert_eq!(
        transaction(&mut dev, &[INST_READ_STATUS, 0])[1] & 0x54,
        0x54
    );

    transaction(&mut dev, &[0x41, 0xAA, 0xBB]);
    assert_eq!(read(&mut dev, REG_TXB0D0, 2), [0xAA, 0xBB]);
    transaction(&mut dev, &[0x43, 0xCC]);
    assert_eq!(read(&mut dev, REG_TXB1D0, 1), [0xCC]);
    transaction(&mut dev, &[0x45, 0xDD]);
    assert_eq!(read(&mut dev, REG_TXB2D0, 1), [0xDD]);
}

#[test]
fn combined_rts_opcodes_set_every_selected_txreq() {
    for (command, expected) in [
        (0x83, [true, true, false]),
        (0x85, [true, false, true]),
        (0x86, [false, true, true]),
        (0x87, [true, true, true]),
    ] {
        let mut dev = Mcp2515::new("PA4");
        transaction(&mut dev, &[command]);
        for (ctrl, pending) in [REG_TXB0CTRL, REG_TXB1CTRL, REG_TXB2CTRL]
            .into_iter()
            .zip(expected)
        {
            assert_eq!(read(&mut dev, ctrl, 1)[0] & TXREQ != 0, pending);
        }
    }
}

#[test]
fn read_rx_buffer_variants_auto_clear_only_the_selected_rx_flag() {
    let mut dev = Mcp2515::new("PA4");
    let header0 = [0x24, 0x60, 0, 0, 2, 0xDE, 0xAD];
    let header1 = [0x64, 0x20, 0, 0, 2, 0xBE, 0xEF];
    write(&mut dev, REG_RXB0SIDH, &header0);
    write(&mut dev, REG_RXB1SIDH, &header1);
    assert_eq!(transaction(&mut dev, &[0x90, 0, 0])[1..], header0[..2]);
    assert_eq!(transaction(&mut dev, &[0x92, 0, 0])[1..], [0xDE, 0xAD]);
    assert_eq!(transaction(&mut dev, &[0x94, 0, 0])[1..], header1[..2]);
    assert_eq!(transaction(&mut dev, &[0x96, 0, 0])[1..], [0xBE, 0xEF]);

    write(&mut dev, REG_CANINTF, &[CANINTF_RX0IF | CANINTF_RX1IF]);
    transaction(&mut dev, &[0x90]);
    assert_eq!(read(&mut dev, REG_CANINTF, 1), [CANINTF_RX1IF]);
    transaction(&mut dev, &[0x94, 0, 0, 0]);
    assert_eq!(read(&mut dev, REG_CANINTF, 1), [0]);
}

#[test]
fn ordinary_read_and_odd_rx_opcodes_preserve_rx_flags() {
    let mut dev = Mcp2515::new("PA4");
    write(&mut dev, REG_RXB0SIDH, &[0x24, 0x60]);
    write(&mut dev, REG_CANINTF, &[CANINTF_RX0IF | CANINTF_RX1IF]);

    assert_eq!(read(&mut dev, REG_RXB0SIDH, 2), [0x24, 0x60]);
    assert_eq!(
        read(&mut dev, REG_CANINTF, 1),
        [CANINTF_RX0IF | CANINTF_RX1IF]
    );

    for command in [0x91, 0x93, 0x95, 0x97] {
        assert_eq!(transaction(&mut dev, &[command, 0xAA, 0xBB]), [0, 0, 0]);
        assert_eq!(
            read(&mut dev, REG_CANINTF, 1),
            [CANINTF_RX0IF | CANINTF_RX1IF]
        );
    }
}

#[test]
fn status_commands_reflect_interrupts_rx_full_and_standard_frame_kind() {
    let mut dev = Mcp2515::new("PA4");
    write(
        &mut dev,
        REG_CANINTF,
        &[CANINTF_RX0IF | CANINTF_RX1IF | CANINTF_TX0IF],
    );
    assert_eq!(transaction(&mut dev, &[INST_READ_STATUS, 0])[1], 0x0B);
    assert_eq!(transaction(&mut dev, &[INST_RX_STATUS, 0])[1] & 0xC0, 0xC0);

    write(&mut dev, 0x60, &[0x01]); // RXB0CTRL FILHIT0
    write(&mut dev, 0x62, &[0x10]); // RXB0SIDL standard RTR/SRR
    assert_eq!(transaction(&mut dev, &[INST_RX_STATUS, 0])[1] & 0x0F, 0x09);
}

#[test]
fn rx_status_decodes_all_standard_and_extended_frame_types() {
    for (sidl, dlc, expected_type) in [
        (0x00, 0x00, 0x00), // standard data
        (0x10, 0x00, 0x08), // standard remote (SRR/RTR in SIDL)
        (0x08, 0x00, 0x10), // extended data
        (0x08, 0x40, 0x18), // extended remote (RTR in DLC)
    ] {
        let mut dev = Mcp2515::new("PA4");
        write(&mut dev, REG_RXB0SIDH, &[0x24, sidl, 0x12, 0x34, dlc]);
        write(&mut dev, REG_CANINTF, &[CANINTF_RX0IF]);
        assert_eq!(
            transaction(&mut dev, &[INST_RX_STATUS, 0])[1] & 0x18,
            expected_type
        );
    }
}

#[test]
fn btlmode_zero_derives_phase2_and_accepts_valid_500k_timing() {
    let mut dev = Mcp2515::new("PA4");
    // PROPSEG=5, PHSEG1=5, derived PHSEG2=max(PHSEG1, IPT)=5: 16 TQ.
    // CNF3 requests PHSEG2=8, but BTLMODE=0 means that field is ignored.
    write(&mut dev, REG_CNF3, &[0x07, 0x24, 0x00]);
    write(&mut dev, REG_CANCTRL, &[0x00]);
    assert_eq!(read(&mut dev, REG_CANSTAT, 1)[0] & 0xE0, 0x00);
}

#[test]
fn cnf_registers_are_writable_only_in_actual_configuration_mode() {
    let mut dev = Mcp2515::new("PA4");
    write(&mut dev, REG_CNF3, &[0x01, 0xBC, 0x00]);
    write(&mut dev, REG_CANCTRL, &[0x00]);
    assert_eq!(read(&mut dev, REG_CANSTAT, 1)[0] & 0xE0, 0x00);

    write(&mut dev, REG_CNF3, &[0x07, 0xAA, 0x55]);
    transaction(&mut dev, &[INST_BITMOD, REG_CNF2, 0xFF, 0x11]);
    assert_eq!(read(&mut dev, REG_CNF3, 3), [0x01, 0xBC, 0x00]);

    write(&mut dev, REG_CANCTRL, &[0x80]);
    write(&mut dev, REG_CNF3, &[0x07]);
    transaction(&mut dev, &[INST_BITMOD, REG_CNF2, 0xFF, 0x11]);
    assert_eq!(read(&mut dev, REG_CNF3, 2), [0x07, 0x11]);
}

#[test]
fn standard_identifier_helpers_round_trip_and_reject_extended_form() {
    for id in [0, 0x123, 0x7FF] {
        let encoded = encode_standard_id(id).unwrap();
        assert_eq!(decode_standard_id(encoded).unwrap(), id);
    }
    assert!(encode_standard_id(0x800).is_none());
    assert!(decode_standard_id([0, 0x08, 0, 0]).is_none());
}

#[test]
fn chip_select_boundary_discards_partial_command_state() {
    let mut dev = Mcp2515::new("PA4");
    transaction(&mut dev, &[INST_WRITE, REG_CNF3]);
    transaction(&mut dev, &[0x01]);
    assert_eq!(read(&mut dev, REG_CNF3, 1), [0]);
}
