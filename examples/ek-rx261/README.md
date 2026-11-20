# EK-RX261 (R5F52618BGFP) — first Renesas RX board

The Renesas **EK-RX261** evaluation kit (RTK5EK2610S00001BE) carries an
**R5F52618BGFP**: an RX261 MCU (RXv3 core, single-precision FPU, 512 KiB code
flash, 128 KiB RAM). This is LabWired's first **Renesas RX** part and the first
board on its RX core.

What runs: the new RX core (`crates/core/src/cpu/rx.rs`: the RXv3 integer,
DSP-accumulator and FPU instruction set, with the ICU folded into the core),
the SCI6 console as a classic SCI, the CMT0-3 compare match timers
(`rx_cmt`), and the SYSTEM clock / FLASH wait-state handshakes the Renesas
RDP start-up code polls (`rx_sysc`, `rx_flash`). Stock, unmodified **Zephyr
v4.3.0** images for `ek_rx261` start at the fixed reset vector and pass:

| Script | Firmware | What it proves |
| :--- | :--- | :--- |
| `zephyr-hello.yaml` | `samples/hello_world` | reset vector → RDP clock setup → C runtime → kernel init → SCI6 console |
| `zephyr-sync.yaml` | `samples/synchronization` | two threads alternating on a semaphore: CMT0 compare-match interrupts through the ICU, CMT1 cycle counter, `INT #1` context switches, WAIT idle |
| `zephyr-kernel-common.yaml` | `tests/kernel/common` | 12 ztest suites, 61 pass + 1 skip: atomics, bitarray, byteorder, clock, irq_offload (`INT`), timeout order, errno/thread context, printk, pow2 |
| `zephyr-uart-basic-api.yaml` | `tests/drivers/uart/uart_basic_api` | 7/7 pass on the interrupt-driven SCI driver: fifo_fill on TXI6/TEI6, fifo_read and pending on RXI6 (ICU vectors 228/229/227), poll_in/poll_out, config_get/configure |

```sh
cargo run -q -p labwired-cli -- test --script examples/ek-rx261/zephyr-hello.yaml
cargo run -q -p labwired-cli -- test --script examples/ek-rx261/zephyr-kernel-common.yaml
examples/ek-rx261/ci/test.sh
```

Rebuild the fixtures (byte-identical): `crates/firmware-ek-rx261-zephyr/build.sh`.

The generic NTC and diode co-simulation proof is
`cargo test -p labwired-core --test cosim_ek_rx261_analog`. The example
`analog-ntc-diode-system.yaml` puts a Beta NTC divider on P40/AN000 and a
Shockley diode on P41/AN001. The S12AD (12-bit A/D converter, on the
`ra_adc12` model) converts both. The bare-metal firmware
`firmware/adc-ntc-diode.c` (rebuild: `firmware/build.sh`, Zephyr SDK RX GCC)
starts a single scan, polls `ADCSR.ADST` and prints `N=7FF D=...` at 25 °C on
SCI6. The test runs that firmware in a `Session` that solves the netlists. The
S12ADI0 interrupt is not delivered, and VREFH0 = 3.3 V is an assumption.

Not modelled yet: PORT/MPC register behavior (so no firmware-accessible LEDs or switches; the AN pads are analog routes only), the S12AD scan-end interrupt, SCI6
ERI (receive-error interrupt), DTC, group interrupts, IRQ pins, the
RXv3 register banks and user-mode privilege checks. See `VALIDATION.md`.
