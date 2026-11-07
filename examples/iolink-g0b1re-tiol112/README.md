# STM32G0B1RE + TIOL112 + timed IO-Link master

This rig executes a real Cortex-M0+ ELF through the G0 USART1 driver path,
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
cargo test -p labwired-core --features event-scheduler --test g0_tiol112 --test modern_uart_timing
cargo test -p labwired-core --features event-scheduler --test stm32g0_uart_dma --test stm32g0_dmamux_generators --test stm32_dma_walk_differential
```

## Run the one-action functional fixture

`stream-system64.yaml` is the UART peer configuration used by the interactive
diagram. The PHY must attach before the master. The master's `start` simulator
input is a momentary action: asserting it starts the evaluation, which pulses
logical WAKE for 1 ms, reads the DPP, validates capability/PD sizes/minimum
cycle, issues DeviceOperate and alternates READ/WRITE requests. It rejects the
legacy global-default MinCycleTime 0x0A instead of concealing that mismatch.

The master's read-only evaluation phase distinguishes armed, wake, each DPP
read, DeviceOperate, OPERATE and fault. Its error text identifies the failed
validation. The browser accessor exports these as optional
`evaluation_phase` / `evaluation_error` fields; legacy peers return null.
Full process-data bytes and raw requests/replies remain in the master trace.

This same path is exercised against a real Cortex-M0+ ELF by:

```bash
IOLINKI_G0_ELF=/absolute/path/protocol-rig-64mhz.elf \
cargo test -p labwired-core --features event-scheduler --test iolink_master_config \
  real_type2v_elf_runs_the_same_one_action_uart_fixture -- --ignored --nocapture
```

The test checks selectors 7/9 against all 32 current-reply bytes and observes
both 34-byte READ and 33-byte WRITE replies. This stream fixture proves
firmware/driver/protocol integration. Its byte interface does not establish CQ
tA; the separate test below measures wire timing.

## Run the actual TYPE2V profile

Build `examples/stm32g0_tiol112` in the companion `w1ne/iolinki` source with
`IOLINK_G0_SYSCLK_HZ=64000000 IOLINK_G0_PROTOCOL_PROFILE=1`. The example's
`build.sh` needs ARM GCC, ST CMSIS and Arm CMSIS Core as documented there.
Provide the resulting `protocol-rig-64mhz.elf` explicitly; the test never turns
a missing firmware artifact into a pass.

```bash
IOLINKI_G0_ELF=/absolute/path/protocol-rig-64mhz.elf \
IOLINKI_G0_HZ=64000000 IOLINKI_G0_BAUD=230400 \
IOLINKI_G0_PROFILE=type2v IOLINKI_G0_CYCLES=100 \
IOLINKI_G0_DLL_TIMING_ENFORCED=0 \
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
The oracle reads the firmware's GPIO modes and AF1 selection, pull-ups,
USART 8E1 registers, BRR, TIM2 prescaler and PLL configuration after wake.
It records the number of received reply characters, parity/framing errors,
validated checksums and checked process-data bytes. All 32 payload bytes are
checked in both the benchmark and the eight application-cadence replies.

For an explicitly enabled firmware build, set
`IOLINKI_G0_DLL_TIMING_ENFORCED=1`. The oracle resolves the exported
`rig_timing_enforced` symbol and verifies its RAM value against this declaration.
Optionally set `IOLINKI_G0_DLL_LAYOUT=/absolute/path/arm-eabi-layout.json`
to read the DLL's actual enforcement state and diagnostic counters as well.
The layout must be derived from that ELF's compiler-generated DWARF; it carries
the exact ELF SHA256, each field's offset and width, and the DLL size. The oracle
rejects a hash mismatch or a field outside G0B1 SRAM before reading memory.
Do not reuse offsets across compilers. A zero timing-error count is required
when this probe is supplied for an enabled build; deliberate invalid-CKT
CRC/framing counts are reported separately. These reads never change firmware
state or external C/Q timing bounds.

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

## CMSIS DMA and ST HAL DMA profiles

The companion firmware selects `IOLINK_G0_UART_BACKEND=0` for its IRQ port,
`1` for CMSIS DMA, or `2` for ST HAL UART/DMA. The HAL profile links the actual
vendor UART/DMA implementation; it does not replace the board's CMSIS clock,
GPIO or TIM2 initialization. Build each backend and COM mode as a separate
image with `IOLINK_G0_SYSCLK_HZ=64000000`, `IOLINK_G0_PROTOCOL_PROFILE=1`,
`IOLINK_RIG_TIMING_ENFORCE=1` and `IOLINK_RIG_COM=2|3`. Follow the companion
README for the pinned CMSIS/HAL dependencies. GCC/newlib and diagnostic
Clang builds are separate evidence lanes.

| Function | Exact DMA path |
|---|---|
| Receive | USART1 RDR → DMA1 channel 1 → 128-byte circular RAM buffer |
| Transmit | RAM → DMA1 channel 2 → USART1 TDR, normal mode |
| Request routing | DMAMUX channel 0 / request 50 RX; channel 1 / request 51 TX |
| Interrupts | DMA1 channel 1 IRQ9; channels 2/3 IRQ10; USART1 IRQ27 |
| Driver release | USART TC after the last stop bit, separately from DMA TC |

The strict G0 DMA model advances CNDTR, memory pointers and HT/TC only after
an actual successful bus transfer. A failed transfer raises TE instead. Its
observed channel item counts are independent of the firmware counters. The
report resolves `board_uart_backend` and `board_dma_stats` from the ELF and
uses that ELF's hash-bound DWARF layout; a label alone cannot satisfy the DMA
assertions. The fixture requires every reply byte to reach TDR through DMA
and every request byte to reach RAM through DMA.

For a bounded probe, add `IOLINKI_G0_EXPECT_BACKEND=1|2` and the matching
`IOLINKI_G0_DLL_LAYOUT` to the timed command above. Use
`--features event-scheduler`, the feature enabled by the browser and default
CLI. The optional matrix runner builds/runs this same test and requires both
DMA backends at both COM modes:

```bash
python3 scripts/run-iolink-uart-matrix.py /absolute/path/images.json \
  --output /absolute/path/results
```

Each `images` entry supplies `id`, `backend` (`cmsis-dma` or `hal-dma`),
`baud`, `elf`, `sha256`, `layout` and the explicit supply assumption
`power_model: ideal_vdd`. Source/compiler/HAL provenance fields are retained.
The runner checks file hashes before and after each invocation, observes the
executing backend, checks DMA/UART completion counts against actual replies,
and refuses source changes during a matrix. Short or selected runs are marked
diagnostic, not a complete four-profile result.

### Separate fault and circular-buffer diagnostic

Set `IOLINKI_G0_DMA_RECOVERY=1` on a bounded timed run to exercise the same
firmware instance through parity, framing and DMA bus errors. Only the final
character of an otherwise checksum-valid request is damaged. The driver must
reject it, record an actual UART error, rearm itself and answer a fresh request
with the current PD32 payload. The DMA error case deliberately writes an
unmapped destination into the active receive channel; the firmware's TE path
must restore its buffer address and resume reception. The single intentional
write to `0x70000000` remains visible in `fidelity_gaps`; any other unmapped
access, extra hit or undecoded instruction still fails.

Successful replies rearm the receive ring. To test a full ring lap, the
optional diagnostic first sends 66 checksum-valid `0F 2D` legacy transition
commands after the real DPP reads and before DeviceOperate. They are silent
in the stack's PREOPERATE/ESTAB_COM path and are spaced by 100 Tbit. The test
requires 132 independent DMA transfers and 132 firmware-consumed bytes, one
HT event, one TC event, CNDTR=124, no overrun and no rearm. It then completes
normal DeviceOperate and requires the usual current-selector replies. This
legacy transition sequence is a transport diagnostic, not an IO-Link
conformance sequence. There is no reset, firmware reload, forced DLL state,
masked fallback policy or fabricated DMA completion.

Recovery results are kept separately from the clean matrix. DMA2, analog
noise injection, bus-arbitration contention and silicon timing correlation
remain outside the admitted model. Live byte-stream success does not
establish C/Q timing; the native waveform report supplies that separate gate.
