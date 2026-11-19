//! ESP32-C2 (ESP8684H4X) bare-ELF slice: a committed RV32IMC fixture built
//! from examples/esp32c2/main.c runs from the IROM window, keeps its stack at
//! the top of the 256 KiB DRAM view, prints on UART0 and toggles GPIO10.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::riscv::configure_riscv, Bus, Machine};

const FIXTURE: &str = "tests/fixtures/esp32c2-smoke.elf";

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn descriptor() -> ChipDescriptor {
    ChipDescriptor::from_file(root().join("configs/chips/esp32c2.yaml"))
        .expect("ESP32-C2 descriptor")
}

struct Run {
    uart: Vec<u8>,
    edges: [u32; 2],
    mcause: u32,
}

/// Runs the fixture and counts output edges on GPIO10 (the driven pin) and
/// GPIO9 (a negative control the firmware never touches).
fn run(chip: &ChipDescriptor, steps: usize) -> Run {
    let mut bus =
        SystemBus::from_config(chip, &SystemManifest::default()).expect("ESP32-C2 peripherals");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&root().join(FIXTURE)).expect("compiled RV32IMC ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load ESP32-C2 ELF");
    let gpio = machine
        .bus
        .find_peripheral_index_by_name("gpio")
        .expect("GPIO registered from descriptor");

    let mut previous = [None; 2];
    let mut edges = [0; 2];
    for _ in 0..steps {
        machine.step().expect("compiled RV32IMC instruction");
        for (slot, pin) in [10u8, 9].into_iter().enumerate() {
            if let Some(state) = machine.bus.peripherals[gpio].dev.read_gpio_output(pin) {
                if previous[slot].is_some_and(|prior| prior != state) {
                    edges[slot] += 1;
                }
                previous[slot] = Some(state);
            }
        }
    }
    let uart = sink.lock().unwrap().clone();
    Run {
        uart,
        edges,
        mcause: machine.cpu.mcause,
    }
}

#[test]
fn esp32c2_compiled_rv32imc_firmware_prints_on_uart0_and_toggles_gpio10() {
    // The firmware prints the hello only if its MUL/DIVU result and the
    // write/readback of the last DRAM word (0x3FCDFFFC) both match.
    let run = run(&descriptor(), 600_000);
    assert_eq!(
        String::from_utf8_lossy(&run.uart),
        "Hello from ESP32-C2\r\n",
        "UART0 TX stream"
    );
    assert!(
        run.edges[0] >= 2,
        "GPIO10 produced only {} edges",
        run.edges[0]
    );
    assert_eq!(run.edges[1], 0, "GPIO9 must stay idle");
    assert_eq!(run.mcause, 0, "compiled firmware must not trap");
}

#[test]
fn esp32c2_dram_ends_at_0x3fce0000() {
    // soc.h SOC_DRAM_HIGH = 0x3FCE0000: the last DRAM word is mapped, the next
    // word is not. The fixture's stack and readback depend on this edge.
    let chip = descriptor();
    let bus = SystemBus::from_config(&chip, &SystemManifest::default()).expect("bus");
    assert!(bus.read_u32(0x3FCD_FFFC).is_ok(), "last DRAM word mapped");
    assert!(
        bus.read_u32(0x3FCE_0000).is_err(),
        "0x3FCE0000 must be unmapped on the C2 (C3 DRAM continues to 0x3FCE4000)"
    );
}

#[test]
fn esp32c2_descriptor_matches_esp8684h4x_product_data() {
    // Espressif product selector, ESP8684H4X: 120 MHz, 4 MB flash, 576 KB ROM.
    // esp-idf v5.3 esp32c2 soc.h: DRAM 0x3FCA0000..0x3FCE0000.
    let chip = descriptor();
    assert_eq!(chip.cpu_hz, 120_000_000);
    assert_eq!(chip.flash.base, 0x4200_0000);
    assert_eq!(chip.flash.size, 4 * 1024 * 1024);
    assert_eq!(chip.ram.base, 0x3FCA_0000);
    assert_eq!(chip.ram.size, 256 * 1024);
    let rom = chip
        .memory_regions
        .iter()
        .find(|r| r.name == "rom")
        .expect("mask ROM window");
    assert_eq!((rom.base, rom.size), (0x4000_0000, 576 * 1024));
    // One timer group: reg_base.h declares no DR_REG_TIMERGROUP1_BASE.
    let ids: Vec<&str> = chip.peripherals.iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&"timg0") && !ids.contains(&"timg1"));
}

const ESP_HAL_FIXTURE: &str = "tests/fixtures/esp32c2-esp-hal-delay.elf";

struct HalRun {
    uart: String,
    gpio8_edges: u32,
    error: Option<String>,
}

/// Runs the esp-hal 1.2 fixture (examples/esp32c2/esp-hal) for `steps`
/// instructions, optionally replaying the ROM `.data` unpack first, as the
/// fast-boot builder does for the `esp32c2` chip. Stops at the first
/// simulation error instead of panicking, so a negative control can read it.
fn run_esp_hal(seed_rom_data: bool, steps: usize) -> HalRun {
    let chip = descriptor();
    let mut bus =
        SystemBus::from_config(&chip, &SystemManifest::default()).expect("ESP32-C2 peripherals");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&root().join(ESP_HAL_FIXTURE)).expect("esp-hal ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load esp-hal ELF");
    if seed_rom_data {
        labwired_core::boot::esp32c2_rom::seed_rom_data(&mut machine.bus);
        labwired_core::boot::esp32c2_rom::seed_console_uart(&mut machine.bus);
    }
    let gpio = machine
        .bus
        .find_peripheral_index_by_name("gpio")
        .expect("GPIO registered from descriptor");
    let (mut previous, mut gpio8_edges, mut error) = (None, 0, None);
    for _ in 0..steps {
        if let Err(e) = machine.step() {
            error = Some(format!("{e:?}"));
            break;
        }
        if let Some(state) = machine.bus.peripherals[gpio].dev.read_gpio_output(8) {
            if previous.is_some_and(|prior| prior != state) {
                gpio8_edges += 1;
            }
            previous = Some(state);
        }
    }
    let uart = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    HalRun {
        uart,
        gpio8_edges,
        error,
    }
}

#[test]
fn esp32c2_esp_hal_init_delay_and_gpio_run_on_the_mask_rom() {
    // esp_hal::init calls the mask ROM (memcpy, rom_i2c_* for the BBPLL),
    // waits for BBPLL_CAL_DONE and powers up the radio blocks; the app then
    // prints on UART0, toggles GPIO8 three times and times 2 ms delays on
    // SYSTIMER.
    let run = run_esp_hal(true, 3_000_000);
    assert_eq!(run.error, None, "UART so far: {:?}", run.uart);
    assert!(run.uart.starts_with("LW-C2 boot\n"), "{:?}", run.uart);
    assert!(run.uart.ends_with("LW-C2 done\n"), "{:?}", run.uart);
    let ticks: Vec<u64> = run
        .uart
        .lines()
        .filter_map(|l| l.strip_prefix("LW-C2 tick "))
        .map(|l| {
            let us = l.split(' ').nth(1).expect("elapsed field");
            us.trim_end_matches("us").parse().expect("microseconds")
        })
        .collect();
    assert_eq!(ticks.len(), 3, "{:?}", run.uart);
    // delay_millis(2) plus one toggle and the timing reads: 2.0..2.5 ms of
    // SYSTIMER time at the 120 MHz descriptor clock.
    for us in ticks {
        assert!((2_000..2_500).contains(&us), "delay measured {us} us");
    }
    assert_eq!(run.gpio8_edges, 3, "GPIO8 toggles");
}

#[test]
fn esp32c2_esp_hal_without_rom_data_unpack_jumps_through_a_null_table() {
    // Negative control: without the `_rom_store_table` replay, the ROM's
    // `rom_phyFuns` pointer is zero and `rom_i2c_readReg` jumps to
    // 0xFFFF_FFFE before anything is printed.
    let run = run_esp_hal(false, 1_200_000);
    let error = run.error.expect("must fault");
    assert_eq!(error, "MemoryViolation(4294967294)");
    assert_eq!(run.uart, "");
}

#[test]
fn esp32c2_matrix_control_offsets_come_from_the_register_descriptor() {
    // The C2 bank has 43 MAPs (0x00..0xA8), so its controls sit lower than
    // the C3's. The C2 mask ROM's own `_interrupt_handler` reads
    // CPU_INT_THRESH at 0x600C_2148 and CPU_INT_PRI_n at 0x600C_20C8 + 4n.
    use labwired_config::PeripheralDescriptor;
    use labwired_core::bus::MatrixCtrlOffsets;
    let bank = |chip: &str| {
        PeripheralDescriptor::from_file(
            root().join(format!("configs/peripherals/{chip}/interrupt_core0.yaml")),
        )
        .expect("interrupt_core0 descriptor")
    };
    let c2 = MatrixCtrlOffsets::from_descriptor(&bank("esp32c2"));
    assert_eq!(
        c2,
        MatrixCtrlOffsets {
            enable: 0xB8,
            pri0: 0xC8,
            thresh: 0x148,
            status0: 0xAC,
            from_cpu_source: 36,
        }
    );
    // The C3 descriptor yields the offsets the bus used before this change.
    assert_eq!(
        MatrixCtrlOffsets::from_descriptor(&bank("esp32c3")),
        MatrixCtrlOffsets::default()
    );
}

#[test]
fn esp32c2_from_cpu_doorbell_reaches_its_cpu_line_and_intr_status() {
    // Register level: map FROM_CPU_INTR0 (source 36) to CPU line 5 at
    // priority 1, ring the SYSTEM doorbell, and read the routed line and the
    // INTR_STATUS_REG_1 mirror (source 36 = word 1, bit 4).
    const MATRIX: u64 = 0x600C_2000;
    const DOORBELL0: u64 = 0x600C_0028;
    let chip = descriptor();
    let mut bus =
        SystemBus::from_config(&chip, &SystemManifest::default()).expect("ESP32-C2 peripherals");
    labwired_core::boot::esp32c2_rom::arm_interrupt_matrix(&mut bus);
    bus.write_u32(MATRIX + 36 * 4, 5).unwrap();
    bus.write_u32(MATRIX + 0xC8 + 5 * 4, 1).unwrap();
    bus.write_u32(MATRIX + 0x148, 1).unwrap();
    bus.write_u32(MATRIX + 0xB8, 1 << 5).unwrap();
    assert_eq!(bus.irq_fabric.esp32c3.irq_lines, 0);
    assert_eq!(bus.read_u32(MATRIX + 0xB0).unwrap(), 0);

    bus.write_u32(DOORBELL0, 1).unwrap();
    assert_eq!(bus.irq_fabric.esp32c3.irq_lines, 1 << 5);
    assert_eq!(bus.read_u32(MATRIX + 0xB0).unwrap(), 1 << 4);

    // Threshold above the line's priority masks the line, not the status.
    bus.write_u32(MATRIX + 0x148, 2).unwrap();
    assert_eq!(bus.irq_fabric.esp32c3.irq_lines, 0);
    assert_eq!(bus.read_u32(MATRIX + 0xB0).unwrap(), 1 << 4);

    bus.write_u32(DOORBELL0, 0).unwrap();
    assert_eq!(bus.read_u32(MATRIX + 0xB0).unwrap(), 0);
}

const ESP_HAL_IRQ_FIXTURE: &str = "tests/fixtures/esp32c2-esp-hal-irq.elf";
const ESP_HAL_GPIO_IRQ_FIXTURE: &str = "tests/fixtures/esp32c2-esp-hal-gpio-irq.elf";

/// Runs an esp-hal interrupt fixture (examples/esp32c2/esp-hal-irq,
/// esp-hal-gpio-irq, esp-hal-embassy or esp-hal-gpio-async) with the C2
/// fast-boot CPU state and ROM unpack, with or without the interrupt matrix
/// armed. Returns the UART0 text once the app prints its `PASS`/`FAIL`
/// verdict or `steps` run out. `Machine::step` never idle-fast-forwards, so
/// the timings the apps print are the per-instruction ones.
fn run_esp_hal_irq(fixture: &str, arm_matrix: bool, steps: usize) -> String {
    let chip = descriptor();
    let mut bus =
        SystemBus::from_config(&chip, &SystemManifest::default()).expect("ESP32-C2 peripherals");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&root().join(fixture)).expect("esp-hal irq ELF");
    let mut cpu = configure_riscv(&mut bus);
    labwired_core::boot::esp32c2_rom::prepare_fast_boot_cpu(&mut cpu);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load esp-hal irq ELF");
    labwired_core::boot::esp32c2_rom::seed_rom_data(&mut machine.bus);
    labwired_core::boot::esp32c2_rom::seed_console_uart(&mut machine.bus);
    if arm_matrix {
        labwired_core::boot::esp32c2_rom::arm_interrupt_matrix(&mut machine.bus);
    }
    let uart = || String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    for chunk in 0..steps / 10_000 {
        for _ in 0..10_000 {
            machine
                .step()
                .unwrap_or_else(|e| panic!("step {chunk}: {e:?}; UART {:?}", uart()));
        }
        let text = uart();
        if text.contains(" PASS\n") || text.contains(" FAIL\n") {
            return text;
        }
    }
    uart()
}

#[test]
fn esp32c2_esp_hal_software_interrupt_and_systimer_alarm_handlers_run() {
    // Three FROM_CPU raises run the handler three times; a 1 ms periodic
    // SYSTIMER alarm fires 4..6 times during a 5 ms delay (5 measured).
    let uart = run_esp_hal_irq(ESP_HAL_IRQ_FIXTURE, true, 4_000_000);
    assert!(uart.contains("LW-C2 swi hits 3\n"), "{uart:?}");
    let alarms: u32 = uart
        .lines()
        .find_map(|l| l.strip_prefix("LW-C2 alarm hits "))
        .expect("alarm count line")
        .parse()
        .expect("alarm count");
    assert!((4..=6).contains(&alarms), "{alarms} alarm handler runs");
    assert!(uart.contains("LW-C2 irq PASS"), "{uart:?}");
}

#[test]
fn esp32c2_esp_hal_raise_without_matrix_routing_never_returns() {
    // Negative control: with the matrix not armed nothing mirrors the
    // doorbell into INTR_STATUS, so `SoftwareInterrupt::raise` spins on its
    // pending check and the app never gets past the boot line.
    let uart = run_esp_hal_irq(ESP_HAL_IRQ_FIXTURE, false, 2_000_000);
    assert_eq!(uart, "LW-C2 irq boot\n");
}

#[test]
fn esp32c2_esp_hal_gpio_edge_interrupts_reach_the_io_handler() {
    // GPIO2 drives its own input buffer. Three pulses with RisingEdge run the
    // Io handler three times, two with AnyEdge four times, and two after
    // unlisten not at all (GPIO_STATUS latches nothing without INT_TYPE).
    let uart = run_esp_hal_irq(ESP_HAL_GPIO_IRQ_FIXTURE, true, 4_000_000);
    assert!(uart.contains("LW-C2 rising hits 3\n"), "{uart:?}");
    assert!(uart.contains("LW-C2 any-edge hits 4\n"), "{uart:?}");
    assert!(uart.contains("LW-C2 unlistened hits 0\n"), "{uart:?}");
    assert!(uart.contains("LW-C2 gpio-irq PASS"), "{uart:?}");
}

#[test]
fn esp32c2_esp_hal_gpio_edges_without_matrix_routing_run_no_handler() {
    // Negative control: GPIO_STATUS still latches, but with the matrix not
    // armed source 13 never reaches a CPU line, so the handler count stays 0.
    let uart = run_esp_hal_irq(ESP_HAL_GPIO_IRQ_FIXTURE, false, 4_000_000);
    assert!(uart.contains("LW-C2 rising hits 0\n"), "{uart:?}");
    assert!(uart.contains("LW-C2 gpio-irq FAIL"), "{uart:?}");
}

const ESP_HAL_EMBASSY_FIXTURE: &str = "tests/fixtures/esp32c2-esp-hal-embassy.elf";

#[test]
fn esp32c2_embassy_tasks_and_timers_run_on_esp_rtos() {
    // esp-rtos takes SYSTIMER alarm 0 as the Embassy time driver and
    // FROM_CPU_INTR0 for task switches. A spawned task awaits three 1 ms
    // Timers and signals main; `with_timeout` on a signal nobody sends
    // expires after 1 ms.
    let uart = run_esp_hal_irq(ESP_HAL_EMBASSY_FIXTURE, true, 4_000_000);
    let line = uart
        .lines()
        .find_map(|l| l.strip_prefix("LW-C2 ticks "))
        .unwrap_or_else(|| panic!("tick line missing: {uart:?}"));
    let (ticks, elapsed) = line.split_once(" elapsed_us ").expect("elapsed field");
    assert_eq!(ticks, "3", "{uart:?}");
    let elapsed: u64 = elapsed.parse().expect("microseconds");
    // Three 1 ms timers in sequence: 3.0 ms plus wake-up latency.
    assert!((3_000..3_500).contains(&elapsed), "elapsed {elapsed} us");
    assert!(uart.contains("LW-C2 timeout fired true\n"), "{uart:?}");
    assert!(uart.contains("LW-C2 embassy PASS"), "{uart:?}");
}

#[test]
fn esp32c2_embassy_without_matrix_routing_never_reaches_the_ticks() {
    // Negative control: with the matrix not armed the SYSTIMER alarm and the
    // FROM_CPU task-switch doorbell never reach a CPU line, so no timer
    // completes and main never prints its tick count.
    let uart = run_esp_hal_irq(ESP_HAL_EMBASSY_FIXTURE, false, 2_000_000);
    assert_eq!(uart, "LW-C2 embassy boot\n");
}

const ESP_HAL_GPIO_ASYNC_FIXTURE: &str = "tests/fixtures/esp32c2-esp-hal-gpio-async.elf";

#[test]
fn esp32c2_embassy_gpio_wait_for_rising_edge_wakes_on_the_pulse() {
    // GPIO2 is split into Input and Output drivers. A task raises the pad
    // 1 ms after each request; main's `wait_for_rising_edge` is woken by the
    // esp-hal default GPIO handler (source 13). Each wait ends 1 ms plus the
    // wake-up latency after the request (1033..1035 us measured). With no
    // pulse requested, a 2 ms `with_timeout` wait expires.
    let uart = run_esp_hal_irq(ESP_HAL_GPIO_ASYNC_FIXTURE, true, 6_000_000);
    let waits: Vec<u64> = uart
        .lines()
        .filter_map(|l| l.strip_prefix("LW-C2 edge wait_us "))
        .map(|v| v.parse().expect("microseconds"))
        .collect();
    assert_eq!(waits.len(), 3, "{uart:?}");
    for wait in waits {
        assert!((1_000..1_100).contains(&wait), "wait {wait} us: {uart:?}");
    }
    assert!(
        uart.contains("LW-C2 idle wait timed out true\n"),
        "{uart:?}"
    );
    assert!(uart.contains("LW-C2 gpio-async PASS"), "{uart:?}");
}

#[test]
fn esp32c2_embassy_gpio_wait_without_matrix_routing_never_completes() {
    // Negative control: with the matrix not armed neither the Embassy time
    // driver nor the GPIO source reaches a CPU line, so no edge wait ends.
    let uart = run_esp_hal_irq(ESP_HAL_GPIO_ASYNC_FIXTURE, false, 2_000_000);
    assert_eq!(uart, "LW-C2 gpio-async boot\n");
}

const ESP_HAL_UART_ASYNC_FIXTURE: &str = "tests/fixtures/esp32c2-esp-hal-uart-async.elf";

#[test]
fn esp32c2_embassy_uart_loopback_read_exact_async_gets_each_frame() {
    // UART1 in loopback (CONF0 bit 14). A task writes a 16-byte frame with
    // `write_async` 1 ms after each request; main's `read_exact_async` is
    // woken by the esp-hal UART1 handler (source 18). Each wait is 1 ms
    // plus 16 frames of 10 bits at 115200 baud (1389 us) plus the wake-up
    // latency: 2428..2430 us measured. With nothing sent, a 2 ms
    // `with_timeout` read expires. The sink also captures UART1, so the
    // printable frame bytes sit in front of each "frame" line.
    let uart = run_esp_hal_irq(ESP_HAL_UART_ASYNC_FIXTURE, true, 8_000_000);
    let frames: Vec<&str> = uart
        .lines()
        .filter_map(|l| l.split_once("LW-C2 frame ").map(|(_, rest)| rest))
        .collect();
    assert_eq!(frames.len(), 3, "{uart:?}");
    for (seq, frame) in frames.iter().enumerate() {
        let (head, verdict) = frame.split_once(" match ").expect("match field");
        assert_eq!(verdict, "true", "{uart:?}");
        let (got_seq, wait) = head.split_once(" wait_us ").expect("wait field");
        assert_eq!(got_seq, seq.to_string());
        let wait: u64 = wait.parse().expect("microseconds");
        assert!((2_389..2_500).contains(&wait), "wait {wait} us: {uart:?}");
    }
    // The loopback bytes themselves reach the capture: frame 0 is
    // b'a' + (i * 3) % 26 for i in 0..16.
    assert!(uart.contains("adgjmpsvybehknqt"), "{uart:?}");
    assert!(
        uart.contains("LW-C2 idle read timed out true\n"),
        "{uart:?}"
    );
    assert!(uart.contains("LW-C2 uart-async PASS"), "{uart:?}");
}

#[test]
fn esp32c2_embassy_uart_read_without_matrix_routing_never_completes() {
    // Negative control: with the matrix not armed neither the Embassy time
    // driver nor the UART1 source reaches a CPU line, so main never gets
    // past its first 5 ms settle timer.
    let uart = run_esp_hal_irq(ESP_HAL_UART_ASYNC_FIXTURE, false, 2_000_000);
    assert_eq!(uart, "LW-C2 uart-async boot\n");
}
