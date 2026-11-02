# PolarFire SoC Discovery Kit (MPFS095T) — first RV64 board

The Microchip **PolarFire SoC Discovery Kit** (MPFS-DISCO-KIT) carries an
**MPFS095T**: an FPGA with a hardened RISC-V microprocessor subsystem (MSS) of
one E51 monitor core (RV64IMAC) and four U54 application cores (RV64GC). This
is LabWired's first 64-bit RISC-V part.

What runs: **one U54 hart** (`mhartid` 1) on the new RV64 core
(`crates/core/src/cpu/riscv64.rs`), with the CLINT inside the core and the
five MMUARTs as 16550s. Stock, unmodified **Zephyr v4.1.0** images for
`mpfs_icicle/polarfire/u54` (the same MSS) boot from the DDR window at
`0x8000_0000` and pass:

| Script | Firmware | What it proves |
| :--- | :--- | :--- |
| `zephyr-hello.yaml` | `samples/hello_world` | reset → C runtime → kernel init → MMUART1 console (16550 divisor through LCR.DLAB) |
| `zephyr-sync.yaml` | `samples/synchronization` | two threads alternating on a semaphore: CLINT compare interrupts, `k_busy_wait` on `mtime`, context switch, MRET |
| `zephyr-kernel-common.yaml` | `tests/kernel/common` | 12 ztest suites, 63/63 cases: atomics (AMO + LR/SC), bitarray, byteorder, clock, irq_offload (ECALL), nested IRQs, timeout order, errno, printk |

```sh
cargo run -q -p labwired-cli -- test --script examples/mpfs-disco-kit/zephyr-hello.yaml
cargo run -q --release -p labwired-cli -- test --script examples/mpfs-disco-kit/zephyr-kernel-common.yaml
examples/mpfs-disco-kit/ci/test.sh
```

Rebuild the fixtures (byte-identical): `crates/firmware-mpfs-zephyr/build.sh`.

Not modelled yet: F/D, S/U modes and Sv39 (so no OpenSBI/Linux), the PLIC
(no external interrupts), the E51 and the other three U54s, MSS GPIO/I2C/SPI,
the HSS boot flow and DDR training. See `VALIDATION.md`.
