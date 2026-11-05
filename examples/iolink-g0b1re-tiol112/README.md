# STM32G0B1RE + TIOL112 + timed IO-Link master

This rig executes a real Cortex-M0+ ELF through the G0 USART1 interrupt path,
the TIOL112 digital driver and a timed virtual master. It records the final
request stop-bit end and the first reply start bit on CQ. The 32-byte reply is
selected from the current one-byte PDout, so a stale previous-cycle selection
fails the payload assertions.

The exact chip descriptor is `stm32g0b1re`, with 512 KiB flash and 144 KiB SRAM,
and reset clock HSI16. The 64 MHz operating profile requires firmware to program
`RCC_PLLCFGR=0x30000802` (HSI16/M1*N8/R2, PLLR enabled) and switch to PLLR;
TIM2 PSC must be 63 and COM3 USART BRR 278. The test checks these conditions
before making a timing measurement. The fixed timebase does not time reset
oscillator startup or the clock switch.
For an explicitly built COM2 image at the same 64 MHz operating point, use
`IOLINKI_G0_BAUD=38400`; USART BRR is 1667 and the current profile advertises
MinCycleTime 0x56 (15.2 ms). Its tA bounds are 26.042..260.417 µs. COM2 and
COM3 runs use distinct ELF hashes and reports.

| Connection | G0B1 pad |
|---|---|
| TIOL112 TX / RX | USART1 PA9 / PA10, AF1 |
| EN | PA8 |
| WAKE | PB0 / EXTI0 |
| NFAULT | PB1 |
| CQ | Timed virtual master |

## Run the digital model tests

```bash
cargo test -p labwired-core --test g0_tiol112 --test modern_uart_timing
cargo test -p labwired-core --test iolink_master_config
```

## Run the one-action functional fixture

`stream-system64.yaml` is the UART peer configuration used by the interactive
diagram. The PHY must attach before the master. The master's `start` simulator
input is a momentary action: asserting it starts the evaluation, which pulses
logical WAKE for 1 ms, reads the DPP, validates capability/PD sizes/minimum
cycle, issues DeviceOperate and alternates READ/WRITE requests. It rejects the
legacy global-default MinCycleTime 0x0A instead of concealing that mismatch.

This same path is exercised against a real Cortex-M0+ ELF by:

```bash
IOLINKI_G0_ELF=/absolute/path/protocol-rig-64mhz.elf \
cargo test -p labwired-core --test iolink_master_config \
  real_type2v_elf_runs_the_same_one_action_uart_fixture -- --ignored --nocapture
```

The test checks selectors 7/9 against all 32 current-reply bytes and observes
both 34-byte READ and 33-byte WRITE replies. This stream fixture proves
firmware/driver/protocol integration. Its byte interface does not establish CQ
tA; the separate test below measures wire timing.

## Run the actual customer profile

Build `examples/stm32g0_tiol112` in the companion `w1ne/iolinki` source with
`IOLINK_G0_SYSCLK_HZ=64000000 IOLINK_G0_PROTOCOL_PROFILE=1`. The example's
`build.sh` needs ARM GCC, ST CMSIS and Arm CMSIS Core as documented there.
Provide the resulting `protocol-rig-64mhz.elf` explicitly; the test never turns
a missing firmware artifact into a pass.

```bash
IOLINKI_G0_ELF=/absolute/path/protocol-rig-64mhz.elf \
IOLINKI_G0_HZ=64000000 IOLINKI_G0_BAUD=230400 \
IOLINKI_G0_PROFILE=type2v IOLINKI_G0_CYCLES=100 \
IOLINKI_G0_REPORT=out/type2v-cq-timing.json \
cargo test -p labwired-core --test g0_customer_timing -- --ignored --nocapture
```

The session performs GPIO WAKE, real TYPE0 startup and capability reads,
DeviceOperate, an invalid-CKT rejection check, then 100 READ and 100 WRITE
transactions. The bus period follows the advertised MinCycleTime plus 100 µs
of margin. The current COM3 profile advertises 3 ms; an older 20 ms firmware
artifact is recorded and respected if supplied. Each TYPE2V request carries
one-byte PDout. READ replies contain OD1 + PDin32 + CKS (34 bytes); WRITE
replies contain PDin32 + CKS (33 bytes).

A separate eight-request phase schedules master starts exactly 29.412 ms
apart, approximately 34 blocks/s. Its measured intervals are recorded in the
JSON, independently of reply duration. Every transaction checks the current
selector, payload, checksum, parity, framing, driver release and unsupported
firmware accesses. The report separates STARTUP/PREOPERATE/OPERATE timing and
includes reply byte gaps. Set `IOLINKI_G0_CYCLES=4` for an initial short run.

The default PHY scenario uses 1.2 µs TX propagation, 0.3 µs RX propagation,
and 4 µs enable/disable delays. The last two values are typical assumptions,
not maximum specifications. The companion BSP waits 6 µs after asserting EN;
the no-wait negative control in `g0_tiol112` proves that an early character
does not become a clean modeled CQ reply. GPIO WAKE is a logical stimulus,
without electrical current-pulse detection.

The reference startup ELF can also run through the same timed chain at its
actual 16 MHz/COM2 setting: set only `IOLINKI_G0_ELF`, omit the profile override.
That is a prerequisite result and cannot establish the 64 MHz PD32 behavior.

See [VALIDATION](VALIDATION.md) for measured/assumed boundaries,
[sources](REQUIRED_DOCS.md), and [external components](EXTERNAL_COMPONENTS.md).
