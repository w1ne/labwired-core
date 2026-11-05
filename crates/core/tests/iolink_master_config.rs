//! Bounded UART test-peer configuration and wire framing. These tests exercise
//! the public constructors and the same manifest attachment used by the UI.
//! They do not certify IO-Link master conformance or physical line timing.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::iolink_master::{
    IolinkComSpeed, IolinkFrameKind, IolinkLinkState, IolinkMaster,
};
use labwired_core::peripherals::uart::{Uart, UartStreamDevice};
use labwired_core::sim_input::SimInput;

fn requested(od: usize) -> IolinkMaster {
    IolinkMaster::new_configured(32, 1, 6, od, IolinkComSpeed::Com3, 8).unwrap()
}

fn bus_from_yaml(config: &str) -> anyhow::Result<SystemBus> {
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32g0b1re.yaml"
    ))?;
    let config: serde_yaml::Value = serde_yaml::from_str(config)?;
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: master-config\nchip: ignored\nexternal_devices:\n- id: master\n  type: iolink-master\n  connection: usart1\n  config: {}\n",
        serde_json::to_string(&config)?
    ))?;
    SystemBus::from_config(&chip, &manifest)
}

fn evaluation_bus(com: &str) -> SystemBus {
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32g0b1re.yaml"
    ))
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: evaluation\nchip: ignored\ncpu_hz: 64000000\nexternal_devices:\n- id: phy\n  type: tiol112\n  connection: usart1\n  config: {{en: PA8, wake: PB0, nfault: PB1}}\n- id: master\n  type: iolink-master\n  connection: usart1\n  config: {{pd_in_len: 32, pd_out_len: 1, m_seq_type: 6, od_len: 1, com: {com}, pd_out_pattern: counter, startup_profile: tiol112-eval}}\n"
    )).unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}

// Independent AnnexA parity-mask expression rather than calling the encoder.
fn oracle_cks(bytes: &[u8], flags: u8) -> u8 {
    let fold = bytes.iter().fold(0x52 ^ flags, |sum, byte| sum ^ byte);
    let parity = |mask| ((fold & mask) as u8).count_ones() as u8 & 1;
    flags
        | (parity(0xAA) << 5)
        | (parity(0x55) << 4)
        | (parity(0xC0) << 3)
        | (parity(0x30) << 2)
        | (parity(0x0C) << 1)
        | parity(0x03)
}

fn feed(master: &mut IolinkMaster, bytes: &[u8]) {
    for &byte in bytes {
        master.on_tx_byte(byte);
    }
}

fn evaluation_frame(master: &mut IolinkMaster, length: usize, now: &mut u64) -> (u64, Vec<u8>) {
    let mut frame = Vec::new();
    let mut start = None;
    for _ in 0..100_000 {
        *now += 1;
        if let Some(byte) = master.poll(1) {
            start.get_or_insert(*now);
            frame.push(byte);
            if frame.len() == length {
                return (start.unwrap(), frame);
            }
        }
        assert!(
            master.evaluation_error().is_none(),
            "{:?}",
            master.evaluation_error()
        );
    }
    panic!("evaluation did not emit the expected frame");
}

fn prepare_evaluation(master: &mut IolinkMaster, minimum: u8, now: &mut u64) -> Vec<u64> {
    assert_eq!(master.evaluation_phase(), Some("armed"));
    master.set_input("start", 1.0).unwrap();
    master.set_input("start", 0.0).unwrap();
    assert_eq!(master.evaluation_phase(), Some("wake"));
    let mut starts = Vec::new();
    for (request, value, phase) in [
        ([0xA2, 0x00], minimum, "min_cycle"),
        ([0xA3, 0x11], 0x09, "capability"),
        ([0xA5, 0x22], 0x9F, "pd_in"),
        ([0xA6, 0x12], 0x08, "pd_out"),
    ] {
        let (at, frame) = evaluation_frame(master, 2, now);
        assert_eq!(master.evaluation_phase(), Some(phase));
        starts.push(at);
        assert_eq!(frame, request);
        feed(master, &[value, oracle_cks(&[value], 0)]);
    }
    let (at, frame) = evaluation_frame(master, 3, now);
    starts.push(at);
    assert_eq!(frame, [0x20, 0x06, 0x99]);
    assert_eq!(master.evaluation_phase(), Some("operate_command"));
    feed(master, &[0x2D]);
    starts
}

fn expected_payload(selector: u8) -> Vec<u8> {
    (0..32)
        .map(|i| {
            if i == 0 {
                selector
            } else {
                let offset = u32::from(selector) * 32 + i;
                ((offset * 13) ^ (offset >> 3) ^ 0x5A) as u8
            }
        })
        .collect()
}

fn attached_master(bus: &mut SystemBus) -> &mut IolinkMaster {
    let idx = bus.find_peripheral_index_by_name("usart1").unwrap();
    bus.peripherals[idx]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Uart>()
        .unwrap()
        .attached_streams[0]
        .as_any_mut()
        .unwrap()
        .downcast_mut::<IolinkMaster>()
        .unwrap()
}

fn drain(master: &mut IolinkMaster) -> Vec<u8> {
    let mut bytes = Vec::new();
    for _ in 0..12_100 {
        match master.poll(1000) {
            Some(byte) => bytes.push(byte),
            None if !bytes.is_empty() => return bytes,
            None => {}
        }
    }
    panic!("no complete frame within the configured test budget");
}

fn enter_operate(master: &mut IolinkMaster) -> Vec<u8> {
    assert_eq!(drain(master), [0x55]);
    assert_eq!(drain(master), [0xA2, 0x00]);
    assert_eq!(drain(master), [0x20, 0x06, 0x99]);
    let first = drain(master);
    assert_eq!(master.link_state, IolinkLinkState::Operate);
    first
}

#[test]
fn manifest_preserves_requested_profile_and_legacy_defaults() {
    let mut bus = bus_from_yaml(
        "pd_in_len: 32\npd_out_len: 1\nm_seq_type: 6\nod_len: 1\ncom: COM3\nframe_gap_ticks: 8",
    )
    .unwrap();
    let master = attached_master(&mut bus);
    assert_eq!(master.evaluation_phase(), None);
    master.set_process_data_output(&[3]).unwrap();
    assert_eq!(enter_operate(master), [0x00, 0xB5, 0x03, 0x00]);
    drain(master);
    let trace = master.trace_snapshot();
    let sent = trace.last().unwrap();
    assert_eq!(sent.com, IolinkComSpeed::Com3);
    assert_eq!(sent.pd_out, [3]);
    assert_eq!(master.backend_name_for_test(), "hand-rolled");

    let mut legacy = bus_from_yaml("{}").unwrap();
    assert_eq!(
        enter_operate(attached_master(&mut legacy)),
        [0x00, 0x75, 0x00]
    );
    assert!(bus_from_yaml("pd_in_len: 9\nm_seq_type: 1").is_ok());
}

#[test]
fn manifest_rejects_bad_types_bounds_and_unsupported_modes() {
    for config in [
        "pd_in_len: -1",
        "pd_in_len: 33",
        "pd_in_len: 1.5",
        "pd_in_len: '32'",
        "pd_out_len: -1",
        "pd_out_len: 33",
        "pd_out_len: true",
        "m_seq_type: 2",
        "m_seq_type: 3",
        "m_seq_type: 256",
        "m_seq_type: -1",
        "od_len: 0",
        "od_len: 3",
        "od_len: 33",
        "com: COM4",
        "com: 3",
        "frame_gap_ticks: 0",
        "frame_gap_ticks: -1",
        "frame_gap_ticks: 4294967296",
        "frame_gap_ticks: 1.5",
        "pd_out_pattern: counter",
        "pd_out_pattern: random",
        "pd_out_pattern: 1",
    ] {
        assert!(
            bus_from_yaml(config).is_err(),
            "accepted invalid config {config}"
        );
    }
}

#[test]
fn explicit_constructor_checks_pd_od_and_m_sequence_compatibility() {
    let make =
        |pi, po, typ, od| IolinkMaster::new_configured(pi, po, typ, od, IolinkComSpeed::Com2, 1);
    for od in [1, 2, 8, 32] {
        for (pi, po) in [(32, 32), (0, 32), (32, 0)] {
            assert!(make(pi, po, 6, od).is_ok());
        }
        assert!(make(0, 0, 6, od).is_err());
    }
    for args in [
        (1, 0, 6, 2),
        (0, 1, 6, 32),
        (2, 1, 6, 1),
        (1, 2, 6, 1),
        (1, 0, 4, 1),
        (2, 0, 5, 1),
    ] {
        assert!(make(args.0, args.1, args.2, args.3).is_ok());
    }
    for args in [
        (33, 0, 6, 2),
        (0, 33, 6, 2),
        (1, 1, 6, 1),
        (1, 0, 6, 1),
        (32, 1, 4, 1),
        (1, 0, 4, 2),
        (2, 1, 5, 1),
        (1, 1, 1, 1),
        (1, 0, 1, 2),
    ] {
        assert!(make(args.0, args.1, args.2, args.3).is_err());
    }
    assert!(IolinkMaster::new_configured(32, 1, 6, 1, IolinkComSpeed::Com3, u32::MAX).is_ok());
}

#[test]
fn selector_changes_next_frame_and_trace_keeps_current_response() {
    let mut master = requested(1);
    master.set_process_data_output(&[3]).unwrap();
    assert!(master.set_process_data_output(&[]).is_err());
    assert!(master.set_process_data_output(&[1, 2]).is_err());
    assert_eq!(enter_operate(&mut master), [0x00, 0xB5, 0x03, 0x00]);
    master.set_process_data_output(&[0xA5]).unwrap();
    for byte in 0..32 {
        master.on_tx_byte(byte);
    }
    assert!(!master.pd_valid, "partial response cannot be validated");
    master.on_tx_byte(0x2D); // Independent vector: XOR(00..1F)=0.
    assert_eq!(master.process_data_input(), &(0..32).collect::<Vec<u8>>());
    assert!(master.pd_valid);
    assert_eq!(drain(&mut master), [0x00, 0x8A, 0xA5, 0x00]);
    let trace = master.trace_snapshot();
    let x = trace.last().unwrap();
    assert_eq!(x.kind, IolinkFrameKind::Cyclic);
    assert_eq!(x.pd_out, [3]);
    assert_eq!(x.pd_in, (0..32).collect::<Vec<u8>>());
    assert_eq!((x.raw_master.len(), x.raw_device.len()), (4, 33));
    assert_eq!(x.ck_ok, Some(true));
}

#[test]
fn type2_read_omits_request_od_and_extracts_pd_after_32_response_od_bytes() {
    let mut master = requested(32);
    master.set_process_data_output(&[0xA5]).unwrap();
    assert_eq!(enter_operate(&mut master).len(), 35);
    // Zero PD with event flag: ck6(32*00,80)=05, hence CKS=85.
    for byte in [0u8; 32].into_iter().chain([0x85]) {
        master.on_tx_byte(byte);
    }
    assert_eq!(drain(&mut master), [0xC0, 0xBA, 0xA5]);
    // OD 32*EE and PD 00..1F both XOR to zero: independent CKS=2D.
    for byte in [0xEE; 32].into_iter().chain(0..32).chain([0x2D]) {
        master.on_tx_byte(byte);
    }
    assert_eq!(master.process_data_input(), &(0..32).collect::<Vec<u8>>());
    drain(&mut master);
    let trace = master.trace_snapshot();
    let x = trace.last().unwrap();
    assert_eq!(x.kind, IolinkFrameKind::EventReadout);
    assert_eq!(x.raw_device.len(), 65);
    assert_eq!(x.pd_in.len(), 32);
    assert_eq!(x.pd_out, [0xA5]);
    assert_eq!(x.ck_ok, Some(true));
}

#[test]
fn wrong_checksum_or_invalid_pd_does_not_latch_payload() {
    let mut master = requested(1);
    enter_operate(&mut master);
    for byte in (0..32).chain([0x2C]) {
        master.on_tx_byte(byte);
    }
    assert!(!master.pd_valid);
    drain(&mut master);
    for byte in (0..32).chain([0x75]) {
        master.on_tx_byte(byte);
    }
    assert!(!master.pd_valid);
    assert_eq!(master.process_data_input(), &[0; 32]);
}

#[test]
fn configured_counter_wraps_and_leaves_sent_trace_unchanged() {
    let mut bus = bus_from_yaml("pd_in_len: 32\npd_out_len: 1\nm_seq_type: 6\nod_len: 1\ncom: COM3\npd_out_pattern: counter\nframe_gap_ticks: 8").unwrap();
    let master = attached_master(&mut bus);
    master.set_process_data_output(&[254]).unwrap();
    assert_eq!(enter_operate(master)[2], 254);
    assert_eq!(drain(master)[2], 255);
    assert_eq!(drain(master)[2], 0);
    let trace = master.trace_snapshot();
    assert_eq!(trace[3].pd_out, [254]);
    assert_eq!(trace[4].pd_out, [255]);
}

#[test]
fn explicit_start_profile_waits_for_action_validates_dpp_and_alternates_read_write() {
    let mut bus = evaluation_bus("COM3");
    let master = attached_master(&mut bus);
    assert!(master.paced_by_device());
    assert_eq!(master.declared_baud(), Some(230_400));
    for _ in 0..100 {
        assert_eq!(master.poll(1000), None);
    }
    let mut now = 0;
    let starts = prepare_evaluation(master, 0x1E, &mut now);
    assert!(
        starts[0] >= 1_100,
        "logical WAKE pulse and release settle precede DPP"
    );
    assert!(starts.windows(2).all(|w| w[1] - w[0] >= 435));
    let mut cyclic_starts = Vec::new();
    for selector in 0..11u8 {
        let read = selector % 2 == 1;
        let (at, request) = evaluation_frame(master, if read { 3 } else { 4 }, &mut now);
        assert_eq!(master.evaluation_phase(), Some("operate"));
        cyclic_starts.push(at);
        assert_eq!(request[0], if read { 0xF1 } else { 0x71 });
        assert_eq!(request[2], selector);
        assert_eq!(request[1] & 0xC0, 0x80);
        let payload = expected_payload(selector);
        let mut reply = if read { vec![0] } else { Vec::new() };
        reply.extend_from_slice(&payload);
        reply.push(oracle_cks(&reply, 0));
        feed(master, &reply);
        assert_eq!(master.process_data_input(), payload);
    }
    assert!(cyclic_starts.windows(2).all(|w| w[1] - w[0] >= 3_000));
    assert!(master.evaluation_error().is_none());
    assert!(master.pd_valid);
    for selector in [7, 9] {
        let trace = master.trace_snapshot();
        let exchange = trace.iter().find(|x| x.pd_out == [selector]).unwrap();
        assert_eq!(exchange.pd_in, expected_payload(selector));
        assert_eq!(exchange.raw_device.len(), 34);
    }
}

#[test]
fn com2_honors_quantized_15_point_2_ms_minimum() {
    let mut bus = evaluation_bus("COM2");
    let master = attached_master(&mut bus);
    let mut now = 0;
    let starts = prepare_evaluation(master, 0x56, &mut now);
    assert!(starts.windows(2).all(|w| w[1] - w[0] >= 2_605));
    let (first, _) = evaluation_frame(master, 4, &mut now);
    let payload = expected_payload(0);
    let mut reply = payload.clone();
    reply.push(oracle_cks(&reply, 0));
    feed(master, &reply);
    let (second, _) = evaluation_frame(master, 3, &mut now);
    assert!(second - first >= 15_200);
}

#[test]
fn evaluation_fails_closed_on_wrong_capability_bad_checksum_or_no_response() {
    for fault in ["capability", "checksum", "timeout"] {
        let mut bus = evaluation_bus("COM3");
        let master = attached_master(&mut bus);
        master.set_input("start", 1.0).unwrap();
        let mut now = 0;
        assert_eq!(evaluation_frame(master, 2, &mut now).1, [0xA2, 0]);
        if fault == "capability" {
            feed(master, &[0x1E, 0x28]);
            assert_eq!(evaluation_frame(master, 2, &mut now).1, [0xA3, 0x11]);
            feed(master, &[0x0B, oracle_cks(&[0x0B], 0)]);
        } else if fault == "checksum" {
            feed(master, &[0x1E, 0x29]);
        }
        for _ in 0..60 {
            master.poll(1000);
        }
        assert!(
            master.evaluation_error().is_some(),
            "fault {fault} must stop the profile"
        );
        assert_eq!(master.link_state, IolinkLinkState::Startup);
        assert_eq!(master.evaluation_phase(), Some("fault"));
        assert!(!master.pd_valid);
        assert_eq!(master.poll(1_000_000), None);
    }
    assert!(
        bus_from_yaml(
            "pd_in_len: 32\npd_out_len: 1\nm_seq_type: 6\nod_len: 1\nstartup_profile: tiol112-eval"
        )
        .is_err(),
        "profile must not attach without a TIOL112"
    );
    assert!(bus_from_yaml("startup_profile: automatic").is_err());
}

#[test]
#[ignore = "requires source-pinned IOLINKI_G0_ELF at 64 MHz/COM3 with the Type2V payload"]
fn real_type2v_elf_runs_the_same_one_action_uart_fixture() {
    use labwired_core::cpu::CortexM;
    use labwired_core::system::cortex_m::configure_cortex_m;
    use labwired_core::Machine;
    let elf =
        std::env::var("IOLINKI_G0_ELF").expect("IOLINKI_G0_ELF required; no missing-artifact pass");
    let mut bus = evaluation_bus("COM3");
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine: Machine<CortexM> = Machine::new(cpu, bus);
    machine
        .load_firmware(&labwired_loader::load_elf(std::path::Path::new(&elf)).unwrap())
        .unwrap();
    for _ in 0..500_000 {
        machine.step().unwrap();
    }
    machine.bus.set_input(Some("master"), "start", 1.0).unwrap();
    machine.bus.set_input(Some("master"), "start", 0.0).unwrap();
    let mut seen_read = false;
    let mut seen_write = false;
    let mut seen_7 = false;
    let mut seen_9 = false;
    for block in 0..10_000 {
        for _ in 0..1_000 {
            machine.step().unwrap();
        }
        let master = attached_master(&mut machine.bus);
        assert!(
            master.evaluation_error().is_none(),
            "error {:?}; logs {:?}",
            master.evaluation_error(),
            master.logs()
        );
        for exchange in master
            .trace_snapshot()
            .iter()
            .filter(|x| x.kind == IolinkFrameKind::Cyclic)
        {
            assert_eq!(exchange.ck_ok, Some(true));
            let selector = exchange.pd_out[0];
            assert_eq!(
                exchange.pd_in,
                expected_payload(selector),
                "same-cycle payload for selector {selector}"
            );
            let read = exchange.raw_master[0] & 0x80 != 0;
            assert_eq!(exchange.raw_device.len(), if read { 34 } else { 33 });
            seen_read |= read;
            seen_write |= !read;
            seen_7 |= selector == 7;
            seen_9 |= selector == 9;
        }
        if seen_read && seen_write && seen_7 && seen_9 {
            assert_eq!(master.evaluation_phase(), Some("operate"));
            assert!(
                master.pd_valid,
                "the UI must receive a valid process-data state"
            );
            println!("one-action fixture passed: actual ELF={elf}, selector7/9 current PD, TYPE_2_V read/write, blocks={block}, cycles={}", machine.total_cycles);
            return;
        }
    }
    panic!("real firmware did not complete selector7/9: read={seen_read}, write={seen_write}, seven={seen_7}, nine={seen_9}, trace={:?}", attached_master(&mut machine.bus).trace_snapshot());
}
