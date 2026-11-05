//! Real Cortex-M0+ ELF through STM32G0B1RE IRQ UART -> TIOL112 -> timed master.
//! Requires an external, source/compiler-pinned ELF; never skips a missing one.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::network::timed_uart::{
    cycles_to_ps, ps_to_cycles_ceil, LineFormat, Parity, TimedUartNet, TimedUartPort,
};
use labwired_core::peripherals::components::tiol112::Tiol112;
use labwired_core::peripherals::stm32g0_clock::validate_timed_uart_clock;
use labwired_core::peripherals::uart::Uart;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug)]
struct DllProbe {
    base: u64,
    fields: std::collections::BTreeMap<String, (u64, u64)>,
    proof: Value,
}
static DLL_PROBE: OnceLock<DllProbe> = OnceLock::new();

fn setup_dll_probe(elf_bytes: &[u8]) {
    let Ok(path) = std::env::var("IOLINKI_G0_DLL_LAYOUT") else {
        return;
    };
    let layout: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let sha = format!("{:x}", Sha256::digest(elf_bytes));
    assert_eq!(
        layout["elf_sha256"].as_str().unwrap(),
        sha,
        "DLL layout belongs to another ELF"
    );
    let device = labwired_loader::resolve_symbol_in_elf(elf_bytes, "protocol_device")
        .expect("DLL probe requires exported protocol_device");
    let base = u64::from(device) + layout["offsets"]["offsetof_dll"].as_u64().unwrap();
    let size = layout["offsets"]["sizeof_dll"].as_u64().unwrap();
    assert!(
        base >= 0x2000_0000 && base + size <= 0x2002_4000,
        "DLL probe must stay in exact G0B1 SRAM"
    );
    let mut fields = std::collections::BTreeMap::new();
    for (key, width) in layout["field_width_bytes"].as_object().unwrap() {
        let offset = layout["offsets"][key].as_u64().unwrap();
        let width = width.as_u64().unwrap();
        assert!([1, 2, 4].contains(&width) && offset + width <= size);
        fields.insert(key.clone(), (offset, width));
    }
    DLL_PROBE.set(DllProbe {
        base, fields,
        proof: json!({"elf_sha256":sha, "verification":layout["verification"], "offsets":layout["offsets"], "field_width_bytes":layout["field_width_bytes"]}),
    }).unwrap();
}

fn dll_snapshot(machine: &Machine<CortexM>) -> Value {
    let Some(probe) = DLL_PROBE.get() else {
        return Value::Null;
    };
    let values: serde_json::Map<String, Value> = probe
        .fields
        .iter()
        .map(|(key, (offset, width))| {
            let raw = machine.bus.read_u32(probe.base + offset).unwrap();
            let value = match width {
                1 => raw & 0xff,
                2 => raw & 0xffff,
                _ => raw,
            };
            (key.clone(), json!(value))
        })
        .collect();
    json!({"base_address":format!("0x{:08x}",probe.base), "values":values, "layout_proof":probe.proof})
}

fn save_auxiliary(suffix: &str, value: &Value) {
    if let Ok(path) = std::env::var("IOLINKI_G0_REPORT") {
        let path = format!("{path}.{suffix}.json");
        if let Some(parent) = Path::new(&path).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }
}

fn checksum(frame: &[u8]) -> u8 {
    let x = frame.iter().fold(0x52, |x, b| x ^ b);
    let b = |n: u8| (x >> n) & 1u8;
    ((b(7) ^ b(5) ^ b(3) ^ b(1)) << 5)
        | ((b(6) ^ b(4) ^ b(2) ^ b(0)) << 4)
        | ((b(7) ^ b(6)) << 3)
        | ((b(5) ^ b(4)) << 2)
        | ((b(3) ^ b(2)) << 1)
        | (b(1) ^ b(0))
}
fn request(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes[1] |= checksum(&bytes);
    bytes
}
fn step_to(machine: &mut Machine<CortexM>, cycle: u64) {
    while machine.total_cycles < cycle {
        machine.step().unwrap();
    }
}
fn transceiver(machine: &Machine<CortexM>) -> Tiol112 {
    let idx = machine.bus.find_peripheral_index_by_name("usart1").unwrap();
    machine.bus.peripherals[idx]
        .dev
        .as_any()
        .unwrap()
        .downcast_ref::<Uart>()
        .unwrap()
        .tiol112()
        .unwrap()
        .clone()
}
fn exchange(
    machine: &mut Machine<CortexM>,
    master: &TimedUartPort,
    fmt: LineFormat,
    data: Vec<u8>,
    expected: usize,
) -> (Vec<u8>, Value) {
    exchange_at(machine, master, fmt, data, expected, None)
}
fn exchange_at(
    machine: &mut Machine<CortexM>,
    master: &TimedUartPort,
    fmt: LineFormat,
    data: Vec<u8>,
    expected: usize,
    scheduled_start: Option<u64>,
) -> (Vec<u8>, Value) {
    let hz = machine.bus.cpu_hz;
    validate_timed_uart_clock(&machine.bus, "usart1").unwrap();
    let gate = transceiver(machine);
    assert!(
        !gate.driver_enabled(),
        "device must release CQ before a master M-sequence"
    );
    let before_count = gate.tx_frame_count();
    // Allow the configured typical 4 us disable scenario to settle between
    // master messages; this idle guard is not part of device response tA.
    let earliest_start = machine.total_cycles + ps_to_cycles_ceil(25_000_000, hz);
    let start = scheduled_start.unwrap_or(earliest_start);
    assert!(
        start >= earliest_start,
        "scheduled request overlaps the reply/release guard: start={start} now={} earliest={earliest_start} deficit_us={:.3}",
        machine.total_cycles,
        (earliest_start.saturating_sub(start)) as f64 * 1_000_000.0 / hz as f64
    );
    let char_cycles = ps_to_cycles_ceil(fmt.frame_ps(), hz);
    for (i, byte) in data.iter().enumerate() {
        master.transmit(start + i as u64 * char_cycles, u16::from(*byte), fmt);
    }
    machine.timed_uart_sync(); // Same external-peer handoff as World's timed rounds.
    let last_start = start + (data.len() as u64 - 1) * char_cycles;
    let stop_end_ps = cycles_to_ps(last_start, hz) + fmt.frame_ps();
    let deadline = last_start + hz / 50; //20ms includes the longest expected reply
    let mut response = Vec::new();
    let profile_pc = std::env::var_os("IOLINKI_G0_PROFILE_PC").is_some();
    let mut pre_en_pc_cycles = std::collections::BTreeMap::<String, u64>::new();
    while machine.total_cycles < deadline {
        let sample = profile_pc
            && cycles_to_ps(machine.total_cycles, hz) >= stop_end_ps
            && !gate.driver_enabled();
        let pc = machine.cpu.get_pc();
        let before_step = machine.total_cycles;
        machine.step().unwrap();
        if sample {
            *pre_en_pc_cycles.entry(format!("0x{pc:08x}")).or_default() +=
                machine.total_cycles - before_step;
        }
        let (got, overrun) = master.receive_due(machine.total_cycles, &fmt, false);
        assert_eq!(overrun, 0, "virtual master overrun");
        if let Some(got) = got {
            assert!(
                !got.framing_error && !got.parity_error,
                "reply parity/framing error"
            );
            response.push(got.value as u8);
            if response.len() == expected {
                break;
            }
        }
    }
    if response.len() != expected {
        save_auxiliary(
            "failure",
            &json!({
                "failure": "response_length", "request":data, "reply":response,
                "expected_reply_bytes":expected, "cycle":machine.total_cycles,
                "pc":format!("0x{:08x}",machine.cpu.get_pc()),
                "dll":dll_snapshot(machine), "phy_clock_error":gate.clock_error(),
                "phy_enable_not_ready":gate.enable_not_ready_count()
            }),
        );
    }
    assert_eq!(
        response.len(),
        expected,
        "request={data:02X?} response={response:02X?} UART_ISR={:08X} EN={} cycles={} enable_not_ready={} clock_error={:?}",
        machine.bus.read_u32(0x4001381c).unwrap(),
        gate.driver_enabled(),
        machine.total_cycles,
        gate.enable_not_ready_count(),
        gate.clock_error()
    );
    let mut masked = response.clone();
    *masked.last_mut().unwrap() &= 0xc0;
    assert_eq!(
        checksum(&masked),
        response.last().unwrap() & 0x3f,
        "reply CKS"
    );
    // Receive sampling is at mid-stop. Let the actual firmware wait for TC
    // and release EN before sending the next master request.
    let release_deadline = machine.total_cycles + hz / 1000;
    while gate.driver_enabled() && machine.total_cycles < release_deadline {
        machine.step().unwrap();
    }
    assert!(!gate.driver_enabled(), "firmware left EN asserted after TC");
    let frames = gate.tx_frames();
    assert_eq!(
        gate.tx_frame_count() - before_count,
        expected as u64,
        "every delivered byte needs a CQ waveform"
    );
    let before = frames.len() - expected;
    assert!(
        frames[before..].iter().all(|f| f.truncated_ps.is_none()),
        "EN truncated a response character"
    );
    let first = &frames[before];
    let last = frames.last().unwrap();
    let ta_ps = i128::from(first.start_ps) - i128::from(stop_end_ps);
    let gaps: Vec<i128> = frames[before..]
        .windows(2)
        .map(|pair| i128::from(pair[1].start_ps) - i128::from(pair[0].stop_end_ps))
        .collect();
    assert!(
        gaps.iter().all(|gap| *gap >= 0),
        "reply characters overlap at CQ"
    );
    let record = json!({
        "request": data, "reply": response,
        "master_request_start_cycle": start,
        "master_request_start_ps": cycles_to_ps(start, hz),
        "last_master_stop_end_ps": stop_end_ps,
        "first_device_cq_start_ps": first.start_ps,
        "last_device_cq_stop_end_ps": last.stop_end_ps,
        "en_released_by_cycle": machine.total_cycles,
        "request_to_en_release_us": (machine.total_cycles - start) as f64 * 1_000_000.0 / hz as f64,
        "ta_us": ta_ps as f64 / 1_000_000.0,
        "reply_complete_us": (i128::from(last.stop_end_ps) - i128::from(stop_end_ps)) as f64 / 1_000_000.0,
        "cpu_hz": hz, "driver_delay_ns": gate.driver_delay_ps / 1000,
        "receiver_delay_ns": gate.receiver_delay_ps / 1000,
        "enable_delay_ns_scenario": gate.enable_delay_ps / 1000,
        "disable_delay_ns_scenario": gate.disable_delay_ps / 1000,
        "en_rise_ps": first.en_rise_ps,
        "en_to_first_uart_start_us": (first.start_ps - gate.driver_delay_ps - first.en_rise_ps) as f64 /1_000_000.0,
        "byte_gap_min_ns": gaps.iter().min().map(|v| *v as f64 /1000.0),
        "byte_gap_max_ns": gaps.iter().max().map(|v| *v as f64 /1000.0),
        "byte_gap_max_tbit": gaps.iter().max().map(|v| *v as f64 /fmt.bit_ps as f64)
        ,"pre_en_pc_cycles": if profile_pc { json!(pre_en_pc_cycles) } else { Value::Null }
        ,"firmware_dll_snapshot": dll_snapshot(machine)
    });
    // Preserve completed exchanges even if a later payload/cadence assertion
    // aborts before the final summary. JSONL is diagnostic evidence, not a pass.
    if let Ok(path) = std::env::var("IOLINKI_G0_REPORT") {
        use std::io::Write;
        let path = format!("{path}.transactions.jsonl");
        if let Some(parent) = Path::new(&path).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "{record}").unwrap();
    }
    (response, record)
}

fn invalid_ckt_is_silent(
    machine: &mut Machine<CortexM>,
    master: &TimedUartPort,
    fmt: LineFormat,
) -> Value {
    let hz = machine.bus.cpu_hz;
    let gate = transceiver(machine);
    let before = gate.tx_frame_count();
    let mut data = request(vec![0x80, 0x80, 0x6b]);
    data[1] ^= 1; // Flip a checksum bit, preserving TYPE_2 and MC fields.
    let start = machine.total_cycles + ps_to_cycles_ceil(25_000_000, hz);
    let char_cycles = ps_to_cycles_ceil(fmt.frame_ps(), hz);
    for (i, byte) in data.iter().enumerate() {
        master.transmit(start + i as u64 * char_cycles, u16::from(*byte), fmt);
    }
    machine.timed_uart_sync();
    let deadline = start + data.len() as u64 * char_cycles + hz / 200;
    let mut unexpected = Vec::new();
    while machine.total_cycles < deadline {
        machine.step().unwrap();
        let (got, overrun) = master.receive_due(machine.total_cycles, &fmt, false);
        assert_eq!(overrun, 0);
        if let Some(got) = got {
            unexpected.push(got.value);
        }
    }
    assert!(
        unexpected.is_empty(),
        "invalid CKT received a reply: {unexpected:02X?}"
    );
    assert_eq!(
        gate.tx_frame_count(),
        before,
        "invalid CKT must not enable a CQ reply"
    );
    json!({
        "case": "invalid_ckt_no_response",
        "phase": "OPERATE",
        "request": data,
        "master_request_start_cycle": start,
        "master_request_start_ps": cycles_to_ps(start, hz),
        "observation_us_after_request": 5000,
        "reply": [],
        "cq_reply_characters": gate.tx_frame_count() - before,
        "passed": true
    })
}

#[test]
#[ignore = "requires IOLINKI_G0_ELF; optional HZ/BAUD/PROFILE select the built firmware"]
fn real_g0_firmware_timed_cq_session() {
    if let Ok(path) = std::env::var("IOLINKI_G0_REPORT") {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{path}.transactions.jsonl"));
        let _ = std::fs::remove_file(format!("{path}.failure.json"));
        let _ = std::fs::remove_file(format!("{path}.startup-state.json"));
    }
    let elf = std::env::var("IOLINKI_G0_ELF")
        .expect("IOLINKI_G0_ELF is required; no missing-artifact pass");
    let elf_bytes = std::fs::read(&elf).unwrap();
    setup_dll_probe(&elf_bytes);
    let hz: u64 = std::env::var("IOLINKI_G0_HZ")
        .unwrap_or("16000000".into())
        .parse()
        .unwrap();
    let baud: u64 = std::env::var("IOLINKI_G0_BAUD")
        .unwrap_or("38400".into())
        .parse()
        .unwrap();
    let full_profile = std::env::var("IOLINKI_G0_PROFILE").is_ok_and(|s| s == "type2v");
    if full_profile {
        assert_eq!(hz, 64_000_000);
        assert!(
            [230_400, 38_400].contains(&baud),
            "customer profile supports COM3/COM2"
        );
    }
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32g0b1re.yaml"
    ))
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(&format!("name: g0-customer-timing\nchip: ignored\ncpu_hz: {hz}\nexternal_devices:\n- id: phy\n  type: tiol112\n  connection: usart1\n  config: {{en: PA8, wake: PB0, nfault: PB1}}\n")).unwrap();
    labwired_core::fidelity::reset();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    bus.config.peripheral_tick_interval = 1;
    let net = TimedUartNet::new();
    let (_, device, master) = net.add_link(
        ("stm32g0b1re", "usart1", hz),
        ("virtual-master", "cq", hz),
        0,
        0,
        1,
    );
    bus.attach_timed_uart_by_id("usart1", device).unwrap();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&labwired_loader::load_elf(Path::new(&elf)).unwrap())
        .unwrap();
    for _ in 0..500_000 {
        machine.step().unwrap();
    }
    let declared_enforcement = std::env::var("IOLINKI_G0_DLL_TIMING_ENFORCED")
        .ok()
        .map(|value| match value.as_str() {
            "0" => false,
            "1" => true,
            _ => panic!("IOLINKI_G0_DLL_TIMING_ENFORCED must be 0 or 1"),
        });
    let observed_enforcement =
        labwired_loader::resolve_symbol_in_elf(&elf_bytes, "rig_timing_enforced")
            .map(|addr| machine.bus.read_u32(u64::from(addr)).unwrap() & 0xff != 0);
    if let (Some(declared), Some(observed)) = (declared_enforcement, observed_enforcement) {
        assert_eq!(
            declared, observed,
            "firmware exported timing state differs from build declaration"
        );
    }
    let firmware_timing = json!({
        "enabled": observed_enforcement.or(declared_enforcement),
        "evidence": if observed_enforcement.is_some() { "ELF-resolved rig_timing_enforced RAM value" }
            else if declared_enforcement.is_some() { "source-pinned build declaration; legacy ELF has no exported timing flag" }
            else { "not inspected" },
        "external_cq_ta_and_gap_assertions": "always enabled, independent of DLL enforcement"
    });
    let initial_dll = dll_snapshot(&machine);
    if let Some(value) = initial_dll["values"]["dll.enforce_timing"].as_u64() {
        if let Some(expected) = observed_enforcement.or(declared_enforcement) {
            assert_eq!(
                value != 0,
                expected,
                "actual DLL state differs from exported/declaration state"
            );
        }
    }
    save_auxiliary(
        "startup-state",
        &json!({"firmware_dll_timing_enforcement":firmware_timing, "dll":initial_dll}),
    );
    validate_timed_uart_clock(&machine.bus, "usart1").unwrap();
    assert_eq!(
        machine.bus.read_u32(0x40000028).unwrap() as u64 + 1,
        hz / 1_000_000
    );
    machine.bus.set_input(Some("phy"), "wake", 1.0).unwrap();
    let wake_end = machine.total_cycles + hz / 1000;
    step_to(&mut machine, wake_end);
    machine.bus.set_input(Some("phy"), "wake", 0.0).unwrap();
    for _ in 0..10_000 {
        machine.step().unwrap();
    }
    assert_eq!(
        machine.bus.read_u32(0x4001380c).unwrap() as u64,
        (hz + baud / 2) / baud,
        "firmware configured the selected COM baud"
    );
    assert_eq!(
        machine.bus.read_u32(0x40021810).unwrap() & 1,
        0,
        "actual EXTI handler cleared WAKE"
    );
    let fmt = LineFormat {
        bit_ps: 1_000_000_000_000 / baud,
        data_bits: 8,
        parity: Parity::Even,
        stop_half_bits: 2,
        rx_enabled: true,
        tx_enabled: true,
    };
    master.publish_format(fmt);
    let mut records = Vec::new();
    let (startup_reply, mut startup) =
        exchange(&mut machine, &master, fmt, request(vec![0xa2, 0]), 2);
    startup["phase"] = json!("STARTUP");
    // Tables A.7/A.8: TYPE_0 request starts require 100 Tbit recovery.
    let recovery = ps_to_cycles_ceil(100 * fmt.bit_ps, hz);
    let capability_start = startup["master_request_start_cycle"].as_u64().unwrap() + recovery;
    records.push(startup);
    let (capability, mut capability_timing) = exchange_at(
        &mut machine,
        &master,
        fmt,
        request(vec![0xa3, 0]),
        2,
        Some(capability_start),
    );
    capability_timing["phase"] = json!("PREOPERATE");
    capability_timing["actual_request_interval_us"] =
        json!(recovery as f64 * 1_000_000.0 / hz as f64);
    records.push(capability_timing);
    if full_profile {
        assert_eq!(
            (capability[0] >> 1) & 7,
            4,
            "DPP TYPE2V/OD1 capabilitycode4"
        );
    }
    let (_, mut operate) = exchange_at(
        &mut machine,
        &master,
        fmt,
        request(vec![0x20, 0, 0x99]),
        1,
        Some(capability_start + recovery),
    );
    operate["phase"] = json!("PREOPERATE_DEVICE_OPERATE");
    operate["actual_request_interval_us"] = json!(recovery as f64 * 1_000_000.0 / hz as f64);
    records.push(operate);
    let mut cyclic_ta = Vec::new();
    let mut negative_controls = Vec::new();
    let mut read_count = 0;
    let mut write_count = 0;
    let mut application_intervals_us = Vec::new();
    if full_profile {
        let count: u32 = std::env::var("IOLINKI_G0_CYCLES")
            .unwrap_or("100".into())
            .parse()
            .unwrap();
        assert!(count > 0);
        let min_cycle_us = match startup_reply[0] >> 6 {
            0 => u64::from(startup_reply[0] & 63) * 100,
            1 => 6400 + u64::from(startup_reply[0] & 63) * 400,
            2 => 32000 + u64::from(startup_reply[0] & 63) * 1600,
            _ => panic!("reserved MinCycleTime code"),
        };
        // Both customer READ and WRITE exchanges have 37 UART characters in
        // total. Even zero byte gaps and minimum tA must fit the advertised
        // cycle; do not silently repair an inconsistent device advertisement.
        let serial_minimum_us = (37u64 * 11 * 1_000_000 + 1_000_000).div_ceil(baud);
        assert!(
            min_cycle_us >= serial_minimum_us,
            "advertised MinCycleTime 0x{:02X} = {} us cannot contain PD32/PDout1/OD1: at least {} us serial time",
            startup_reply[0], min_cycle_us, serial_minimum_us
        );
        let negative_ready = records[2]["master_request_start_cycle"].as_u64().unwrap()
            + (hz * (min_cycle_us + 100)).div_ceil(1_000_000);
        step_to(&mut machine, negative_ready);
        negative_controls.push(invalid_ckt_is_silent(&mut machine, &master, fmt));
        let cadence = (hz * (min_cycle_us + 100)).div_ceil(1_000_000);
        let mut next = machine.total_cycles + cadence;
        let mut prior_start = None;
        for n in 0..count * 2 {
            let selector = ((n * 73 + 7) & 255) as u8;
            let write = n >= count;
            let bytes = if write {
                vec![0x00, 0x80, selector, 0]
            } else {
                vec![0x80, 0x80, selector]
            };
            let (reply, mut timing) = exchange_at(
                &mut machine,
                &master,
                fmt,
                request(bytes),
                if write { 33 } else { 34 },
                Some(next),
            );
            next += cadence;
            let start = timing["master_request_start_cycle"].as_u64().unwrap();
            timing["actual_request_interval_us"] = prior_start
                .map(|prior| json!((start - prior) as f64 * 1_000_000.0 / hz as f64))
                .unwrap_or(Value::Null);
            prior_start = Some(start);
            timing["phase"] = json!("OPERATE");
            timing["direction"] = json!(if write { "WRITE" } else { "READ" });
            timing["case"] = json!("cyclic_benchmark");
            if write {
                write_count += 1;
            } else {
                read_count += 1;
            }
            let pd_offset = usize::from(!write);
            assert_eq!(
                reply[pd_offset], selector,
                "current request selector must choose this reply"
            );
            for i in 1..32usize {
                let offset = usize::from(selector) * 32 + i;
                assert_eq!(
                    reply[i + pd_offset],
                    ((offset * 13) ^ (offset >> 3) ^ 0x5a) as u8,
                    "selector={selector} PDbyte={i}"
                );
            }
            assert_eq!(reply.last().unwrap() & 0x40, 0, "PD valid");
            let ta = timing["ta_us"].as_f64().unwrap();
            cyclic_ta.push(ta);
            records.push(timing);
            if n % 8 == 0 {
                eprintln!(
                    "native CQ cycles={}/{} selector={} tA={:.3}us",
                    n + 1,
                    count * 2,
                    selector,
                    ta
                );
            }
        }
        // Separate 34 blocks/s cadence case; fast service cycles and application
        // production cadence are different requirements.
        let app_period = (hz * 29_412).div_ceil(1_000_000);
        let mut previous_start = prior_start.unwrap();
        for n in 0..8u8 {
            let target = previous_start + app_period;
            let selector = n.wrapping_mul(31).wrapping_add(3);
            let (reply, mut timing) = exchange_at(
                &mut machine,
                &master,
                fmt,
                request(vec![0x80, 0x80, selector]),
                34,
                Some(target),
            );
            assert_eq!(reply[1], selector);
            let actual_start = timing["master_request_start_cycle"].as_u64().unwrap();
            let interval = (actual_start - previous_start) as f64 * 1_000_000.0 / hz as f64;
            assert!((interval - 29_412.0).abs() <= 1_000_000.0 / hz as f64);
            application_intervals_us.push(interval);
            timing["actual_request_interval_us"] = json!(interval);
            timing["phase"] = json!("OPERATE");
            timing["direction"] = json!("READ");
            timing["cadence_case"] = json!("34_blocks_per_second");
            previous_start = actual_start;
            cyclic_ta.push(timing["ta_us"].as_f64().unwrap());
            records.push(timing);
        }
    }
    let fidelity = labwired_core::fidelity::report();
    assert!(
        fidelity.is_empty(),
        "unmodelled firmware access: {fidelity}"
    );
    let report = json!({
        "profile": if full_profile { "type2v_pd32" } else { "reference_startup" },
        "firmware": elf, "cpu_hz": hz, "baud": baud,
        "firmware_dll_timing_enforcement": firmware_timing,
        "firmware_dll_initial": initial_dll,
        "firmware_dll_final": dll_snapshot(&machine),
        "transactions": records, "cyclic_count": cyclic_ta.len(),
        "benchmark_read_count": read_count, "benchmark_write_count": write_count,
        "negative_controls": negative_controls,
        "startup_ta_us": records[0]["ta_us"],
        "preoperate_ta_us": [records[1]["ta_us"], records[2]["ta_us"]],
        "cyclic_ta_min_us": cyclic_ta.iter().copied().reduce(f64::min),
        "cyclic_ta_max_us": cyclic_ta.iter().copied().reduce(f64::max),
        "application_cadence_period_us": if full_profile { Some(29_412) } else { None },
        "actual_application_request_intervals_us": application_intervals_us,
        "reply_byte_gap_max_ns": records.iter().filter_map(|r| r["byte_gap_max_ns"].as_f64()).reduce(f64::max),
        "reply_byte_gap_max_tbit": records.iter().filter_map(|r| r["byte_gap_max_tbit"].as_f64()).reduce(f64::max),
        "limits_checked": {"ta_min_tbit": 1, "ta_max_tbit": 10, "device_byte_gap_max_tbit": 3},
        "timing_reference": "IO-Link Interface and System Specification v1.1.4, June2024, A.3.4/A.3.5",
        "fidelity_gaps": fidelity.to_gaps(),
        "network": net.report(0, cycles_to_ps(machine.total_cycles, hz)),
        "limits": ["Cortex-M instruction-cycle model is not calibrated silicon WCET", "64MHz uses a fixed post-PLL operating timebase; clock transitions not timed", "TI propagation uses configured datasheet maxima; enable/disable4us are configurable typical scenarios, not maximum guarantees", "WAKE is a GPIO stimulus, not physical current-pulse detection", "No IAR/Beckhoff/electrical conformance claim"]
    });
    if let Ok(path) = std::env::var("IOLINKI_G0_REPORT") {
        if let Some(parent) = Path::new(&path).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    eprintln!(
        "{}",
        json!({"profile": report["profile"], "cyclic_count": report["cyclic_count"], "ta_min_us": report["cyclic_ta_min_us"], "ta_max_us": report["cyclic_ta_max_us"]})
    );
    // Keep the result artifact when a genuine measured timing violation occurs.
    for record in report["transactions"].as_array().unwrap() {
        let ta = record["ta_us"].as_f64().unwrap();
        assert!(
            (1_000_000.0 / baud as f64..=10_000_000.0 / baud as f64).contains(&ta),
            "phase={} measured CQ tA={ta:.3}us violates 1..10 Tbit",
            record["phase"]
        );
        if let Some(gap) = record["byte_gap_max_tbit"].as_f64() {
            assert!(
                gap <= 3.0,
                "device byte gap {gap:.6} Tbit exceeds A.3.4 limit"
            );
        }
    }
    if report["firmware_dll_timing_enforcement"]["enabled"] == json!(true) {
        if let Some(errors) = report["firmware_dll_final"]["values"]["dll.timing_errors"].as_u64() {
            assert_eq!(errors, 0, "firmware DLL timing monitor reported violations; inspect the preserved counter snapshot");
        }
    }
}
