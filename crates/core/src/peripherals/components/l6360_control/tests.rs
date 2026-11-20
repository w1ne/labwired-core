use super::*;

fn fixture(address: u8) -> (L6360Control, L6360I2c) {
    let control = L6360Control::new(address).unwrap();
    let device = control.i2c_device();
    (control, device)
}
fn write(device: &mut L6360I2c, bytes: &[u8]) {
    device.start();
    for &byte in bytes {
        device.write(byte);
    }
    device.stop();
}
fn read(device: &mut L6360I2c, count: usize) -> Vec<u8> {
    device.start();
    let bytes = (0..count).map(|_| device.read()).collect();
    device.stop();
    bytes
}
fn random(device: &mut L6360I2c, start: u8) -> Vec<u8> {
    write(device, &[start]);
    read(device, 9 - start as usize)
}
fn configure(device: &mut L6360I2c) {
    // Independent DS8900 Eq.3 vector: TRI, 220 mA/100 us, 5.5 mA,
    // protected L+, LEDs off; then Eq.2 push-pull configuration.
    write(device, &[0x80, 0xa0, 0x20, 0, 0, 0, 0, 0x50]);
    write(device, &[0xe0, 0xa1]);
}
fn inputs(control: &L6360Control, now: u64, change: impl FnOnce(&mut L6360Inputs)) {
    let mut levels = control.snapshot().inputs;
    change(&mut levels);
    control.update_inputs(now, levels).unwrap();
}

#[test]
fn documented_reset_does_not_invent_received_configuration_parity() {
    let (control, mut device) = fixture(0x60);
    assert_eq!(read(&mut device, 2), [0, 0]);
    let bytes = random(&mut device, 0);
    assert_eq!(bytes, [0, 0x80, 0x21, 0x21, 0, 0, 0, 0, 0]);
    // This is the actual old firmware assumption. It must fail at reset;
    // current-status + known register writes are required before verification.
    let old_readback_parity = bytes[..8].iter().enumerate().fold(0, |acc, (i, value)| {
        acc | (((value.count_ones() % 2) as u8) << (7 - i))
    });
    assert_eq!(old_readback_parity, 0x40);
    assert_ne!(old_readback_parity, bytes[8]);
    let s = control.snapshot();
    assert_eq!(s.mode, L6360Mode::TriState);
    assert!(!s.cq_driver_enabled && s.cq_receiver_enabled && !s.irq_asserted);
    assert_eq!(
        (s.cq_cutoff_current_ma, s.cq_cutoff_delay_ns),
        (220, 100_000)
    );
    assert_eq!(s.cq_restart_delay_ns, Some(25_500_000));
    assert_eq!((s.cq_debounce_ns, s.iq_debounce_ns), (5_000, 5_000));
}

#[test]
fn hardware_address_straps_and_reset_selection_are_exact() {
    for address in 0x60..=0x67 {
        let (control, mut device) = fixture(address);
        assert!(device.claims_address(address));
        for other in 0..=0x7f {
            assert_eq!(device.claims_address(other), other == address);
        }
        inputs(&control, 1, |i| i.reset_n = false);
        assert!(!device.claims_address(address));
        assert_eq!(read(&mut device, 2), [0xff, 0xff]);
        inputs(&control, 2, |i| i.reset_n = true);
        assert!(device.claims_address(address));
        assert_eq!(read(&mut device, 2), [0, 0]);
    }
    for bad in [0, 0x5f, 0x68, 0xc0, 0xff] {
        assert_eq!(
            L6360Control::new(bad).unwrap_err(),
            L6360Error::InvalidAddress(bad)
        );
    }
}

#[test]
fn fixed_current_write_vectors_and_two_byte_commit_order() {
    for (address, data, expected) in [
        (1, 0xe0, 0xa1),
        (1, 0x80, 0x61),
        (1, 0x81, 0xc1),
        (2, 0xf0, 0x02),
        (2, 0xa0, 0x02),
        (3, 0x20, 0x63),
    ] {
        assert_eq!(write_check_byte(address, data), expected);
    }
    let (control, mut device) = fixture(0x60);
    device.start();
    device.write(0xe0);
    assert_eq!(control.snapshot().registers[1], 0x80);
    device.write(0xa1);
    assert_eq!(control.snapshot().registers[1], 0x80);
    device.stop();
    assert_eq!(control.snapshot().mode, L6360Mode::PushPull);
    assert_eq!(
        random(&mut device, 0),
        [0, 0xe0, 0x21, 0x21, 0, 0, 0, 0, 0x40]
    );
}

#[test]
fn known_initialization_and_write_readback_match_physical_driver_contract() {
    let (control, mut device) = fixture(0x61);
    configure(&mut device);
    assert_eq!(
        random(&mut device, 0),
        [0, 0xe0, 0xa0, 0x20, 0, 0, 0, 0, 0x50]
    );
    let s = control.snapshot();
    assert_eq!(
        (s.counters.current_writes, s.counters.sequential_writes),
        (1, 1)
    );
    assert_eq!((s.cq_debounce_ns, s.iq_debounce_ns), (0, 0));
    assert!(s.tx_inverted && s.rx_inverted && s.cq_pulldown_enabled);
    inputs(&control, 1, |i| {
        i.encq = true;
        i.enl = true;
    });
    let s = control.snapshot();
    assert!(s.cq_driver_requested && s.cq_driver_enabled && s.lplus_driver_enabled);
    assert!(!s.cq_receiver_enabled && !s.cq_pulldown_enabled);
    inputs(&control, 2, |i| i.encq = false);
    assert!(!control.snapshot().cq_driver_enabled);
    assert!(control.snapshot().cq_receiver_enabled);
}

#[test]
fn each_bad_current_parity_bit_preserves_register_and_reports_received_data_parity() {
    for bit in [5, 6, 7] {
        let (control, mut device) = fixture(0x60);
        // CONFIG's default 0x80 has odd parity; the rejected DATA=0 has even
        // parity. Its retained parity must describe received data, not readback.
        write(&mut device, &[0x00, 0x01 ^ (1 << bit)]);
        let s = control.snapshot();
        assert_eq!(s.registers[1], 0x80);
        assert_eq!(s.status, STATUS_PE);
        assert_eq!(s.parity, 0x80);
        assert_eq!(s.counters.rejected_writes, 1);
        assert!(s.irq_asserted && s.protocol_error.is_none());
        assert_eq!(read(&mut device, 2), [STATUS_PE, 0x80]);
        assert_eq!(control.snapshot().status, 0);
    }
}

#[test]
fn sequential_parity_failure_is_atomic_and_retains_all_received_parities() {
    let (control, mut device) = fixture(0x60);
    write(&mut device, &[0x80, 0xa0, 0x20, 0, 0, 0, 0, 0x51]);
    let s = control.snapshot();
    assert_eq!(&s.registers[1..8], &RESET_REGISTERS[1..8]);
    assert_eq!(s.status, STATUS_PE);
    assert_eq!(s.parity, 0xd0);
    assert_eq!(
        (s.counters.sequential_writes, s.counters.rejected_writes),
        (0, 1)
    );
    read(&mut device, 2);
    write(&mut device, &[0x80, 0xa0, 0x20, 0, 0, 0, 0, 0x50]);
    assert_eq!(
        random(&mut device, 0),
        [0, 0x80, 0xa0, 0x20, 0, 0, 0, 0, 0x50]
    );
}

#[test]
fn forbidden_active_bridge_changes_reject_current_and_entire_sequential_write() {
    for old in [0x20, 0x40, 0x60, 0xa0, 0xc0, 0xe0] {
        for new in [0x20, 0x40, 0x60, 0xa0, 0xc0, 0xe0] {
            let (control, mut device) = fixture(0x60);
            write(&mut device, &[old, write_check_byte(1, old)]);
            let same = L6360Mode::from_configuration(old) == L6360Mode::from_configuration(new);
            write(&mut device, &[new, write_check_byte(1, new)]);
            assert_eq!(
                control.snapshot().registers[1],
                if same { new } else { old }
            );
            assert_eq!(control.snapshot().status, if same { 0 } else { STATUS_PE });
        }
    }
    let (control, mut device) = fixture(0x60);
    configure(&mut device);
    // HS from PP is forbidden despite valid parity and ENC/Q low.
    write(&mut device, &[0x40, 0xf0, 0x20, 1, 2, 3, 4, 0x5d]);
    assert_eq!(
        &control.snapshot().registers[1..8],
        &[0xe0, 0xa0, 0x20, 0, 0, 0, 0]
    );
    assert_eq!(control.snapshot().status, STATUS_PE);
    read(&mut device, 2);
    write(&mut device, &[0x80, 0x61]);
    write(&mut device, &[0x40, 0xa1]);
    assert_eq!(control.snapshot().mode, L6360Mode::HighSide);
    assert_eq!(control.snapshot().status, 0);
}

#[test]
fn unassigned_bits_do_not_change_modes_but_parity_stays_bound_to_transmitted_byte() {
    let (control, mut device) = fixture(0x60);
    write(&mut device, &[0x81, 0xc1]);
    assert_eq!(random(&mut device, 1), [0x80, 0x21, 0x21, 0, 0, 0, 0, 0]);
    write(&mut device, &[0x60, write_check_byte(3, 0x60)]);
    assert_eq!(control.snapshot().control2, 0x20);
    assert_eq!(control.snapshot().parity & 0x10, 0);
}

#[test]
fn random_pointer_requires_stop_and_applies_to_one_read_only() {
    let (_, mut device) = fixture(0x60);
    write(&mut device, &[2]);
    assert_eq!(read(&mut device, 7), [0x21, 0x21, 0, 0, 0, 0, 0]);
    assert_eq!(read(&mut device, 2), [0, 0]);
    let (control, mut device) = fixture(0x60);
    device.start();
    device.write(2);
    device.start();
    assert_eq!(device.read(), 0xff);
    assert_eq!(
        control.snapshot().protocol_error,
        Some(L6360ProtocolError::MissingStop)
    );
    assert!(!device.claims_address(0x60));
}

#[test]
fn startup_po_is_latched_by_enable_and_only_completed_current_read_clears_it() {
    let (control, mut device) = fixture(0x60);
    configure(&mut device);
    inputs(&control, 1, |i| i.enl = true);
    assert_eq!(control.snapshot().status, STATUS_PO);
    assert!(control.snapshot().irq_asserted);
    assert_eq!(random(&mut device, 0)[0], STATUS_PO);
    assert_eq!(control.snapshot().status, STATUS_PO);
    assert_eq!(read(&mut device, 1), [STATUS_PO]);
    assert_eq!(control.snapshot().status, STATUS_PO);
    device.start();
    assert_eq!(device.read(), STATUS_PO);
    assert_eq!(device.read(), 0xd0);
    assert_eq!(control.snapshot().status, STATUS_PO);
    device.stop();
    assert_eq!(control.snapshot().status, 0);
    assert!(!control.snapshot().irq_asserted);
    assert_eq!(read(&mut device, 2), [0, 0x50]);
}

#[test]
fn persistent_po_and_regulator_fault_survive_read_until_threshold_recovery() {
    let (control, mut device) = fixture(0x60);
    inputs(&control, 1, |i| {
        i.enl = true;
        i.lplus_voltage_valid = false;
        i.regulator_undervoltage = true;
    });
    assert_eq!(read(&mut device, 2), [STATUS_PO | STATUS_REGLN, 0]);
    assert_eq!(control.snapshot().status, STATUS_PO | STATUS_REGLN);
    inputs(&control, 2, |i| {
        i.lplus_voltage_valid = true;
        i.regulator_undervoltage = false;
    });
    assert_eq!(control.snapshot().status, STATUS_PO | STATUS_REGLN);
    read(&mut device, 2);
    assert_eq!(control.snapshot().status, 0);
}

#[test]
fn fault_which_occurs_after_read_begins_is_not_lost_at_stop() {
    let (control, mut device) = fixture(0x60);
    device.start();
    inputs(&control, 1, |i| i.enl = true);
    assert_eq!(device.read(), STATUS_PO);
    device.read();
    device.stop();
    assert_eq!(control.snapshot().status, STATUS_PO);
    read(&mut device, 2);
    assert_eq!(control.snapshot().status, 0);
    inputs(&control, 2, |i| i.enl = false);
    inputs(&control, 3, |i| i.enl = true);
    device.start();
    device.read();
    inputs(&control, 4, |i| i.enl = false);
    inputs(&control, 5, |i| i.enl = true);
    device.read();
    device.stop();
    assert_eq!(control.snapshot().status, STATUS_PO);
}

#[test]
fn temperature_shutdown_holds_both_outputs_until_recovered_status_is_acknowledged() {
    let (control, mut device) = fixture(0x60);
    configure(&mut device);
    inputs(&control, 1, |i| {
        i.enl = true;
        i.encq = true;
    });
    read(&mut device, 2);
    inputs(&control, 2, |i| i.overtemperature = true);
    let s = control.snapshot();
    assert!(s.cq_driver_requested && s.cq_forced_shutdown && s.lplus_forced_shutdown);
    assert!(!s.cq_driver_enabled && !s.lplus_driver_enabled);
    read(&mut device, 2);
    assert_eq!(control.snapshot().status, STATUS_OVT);
    inputs(&control, 3, |i| i.overtemperature = false);
    assert!(!control.snapshot().cq_driver_enabled);
    read(&mut device, 2);
    assert!(control.snapshot().cq_driver_enabled && control.snapshot().lplus_driver_enabled);
}

#[test]
fn cutoff_occurs_at_programmed_deadline_and_auto_restarts_without_clearing_history() {
    let (control, mut device) = fixture(0x60);
    configure(&mut device);
    inputs(&control, 0, |i| {
        i.encq = true;
        i.cq_overload = true;
    });
    inputs(&control, 99_999, |_| {});
    assert!(control.snapshot().cq_driver_enabled);
    inputs(&control, 100_000, |_| {});
    assert!(!control.snapshot().cq_driver_enabled);
    assert_eq!(control.snapshot().status, STATUS_CQOL);
    read(&mut device, 2);
    assert_eq!(control.snapshot().status, STATUS_CQOL);
    inputs(&control, 100_001, |i| i.cq_overload = false);
    inputs(&control, 25_599_999, |_| {});
    assert!(control.snapshot().cq_cutoff_active);
    inputs(&control, 25_600_000, |_| {});
    assert!(control.snapshot().cq_driver_enabled);
    assert_eq!(control.snapshot().status, STATUS_CQOL);
    read(&mut device, 2);
    assert_eq!(control.snapshot().status, 0);
}

#[test]
fn continuous_overload_long_interval_preserves_restart_phase() {
    let (control, mut device) = fixture(0x60);
    configure(&mut device);
    inputs(&control, 0, |i| {
        i.encq = true;
        i.cq_overload = true;
    });
    // First trip at 0.1 ms; repeat period 25.6 ms. One billion periods
    // must not require one billion host iterations.
    let at = 100_000 + 25_600_000 * 1_000_000_000u64;
    inputs(&control, at, |_| {});
    assert!(control.snapshot().cq_cutoff_active);
    inputs(&control, at + 25_500_000, |_| {});
    assert!(control.snapshot().cq_driver_enabled);
    inputs(&control, at + 25_600_000, |_| {});
    assert!(control.snapshot().cq_cutoff_active);
}

#[test]
fn latched_cutoff_releases_on_other_register_access_but_keeps_diagnostic_history() {
    let (control, mut device) = fixture(0x60);
    configure(&mut device);
    write(&mut device, &[0xa4, write_check_byte(2, 0xa4)]);
    inputs(&control, 0, |i| {
        i.encq = true;
        i.cq_overload = true;
    });
    inputs(&control, 100_000, |_| {});
    inputs(&control, 100_001, |i| i.cq_overload = false);
    inputs(&control, 1_000_000_000, |_| {});
    assert!(control.snapshot().cq_cutoff_active);
    random(&mut device, 4);
    assert!(!control.snapshot().cq_cutoff_active && control.snapshot().cq_driver_enabled);
    assert_eq!(control.snapshot().status, STATUS_CQOL);
    read(&mut device, 2);
    assert_eq!(control.snapshot().status, 0);
}

#[test]
fn lplus_zero_delay_latched_protection_and_disabled_cutoff_are_distinct() {
    let (control, mut device) = fixture(0x60);
    configure(&mut device);
    write(&mut device, &[0x2c, write_check_byte(3, 0x2c)]);
    inputs(&control, 0, |i| {
        i.enl = true;
        i.lplus_overload = true;
    });
    assert!(control.snapshot().lplus_cutoff_active);
    assert!(!control.snapshot().lplus_driver_enabled);
    assert_eq!(control.snapshot().status, STATUS_PO | STATUS_LOL);
    inputs(&control, 1, |i| i.lplus_overload = false);
    read(&mut device, 2);
    assert_eq!(control.snapshot().status, 0);
    assert!(control.snapshot().lplus_driver_enabled);
    write(&mut device, &[0x30, write_check_byte(3, 0x30)]);
    inputs(&control, 2, |i| i.lplus_overload = true);
    inputs(&control, 1_000_000_000, |_| {});
    assert!(control.snapshot().lplus_driver_enabled);
    assert_eq!(control.snapshot().status, 0);
}

#[test]
fn all_mode_receiver_and_driver_requests_follow_table23() {
    for (bits, mode, forced) in [
        (0, L6360Mode::Off, false),
        (1, L6360Mode::LowSide, false),
        (2, L6360Mode::HighSide, false),
        (3, L6360Mode::PushPull, false),
        (4, L6360Mode::TriState, false),
        (5, L6360Mode::LowSideOn, true),
        (6, L6360Mode::HighSideOn, true),
        (7, L6360Mode::PushPull, false),
    ] {
        let (control, mut device) = fixture(0x60);
        let config = bits << 5;
        write(&mut device, &[config, write_check_byte(1, config)]);
        assert_eq!(control.snapshot().mode, mode);
        assert_eq!(control.snapshot().cq_driver_enabled, forced);
        assert_eq!(
            control.snapshot().cq_receiver_enabled,
            mode != L6360Mode::Off
        );
        inputs(&control, 1, |i| i.encq = true);
        assert_eq!(control.snapshot().cq_driver_enabled, mode.active());
        assert!(!control.snapshot().cq_receiver_enabled);
    }
}

#[test]
fn reset_aborts_partial_write_and_invalidates_current_selection() {
    let (control, mut device) = fixture(0x60);
    configure(&mut device);
    let generation = device.transaction_generation();
    device.start();
    device.write(0x80);
    inputs(&control, 1, |i| i.reset_n = false);
    device.write(0x61);
    device.stop();
    inputs(&control, 2, |i| i.reset_n = true);
    assert_ne!(device.transaction_generation(), generation);
    assert_eq!(random(&mut device, 0), RESET_REGISTERS);
    assert_eq!(control.snapshot().protocol_error, None);
}

#[test]
fn reset_aborts_read_and_cannot_clear_new_status_after_release() {
    let (control, mut device) = fixture(0x60);
    inputs(&control, 1, |i| i.enl = true);
    device.start();
    device.read();
    inputs(&control, 2, |i| i.reset_n = false);
    inputs(&control, 3, |i| i.reset_n = true);
    assert_eq!(device.read(), 0xff);
    device.stop();
    assert_eq!(control.snapshot().status, STATUS_PO);
    read(&mut device, 2);
    assert_eq!(control.snapshot().status, 0);
}

#[test]
fn malformed_lengths_read_only_registers_and_overreads_fail_closed_without_fake_pe() {
    for bytes in [vec![9], vec![0, 0], vec![0, 8], vec![0, 9], vec![1, 2, 3]] {
        let (control, mut device) = fixture(0x60);
        write(&mut device, &bytes);
        assert!(!device.claims_address(0x60));
        assert!(control.snapshot().protocol_error.is_some());
        assert_eq!(control.snapshot().status, 0);
        assert_eq!(read(&mut device, 2), [0xff, 0xff]);
    }
    let (control, mut device) = fixture(0x60);
    assert_eq!(read(&mut device, 3), [0, 0, 0xff]);
    assert_eq!(
        control.snapshot().protocol_error,
        Some(L6360ProtocolError::ReadPastEnd)
    );
}

#[test]
fn two_physically_addressed_ports_do_not_share_state() {
    let (a, mut da) = fixture(0x60);
    let (b, mut db) = fixture(0x61);
    configure(&mut da);
    configure(&mut db);
    inputs(&a, 1, |i| i.enl = true);
    assert_eq!(a.snapshot().status, STATUS_PO);
    assert_eq!(b.snapshot().status, 0);
    write(&mut da, &[0, 0x21]);
    assert_eq!(a.snapshot().status, STATUS_PO | STATUS_PE);
    assert_eq!(read(&mut db, 2), [0, 0x50]);
    assert_eq!(b.snapshot().mode, L6360Mode::PushPull);
}

#[test]
fn monotonic_simulation_time_is_checked_without_mutating_state_on_error() {
    let (control, _) = fixture(0x60);
    inputs(&control, 10, |_| {});
    let before = control.snapshot();
    let error = control
        .update_inputs(
            9,
            L6360Inputs {
                enl: true,
                ..before.inputs
            },
        )
        .unwrap_err();
    assert_eq!(
        error,
        L6360Error::TimeWentBackwards {
            previous_ns: 10,
            requested_ns: 9
        }
    );
    assert_eq!(control.snapshot(), before);
}

#[test]
fn explicit_reset_recovers_a_fail_closed_protocol_endpoint() {
    let (control, mut device) = fixture(0x60);
    write(&mut device, &[9]);
    assert!(!device.claims_address(0x60));
    inputs(&control, 1, |i| i.reset_n = false);
    inputs(&control, 2, |i| i.reset_n = true);
    assert!(device.claims_address(0x60));
    assert_eq!(random(&mut device, 0), RESET_REGISTERS);
    assert!(control.snapshot().protocol_error.is_none());
    assert_eq!(control.snapshot().counters.protocol_errors, 1);
}

#[test]
fn absent_supply_disables_output_and_invalidates_an_inflight_read() {
    let (control, mut device) = fixture(0x61);
    configure(&mut device);
    inputs(&control, 1, |i| {
        i.encq = true;
        i.enl = true;
    });
    device.start();
    assert_eq!(device.read(), STATUS_PO);
    let generation = device.transaction_generation();
    inputs(&control, 2, |i| i.vcc_valid = false);
    let snapshot = control.snapshot();
    assert!(
        !snapshot.cq_driver_enabled
            && !snapshot.cq_receiver_enabled
            && !snapshot.lplus_driver_enabled
    );
    assert!(!device.claims_address(0x61));
    assert_ne!(device.transaction_generation(), generation);
    assert_eq!(device.read(), 0xff);
    device.stop();
    assert_eq!(control.snapshot().status, STATUS_PO);
    // An explicit physical reset establishes defaults after restoring supply;
    // the test does not assert undocumented brownout retention behavior.
    inputs(&control, 3, |i| {
        i.reset_n = false;
        i.enl = false;
    });
    inputs(&control, 4, |i| {
        i.vcc_valid = true;
        i.reset_n = true;
    });
    assert_eq!(random(&mut device, 0), RESET_REGISTERS);
}
