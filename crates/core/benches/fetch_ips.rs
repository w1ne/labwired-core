// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Establishes a baseline IPS number for the interpreter. Keep this green
// on every PR: if numbers drop more than ~5% without justification,
// something on the hot path regressed.
//
// Current targets (informational only, not a correctness claim):
//   - fetch_u16_hot      : dense bus fetch throughput, no peripheral lookup
//   - step_from_fixture  : full interpreter step over a real blinky ELF
//
// Run with: cargo bench -p labwired-core

use std::path::PathBuf;

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use labwired_core::{bus::SystemBus, memory::LinearMemory, system::cortex_m, DebugControl, Machine};

fn workspace_root() -> PathBuf {
    // crates/core/benches -> crates/core -> crates -> <root>
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .expect("workspace root")
}

/// Exercises only the bus -> LinearMemory fast path. This is the floor
/// of instruction-fetch cost; anything slower than this is overhead.
fn bench_fetch_u16(c: &mut Criterion) {
    let mut mem = LinearMemory::new(64 * 1024, 0x0000_0000);
    // Fill with a NOP-ish thumb pattern so the optimizer can't fold the load.
    for i in 0..mem.data.len() {
        mem.data[i] = (i & 0xFF) as u8;
    }
    let bus = {
        let mut b = SystemBus::new();
        b.flash = mem;
        b
    };

    let mut group = c.benchmark_group("fetch_u16");
    // 32 KiB of reads per iteration.
    group.throughput(Throughput::Bytes(32 * 1024));
    group.bench_function("linear_sweep", |b| {
        b.iter(|| {
            let mut acc: u32 = 0;
            let mut addr: u64 = 0;
            while addr < 32 * 1024 {
                let v = bus.read_u16(addr).unwrap();
                acc = acc.wrapping_add(v as u32);
                addr += 2;
            }
            black_box(acc)
        })
    });
    group.finish();
}

/// End-to-end interpreter loop on the CI fixture ELF. Measures real-world IPS
/// with decode, execute, and peripheral tick costs included.
fn bench_step_from_fixture(c: &mut Criterion) {
    let root = workspace_root();
    let elf = root.join("tests/fixtures/uart-ok-thumbv7m.elf");
    if !elf.exists() {
        eprintln!(
            "skipping step_from_fixture: fixture missing at {}",
            elf.display()
        );
        return;
    }

    let mut group = c.benchmark_group("step");
    // Budget per measurement window: step this many instructions.
    const N_STEPS: u32 = 1_000;
    group.throughput(Throughput::Elements(N_STEPS as u64));

    group.bench_function("uart_fixture_1k_steps", |b| {
        b.iter_batched(
            || {
                let program =
                    labwired_loader::load_elf(&elf).expect("load uart fixture elf");
                let mut bus = SystemBus::stm32f103();
                let (cpu, _nvic) = cortex_m::configure_cortex_m(&mut bus);
                let mut m = Machine::new(cpu, bus);
                m.load_firmware(&program).expect("load firmware");
                m
            },
            |mut machine| {
                let reason = machine.run(Some(N_STEPS)).expect("run steps");
                black_box(reason);
            },
            criterion::BatchSize::SmallInput,
        )
    });
    group.finish();
}

criterion_group!(benches, bench_fetch_u16, bench_step_from_fixture);
criterion_main!(benches);
