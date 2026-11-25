# Nuvoton NPCX9 validation status

## Current measurement (2026-11-17)

The imported NPCX9 system booted `firmware-nuvoton-npcx9-smoke`, rebooted
once through the TWD watchdog, and ran to **100,000,000 cycles**
(569,374 instructions). It passed **19/19 smoke assertions**. Firmware
SHA-256 `206c39d7c0ffe12882c0ada404a669f78240e65f7eb4ce4e153730cc25a54ba7`.
UART capture (UART1..4 share one capture stream):

```text
NPCX9 UART1 smoke
smbus reset=8 en=8 nack=8 clear=8 stop=8 ok
itim regs=1 en=1 run=1 stop=1 irq=3 sts_cleared=1 sts=1 masked=1 pend=1 rel=1 each=6 w64=1 borrow=1 lfclk=1 ok
gpio ports=16 latch=1 drive=1 pull=1 od=1 lock=1 ok
mdma src=1 data=1 dst=1 count=1 tc=1 fifo=1 ok
cr_uart2 tx
cr_uart3 tx
cr_uart4 tx
uarts tbe=3 rx=3 isolated=1 ok
uart irq fts=4 txp=4 txrel=4 frs=3 rx=3 hits=3 drained=3 ok
mtc ttc=1 pt=1 irq=3 steps=1 ms1=1000 ms2=1000 ok
twd regs=1 t0=1 irq=3 ms1=1000 ms2=1000 tc=1 lock=1 unlock=1 run=1 touch=1 dec=1 stop=1 ok
cdcg load=1 div=1 hlock=1 lf=1 llock=1 ok
mem code=1 data=1 image=1 pm=4 bb=1 ok
spip reset=1 off=1 x8=1 x16=1 cfg=1 quiet=1 rirq=1 rclr=1 wirq=1 wclr=1 ok
fiu reset=1 wel=1 prog=1 clear=1 oneway=1 erase=1 start=1 ok
twd expire .........|twd reboot wdrst=1 stopped=1 clr=1 t0=1 ok
bbram kept=1 ok
```

The result, UART capture, JUnit assertions, snapshot and build/run logs are
under
[`docs/coverage/renode-onboarding/metrics/nuvoton_npcx9/`](../coverage/renode-onboarding/metrics/nuvoton_npcx9/).
The NPCX9M6F EVB row runs the same firmware on the same chip surface
([`nuvoton_npcx9m6fb_evb.md`](nuvoton_npcx9m6fb_evb.md)).

Checks (source: `crates/firmware-nuvoton-npcx9-smoke/src/main.rs`):

0. SMBus0..7 at 0x40009000, 0x4000B000, 0x400C0000, 0x400C2000,
   0x40008000, 0x40017000, 0x40018000 and 0x40019000: all eight report their
   reset values, enable, NACK address 0x50 on the intentionally empty board
   buses, clear NEGACK with W1C, and return idle after STOP. The model unit
   tests separately verify byte writes and reads through an attached I²C
   device.
1. ITIM32_1 ITPRE and ITCNT hold while ITEN is clear.
2. ITEN starts the down-count at 15 MHz / (ITPRE + 1); clearing ITEN stops
   it, restores the load value and clears TO_STS.
3. ITCNT = 99 with TO_IE: three timeouts enter NVIC 28.
4. ITIM32_2 with NVIC 27 disabled: TO_STS without TO_IE leaves the line idle,
   TO_IE pends it, clearing TO_STS lets ICPR release it.
5. ITIM32_2..6 and ITIM64 each take one timeout IRQ on their own line.
6. ITIM64 holds a 64-bit load and borrows across the word boundary.
7. CKSEL selects the 32768 Hz LFCLK.
8. GPIO0..GPIOF (0x40081000 + n * 0x2000) each hold PDIR, PPULL, PPUD,
   PENVDD, PTYPE and PLOCK_CTL values.
9. PDOUT written before PDIR (the Zephyr `gpio_npcx.c` order) holds while
   the pin is an input and reads on PDIN once PDIR sets it as an output.
10. Pull-up / pull-down inputs read 1 / 0 on PDIN.
11. An open-drain output with pull-up reads 1 when PDOUT releases the pad
    and 0 when PDOUT drives it low.
12. A set PLOCK_CTL bit keeps its PDOUT bit.
13. UART2..4 (0x400E2000 / 0x400E4000 / 0x400E6000): the script injects
    "bB", "cC", "dD" into `cr_uart2`..`cr_uart4`. Each port reads TBE set and
    RBF set, and URBUF returns its own first byte. UART1 RBF stays clear.
14. Each of UART2..4 transmits its own `cr_uartN tx` line through its UTBUF.
15. FIFO-mode interrupts, UART1..4 on NVIC 33 / 32 / 38 / 39. UFTSTS reads
    0xD0 (NXMIP, TFIFO_EMPTY_STS, TEMPTY_LVL 16) on all four ports. With the
    NVIC line disabled, UFTCTL.TEMPTY_EN pends it; clearing TEMPTY_EN lets
    ICPR release it (level). On UART2..4, UFRSTS reads 0x61 (one byte left,
    RFULL_LVL_SEL reset value 1). UFRCTL.RNEMPTY_EN takes exactly one IRQ per
    port; the handler reads the upper-case byte from URBUF, which drops the
    level, and RBF / RFIFO_NEMPTY_STS then read clear.
16. MTC at 0x400B7000 on NVIC 5: TTC takes a written value; WTC holds a
    25-bit PT with WIE clear. With WIE set and PT = TTC + 1 the core idles in
    WFI; three IRQs arrive, each one TTC increment after the last, and the
    handler clears PTO and moves PT on. ITIM64 at 1 MHz measures both gaps
    as 1000 ms (Renode's 1 Hz tick). The core waits in WFI idle
    fast-forward.
17. TWD at 0x400D8000 on NVIC 31: reset values (TWCFG 0, TWDT0 0xFFFF,
    WDCNT 0x0F, watchdog stopped). MDIV 3 and TWDT0 4095 give a 1 s T0
    period: three IRQs, ITIM64 times both gaps at 1000 ms, and T0CSR.TC
    reads 1 once. LTWDT0 makes TWDT0 read 0 and drop a write; 87h/61h/63h
    with the watchdog stopped clears the lock. With a 32-tick T0 period,
    WDSDME and WDCNT 10, a 5Ch touch every 5 periods keeps the core running
    for 30 periods, and TWMWD reads 7 three periods after a touch.
    87h/61h/63h stops the watchdog (WD_RUN 0) and 15 periods pass. WDCNT 10
    again with no touch: the handler prints one `.` per T0 IRQ, and the
    watchdog resets the chip on the 10th period, so the capture shows exactly
    nine dots. The second boot reads WDRST_STS 1 and WD_RUN 0, clears
    WDRST_STS (W1C) and reads TWDT0 back at 0xFFFF. T0 keeps waking the idle
    core every 2 s, so the run ends on `max_cycles`, not `max_steps`.
18. HFCG at 0x400B5000 and LFCG at 0x400B5100 (runs before the watchdog
    expiry of check 17): HFCGN / HFCGML / HFCGMH, then LOAD, in Zephyr
    `clock_control_npcx.c` order; the CLK_CHNG poll ends at once, LOAD reads
    0 and M / N read back. HFCGP, HFCBCD and HFCBCD1..3 read back with the
    reserved bits clear (0x5F -> 0x53, 0xFF -> 0x32). LOCK drops a later
    HFCGML write. LFCG HFRDI keeps 12 bits (0xFFFF -> 0x0FFF),
    LFCGCTL2.XT_OSC_SL_EN reads back, and LFLOC drops a later LFCGCTL2
    write.
19. Memory windows (runs before the watchdog expiry of check 17): a word
    written at code_ram 0x10058000 reads back through 0x20058000, and a word
    written through 0x20058000 + 0x67FFC reads back at 0x10058000 + 0x67FFC;
    a word written through the data RAM view 0x100D0000 reads back at
    0x200D0000; image_type (0x4000C009) and PM1..4 (+0x00 and +0xFF) read
    back. A signature in battery-backed RAM (0x400AF000 and +0x7C) reads
    back at once, and again on the second boot after the watchdog reset
    (`bbram kept=1`).
20. SPIP (runs before the watchdog expiry of check 17; no SPI device is
    attached, so MISO reads 0): CTL1 and STAT read 0 at reset; a DATA write
    with SPIEN clear does not set RBF. With SPIEN a DATA write sets RBF (BSY
    reads 0), and the DATA read returns 0 and clears RBF, in 8-bit and 16-bit
    (MOD) mode. SCM / SCIDL / SCDV read back. With NVIC 57 enabled, EIR plus
    a transfer gives exactly one IRQ (the handler reads DATA and clears
    EIR); EIW with RBF clear gives exactly one IRQ (the handler clears EIW,
    as the Zephyr ISR does), and the line is not pending afterwards.
21. MDMA1: the smoke injects `m` into UART1, enables UART receive DMA, and
    checks the fixed URBUF source, copied byte in code RAM, destination and
    current counts, terminal-count status and empty UART FIFO.
22. FIU at 0x40020000: the smoke checks reset values, WREN / RDSR1, page
    programming, readback, the NOR 1-to-0 rule, WEL clearing, and a 64 KiB
    block erase through UMA. The internal flash array is sparse and erased by
    default.

## Claimed peripherals

| id | model | scope |
|---|---|---|
| `smbus0`..`smbus7` | `npcx_smbus` | Byte-oriented master START/address/data/STOP flow with attached I²C devices, address NACK and level IRQ predicate. The profile smoke checks reset, enable, NACK, W1C clear and STOP on all eight empty buses; attached-device transfers are covered by model unit tests. No slave mode, FIFO burst, SMBus extensions, bus timing or arbitration. |
| `mdma1`..`mdma4` | `npcx_mdma` | UART RX context 0 copies one byte from each fixed URBUF source to configured memory on receive requests; MDMA1 is exercised by the smoke. No TX context or shared UART/MDMA IRQ combiner. |
| `mdma5` | `npcx_mdma` | Context 0 register source/destination mapping only; I3C routing is not modelled. |
| `cr_uart1`..`cr_uart4` | `npcx_uart` | Polled TX (UTBUF, UICTRL TBE) and RX (URBUF, UICTRL RBF); FIFO-mode UFTSTS / UFRSTS / UFTCTL / UFRCTL; level IRQ on NVIC 33, 32, 38, 39 from TEMPTY_EN, NXMIP_EN, RNEMPTY_EN and RFULL_LVL_EN. |
| `port0`..`portf` | `npcx_gpio` | PDOUT latch, PDIN pad level (own drive, external level, pull), PDIR, PPULL/PPUD pulls, PTYPE open-drain, PLOCK_CTL PDOUT lock, PENVDD storage. |
| `itim32_1`..`itim32_6` | `npcx_itim32` | Down-counter, prescaler, APB2/LFCLK, TO_STS level IRQ on NVIC 28, 27, 17, 43, 44, 45. |
| `itim64` | `npcx_itim64` | 64-bit down-counter, IRQ on NVIC 58. |
| `mtc` | `npcx_mtc` | TTC 1 Hz count (read/write, survives system reset), WTC PT / PTO (W1C) / WIE; level IRQ WIE && PTO on NVIC 5. |
| `twd` | `npcx_twd` | T0 at 32768 Hz / 2^MDIV, TWDT0 period, TC (read clears), pulse IRQ per terminal count on NVIC 31; watchdog WDCNT / TWMWD / WDCP, WDCT0I clock select, 87h/61h/63h stop and unlock, 5Ch touch (WDSDME), bad service byte reset, lock bits, system reset with WDRST_STS. |
| `hfcg` | `npcx_hfcg` | HFCGCTRL LOAD (reads 0, latches M / N), CLK_CHNG (reads 0), LOCK (set-only, drops HFCG writes); HFCGML / HFCGMH / HFCGN / HFCGP / HFCBCD / HFCBCD1..3 / HFCGPIDL read back with reserved bits clear. Register file only. |
| `spip` | `npcx_spip` | SPIP_DATA transfer through attached devices (8-bit, or 16-bit low byte first with MOD), RBF (DATA read clears), BSY reads 0, CTL1 SPIEN / MOD / EIR / EIW / SCM / SCIDL / SCDV read back; level IRQ on NVIC 57 from EIR && RBF or EIW && !RBF (the EIW term is an assumption from the Zephyr ISR). No chip select, no timing. |
| `fiu` | `npcx_fiu` | UMA status, read, page program and 64 KiB block erase against a sparse 1 MiB internal flash medium. WEL, 3/4-byte address selection, lock and reset behavior. Direct XIP mapping is not declared. |
| `lfcg` | `npcx_lfcg` | LFCGCTL LREFEN, UDCP (reads 0), LFLOC (set-only, drops LFCG writes); HFRDI (12 bits), HFRDF, FRCDIV, DIVCOR1/2, LFCGCTL2 read back. Register file only. |

## Memory windows

| name | base | size | scope |
|---|---|---|---|
| `ram` | 0x200C0000 | 0x17800 | Renode `data_ram`. |
| `data_ram_alias` | 0x100C0000 | 0x17800 | Mirror of `ram` (Renode's second `data_ram` view). |
| `code_ram` | 0x10058000 | 0x68000 | Renode `code_ram`. |
| `code_ram_alias` | 0x20058000 | 0x68000 | Mirror of `code_ram`. |
| `battery_backed_ram` | 0x400AF000 | 0x80 | Size from Zephyr `npcx9.dtsi` `bbram` (Renode maps 0x400 and notes it is too large). Kept across a system reset. |
| `image_type` | 0x4000C009 | 1 | Renode `image_type` ArrayMemory. |
| `pm1`..`pm4` | 0x400C9000 + n * 0x2000 | 0x100 each | Renode's read/write memory mock of the PMCH registers. |

Not modelled in these windows: VBAT loss and the BBRAM status byte at
0x400AF100 (unmapped), PMCH host-interface behaviour (OBF / IBF, host IRQs),
and direct XIP reads at 0x64000000. FIU accesses use a separate sparse backing
array until the engine can share it with a mapped memory window.

GPIO ids are `port0`..`portf`, not Zephyr's `gpio0`..`gpiof`: a `gpio0` or
`gpio1` id switches on the bus's Nordic GPIOTE pass.

Sources: Renode `platforms/cpus/nuvoton_npcx9.repl`, `NPCX_FIU.cs`,
`NPCX_Flash.cs`, `NPCX_GPIO.cs`,
`NPCX_HFCG.cs`, `NPCX_ITIM.cs`, `NPCX_LFCG.cs`, `NPCX_MTC.cs`,
`NPCX_MDMA.cs`, `NPCX_SMBus.cs`, `NPCX_TWD.cs` and `NPCX_UART.cs`; Zephyr
`drivers/serial/uart_npcx.c`, `drivers/clock_control/clock_control_npcx.c`; Zephyr `soc/nuvoton/npcx/common/reg/reg_def.h`
(`cdcg_reg`, `gpio_reg`, `itim32_reg`, `itim64_reg`), `drivers/gpio/gpio_npcx.c` and
`dts/arm/nuvoton/npcx/npcx.dtsi` / `npcx9.dtsi`.

Differences from Renode `NPCX_GPIO`: Renode drops a PDOUT write to an input
pin, reads PDIN from the external level only and tags the pull and type
registers. This model keeps the PDOUT latch (Zephyr writes PDOUT before PDIR),
includes the chip's own drive in PDIN, and decodes pulls and open-drain.

Not modelled:

- UART: UICTRL ETI/ERI/EEI (non-FIFO-mode interrupts; Renode does not model
  them either), UFTSTS TEMPTY_LVL_STS and UFRSTS ERR (read 0, as in Renode),
  UFTCTL TEMPTY_LVL_EN, USTAT errors, MDMA transmit, baud and clock
  configuration. The TX FIFO is always empty: a written byte leaves at once.
- MDMA: context 1, MDMA5 I3C transfer routing, transfer widths other than the
  byte RX path, and the shared UART/MDMA IRQ combiner.
- MIWU: no GPIO wake-up or pin interrupt reaches the NVIC.
- GPIO: PENVDD drive behaviour, 1.8 V input detection, DEVALT pad muxing,
  PLOCK_CTL stickiness (the lock bits are plain read/write, as in Renode).
- ITIM: deep-sleep wake-up, ITEN sync delay, PWDWN_CTL gates, CDCG/LFCG
  clock trees.
- MTC: deep-sleep wake-up (WIE only gates the NVIC line), LFCG clock
  selection (the tick is fixed at 1 Hz).
- TWD: T0CSR TESDIS and WDLTD (Renode tags them), LTWCFG gating TWCFG
  (Renode stores the bit only), T0 wake-up through MIWU, 32-bit accesses
  beyond the 16-bit register at the address.
- HFCG / LFCG: the clock tree. CORE_CLK stays at the descriptor's 15 MHz
  `cpu_hz`, and the timers keep their rates, whatever M, N, FPRED or the APB
  dividers say. CLK_CHNG shows no settling delay. LFLER and XTCLK_VAL read 0.
  Reset values are 0 (Renode reads 0; datasheet values are not in the
  sources); a reset clearing LOCK / LFLOC is an assumption.

Difference from Renode `NPCX_HFCG` / `NPCX_LFCG`: Renode defines every field
as a tag (reads 0, drops writes). This model keeps the written values, so
Zephyr's "M / N already match" test before the reload sees what it wrote.

Differences from Renode `NPCX_TWD`, each toward the documented register
behaviour: TWCP and WDCP read back the written field (Renode returns the
divider 2^n); the watchdog decrements once per 2^WDIV input ticks (Renode's
helper takes 2^WDIV + 1); the TWDT0 reset preset is 0xFFFF (Renode
initialises its limit to 0xFFFF1, which the 16-bit field cannot hold);
WDRST_STS survives the watchdog reset it records (Renode clears it in the
machine reset); a completed 87h/61h/63h sequence returns to idle, so a second
sequence is accepted (Renode resets on the next 87h).

SPIP differences from Renode `NPCX_SPIP`: the IRQ is wired to NVIC 57
(Zephyr `npcx.dtsi` `spip0`; Renode leaves it unconnected), and EIW raises
it while RBF is clear (Renode ignores EIW). With no attached device MISO
reads 0 (Renode needs a registered device).

MDMA limits: context 0 byte receive-to-memory is modelled for MDMA1..4;
MDMA5's I3C route and context 1 remain outside this profile.

SMBus limits: this model covers master byte transfers and maps attached
LabWired I²C devices, but not slave mode, FIFO bursts, SMBus protocol
extensions, clock timing or arbitration. The onboarding system has no
off-chip I²C device attached, so its smoke intentionally verifies NACK and
recovery on all eight controllers.

Not declared (no model in this engine): the direct XIP internal flash window,
DEV_CTL4 / SWRST_CTL4 Python peripherals, and the other Renode NPCX blocks.

## Negative controls

- [`attempt-history/nuvoton_npcx9/2026-11-15-negative-control-head-descriptor/`](../coverage/renode-onboarding/attempt-history/nuvoton_npcx9/2026-11-15-negative-control-head-descriptor/):
  without the ITIM windows; BusFault at 0x400B0001 (ITPRE), FAIL 2/3.
- [`attempt-history/nuvoton_npcx9/2026-11-16-negative-control-gpio-unmapped/`](../coverage/renode-onboarding/attempt-history/nuvoton_npcx9/2026-11-16-negative-control-gpio-unmapped/):
  without the GPIO ports; BusFault at 0x40081002 (GPIO0 PDIR), FAIL 3/4.
- [`attempt-history/nuvoton_npcx9/2026-11-16-negative-control-uart2-4-unmapped/`](../coverage/renode-onboarding/attempt-history/nuvoton_npcx9/2026-11-16-negative-control-uart2-4-unmapped/):
  previous descriptor without UART2..4 and without the injections (the CLI
  rejects injections to an unknown id); BusFault at 0x400E2004 (UART2
  UICTRL), FAIL 4/8.
- [`attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-uart-injection/`](../coverage/renode-onboarding/attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-uart-injection/):
  descriptor and firmware of that round, no RX injection; `uarts tbe=3 rx=0
  drained=3 isolated=1 FAIL`, FAIL 7/8.
- [`attempt-history/nuvoton_npcx9/2026-11-16-negative-control-uart-irq-unwired/`](../coverage/renode-onboarding/attempt-history/nuvoton_npcx9/2026-11-16-negative-control-uart-irq-unwired/):
  same CLI and firmware (SHA-256 `317a070b…`), previous descriptor without
  the UART `irq` lines; `uart irq fts=4 txp=0 txrel=4 frs=3 rx=0 hits=0
  drained=0 FAIL`, FAIL 8/9. The FIFO registers read correctly; only the
  NVIC wiring is missing.
- [`attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-mtc/`](../coverage/renode-onboarding/attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-mtc/):
  same CLI and firmware (SHA-256 `85e272a6…`), previous descriptor without
  the `mtc` entry; BusFault at 0x400B7000 (TTC), FAIL 9/10.
- [`attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-twd/`](../coverage/renode-onboarding/attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-twd/):
  same CLI and firmware (SHA-256 `197d3e2f…`), previous descriptor without
  the `twd` entry; BusFault at 0x400D8006 (T0CSR read at entry), FAIL 0/12.
- [`attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-cdcg/`](../coverage/renode-onboarding/attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-cdcg/):
  same CLI and firmware (SHA-256 `54805aef…`), previous descriptor without
  the `hfcg` / `lfcg` entries; BusFault at 0x400B5006 (HFCGN write), FAIL
  10/13.
- [`attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-memory-windows/`](../coverage/renode-onboarding/attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-memory-windows/):
  same CLI and firmware (SHA-256 `4e7b49d9…`), previous descriptor without
  `memory_regions`; BusFault at 0x10058000 (code_ram write), FAIL 11/15.
- [`attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-spip/`](../coverage/renode-onboarding/attempt-history/nuvoton_npcx9/2026-11-16-negative-control-no-spip/):
  same CLI and firmware (SHA-256 `31a428ef…`), previous descriptor without
  `spip`; BusFault at 0x400D2002 (SPIP CTL1 read), FAIL 12/16.

The profile's `verified` field remains false. This result is limited to the
local model behaviour; it is not datasheet validation or hardware
confirmation.
