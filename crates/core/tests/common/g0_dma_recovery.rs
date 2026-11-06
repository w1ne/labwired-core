//! Explicit fault injection in the same real firmware instance. No CPU reset,
//! firmware reload, forced DLL state, or synthetic DMA completion is used.
use super::*;

fn value(snapshot: &Value, field: &str) -> u64 {
    snapshot["firmware_stats"][field].as_u64().unwrap()
}

fn silent_stimulus(
    machine: &mut Machine<CortexM>,
    master: &TimedUartPort,
    fmt: LineFormat,
    bytes: &[(u8, LineFormat)],
    message_bytes: Option<usize>,
) -> Value {
    let hz = machine.bus.cpu_hz;
    let gate = transceiver(machine);
    assert!(!gate.driver_enabled());
    let before = gate.tx_frame_count();
    let start = machine.total_cycles + hz / 50; // >=20 ms after the prior reply.
    let mut cursor = start;
    for (index, (byte, format)) in bytes.iter().enumerate() {
        if let Some(width) = message_bytes {
            if index % width == 0 {
                cursor = start + (index / width) as u64 * ps_to_cycles_ceil(100 * fmt.bit_ps, hz);
            }
        }
        master.transmit(cursor, u16::from(*byte), *format);
        cursor += ps_to_cycles_ceil(format.frame_ps(), hz);
    }
    machine.timed_uart_sync();
    let mut replies = Vec::new();
    let deadline = cursor + hz / 100;
    while machine.total_cycles < deadline {
        machine.step().unwrap();
        let (got, lost) = master.receive_due(machine.total_cycles, &fmt, false);
        assert_eq!(lost, 0);
        if let Some(got) = got {
            replies.push(got.value);
        }
    }
    json!({"request_bytes":bytes.iter().map(|(b,_)| *b).collect::<Vec<_>>(),
        "formats":bytes.iter().map(|(_,f)| f).collect::<Vec<_>>(),
        "message_bytes":message_bytes,
        "message_start_period_tbit":message_bytes.map(|_|100),
        "master_start_cycle":start,"master_end_cycle":cursor,
        "observation_us_after_stimulus":10000,"unexpected_reply":replies,
        "cq_reply_characters":gate.tx_frame_count()-before})
}

pub(super) fn circular_startup(
    machine: &mut Machine<CortexM>,
    master: &TimedUartPort,
    fmt: LineFormat,
    elf_bytes: &[u8],
) -> Value {
    let settled = machine.total_cycles + machine.bus.cpu_hz / 50;
    step_to(machine, settled);
    let before = transport_snapshot(machine, elf_bytes);
    assert!(before["backend"].as_u64().is_some_and(|b| b == 1 || b == 2));
    // The existing legacy transition is accepted silently in PREOPERATE and
    // ESTAB_COM. This is a transport diagnostic, not a conformance sequence.
    // Invalid CKT cannot serve this purpose: three errors trigger real fallback.
    let command = request(vec![0x0f, 0]);
    assert_eq!(command, [0x0f, 0x2d]);
    let bytes = command
        .iter()
        .copied()
        .cycle()
        .take(132)
        .map(|b| (b, fmt))
        .collect::<Vec<_>>();
    let stimulus = silent_stimulus(machine, master, fmt, &bytes, Some(2));
    let after = transport_snapshot(machine, elf_bytes);
    let mut record = json!({"case":"circular_wrap", "phase":"PREOPERATE_ESTAB_COM",
        "scope":"legacy-transition transport diagnostic, not IO-Link conformance",
        "before":before,"stimulus":stimulus,"after_stimulus":after,"passed":false});
    save_auxiliary("dma-recovery", &json!([record]));
    assert!(stimulus["unexpected_reply"].as_array().unwrap().is_empty());
    assert_eq!(stimulus["cq_reply_characters"].as_u64(), Some(0));
    assert_eq!(
        value(&after, "rx_half_events") - value(&before, "rx_half_events"),
        1
    );
    assert_eq!(value(&after, "rx_wraps") - value(&before, "rx_wraps"), 1);
    assert_eq!(value(&after, "rx_overruns"), value(&before, "rx_overruns"));
    assert_eq!(
        value(&after, "rx_rearms"),
        value(&before, "rx_rearms"),
        "a rearm would hide the circular wrap"
    );
    assert_eq!(
        after["dma_model"]["rx_transfers"].as_u64().unwrap()
            - before["dma_model"]["rx_transfers"].as_u64().unwrap(),
        132
    );
    assert_eq!(value(&after, "rx_bytes") - value(&before, "rx_bytes"), 132);
    assert_eq!(machine.bus.read_u32(0x4002_000c).unwrap(), 124);
    record["passed"] = json!(true);
    save_auxiliary("dma-recovery", &json!([record]));
    record
}

pub(super) fn run(
    machine: &mut Machine<CortexM>,
    master: &TimedUartPort,
    fmt: LineFormat,
    elf_bytes: &[u8],
    mut cases: Vec<Value>,
) -> (Vec<Value>, Vec<Value>) {
    let mut replies = Vec::new();
    for (index, kind) in ["parity", "framing", "dma_bus_error"]
        .into_iter()
        .enumerate()
    {
        // exchange() returns after EN falls; the driver can still be finishing
        // its normal post-send RX rearm. Let that complete before recording
        // the baseline or injecting a channel configuration fault.
        let settled = machine.total_cycles + machine.bus.cpu_hz / 50;
        step_to(machine, settled);
        let before = transport_snapshot(machine, elf_bytes);
        assert!(before["backend"].as_u64().is_some_and(|b| b == 1 || b == 2));
        let original_ccr = machine.bus.read_u32(0x4002_0008).unwrap();
        assert_ne!(original_ccr & 1, 0, "inject only while RX DMA is active");
        let original_cmar = machine.bus.read_u32(0x4002_0014).unwrap();
        let mut stimulus_bytes = Vec::new();
        match kind {
            "parity" | "framing" => {
                // Corrupt only the last character of a complete checksum-valid
                // TYPE2V READ. Silence therefore tests rejection of a damaged
                // request, not merely lack of a reply to an incomplete frame.
                let data = request(vec![0x80, 0x80, 0x00]);
                stimulus_bytes.extend(data[..2].iter().map(|b| (*b, fmt)));
                // A 9E1 all-zero item holds the line low at the 8E1 receiver's
                // stop sample. Its parity sample remains zero: a framing fault.
                let damaged = if kind == "parity" {
                    LineFormat {
                        parity: Parity::Odd,
                        ..fmt
                    }
                } else {
                    LineFormat {
                        data_bits: 9,
                        ..fmt
                    }
                };
                stimulus_bytes.push((data[2], damaged));
            }
            "dma_bus_error" => {
                // Fault a real RDR→memory write. Recovery must come from the
                // firmware's TE interrupt and normal receive rearm path.
                machine
                    .bus
                    .write_u32(0x4002_0008, original_ccr & !1)
                    .unwrap();
                machine.bus.write_u32(0x4002_0014, 0x7000_0000).unwrap();
                machine.bus.write_u32(0x4002_0008, original_ccr).unwrap();
                stimulus_bytes.push((0x55, fmt));
            }
            _ => unreachable!(),
        }
        let stimulus = silent_stimulus(machine, master, fmt, &stimulus_bytes, None);
        let after_fault = transport_snapshot(machine, elf_bytes);
        let cmar_after = machine.bus.read_u32(0x4002_0014).unwrap();
        let mut record = json!({"case":kind,"before":before,"stimulus":stimulus,
            "after_stimulus":after_fault,"original_rx_cmar":original_cmar,
            "rx_cmar_after_stimulus":cmar_after,"passed":false});
        cases.push(record.clone());
        save_auxiliary("dma-recovery", &json!(cases));
        assert!(
            stimulus["unexpected_reply"].as_array().unwrap().is_empty(),
            "faulted input generated a reply"
        );
        assert_eq!(stimulus["cq_reply_characters"].as_u64(), Some(0));
        match kind {
            "parity" | "framing" => {
                assert!(
                    value(&after_fault, "uart_errors") > value(&before, "uart_errors"),
                    "real UART error IRQ/callback must run"
                );
                assert!(
                    value(&after_fault, "rx_rearms") > value(&before, "rx_rearms"),
                    "driver must rearm itself"
                );
            }
            "dma_bus_error" => {
                assert!(value(&after_fault, "dma_errors") > value(&before, "dma_errors"));
                assert!(
                    after_fault["dma_model"]["channels"][0]["transfer_errors"]
                        .as_u64()
                        .unwrap()
                        > before["dma_model"]["channels"][0]["transfer_errors"]
                            .as_u64()
                            .unwrap()
                );
                assert!(value(&after_fault, "rx_rearms") > value(&before, "rx_rearms"));
                assert_eq!(
                    cmar_after, original_cmar,
                    "only firmware may restore the correct ring address"
                );
            }
            _ => unreachable!(),
        }
        let settle = machine.total_cycles + machine.bus.cpu_hz / 50;
        step_to(machine, settle);
        let selector = 0x91u8.wrapping_add(index as u8 * 13);
        let (reply, mut timing) = exchange(
            machine,
            master,
            fmt,
            request(vec![0x80, 0x80, selector]),
            34,
        );
        assert_current_pd32(&reply, 1, selector);
        timing["phase"] = json!("DMA_RECOVERY");
        timing["case"] = json!(kind);
        timing["pd_payload_bytes_checked"] = json!(32);
        record["valid_reply"] = timing.clone();
        record["after_valid_reply"] = transport_snapshot(machine, elf_bytes);
        record["passed"] = json!(true);
        *cases.last_mut().unwrap() = record;
        replies.push(timing);
        save_auxiliary("dma-recovery", &json!(cases));
    }
    (replies, cases)
}
