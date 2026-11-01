//! Stock Zephyr v4.1.0 `kv260_r5` images on the native Cortex-R5F (pinned tlib
//! `cortex-r5f`, PMSAv7 16-region MPU, VFPv3-D16) over the Zynq UltraScale+
//! RPU platform slice: RPU GIC, PS UART1 console and TTC0 system timer.
//! Fixtures and their provenance: crates/firmware-kv260-r5-zephyr/build.sh.
//! tests/kernel/common (~395 M instructions) runs through the release native
//! CLI instead: examples/kv260-r5/ci/test.sh.
#![cfg(feature = "qemu-arm-system")]
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{ArmModel, CortexA7System};
use labwired_core::{AdvanceRequest, Cpu, Machine};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// XCK26 RPU core clock (DS987: 533 MHz for the K26 SOM speed grade).
const CPU_HZ: u64 = 533_000_000;
const RAM: usize = 64 << 20;

fn boot(fixture: &str) -> (Machine<CortexA7System>, Arc<Mutex<Vec<u8>>>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(fixture)).unwrap();
    let mut bus = SystemBus::new();
    bus.peripherals.clear();
    bus.cpu_hz = CPU_HZ;
    // No flash: the RPU image is linked into the low RAM window (TCM/DDR).
    bus.flash = labwired_core::memory::LinearMemory::new(0, 0xc000_0000);
    bus.ram = labwired_core::memory::LinearMemory::new(RAM, 0);
    bus.configure_armv7_zynqmp_rpu().unwrap();
    let sink = Arc::new(Mutex::new(Vec::new()));
    let uart1 = bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "uart1")
        .unwrap();
    assert!(uart1
        .dev
        .as_uart_stream_host()
        .unwrap()
        .attach_console_sink(Some(sink.clone()), false));
    let mut cpu = CortexA7System::with_model(ArmModel::CortexR5F).unwrap();
    cpu.executable_range(0, RAM as u64).unwrap();
    let mut m = Machine::new(cpu, bus);
    m.config.idle_fast_forward_enabled = true;
    m.load_firmware(&image).unwrap();
    (m, sink)
}

/// Run until `needle` appears on UART1 or the cycle budget is spent.
fn run_until(
    m: &mut Machine<CortexA7System>,
    sink: &Arc<Mutex<Vec<u8>>>,
    needle: &str,
    cycles: u64,
) -> String {
    let chunk = 2_000_000;
    let mut spent = 0;
    while spent < cycles {
        let start = m.total_cycles;
        if let Err(e) = m.advance(AdvanceRequest::run(None).with_cycle_limit(start + chunk)) {
            let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
            panic!(
                "execution error at pc={:#x} cpsr={:#x}: {e}\nUART1 so far:\n{out}",
                m.cpu.get_pc(),
                m.cpu.get_register(16)
            );
        }
        spent += m.total_cycles - start;
        let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
        if out.contains(needle) {
            return out;
        }
        if m.total_cycles == start {
            break;
        }
    }
    let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    panic!(
        "{needle:?} not seen after {spent} cycles (pc={:#x}); UART1:\n{out}",
        m.cpu.get_pc()
    );
}

#[test]
fn hello_world_prints_banner_and_greeting() {
    let (mut m, sink) = boot("kv260-r5-zephyr-hello.elf");
    let out = run_until(&mut m, &sink, "Hello World! kv260_r5", 200_000_000);
    assert!(out.contains("*** Booting Zephyr OS build"), "{out}");
}

#[test]
fn synchronization_alternates_threads_on_the_ttc_tick() {
    // Each thread prints, then k_msleep(500)s and hands a semaphore over:
    // progress needs TTC0 match IRQs through the GIC, WFI idle fast-forward
    // and context switches with VFP state.
    assert!(
        labwired_core::cycle_clock::event_scheduler_enabled(),
        "500 ms sleeps need idle fast-forward: run with --features event-scheduler"
    );
    let (mut m, sink) = boot("kv260-r5-zephyr-sync.elf");
    let line = "thread_b: Hello World from cpu 0 on kv260_r5!\r\nthread_a: Hello World from cpu 0 on kv260_r5!\r\nthread_b: Hello World";
    let out = run_until(&mut m, &sink, line, 4 * CPU_HZ);
    assert!(
        out.starts_with("*** Booting Zephyr OS build v4.1.0 ***\r\nthread_a: Hello World"),
        "{out}"
    );
    // Three 500 ms sleeps precede the third greeting.
    assert!(
        m.total_cycles >= 3 * CPU_HZ / 2,
        "only {} cycles",
        m.total_cycles
    );
}
