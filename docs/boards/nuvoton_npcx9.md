# Nuvoton NPCX9 validation status

## Current measurement (2026-11-16)

The imported NPCX9 system booted `firmware-nuvoton-npcx9-smoke` for
**400,000 instructions** and passed **8/8 smoke assertions**. Firmware
SHA-256 `bb0011248b792c6fa60b1b51fdd9ccf16167ce9c47b1c16219193c785a8d7ab8`.
UART capture (UART1..4 share one capture stream):

```text
NPCX9 UART1 smoke
itim regs=1 en=1 run=1 stop=1 irq=3 sts_cleared=1 sts=1 masked=1 pend=1 rel=1 each=6 w64=1 borrow=1 lfclk=1 ok
gpio ports=16 latch=1 drive=1 pull=1 od=1 lock=1 ok
cr_uart2 tx
cr_uart3 tx
cr_uart4 tx
uarts tbe=3 rx=3 drained=3 isolated=1 ok
```

The result, UART capture, JUnit assertions, snapshot and build/run logs are
under
[`docs/coverage/renode-onboarding/metrics/nuvoton_npcx9/`](../coverage/renode-onboarding/metrics/nuvoton_npcx9/).
The NPCX9M6F EVB row runs the same firmware on the same chip surface
([`nuvoton_npcx9m6fb_evb.md`](nuvoton_npcx9m6fb_evb.md)).

Checks (source: `crates/firmware-nuvoton-npcx9-smoke/src/main.rs`):

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
    'b', 'c', 'd' into `cr_uart2`..`cr_uart4`. Each port reads TBE set and
    RBF set, URBUF returns its own byte, and the read clears RBF. UART1 RBF
    stays clear.
14. Each of UART2..4 transmits its own `cr_uartN tx` line through its UTBUF.

## Claimed peripherals

| id | model | scope |
|---|---|---|
| `cr_uart1` | `npcx_uart` | TX (UTBUF, UICTRL TBE). |
| `cr_uart2`..`cr_uart4` | `npcx_uart` | Polled TX (UTBUF, UICTRL TBE) and RX (URBUF, UICTRL RBF). No IRQ. |
| `port0`..`portf` | `npcx_gpio` | PDOUT latch, PDIN pad level (own drive, external level, pull), PDIR, PPULL/PPUD pulls, PTYPE open-drain, PLOCK_CTL PDOUT lock, PENVDD storage. |
| `itim32_1`..`itim32_6` | `npcx_itim32` | Down-counter, prescaler, APB2/LFCLK, TO_STS level IRQ on NVIC 28, 27, 17, 43, 44, 45. |
| `itim64` | `npcx_itim64` | 64-bit down-counter, IRQ on NVIC 58. |

GPIO ids are `port0`..`portf`, not Zephyr's `gpio0`..`gpiof`: a `gpio0` or
`gpio1` id switches on the bus's Nordic GPIOTE pass.

Sources: Renode `platforms/cpus/nuvoton_npcx9.repl`, `NPCX_GPIO.cs` and
`NPCX_ITIM.cs`; Zephyr `soc/nuvoton/npcx/common/reg/reg_def.h`
(`gpio_reg`, `itim32_reg`, `itim64_reg`), `drivers/gpio/gpio_npcx.c` and
`dts/arm/nuvoton/npcx/npcx.dtsi` / `npcx9.dtsi`.

Differences from Renode `NPCX_GPIO`: Renode drops a PDOUT write to an input
pin, reads PDIN from the external level only and tags the pull and type
registers. This model keeps the PDOUT latch (Zephyr writes PDOUT before PDIR),
includes the chip's own drive in PDIN, and decodes pulls and open-drain.

Not modelled:

- UART: UICTRL ETI/ERI interrupts (NVIC 33, 32, 38, 39 are not wired),
  USTAT errors, FIFO mode, MDMA receive, baud and clock configuration.
- MIWU: no GPIO wake-up or pin interrupt reaches the NVIC.
- GPIO: PENVDD drive behaviour, 1.8 V input detection, DEVALT pad muxing,
  PLOCK_CTL stickiness (the lock bits are plain read/write, as in Renode).
- ITIM: deep-sleep wake-up, ITEN sync delay, PWDWN_CTL gates, CDCG/LFCG
  clock trees.

Not declared (no model in this engine): TWD, MTC, MDMA, SPIP,
HFCG, LFCG, FIU, SMBus, and the other Renode NPCX blocks.

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
  current descriptor, no RX injection; `uarts tbe=3 rx=0 drained=3
  isolated=1 FAIL`, FAIL 7/8.

The profile's `verified` field remains false. This result is limited to the
local model behaviour; it is not datasheet validation or hardware
confirmation.
