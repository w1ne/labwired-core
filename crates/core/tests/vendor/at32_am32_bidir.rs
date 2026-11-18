// SPDX-License-Identifier: MIT
//! Exact native flash image: inverted DShot input and physical-pad turnaround.
//! No RAM injection, forced arming, motor plant or flight qualification.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, cpu::CortexM, system::cortex_m::configure_cortex_m, Bus, Cpu, Machine,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const APP_SHA: &str = "2dc65fec5924f31548cc3041690b9b5a1645466832a09e46d012c33b14f8452d";
const COMPLETE_SHA: &str = "c6ae7b29e1fe35dd56070435aa271b18ec9cba77ec5329f092b3296776664e2a";
fn scalar(machine: &Machine<CortexM>, elf: &[u8], name: &str) -> u32 {
    let address = u64::from(labwired_loader::resolve_symbol_in_elf(elf, name).expect(name));
    match name {
        "armed"
        | "sensor_configuration_error"
        | "inputSet"
        | "dshot"
        | "servoPwm"
        | "dshot_telemetry"
        | "high_pin_count"
        | "ic_timer_prescaler"
        | "output_timer_prescaler"
        | "out_put"
        | "EDT_ARMED" => u32::from(machine.bus.read_u8(address).unwrap()),
        "dshot_goodcounts"
        | "dshot_badcounts"
        | "dshot_frametime"
        | "newinput"
        | "adjusted_input"
        | "smallestnumber"
        | "zero_input_count"
        | "armed_timeout_count"
        | "dshot_frametime_high"
        | "dshot_frametime_low" => u32::from(machine.bus.read_u16(address).unwrap()),
        _ => machine.bus.read_u32(address).unwrap(),
    }
}
fn state(machine: &Machine<CortexM>, elf: &[u8]) -> Value {
    let mut values = serde_json::Map::new();
    for name in [
        "system_core_clock",
        "armed",
        "sensor_configuration_error",
        "inputSet",
        "dshot",
        "servoPwm",
        "dshot_telemetry",
        "high_pin_count",
        "ic_timer_prescaler",
        "output_timer_prescaler",
        "out_put",
        "EDT_ARMED",
        "dshot_goodcounts",
        "dshot_badcounts",
        "dshot_frametime",
        "newinput",
        "adjusted_input",
        "smallestnumber",
        "average_signal_pulse",
        "zero_input_count",
        "armed_timeout_count",
        "dshot_frametime_high",
        "dshot_frametime_low",
    ] {
        values.insert(name.into(), json!(scalar(machine, elf, name)));
    }
    values.insert("cycles".into(), json!(machine.total_cycles));
    values.insert("pc".into(), json!(machine.cpu.get_pc()));
    Value::Object(values)
}
fn advance(machine: &mut Machine<CortexM>, cycles: u64) {
    let target = machine.total_cycles + cycles;
    while machine.total_cycles < target {
        machine.step().unwrap();
    }
}
fn frame(machine: &mut Machine<CortexM>, gpio: usize, value: u16, corrupt_crc: bool) {
    assert!(value < 2048);
    let payload = value << 1;
    let checksum = (!(payload ^ (payload >> 4) ^ (payload >> 8))) & 15;
    let bits = (payload << 4) | (checksum ^ u16::from(corrupt_crc));
    for bit in (0..16).rev() {
        let active = if bits & (1 << bit) == 0 { 75 } else { 150 };
        assert!(machine.bus.set_peripheral_gpio_input(gpio, 4, false));
        advance(machine, active);
        assert!(machine.bus.set_peripheral_gpio_input(gpio, 4, true));
        advance(machine, 200 - active);
    }
    // End the FC stimulus drive. The firmware's own pull-up and timer output
    // then own the pad; do not hold a competing artificial high during reply.
    assert!(machine
        .bus
        .set_pad_external(gpio, 4, labwired_core::pins::External::Released)
        .is_some());
    advance(machine, 120_000 - 16 * 200);
}

// Wire-level oracle, independent of the AM32 encoder and its RAM contents.
// The 5b/4b codewords and inverted-nibble checksum are also specified by
// Betaflight src/main/drivers/dshot_bitbang_decode.c at 31066c0f3789d2db.
fn decode_gcr_word(encoded: u32) -> Result<u16, &'static str> {
    const CODEWORDS: [u32; 16] = [
        25, 27, 18, 19, 29, 21, 22, 23, 26, 9, 10, 11, 30, 13, 14, 15,
    ];
    let mut word = 0u16;
    for shift in 0..4 {
        let code = (encoded >> (shift * 5)) & 31;
        let nibble = CODEWORDS
            .iter()
            .position(|&candidate| candidate == code)
            .ok_or("invalid GCR symbol")?;
        word |= (nibble as u16) << (shift * 4);
    }
    if (word ^ (word >> 4) ^ (word >> 8) ^ (word >> 12)) & 15 != 15 {
        return Err("telemetry checksum mismatch");
    }
    Ok(word >> 4)
}

fn decode_reply_edges(edges: &[(u64, bool)]) -> Result<(u16, u64), &'static str> {
    let first_low = edges
        .iter()
        .position(|(_, high)| !high)
        .ok_or("no reply start")?;
    let edges = &edges[first_low..];
    let widths: Vec<_> = edges.windows(2).map(|pair| pair[1].0 - pair[0].0).collect();
    let bit_cycles = *widths.iter().min().ok_or("missing reply edges")?;
    // DShot600 reply is nominally 750 kbit/s. This 120 MHz image uses
    // PSC=1/ARR=76 (154 cycles/bit); allow bounded wire-clock quantization.
    if !(150..=170).contains(&bit_cycles) {
        return Err("reply bit rate outside bounded range");
    }
    let mut bits = 0u32;
    let mut encoded = 0u32;
    for (pair, width) in edges.windows(2).zip(widths) {
        if pair[0].1 == pair[1].1 {
            return Err("edge did not change level");
        }
        let count = (width + bit_cycles / 2) / bit_cycles;
        if count == 0 || count > 3 || width.abs_diff(count * bit_cycles) > 2 {
            return Err("invalid GCR run length");
        }
        bits += count as u32;
        if bits > 21 {
            return Err("reply is too long");
        }
        encoded = (encoded << count) | (1 << (count - 1));
    }
    if !(18..=21).contains(&bits) {
        return Err("reply is too short");
    }
    let remaining = 21 - bits;
    if remaining > 0 {
        encoded = (encoded << remaining) | (1 << (remaining - 1));
    }
    Ok((decode_gcr_word(encoded & 0xfffff)?, bit_cycles))
}

#[test]
fn wire_reply_oracle_checks_symbols_checksum_and_timing() {
    // Stopped period 0xFFF, inverted checksum 0: codewords F/F/F/0.
    assert_eq!(decode_gcr_word(0x7bdf9), Ok(0x0fff));
    assert_eq!(decode_gcr_word(0x7bdfb), Err("telemetry checksum mismatch"));
    assert_eq!(decode_gcr_word(0), Err("invalid GCR symbol"));
    let widths = [
        308, 154, 154, 154, 308, 154, 154, 154, 308, 154, 154, 154, 154, 154, 462,
    ];
    let mut edges = vec![(0, false)];
    for width in widths {
        let &(cycle, high) = edges.last().unwrap();
        edges.push((cycle + width, !high));
    }
    assert_eq!(decode_reply_edges(&edges), Ok((0x0fff, 154)));
    let mut bad_timing = edges.clone();
    bad_timing[4].0 += 30;
    assert!(decode_reply_edges(&bad_timing).is_err());
    let mut missing_edge = edges.clone();
    missing_edge.remove(3);
    assert!(decode_reply_edges(&missing_edge).is_err());
}

#[test]
#[ignore = "requires immutable native flash package and explicit evidence output"]
fn native_complete_image_turns_around_bidirectional_dshot600() {
    use labwired_core::logic_capture::{LogicSource, PadState};
    let artifacts = PathBuf::from(std::env::var("AT32_NATIVE_ARTIFACTS").unwrap());
    let report = PathBuf::from(std::env::var("AT32_NATIVE_BIDIR_REPORT").unwrap());
    let app = std::fs::read(artifacts.join("AM32_OPENESC_30_F421_2.21.elf")).unwrap();
    let complete = artifacts.join("OPENESC_30_F421_complete.elf");
    assert_eq!(format!("{:x}", Sha256::digest(&app)), APP_SHA);
    assert_eq!(
        format!("{:x}", Sha256::digest(std::fs::read(&complete).unwrap())),
        COMPLETE_SHA
    );
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/at32f421g8u7.yaml"
    ))
    .unwrap();
    let system: SystemManifest = serde_yaml::from_str("name: openesc30-native\nchip: at32f421g8u7\ncpu_hz: 120000000\nmax_peripheral_tick_interval: 1\nexternal_devices: []\n").unwrap();
    let mut bus = SystemBus::from_config(&chip, &system).unwrap();
    bus.config.peripheral_tick_interval = 1;
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&labwired_loader::load_elf(&complete).unwrap())
        .unwrap();
    let gpio = machine.bus.find_peripheral_index_by_name("gpiob").unwrap();
    // The FC is not driving the signal during reset. The real bootloader
    // must observe its own pull-down, then enter the application normally.
    assert!(machine
        .bus
        .set_pad_external(gpio, 4, labwired_core::pins::External::Released)
        .is_some());
    advance(&mut machine, 90_000_000);
    let mut stages = vec![json!({"stage":"native-boot","state":state(&machine,&app)})];
    assert_eq!(
        scalar(&machine, &app, "system_core_clock"),
        120_000_000,
        "native bootloader must enter the application without a driven-high programming signal"
    );
    assert!(machine.bus.set_peripheral_gpio_input(gpio, 4, true));
    advance(&mut machine, 120_000);
    let mut cursor = 0;
    let mut traces = Vec::new();
    let mut initial_states = Vec::new();
    for index in 0..115 {
        if index == 96 {
            machine.logic_watch(&[
                Some(LogicSource::driver(gpio, 4)),
                Some(LogicSource::pad(gpio, 4)),
            ]);
            initial_states = machine.logic_initial_states().to_vec();
        }
        let start = machine.total_cycles;
        frame(&mut machine, gpio, 0, false);
        if [0, 1, 96, 100, 101, 102, 104, 114].contains(&index) {
            stages.push(
                json!({"stage":"inverted-zero-frame","index":index,"state":state(&machine,&app)}),
            );
        }
        if index >= 96 {
            let batch = machine.logic_read_states(cursor);
            cursor = batch.cursor;
            traces.push(json!({"index":index,"startCycle":start,"endCycle":machine.total_cycles,"dropped":batch.dropped,"edges":batch.edges}));
        }
    }
    let before_bad = state(&machine, &app);
    frame(&mut machine, gpio, 100, true);
    let after_bad = state(&machine, &app);
    let bad_trace = machine.logic_read_states(cursor);
    cursor = bad_trace.cursor;
    frame(&mut machine, gpio, 100, false);
    let after_valid = state(&machine, &app);
    let valid_trace = machine.logic_read_states(cursor);
    let driven_edges = valid_trace
        .edges
        .iter()
        .filter(|e| e.ch == 0 && matches!(e.state, PadState::High | PadState::Low))
        .count();
    let released = valid_trace
        .edges
        .iter()
        .any(|e| e.ch == 0 && matches!(e.state, PadState::HighZ | PadState::WeakHigh));
    let contention = valid_trace
        .edges
        .iter()
        .any(|e| e.state == PadState::Contention);
    let reply_edges: Vec<_> = valid_trace
        .edges
        .iter()
        .filter_map(|edge| {
            if edge.ch != 0 {
                return None;
            }
            match edge.state {
                PadState::High => Some((edge.cycle, true)),
                PadState::Low => Some((edge.cycle, false)),
                _ => None,
            }
        })
        .collect();
    let decoded_reply = decode_reply_edges(&reply_edges);
    let regs: Vec<_> = [0x40000400u64,0x40000404,0x4000040c,0x40000418,0x40000420,0x40000428,0x4000042c,0x40000434,0x40020044,0x40020048].into_iter().map(|address|json!({"address":format!("0x{address:08x}"),"value":machine.bus.read_u32(address).unwrap()})).collect();
    let receipt = json!({"scope":"Exact native complete image. Inverted DShot600 input, stored checksum counters and actual PB4 output/turnaround trace. Independent GCR/checksum decode verifies stopped telemetry. FC decode and five-node acceptance are separate checks; no forced arming or physical test.","applicationElfSha256":APP_SHA,"completeElfSha256":COMPLETE_SHA,"stimulus":{"bitCycles":200,"zeroActiveCycles":75,"oneActiveCycles":150,"frameCycles":120000,"inverted":true,"releaseDuringBoot":true,"idleHighAfterBootCycles":120000,"releaseAfterPacket":true},"channels":["PB4 chip driver","PB4 resolved pad"],"initialStates":initial_states,"stages":stages,"traces":traces,"beforeCorruptFrame":before_bad,"afterCorruptFrame":after_bad,"afterValidFrame":after_valid,"badFrameTrace":{"cursor":bad_trace.cursor,"dropped":bad_trace.dropped,"edges":bad_trace.edges},"validFrameTrace":{"cursor":valid_trace.cursor,"dropped":valid_trace.dropped,"edges":valid_trace.edges},"output":{"drivenEdges":driven_edges,"released":released,"contention":contention,"decodedReply":decoded_reply},"registers":regs});
    if let Some(parent) = report.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&report, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    eprintln!("native bidirectional receipt: {}", report.display());
    assert_eq!(before_bad["dshot_telemetry"], 1);
    assert!(before_bad["dshot_goodcounts"].as_u64().unwrap() >= 5);
    assert_eq!(
        after_bad["dshot_badcounts"].as_u64().unwrap(),
        before_bad["dshot_badcounts"].as_u64().unwrap() + 1
    );
    assert_eq!(after_bad["newinput"], 0);
    assert_eq!(
        after_valid["dshot_goodcounts"].as_u64().unwrap(),
        after_bad["dshot_goodcounts"].as_u64().unwrap() + 1
    );
    assert_eq!(after_valid["newinput"], 100);
    assert_eq!(after_valid["sensor_configuration_error"], 0);
    assert_eq!(after_valid["armed"], 0);
    assert!(
        driven_edges >= 10,
        "native output must contain telemetry transitions"
    );
    assert!(released, "native output must return to input/pull-up");
    assert!(
        !contention,
        "stimulus must not drive against the native reply"
    );
    assert_eq!(valid_trace.dropped, 0);
    assert_eq!(
        decoded_reply,
        Ok((0x0fff, 154)),
        "wire reply must decode the stopped-period sentinel and valid checksum"
    );
}
