# ESP32-C6-DevKitC-1 — Known Limitations

Scope of this list: the LabWired `esp32c6` chip descriptor
([`configs/chips/esp32c6.yaml`](../../configs/chips/esp32c6.yaml)) and the
ESP32-C6-DevKitC-1 system
([`configs/systems/esp32c6-devkitc.yaml`](../../configs/systems/esp32c6-devkitc.yaml))
as validated by the tier-1 fixture
([`examples/tier1-fixture/esp32c6/`](../../examples/tier1-fixture/esp32c6/)).
If it is not listed under **Proven at tier-1**, treat it as unmodelled no
matter how plausible the register map looks. This is a
**SIM-DERIVED** target: nothing here has been diffed against live C6 silicon,
and no firmware beyond the in-tree fixtures/apps has been run.

## Proven at tier-1 (documented scenarios)

The full twelve-cell row, proven by `tests/fixtures/tier1/esp32c6.elf`:

| Class | What is proven |
|-------|----------------|
| uart | Implicit: the `TIER1` transcript arrives over UART0 (Espressif UART twin, 128-byte FIFO, paced at the descriptor's 160 MHz). |
| clock | PCR is a real register file (full SVD map): `UART0_SCLK_CONF`/`SYSCLK_CONF` round-trip their documented values. `UART0_CONF.CLK_EN=0` makes UART0 reads return 0 and **drops** writes (the pre-gate value survives re-enabling); `CLK_EN=1` restores normal operation. The same gate mechanism is declared for UART1, TIMG0, TIMG1, GDMA, I²C0, SPI2, LEDC and SARADC. |
| gpio | `OUT`/`ENABLE` stores plus real `W1TS`/`W1TC` set/clear side effects; `FUNCn_OUT_SEL_CFG`/`FUNCn_IN_SEL_CFG` words round-trip; `IN` does not follow the output latch. |
| timer | TIMG0 (0x6000_8000, shared `esp32::timg::Timg` via `esp32c6_mwdt`): `T0CONFIG.EN` set → `T0UPDATE`-latched `T0LO/T0HI` advances across a bounded spin; `EN` cleared → the counter is frozen. |
| dma | GDMA (0x6008_0000, `esp32c6_gdma`): a real in-RAM linked-list mem→mem transfer. Descriptors are walked, bytes land in the destination, `IN_SUC_EOF/IN_DONE` and `OUT_TOTAL_EOF/OUT_DONE` latch, the EOF-descriptor addresses are recorded, RX descriptors get owner-cleared with the received length, and `OUT_AUTO_WRBACK` returns TX descriptors. |
| irq | Two real RISC-V traps. Software: `CPU_INTR_FROM_CPU_0` (matrix source 22) → MAP → enabled line 9 → `mcause=0x8000_0009`; the handler runs and acknowledges; disabling the line masks a second doorbell. Peripheral: UART0's `irq: 43` descriptor wiring makes the shared Espressif twin assert source 43 (its `matrix_irq_sources_into`), mapped to enabled line 10; `INT_ENA.TX_DONE` is armed, one byte is shifted out of the TX FIFO so the latched `TX_DONE` raw bit rises, and the trap carries `mcause=0x8000_000A`. The handler's `UART_INT_CLR` W1C write clears the source; it also masks line 10 because a scheduler-driven peripheral's routed level is re-derived at the peripheral tick, not at the MMIO write. The main flow re-enables line 10 and requires no second trap, proving the line de-asserted. |
| i2c | I²C0 (0x6000_4000, `esp32c3_i2c`, C6 source 50): the command-list engine walks RSTART→WRITE(1)→STOP to completion — every executed `COMD` slot latches `command_done`, `CTR.TRANS_START` self-clears, and `INT_RAW.TRANS_COMPLETE` latches after the wire transaction. No slave is attached, so the address byte is NACKed and no read-data path is claimed. |
| spi | GP-SPI2 (0x6008_1000, `esp32c3_spi`, C6 source 72): `SPI_CMD.USR` launch self-clears, `TRANS_DONE` latches in `DMA_INT_RAW`, and `W0` is overwritten with `0xFFFF_FFFF` — the idle pulled-high MISO line was really shifted through the engine. No external device or CS gating. |
| adc | APB_SARADC (0x6000_E000, `esp32c3_apb_saradc`, C6 source 60): a one-shot `ONETIME_START` self-clears, latches `SAR1_DONE`, and `SAR1DATA_STATUS` carries a deterministic channel-dependent 12-bit sample plus the packed channel id; channel 3 and channel 5 differ predictably. No analog source, DMA mode, thresholds or TSENS. |
| pwm | LEDC (0x6000_7000, `esp32c3_ledc`, C6 source 45): TIMER0's live counter advances with elapsed cycles, wraps at the programmed `2^DUTY_RES` period and latches `LSTIMER0_OVF`; `PAUSE` freezes the counter and stops new overflows. No pad drive, gamma, capture or event-task claim. |
| wdt | TIMG0 MWDT (0x6000_8000, `esp32c6_mwdt` with the shared TIMG's `with_mwdt`): `WDTWPROTECT` resets to the key (unlocked); locking it makes `WDTCONFIG0..5` writes drop (readback unchanged) while `WDTFEED` stays writable; `WDTCONFIG1` round-trips; with `STG0_HOLD` programmed, the walk-driven stage-0 countdown latches `INT_RAW_TIMERS.WDT_INT_RAW` and `INT_CLR_TIMERS` clears it W1C. The **four-stage chain** is modelled: WDTCONFIG2..5 are the stage holds, WDTCONFIG0's STG0..3 fields select off/interrupt/reset-per-stage, expiry advances `0→1→2→3→0`, and every interrupt-action stage latches (the fixture proves stages 0 and 1: a second latch arrives without a feed, and a feed restarts stage 0 even after the chain has moved on). The CPU/system reset actions are still NOT performed (advanced through; see "Not modelled"). |
| rtc | LP_TIMER (0x600B_0C00, `esp32c6_lp_rtc`): the `UPDATE` bit-28 strobe latches the live 48-bit counter into `MAIN_BUF0`; without a new strobe the readout is frozen; a second strobe after elapsed cycles is strictly greater and shifts the previous snapshot into `MAIN_BUF1` (the LL's documented double buffer). No RTC-slow rate, alarms, overflow/wakeup IRQs or sleep retention. |

## Partially modelled

| Area | What exists | What is missing / could mislead |
|------|-------------|---------------------------------|
| PCR (`esp32c6_pcr`, full SVD register map) | All documented PCR registers store/read back with SVD reset values; `CLK_EN` gates are resolved and enforced through the shared bus gate. | `RST_EN` is recorded but **not enforced** (the generic gate cannot express its active-low/reset-at-boot polarity without gating the console at reset). No clock tree: dividers, `SYSCLK_CONF` mux, `PLL_DIV_CLK_EN` are storage only; nothing is paced from them (`cpu_hz` stays the single clock fact). Only HP PCR; LP clock registers are absent. |
| UART0/UART1 (`esp32c6_uart`) | Head register map (FIFO/INT/CLKDIV/STATUS/CONF0/CONF1), 128-byte FIFOs, interrupt RAW/ST/ENA/CLR, baud-paced shifting, UART0 echoes to the host sink. | C6 tail registers (`SLEEP_CONF0-2` @0x30-0x38, `CLK_CONF` @0x88, `DATE` @0x8C, `ID` @0x9C) are unmodelled: those offsets answer with the C3/S3 tail layout. Flow control, RS-485, IR modulation, wake-up, and DMA (UHCI) coupling are not modelled. `INT_RAW` bits latch, but **no UART interrupt has been routed through the C6 matrix by a fixture** (see below). |
| GPIO (`esp32c6_gpio`) | Shared C3-sized register block: OUT/W1TS/W1TC, ENABLE, IN, FUNC0_IN_SEL_CFG, FUNC0_OUT_SEL_CFG side effects. | Sized for 26 pins: **GPIO26–GPIO30 are not modelled** (C6 has 31). No GPIO interrupt generator fed to matrix source 30. IO_MUX pad routing is a register stub — the console shifts bytes regardless of `MCU_SEL`. |
| TIMG0/TIMG1 (`esp32c6_mwdt`) | T0 counter with EN gate, LO/HI latch via UPDATE, preload, INT_RAW/INT_CLR plumbing, generic round-trip for other offsets; plus the C3/C6 MWDT path (`with_mwdt`): `WDTWPROTECT` write lock over `WDTCONFIG0..5`, `WDTFEED` reload, and the full four-stage chain — WDTCONFIG2..5 hold values, WDTCONFIG0 STG0..3 actions, expiry advancing `0→1→2→3→0`, every "interrupt" stage latching `INT_RAW_TIMERS.WDT_INT_RAW` (W1C via `INT_CLR_TIMERS`), off/reset stages advancing without latching. The config/hold registers seed their SVD reset values. The mwdt variant is walk-driven so a read-only status poll sees time pass; one count is one peripheral walk tick (512 CPU cycles under the CLI default), **not** 12.5 ns × prescaler, and WDTCONFIG1's prescaler does not scale the countdown. | One general-purpose timer per group on C6 (the model's T1 head at 0x24 is inert phantom storage, as on C3). The GP counter advances 1 per model tick, **not** at the programmed divider/clock-source. Stage actions 2/3 (reset CPU/system) advance the chain but **no reset is ever performed** — the bus exposes no safe reset-request path for a peripheral; a feed does not clear the latched interrupt (only INT_CLR does). **No TIMG interrupt is fired**: `TG0_T0_LEVEL` (51) and `TG0_WDT_LEVEL` (53) are declared on the descriptor but nothing routes them. |
| I²C0 (`esp32c3_i2c`) | Command-list engine (RSTART/WRITE/READ/STOP), TX/RX FIFOs, INT RAW/ST/ENA/CLR, clock stretch/timeout storage. Reused from the C3; the C6 SVD head is offset-identical and the C6 source (I2C_EXT0=50) is passed through `irq:`. | C6-only filter/timing tail registers are storage. The shared C3 pad wiring (`wire_esp32c3_i2c_pads`) keys on the C3 matrix signal ids (I2CEXT0_SCL/SDA 53/54), which differ on the C6, so **matrix-routed C6 I²C is not proven** and a routed external slave would not be seen by the pad gate. No interrupt-matrix delivery of source 50 to the CPU. |
| GP-SPI2 (`esp32c3_spi`) | CPU/W-buffer transaction engine: `SPI_CMD.USR` launch handshake, `TRANS_DONE` latching, W0..W15 shift, master/slave mode storage. Reused from the C3 (offset-identical head), C6 source 72 via `irq:`. | No CS gating, no DMA-coupled transfers, no slave-mode data path; `MSPI` source 40 and SPI0/SPI1 windows are undeclared. No interrupt-matrix delivery of source 72. |
| APB_SARADC (`esp32c3_apb_saradc`) | One-shot conversion engine: `ONETIME_SAMPLE` START self-clear, SAR1/SAR2 DONE latch, channel-dependent 12-bit result + packed channel id, INT RAW/ST/ENA/CLR. Reused from the C3 (offset-identical one-shot surface), C6 source 60. | Register reset seeds are the **C3 silicon capture** — C6 reset values may differ (firmware writes the registers it uses). No external analog injection wiring on the C6 system, no DMA mode, no threshold/TSENS interrupts, no continuous conversion. No matrix delivery of source 60. |
| LEDC (`esp32c3_ledc`) | Four low-speed timers as live up-counters: DUTY_RES period, integer divider, PAUSE/RST, `LSTIMERx_OVF` latch, duty compare values. Reused from the C3 (the driven offsets are identical), C6 source 45. | C6-only gamma RAM, event-task, compare/capture (`TIMERx_CMP`/`CNT_CAP`), and the `EVT_TASK_*` registers are storage, not engines. No pad output/duty waveform claim (no electrical model), no interrupt-matrix delivery of source 45. |
| LP_TIMER (`esp32c6_lp_rtc`) | 48-bit free-running counter advancing one step per elapsed CPU cycle, `UPDATE` bit-28 snapshot into `MAIN_BUF0` with the previous snapshot shifted to `MAIN_BUF1`, register-backed INT/alarm/TAR surface. | No RTC-slow rate (32.768 kHz XTAL / RC_SLOW): the counter is CPU-cycle-based. `TAR0`/`TAR1` comparators are stored, never compared; no overflow/wakeup/alarm interrupt ever asserts; no sleep retention; the `UPDATE` strobe's self-clearing readback is a model choice (IDF never reads it back). |
| GDMA (`esp32c6_gdma`) | 3 channels, C6 register map (SVD), M2M descriptor walk with owner/CHECK_OWNER/OUT_AUTO_WRBACK semantics, RAW/ST/ENA/CLR interrupts, EOF-descriptor addresses. | **No peripheral-coupled pumps** (SPI2/UHCI0/I2S/AES/SHA/ADC/PARLIO): a START with a real peripheral bound stalls visibly (no EOF, `needs_bus_tick` stays true) rather than moving bytes. FIFO data ports (`IN_POP`/`OUT_PUSH`) are stubs; `*FIFO_STATUS` always reports empty. No priority arbitration, ETM, burst tuning, `IN_RST`/`OUT_RST`. Descriptor address reconstruction assumes the 20-bit LINK field prefixes `0x4080_0000` (HP SRAM) — see the model docs for the TRM inference. No fixture has routed the DMA matrix sources (66–71) through the fabric. |
| Interrupt fabric (`interrupt_core0` + `intpri`) | Matrix MAP words are real; asserted sources route through enable/priority/threshold into the RISC-V core; the software doorbells (`CPU_INTR_FROM_CPU_n`) and the UART0 peripheral source (43, via the shared twin's `matrix_irq_sources_into`) deliver real traps. | **Peripheral coverage beyond UART0 is unproven**: UART1 (44), TIMG (51–56), GDMA (66–71) source ids are declared, but no in-tree fixture drives one. C6 GPIO does not emit source 30. A scheduler-driven peripheral's routed level is re-derived at the peripheral tick (≤ one tick interval), not at its MMIO write, so a level ISR sees a re-entry window the fixture masks around. No CLIC/`CPU_INT_TYPE` edge-vs-level programming, no `CPU_INT_CLEAR` write path, no U-mode/privilege handling. |
| INTPRI register block | `CPU_INT_ENABLE`, `CPU_INT_PRI_n`, `CPU_INT_THRESH`, `CPU_INTR_FROM_CPU_n` are functional (the fixture proves level delivery + masking). | Remaining SVD registers in the window (`CPU_INT_TYPE`, `CPU_INT_CLEAR`, `CLOCK_GATE`, `DATE`) are declarative storage only. |
| USB Serial/JTAG (`esp32c6_usb_serial_jtag`) | Shared `UsbSerialJtag` behavioural model at `0x6000_F000` (irq 48): EP1 TX reaches the console tap; SOF/INT_ENA match the C3/S3 contract; `attach_usb_serial_jtag_sink` succeeds. | DevKitC-1 system yaml still defaults `debug_uart` to `uart0` (USB-UART bridge on GPIO16/17). Arduino `HWCDC` / CDC-on-boot as the *primary* console, and a hosted C6 Arduino compile/boot path, are separate gaps. |

## Not modelled

- **LP core** (RV32IMC) and the LP/LP_IO/LP_AON periphery; deep sleep, retention
  and wake-up. LP SRAM is mapped as plain memory so RTC/LP code does not fault.
- **ROM boot path / bootloader**: the 320 KB mask ROM region is optional
  (`LABWIRED_ESP32C6_ROM`) and is not the reset path of the tier-1 ELF. No
  flash-controller model, partition parsing, secure boot, or flash encryption.
- **Radios**: Wi-Fi 6 (MAC/PHY), Bluetooth 5 LE, IEEE 802.15.4, and the
  coexistence block. No MAC, no PHY, no packet path.
- **SYSTIMER, RMT, SPI0/SPI1, I2S, TWAI, PCNT, ETM, PARL_IO, TSENS,
  AES/SHA/RSA/ECC/HMAC/DS, eFuse, PMU, LP_WDT, LP_I2C0, LP_UART0, trace** — not
  declared; accesses to these windows fault loudly rather than pretending to
  work. In particular the **LP_WDT** window (0x600B_1C00) is *not* the wdt
  class: the `wdt` class is the TIMG MWDT inside `timg0`/`timg1`.
- **Watchdog resets**: the MWDT write-protect/feed/stage-chain/INT-latch
  contract is simulated, including the reset-action stage encodings, but a
  timeout never resets the CPU or the system — the simulator exposes no safe
  reset-request path from a bus peripheral (the only reset drains are the
  ESP32-classic `RTC_CNTL` block and the Cortex-M SCB, neither present on the
  C6), so the reset actions are deliberately advanced through and documented
  rather than faked. LP_WDT is absent.
- **Electrical layer**: IO_MUX drive strength/pull/function, pin muxing,
  analog, power, antenna/EMI. Do not run board-level electrical validation.
- **Cycle accuracy**: no bus arbitration, cache, flash wait states, or
  pipelining model; timing is functional, not cycle-accurate.

## Evidence boundaries

- No silicon bench diff: register/reset values come from the vendored
  `tests/fixtures/real_world/esp32c6.svd` and the ESP32-C6 TRM/datasheet,
  cross-checked against esp-idf v5.3 register headers where cited.
- The tier-1 transcript is generated by the committed fixture
  (`tests/fixtures/tier1/esp32c6.elf`, hash-pinned in `MANIFEST.json`) on the
  committed chip descriptor; see
  [`VALIDATION.md`](VALIDATION.md) for the exact commands and observed output.
- The instruction audit covers the fixture's dynamic instruction stream only;
  it is not a static proof for arbitrary firmware.

## Known follow-ups (in rough priority order)

1. Route **more peripheral sources** (e.g. I²C0 `TRANS_COMPLETE` source 50, or
   UART0 `RXFIFO_FULL`) to traps; UART0 `TX_DONE` source 43 is now proven end
   to end.
2. Implement **TIMG alarm interrupts** (`TG0_T0_LEVEL`) and MWDT interrupt
   delivery (`TG0_WDT_LEVEL`, source 53) from the shared model.
3. A safe **bus reset-request path** for peripherals (so MWDT STG=2/3 can
   actually reset the CPU/system instead of just advancing the chain).
4. C6 I²C/SPI **pad wiring**: derive the C6 matrix signal indices
   (I2CEXT0_SCL/SDA, FSPI*) instead of reusing the C3's.
5. Add **coupled GDMA pumps** (UHCI0 first — UART DMA — then SPI2/I2S) and
   cross-check the C6 PERI_SEL encoding.
6. GPIO interrupt generator (source 30) and 31-pin sizing.
7. PCR `RST_EN` enforcement and a clock-tree/frequency model.
8. C6 UART register tail (SLEEP_CONF/CLK_CONF/DATE/ID) and the I²C/LEDC/SARADC
   C6 tails.
