// Task 2: prove SystemBus::attach_uart_stream_by_id wires a *live* stream onto a
// named UART (the seam World::from_manifest uses to wire cross-link endpoints).

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::uart::UartStreamDevice;
use labwired_core::Bus;
use std::sync::{Arc, Mutex};

struct Recorder(Arc<Mutex<Vec<u8>>>);
impl UartStreamDevice for Recorder {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        None
    }
    fn on_tx_byte(&mut self, byte: u8) {
        self.0.lock().unwrap().push(byte);
    }
}

fn l476_bus() -> SystemBus {
    let chip_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32l476.yaml"
    );
    let chip = labwired_config::ChipDescriptor::from_file(chip_path).unwrap();
    let manifest: labwired_config::SystemManifest =
        serde_yaml::from_str("name: seam-test\nchip: ignored\n").unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}

/// Enable the USART1/USART2 peripheral clocks (RCC_APB2ENR.USART1EN bit 14,
/// RCC_APB1ENR1.USART2EN bit 17). On the L476 these are clock-gated and unclocked
/// out of reset (RM0351), so a bare TDR write is ignored until firmware ungates
/// them — these register-poking seam tests must do what firmware does.
fn enable_l476_uart_clocks(bus: &mut SystemBus) {
    const RCC: u64 = 0x4002_1000;
    bus.write_u32(RCC + 0x60, 1 << 14).unwrap(); // APB2ENR: USART1EN
    bus.write_u32(RCC + 0x58, 1 << 17).unwrap(); // APB1ENR1: USART2EN
}

#[test]
fn attach_uart_stream_by_id_wires_a_live_stream() {
    let mut bus = l476_bus();
    enable_l476_uart_clocks(&mut bus);
    let seen = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_stream_by_id("uart2", Box::new(Recorder(seen.clone())))
        .expect("uart2 should accept a stream device");

    // uart2 base 0x40004400, V2 layout TDR at offset 0x28 → writing it transmits.
    bus.write_u32(0x4000_4428, 0x42).unwrap();

    assert_eq!(*seen.lock().unwrap(), vec![0x42]);
}

#[test]
fn detach_uart_sink_by_id_keeps_crosslink_bytes_out_of_the_console() {
    let mut bus = l476_bus();
    enable_l476_uart_clocks(&mut bus);
    let console = Arc::new(Mutex::new(Vec::new()));
    // The console sink is attached to EVERY UART (as the wasm bridge does).
    bus.attach_uart_tx_sink(console.clone(), false);

    // uart1 (debug) keeps feeding the console; uart2 (cross-link) is excluded.
    bus.detach_uart_sink_by_id("uart2")
        .expect("uart2 should be detachable from the sink");

    // uart1 base 0x40013800, V2 TDR at offset 0x28.
    bus.write_u32(0x4001_3828, 0x41).unwrap();
    // uart2 base 0x40004400, V2 TDR at offset 0x28 — protocol byte, must NOT
    // reach the console.
    bus.write_u32(0x4000_4428, 0x99).unwrap();

    assert_eq!(
        *console.lock().unwrap(),
        vec![0x41],
        "only the debug UART (uart1) should reach the console; the cross-link \
         (uart2) protocol byte must be excluded"
    );
}

#[test]
fn attach_uart_stream_by_id_rejects_unknown_and_non_uart() {
    let mut bus = l476_bus();
    assert!(bus
        .attach_uart_stream_by_id(
            "does_not_exist",
            Box::new(Recorder(Arc::new(Mutex::new(Vec::new()))))
        )
        .is_err());
    // spi1 exists on the L476 but is not a UART → must be rejected.
    assert!(bus
        .attach_uart_stream_by_id("spi1", Box::new(Recorder(Arc::new(Mutex::new(Vec::new())))))
        .is_err());
}

// Folded in from the former tests/uart_binary_framing.rs so this suite adds no extra test
// binary (each integration-test binary links the whole core, ~220 MB debug).
mod binary_framing {
    //! `uart_device` binary frames, line-silence framing and real-baud pacing.
    //!
    //! The toy peer below speaks a Modbus-shaped read protocol (slave address,
    //! function, register, count, CRC-16/MODBUS) but is NOT a Modbus slave: it only
    //! exercises the primitive's keys.

    use labwired_core::peripherals::components::declarative_uart::{
        DeclarativeUartDevice, DeclarativeUartKit,
    };
    use labwired_core::peripherals::device::UartStreamDevice;
    use labwired_core::sim_input::SimInput;

    const TOY: &str = r#"
    type: toy-reg-peer
    behavior:
      primitive: uart_device
      uart:
        baud: 19200
        char_bits: 11
        frames: { framing: silence, gap_chars: 3.5, check: crc16_modbus, max_bytes: 64 }
        regs:
          - { reg: 0, input: temperature }
          - { reg: 1, value: 7 }
          - { reg: 2, value: 258 }
        responses:
          # Address 9 is not ours; a guard on a shared pattern skips this entry.
          - match: { bytes: "addr:u8 0x03 reg:u16be count:u16be" }
            when: "var(addr) == 1 && var(count) > 0"
            respond_bytes: ["var(addr)", "0x03", "var(count) * 2", "regs(var(reg), var(count))", "crc16_modbus"]
          - match: { bytes: "addr:u8 0x06 reg:u16be value:u16be" }
            when: "var(addr) == 1"
            respond_bytes: ["var(addr)", "0x06", "var(reg):u16be", "var(value):u16be", "crc16_modbus"]
          - match: { bytes: "0xFF tail:rest" }
            respond_bytes: ["0xFF", "var(tail)"]
    metadata:
      inputs:
        - { key: temperature, label: "Temperature", unit: "C", min: -40, max: 125, default: 21, expr_scale: 10 }
    "#;

    fn toy() -> DeclarativeUartDevice {
        DeclarativeUartKit::from_yaml(TOY)
            .expect("toy peer validates")
            .device("toy-reg-peer")
            .expect("toy peer builds")
    }

    fn with_crc(body: &[u8]) -> Vec<u8> {
        let mut v = body.to_vec();
        v.extend_from_slice(&labwired_config::uart_binary::crc16_modbus(body).to_le_bytes());
        v
    }

    /// Send `frame` as the firmware would, then let the line go quiet and collect
    /// what the part answers. `step_us` is the real time credited per poll.
    fn exchange(dev: &mut DeclarativeUartDevice, frame: &[u8], step_us: u32) -> Vec<u8> {
        for &b in frame {
            dev.on_tx_byte(b);
        }
        let mut out = Vec::new();
        // 200 ms of device time is far more than any answer here needs.
        for _ in 0..(200_000 / step_us) {
            if let Some(b) = dev.poll(step_us) {
                out.push(b);
            }
        }
        out
    }

    #[test]
    fn a_frame_ends_on_silence_and_is_answered_from_the_register_table() {
        let mut dev = toy();
        let resp = exchange(&mut dev, &with_crc(&[1, 3, 0, 1, 0, 2]), 250);
        // addr, func, byte count, regs 1 and 2, CRC.
        assert_eq!(
            resp,
            with_crc(&[1, 3, 4, 0x00, 0x07, 0x01, 0x02]),
            "answer: {resp:02X?}"
        );
    }

    #[test]
    fn the_register_table_follows_the_input_channel() {
        let mut dev = toy();
        dev.set_input("temperature", 25.0).unwrap();
        let resp = exchange(&mut dev, &with_crc(&[1, 3, 0, 0, 0, 1]), 250);
        // expr_scale 10: 25.0 C is 250 = 0x00FA.
        assert_eq!(resp, with_crc(&[1, 3, 2, 0x00, 0xFA]));
    }

    #[test]
    fn a_frame_does_not_end_while_bytes_keep_arriving() {
        // 3.5 chars at 19200 baud/11 bits is 2.006 ms. Credit time between bytes
        // in steps shorter than that: the frame must stay whole.
        let mut dev = toy();
        let frame = with_crc(&[1, 3, 0, 1, 0, 2]);
        let mut out = Vec::new();
        for &b in &frame {
            dev.on_tx_byte(b);
            for _ in 0..3 {
                // 0.9 ms of quiet after each byte is under the 2.006 ms gap
                // only if the count restarts per byte, which is the rule.
                if let Some(x) = dev.poll(300) {
                    out.push(x);
                }
            }
        }
        assert!(out.is_empty(), "frame was cut short: {out:02X?}");
        for _ in 0..400 {
            if let Some(x) = dev.poll(300) {
                out.push(x);
            }
        }
        assert_eq!(out, with_crc(&[1, 3, 4, 0, 7, 1, 2]));
    }

    #[test]
    fn a_bad_checksum_is_dropped_without_an_answer() {
        let mut dev = toy();
        let mut frame = with_crc(&[1, 3, 0, 1, 0, 2]);
        *frame.last_mut().unwrap() ^= 0xFF;
        assert!(exchange(&mut dev, &frame, 250).is_empty());
        assert_eq!(dev.check_errors(), 1);
    }

    #[test]
    fn a_guard_that_fails_leaves_the_part_silent_and_captures_reach_rules() {
        let mut dev = toy();
        // Address 9 matches the pattern but not the guard.
        assert!(exchange(&mut dev, &with_crc(&[9, 3, 0, 1, 0, 2]), 250).is_empty());
        // A write echoes its captures back as 16-bit fields.
        let resp = exchange(&mut dev, &with_crc(&[1, 6, 0x12, 0x34, 0xAB, 0xCD]), 250);
        assert_eq!(resp, with_crc(&[1, 6, 0x12, 0x34, 0xAB, 0xCD]));
        assert_eq!(dev.rule_machine().var("reg"), 0x1234);
    }

    #[test]
    fn rest_of_frame_capture_reports_its_length() {
        let mut dev = toy();
        let resp = exchange(&mut dev, &with_crc(&[0xFF, 1, 2, 3]), 250);
        // 0xFF, then the tail length: three bytes plus two checksum bytes were
        // stripped first, so the tail is the 3 payload bytes.
        assert_eq!(resp, vec![0xFF, 3]);
    }

    #[test]
    fn output_runs_at_the_parts_own_baud() {
        // 19200 baud, 11-bit characters: 572.9 us per byte. A 9-byte answer needs
        // about 5.2 ms on the wire; with 100 us credits it must NOT arrive faster.
        let mut dev = toy();
        for &b in &with_crc(&[1, 3, 0, 1, 0, 2]) {
            dev.on_tx_byte(b);
        }
        let mut first = None;
        let mut last = 0u32;
        let mut count = 0usize;
        let mut t = 0u32;
        while t < 60_000 && count < 9 {
            t += 100;
            if dev.poll(100).is_some() {
                first.get_or_insert(t);
                last = t;
                count += 1;
            }
        }
        assert_eq!(count, 9);
        let span = last - first.unwrap();
        // Eight inter-byte gaps of 572.9 us = 4583 us; allow one 100 us step.
        assert!(
            (4400..=4800).contains(&span),
            "9 bytes spanned {span} us, expected about 4583"
        );
    }

    #[test]
    fn shipped_text_parts_do_not_opt_in() {
        for ty in ["hc-05", "sim800l", "neo6m-gps"] {
            let yaml = labwired_config::embedded_device_yaml(ty).unwrap();
            let dev = DeclarativeUartKit::from_yaml(yaml)
                .unwrap()
                .device(ty)
                .unwrap();
            assert!(!dev.paced_by_device(), "{ty} must keep the host pacing");
            assert_eq!(dev.max_bytes_per_tick(), 1, "{ty}");
            assert_eq!(dev.declared_baud(), None, "{ty} must not raise mismatches");
        }
    }
}

// Folded in from the former tests/uart_device_real_baud_host.rs so this suite adds no extra test
// binary (each integration-test binary links the whole core, ~220 MB debug).
mod real_baud_host {
    //! The hosting UART's side of `uart_device` real-baud pacing: device time
    //! comes from engine cycles at the core clock, a disagreeing programmed baud is
    //! reported, and a part that did not opt in keeps the historical pacing.

    use labwired_core::peripherals::components::declarative_uart::DeclarativeUartKit;
    use labwired_core::peripherals::uart::{Uart, UartRegisterLayout};
    use labwired_core::{CycleClock, Peripheral};

    const CPU_HZ: u64 = 8_000_000;

    const TOY: &str = r#"
    type: toy-reg-peer
    behavior:
      primitive: uart_device
      uart:
        baud: 19200
        char_bits: 11
        frames: { framing: silence, gap_chars: 3.5, check: crc16_modbus }
        regs: [ { reg: 1, value: 7 }, { reg: 2, value: 258 } ]
        responses:
          - match: { bytes: "addr:u8 0x03 reg:u16be count:u16be" }
            respond_bytes: ["var(addr)", "0x03", "var(count) * 2", "regs(var(reg), var(count))", "crc16_modbus"]
    "#;

    fn request() -> Vec<u8> {
        let body = [1u8, 3, 0, 1, 0, 2];
        let mut v = body.to_vec();
        v.extend_from_slice(&labwired_config::uart_binary::crc16_modbus(&body).to_le_bytes());
        v
    }

    fn host(
        brr: u16,
        dev: Box<dyn labwired_core::peripherals::uart::UartStreamDevice>,
    ) -> (Uart, CycleClock) {
        let mut u = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        let clock = CycleClock::default();
        u.attach_cycle_clock(clock.clone());
        u.attach_cpu_hz(CPU_HZ);
        u.attach_stream(dev);
        u.write(0x08, (brr & 0xFF) as u8).unwrap();
        u.write(0x09, (brr >> 8) as u8).unwrap();
        (u, clock)
    }

    fn toy() -> Box<dyn labwired_core::peripherals::uart::UartStreamDevice> {
        Box::new(
            DeclarativeUartKit::from_yaml(TOY)
                .unwrap()
                .device("toy-reg-peer")
                .unwrap(),
        )
    }

    /// Drive the UART with `step_us` of engine time per tick and report the
    /// elapsed microseconds at which each RX byte appeared.
    fn run(u: &mut Uart, clock: &CycleClock, total_us: u64, step_us: u64) -> Vec<u64> {
        let rx = u.rx_buffer();
        let mut seen = 0usize;
        let mut at = Vec::new();
        let mut t = 0;
        while t < total_us {
            t += step_us;
            clock.publish(t * CPU_HZ / 1_000_000);
            u.tick();
            let n = rx.lock().unwrap().len();
            for _ in seen..n {
                at.push(t);
            }
            seen = n;
        }
        at
    }

    #[test]
    fn a_device_on_its_own_baud_answers_in_real_time() {
        // 8 MHz / 19200 baud = 417.
        let (mut u, clock) = host(417, toy());
        for b in request() {
            u.write(0x04, b).unwrap();
        }
        let at = run(&mut u, &clock, 60_000, 50);
        assert_eq!(at.len(), 9, "answer length");
        // Frame ends after 3.5 chars (2.006 ms) of silence; one 50 us step of slack
        // each side, and one more for the first byte going out at once.
        assert!(
            (2000..=2300).contains(&at[0]),
            "first byte at {} us, expected just after the 2.006 ms gap",
            at[0]
        );
        let span = at[8] - at[0];
        assert!(
            (4400..=4800).contains(&span),
            "nine bytes spanned {span} us, expected about 4583 (8 x 572.9)"
        );
    }

    #[test]
    fn a_programmed_baud_that_disagrees_is_reported() {
        labwired_core::fidelity::reset();
        // 8 MHz / 69 = 115942 baud against a part at 19200.
        let (mut u, clock) = host(69, toy());
        run(&mut u, &clock, 1_000, 100);
        let report = labwired_core::fidelity::take();
        let note = report
            .approximations
            .get(labwired_core::fidelity::UART_BAUD_MISMATCH)
            .expect("mismatch is reported");
        assert!(note.detail.contains("19200"), "{}", note.detail);
        assert!(note.detail.contains("115"), "{}", note.detail);
    }

    #[test]
    fn a_matching_baud_reports_nothing() {
        labwired_core::fidelity::reset();
        let (mut u, clock) = host(417, toy());
        run(&mut u, &clock, 1_000, 100);
        assert!(!labwired_core::fidelity::take()
            .approximations
            .contains_key(labwired_core::fidelity::UART_BAUD_MISMATCH));
    }

    #[test]
    fn an_apb_prescaled_divisor_is_not_a_false_alarm() {
        labwired_core::fidelity::reset();
        // 8 MHz / 208 = 38461: exactly what a UART on a /2 APB clock programmed
        // for 19200 baud looks like when the divisor is read against the core clock.
        let (mut u, clock) = host(208, toy());
        run(&mut u, &clock, 1_000, 100);
        assert!(!labwired_core::fidelity::take()
            .approximations
            .contains_key(labwired_core::fidelity::UART_BAUD_MISMATCH));
    }

    #[test]
    fn a_text_part_keeps_the_host_pacing_whatever_the_clock_says() {
        labwired_core::fidelity::reset();
        let yaml = labwired_config::embedded_device_yaml("hc-05").unwrap();
        let hc05 = Box::new(
            DeclarativeUartKit::from_yaml(yaml)
                .unwrap()
                .device("hc-05")
                .unwrap(),
        );
        // Programmed baud far from the part's 38400, and a clock that never moves:
        // neither may change anything for a part that did not opt in.
        let (mut u, _clock) = host(69, hc05);
        for b in b"AT\r\n" {
            u.write(0x04, *b).unwrap();
        }
        let rx = u.rx_buffer();
        for tick in 1..=4 {
            u.tick();
            assert_eq!(rx.lock().unwrap().len(), tick, "one byte per bus tick");
        }
        let got: Vec<u8> = rx.lock().unwrap().iter().copied().collect();
        assert_eq!(got, b"OK\r\n");
        assert!(!labwired_core::fidelity::take()
            .approximations
            .contains_key(labwired_core::fidelity::UART_BAUD_MISMATCH));
    }
}
